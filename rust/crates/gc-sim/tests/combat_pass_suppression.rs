//! Regression test for the pass-charge latch (issue #564): a held pass
//! whose RELEASE edge lands on a tick where combat suppresses soccer input
//! because the player is mid own equipment action (windup/aim/active/
//! recovery — `combat::is_committed`) must forfeit its charge, not leave
//! `pass_charge` stuck above zero forever.
//!
//! Mechanism: `combat::prepare_inputs` zeroes `pass`/`pass_held` for the
//! whole window a player `is_committed` OR has `forced_ticks > 0`
//! (`combat::suppress_soccer_actions`), and `r#match::step` skips
//! `human_outfield_actions` entirely for a blocked owner — so if the
//! button-up tick lands inside that window, the edge that would have fired
//! (or cleared) the pass is gone for good.
//!
//! The `forced_ticks > 0` half of that condition (an opponent's hit landing
//! a stagger/knockback) is already covered: `combat::sanitize_forced_players`
//! calls `cancel_soccer_commitments` — which clears `pass_charge` — for
//! every forced player, every tick, before `prepare_inputs` even reaches the
//! suppression branch. The `is_committed` half (this player's OWN equipment
//! action, not a hit) has no such pre-clear anywhere: nothing revisits
//! `pass_charge` while possession is retained, and that is the actual gap
//! this test pins. (An earlier draft of this test used a forced/stagger
//! state instead and passed even without the fix below, for exactly this
//! reason — the forced path was never the uncovered one.)
//!
//! The fix clears `pass_charge` (and the purely presentational
//! `pass_target` preview marker, which the pass-preview-marker tests
//! establish has no memory and no simulation effect either way) the moment
//! suppression applies in `combat::prepare_inputs`, matching the existing
//! rule that a hit already cancels soccer commitments (a charged pass is
//! one) and that committing to a fresh equipment action already clears a
//! charge on the press edge (the commit branch right next to it).

use gc_core::vec2::Vec2;
use gc_sim::combat;
use gc_sim::combat_snapshot::CombatPhase;
use gc_sim::fixed_clock;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize, Team};
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

fn new_match() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut s = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: None,
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    s.kickoff_hold = 0.0;
    s
}

/// The first home outfielder (never the keeper), so passing logic and
/// `is_human_player` both apply the ordinary way.
fn first_home_outfielder(state: &MatchState) -> i64 {
    state
        .players
        .iter()
        .enumerate()
        .find(|(_, p)| p.team == Team::Home && !p.is_keeper)
        .map(|(zero_index, _)| (zero_index + 1) as i64)
        .expect("a roster always has a non-keeper home player")
}

fn hold_pass_input() -> MatchInput {
    MatchInput {
        pass_held: true,
        ..slot_input::neutral_match_input()
    }
}

fn release_pass_input() -> MatchInput {
    MatchInput {
        pass: true,
        ..slot_input::neutral_match_input()
    }
}

#[test]
fn a_committed_equipment_action_eating_the_pass_release_edge_forfeits_the_charge_instead_of_latching_it()
 {
    let mut state = new_match();
    let carrier = first_home_outfielder(&state);
    state.controlled = carrier;
    state.owner = Some(carrier);
    // Glue the ball to the carrier's feet: `update_ball`'s dribble-control
    // check strips possession the instant the ball is farther than the
    // control radius from the owner, which would silently break the
    // fixture (no charge ever builds) rather than exercising the bug.
    state.ball = state.players[(carrier - 1) as usize].pos;
    state.ball_vel = Vec2::new(0.0, 0.0);
    let mut combat_state = combat::new_state(&mut state, None);
    let tune = Tuning::new();

    // Hold the pass button for several ticks so `pass_charge` builds up
    // above zero.
    for _ in 0..10 {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(hold_pass_input()),
            Some(&mut combat_state),
            &tune,
        );
    }
    let charge_before_commit = state.players[(carrier - 1) as usize].pass_charge;
    assert!(
        charge_before_commit > 0.0,
        "setup: holding pass should have built up a charge, got {charge_before_commit}"
    );
    assert!(
        state.players[(carrier - 1) as usize].pass_target.is_some(),
        "setup: holding pass toward a teammate should preview a receiver"
    );

    // Commit the carrier to its own equipment action (no loadout press
    // needed — this drives the runtime phase directly, the same way
    // `tests/combat.rs`'s `already_committed` case does). `phase_ticks` is
    // set far beyond this test's tick budget so the action never advances
    // out of `Windup` and `finish_tick`'s phase-transition arm (which needs
    // a real `family_id`) is never reached — the point here is only the
    // suppression window `is_committed` opens, not a specific family's
    // resolution.
    combat_state.players[(carrier - 1) as usize].phase = CombatPhase::Windup;
    combat_state.players[(carrier - 1) as usize].phase_ticks = 1000;

    // Release the pass button on a tick where the player is still
    // committed: this is the edge that gets silently eaten by suppression.
    let goals_before = state.score.home + state.score.away;
    sim_match::step(
        &mut state,
        fixed_clock::TICK_SECONDS,
        StepInput::Legacy(release_pass_input()),
        Some(&mut combat_state),
        &tune,
    );

    // The charge is forfeited immediately — being mid own equipment action
    // already suppresses soccer commitments, and a charged pass is one —
    // rather than latching because the release edge never reached
    // `human_outfield_actions`.
    assert_eq!(
        state.players[(carrier - 1) as usize].pass_charge,
        0.0,
        "pass_charge must be cleared, not latched, when the release lands on a suppressed tick"
    );
    assert!(
        state.players[(carrier - 1) as usize].pass_target.is_none(),
        "the stale preview marker must clear with the charge"
    );

    // Run several more suppressed ticks: no pass ever fires (the release
    // was genuinely eaten, not merely delayed), and the charge never
    // re-latches while still committed.
    for _ in 0..20 {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(slot_input::neutral_match_input()),
            Some(&mut combat_state),
            &tune,
        );
        assert_eq!(state.players[(carrier - 1) as usize].pass_charge, 0.0);
    }
    assert_eq!(
        state.score.home + state.score.away,
        goals_before,
        "an eaten release must not fire a pass"
    );
}
