//! Integration coverage for #531's passing seam: the gameplay AI now
//! reaches a pass or throw release only by charging `pass_intent` and
//! materializing an ordinary `MatchInput`, exactly like a human's input
//! (`gc_sim::pass_intent`, `gc_sim::r#match`'s `commit_outfield_pass_intent`
//! / `commit_keeper_pass_intent` / `ai_pass_input`).
//!
//! Each fixture below is a hand-built minimal `MatchState` (the same pattern
//! `tests/rollback_events.rs`'s `new_state`/`make_player` use) rather than
//! `sim_match::new`'s authored roster: precise control over every player's
//! position is the whole point of demonstrating a genuine divergence between
//! the AI's own target pick and the soft cone's, and an authored roster's
//! stats/positions would fight that control. This file owns no production
//! code, so it carries its own copy of the fixture helpers.
//!
//! `PASS_CHARGE_RATE` is overridden in every fixture's `Tuning` to a large
//! value, so a committed intent's charge overshoots its own target by a
//! large, predictable margin in exactly one hold tick, rather than the
//! default's few-pixel slack -- the same technique `tests/passing.rs` uses
//! (`tune.set(...)`) to isolate the property under test. The scenario is
//! still driven entirely through `sim_match::step`; nothing here calls a
//! private seam function directly.

use gc_core::vec2::Vec2;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::keeper::KeeperBehaviorState;
use gc_sim::r#match::{self as sim_match, StepInput};
use gc_sim::match_snapshot::{
    self, ByTeam, MatchEventKind, MatchPlayer, MatchState, PitchSize, Rect, Team,
};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::pass_intent::{self, PassIntentStage};
use gc_sim::possession_transition::{self, TransitionWindows};
use gc_sim::tuning::Tuning;

const FIELD_W: f64 = 960.0;
const FIELD_H: f64 = 540.0;
const DT: f64 = 1.0 / 60.0;

fn make_player(id: &str, team: Team, is_keeper: bool, x: f64, y: f64) -> MatchPlayer {
    MatchPlayer {
        id: id.to_string(),
        name: format!("{id}_name"),
        team,
        pos: Vec2::new(x, y),
        vel: Vec2::new(0.0, 0.0),
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(if team == Team::Home { 1.0 } else { -1.0 }, 0.0),
        anchor: Vec2::new(x, y),
        species_id: "human_base".to_string(),
        owned_verb: gc_data::species::SimVerb::None,
        move_speed: 180.0,
        shot_speed: 500.0,
        dribble: 0.5,
        strength: 0.5,
        first_touch: 0.5,
        header_skill: 0.5,
        volley_skill: 0.5,
        bicycle_skill: 0.5,
        scan_rate: 1.0,
        composure: 1.0,
        outfield_decision: outfield_decision::new_state(None),
        is_keeper,
        radius: 12.0,
        dash_cd: 0.0,
        dodge_cd: 0.0,
        dodge_timer: 0.0,
        dodge_dir: Vec2::new(0.0, 0.0),
        reach: if is_keeper { 30.0 } else { 0.0 },
        handling: if is_keeper { 0.5 } else { 0.0 },
        keeper_aggression: if is_keeper { 40.0 } else { 0.0 },
        keeper_anticipation: if is_keeper { 0.5 } else { 0.0 },
        keeper_state: KeeperBehaviorState::Base,
        keeper_state_timer: 0.0,
        keeper_release_state: None,
        keeper_release_motion: 0.0,
        keeper_release_kind: None,
        keeper_release_depth: 0.0,
        keeper_set: 0.0,
        dive_timer: 0.0,
        dive_dir: Vec2::new(0.0, 0.0),
        dive_delay: 0.0,
        dive_target: None,
        keeper_get_up_timer: 0.0,
        hold_timer: 0.0,
        feet_ball: false,
        slide_timer: 0.0,
        slide_dir: Vec2::new(0.0, 0.0),
        slide_vel: 0.0,
        tackle_timer: 0.0,
        tackle_cd: 0.0,
        stun_timer: 0.0,
        grab_timer: 0.0,
        throw_timer: 0.0,
        receive_timer: 0.0,
        sprint_meter: 1.0,
        sprint_dur: 3.0,
        sprinting: false,
        save_pending: None,
        save_timer: 0.0,
        save_vx: 0.0,
        save_style: None,
        save_tip_emitted: false,
        settle_timer: 0.0,
        header_cd: 0.0,
        aerial_timer: 0.0,
        aerial_style: None,
        aerial_outcome: None,
        aerial_jump: 0.0,
        aerial_recovery: 0.0,
        charge: 0.0,
        pass_charge: 0.0,
        pass_target: None,
        pass_intent: pass_intent::new_state(),
        windup_timer: 0.0,
        windup_shot: None,
        jockey_timer: 0.0,
        action: gc_sim::action_slot::new_state(),
    }
}

/// A hand-built, already-valid `MatchState`: both teams AI-driven
/// (`human_controlled: false`), non-slot-mode, mid-play (no kickoff hold).
fn base_state(players: Vec<MatchPlayer>, owner: Option<i64>, ball: Vec2) -> MatchState {
    MatchState {
        field: PitchSize {
            w: FIELD_W,
            h: FIELD_H,
        },
        goal_home: Rect {
            x: 0.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        goal_away: Rect {
            x: FIELD_W - 10.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        players,
        ball,
        ball_vel: Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner,
        controlled: 2,
        human_controlled: false,
        score: ByTeam { home: 0, away: 0 },
        time_left: 300.0,
        max_goals: 99,
        finished: false,
        pickup_cd: 0.0,
        press: ByTeam { home: 1, away: 1 },
        marking: ByTeam {
            home: MarkingConfig {
                scheme: MarkingScheme::Hybrid,
                man_marks: 1,
                standoff: 32.0,
                compactness: 0.5,
                support: 0.5,
            },
            away: MarkingConfig {
                scheme: MarkingScheme::Zonal,
                man_marks: 0,
                standoff: 40.0,
                compactness: 0.6,
                support: 0.4,
            },
        },
        marks: ByTeam {
            home: vec![None; 10],
            away: vec![None; 10],
        },
        outfield_press: ByTeam {
            home: outfield_press::new_state(),
            away: outfield_press::new_state(),
        },
        transition_windows: TransitionWindows {
            home: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
            away: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
        },
        transition: possession_transition::new_state(),
        formation: ByTeam {
            home: "2-1-1".to_string(),
            away: "2-1-1".to_string(),
        },
        ball_spin: 0.0,
        rng: gc_core::rng::seed(531.0),
        block_grace: 0.0,
        aerial_lock: 0.0,
        kickoff_hold: 0.0,
        events: Vec::new(),
        slot_mode: false,
        input_ownership: None,
        slot_players: vec![None; 8],
        slot_for_player: vec![None; 10],
        input_tick: 0,
        unsupported_reason: None,
    }
}

/// A fixture-local `Tuning` with a large `PASS_CHARGE_RATE` override, so a
/// committed intent's first hold tick overshoots a small target charge by a
/// large, predictable margin -- see the module doc.
fn fast_charge_tuning(rate: f64) -> Tuning {
    let mut tune = Tuning::new();
    tune.set("PASS_CHARGE_RATE", rate);
    tune
}

fn step_once(s: &mut MatchState, tune: &Tuning) {
    sim_match::step(
        s,
        DT,
        StepInput::Legacy(match_snapshot::MatchInput::default()),
        None,
        tune,
    );
}

// ---------------------------------------------------------------------
// An AI outfielder holds charge and the cone picks the receiver.
// ---------------------------------------------------------------------

/// `PASS_RANGE_MAX` is driven down to its registered floor (300px), so h2 --
/// 400px from the carrier, inside `ai_pass_options`' own `40..=420` distance
/// band, and the option that scorer picks -- needs MORE range than a full
/// charge can ever deliver: `desired_pass_charge` clamps its target charge
/// to `1.0`, and even a full meter only ever resolves a 300px range. h3
/// sits 300px out (the practical ceiling), on almost the same ray (about 9.6
/// degrees off) but excluded from `ai_pass_options` by a defender standing
/// 15px from it -- far off `ai_pass_options`' own passing lane, so neither
/// pass's lane/interception read is disturbed, but well under
/// `AI_PASS_MIN_OPEN`. So `ai_pass_options`/`decide_carrier` (unchanged by
/// #531) commit to h2, and once #531's seam starts charging toward it, a
/// full meter (clamped at the 300px ceiling) leaves the resolved range 100px
/// short of h2's own 400px -- and comfortably closer to h3's actual 300px --
/// so the cone (`select_pass_target`, reached through `try_pass`) picks h3.
#[test]
fn ai_outfielder_holds_charge_and_the_cone_picks_the_receiver() {
    let mut tune = fast_charge_tuning(3.0);
    tune.set("PASS_RANGE_MAX", 300.0);

    let carrier_pos = Vec2::new(100.0, 270.0);
    let a_pos = Vec2::new(500.0, 270.0); // h2: ai_pass_options' own pick, 400px out
    let b_pos = Vec2::new(395.0, 320.0); // h3: the cone's pick once charge clamps, ~299px out

    let players = vec![
        make_player("h_keeper", Team::Home, true, 20.0, 50.0),
        make_player("h1", Team::Home, false, carrier_pos.x, carrier_pos.y),
        make_player("h2", Team::Home, false, a_pos.x, a_pos.y),
        make_player("h3", Team::Home, false, b_pos.x, b_pos.y),
        make_player("h4", Team::Home, false, 900.0, 50.0), // filler: nowhere near anything
        make_player("a_keeper", Team::Away, true, 940.0, 270.0),
        // 60px behind the carrier: close enough to shrink `context.space`
        // (the pressure term `carrier_options` weighs heavily against
        // dribbling), and behind it so `carrier_forward_space` (dribble's
        // own space read) never sees it.
        make_player("a1", Team::Away, false, 40.0, 270.0),
        // 15px from h3 -- well under `AI_PASS_MIN_OPEN` -- but 65px off the
        // h1->h2 lane (POSSESS_DIST is 22px) and past h3 along h1->h3's own
        // ray, so neither pass's lane/interception read ever sees it.
        make_player("a2", Team::Away, false, 395.0, 335.0),
        make_player("a3", Team::Away, false, 800.0, 100.0),
        make_player("a4", Team::Away, false, 800.0, 440.0),
    ];
    let carrier_idx = 2; // h_keeper=1, h1=2, h2=3, h3=4, h4=5

    let mut state = base_state(players, Some(carrier_idx), carrier_pos);
    // A near-zero move speed keeps every player from drifting off this
    // fixture's carefully placed geometry while `PASS_CHARGE_RATE`'s
    // registered ceiling stretches the charge over ~20 ticks -- ordinary
    // players' own movement during a real hold is real behaviour, not
    // something this fixture is trying to suppress; it is just orthogonal
    // to what this test checks.
    for player in &mut state.players {
        player.move_speed = 0.01;
    }

    let mut observed_holding_charge = false;
    let mut pass_event_receiver: Option<String> = None;

    for _ in 0..25 {
        step_once(&mut state, &tune);
        let carrier = &state.players[(carrier_idx - 1) as usize];
        if carrier.pass_charge > 0.12 {
            observed_holding_charge = true;
        }
        if let Some(evt) = state.events.iter().find(|e| e.kind == MatchEventKind::Pass) {
            pass_event_receiver = evt.player.clone();
            // `release_pass` sets `receive_timer` on the receiver, but the
            // event itself only names the PASSER (`player: Some(owner_id)`)
            // -- find the receiver via `receive_timer` instead.
            let receiver = state
                .players
                .iter()
                .find(|p| p.team == Team::Home && p.receive_timer > 0.0)
                .map(|p| p.id.clone());
            pass_event_receiver = receiver.or(pass_event_receiver);
            break;
        }
    }

    assert!(
        observed_holding_charge,
        "the AI must actually hold: pass_charge should exceed 0.12 on an intermediate tick, \
         not release instantly"
    );
    assert_eq!(
        pass_event_receiver.as_deref(),
        Some("h3"),
        "the cone must decide the receiver at release, not ai_pass_options' own pick (h2) -- \
         if this reads h2 the seam is laundering the AI's pick through the aim instead of \
         letting the cone re-resolve it"
    );
}

// ---------------------------------------------------------------------
// An AI keeper throw goes through select_throw_target, not
// keeper_distribute's internal pick.
// ---------------------------------------------------------------------

/// The keeper holds the ball in its hands (`feet_ball: false`). h2, 400px
/// out, is the only teammate with `KEEPER_SAFE_DIST` (60px) of clearance --
/// `commit_keeper_pass_intent`'s own tier-1 scan, unchanged by #531, picks
/// it. h3 sits 300px out, on the same ray, but a defender 28px from it
/// drops its clearance under `KEEPER_SAFE_DIST` **and** `THROW_MIN_OPEN`
/// (30px) both, hard-excluding it from `commit_keeper_pass_intent`'s scan
/// entirely (unlike the outfield case, this exclusion has no continuous
/// score to disturb: a candidate below either bound is never scored at
/// all). `PASS_RANGE_MAX` is floored at 300px, the same technique the
/// outfield case uses: h2's 400px need is more range than a full charge can
/// ever deliver, so the resolved range clamps to 300px -- exactly h3's own
/// distance -- and `select_throw_target` (reached through `keeper_throw`)
/// picks h3, not h2.
#[test]
fn ai_keeper_throw_goes_through_select_throw_target() {
    let mut tune = fast_charge_tuning(3.0);
    tune.set("PASS_RANGE_MAX", 300.0);

    let keeper_pos = Vec2::new(100.0, 270.0);
    let h2_pos = Vec2::new(500.0, 270.0); // commit_keeper_pass_intent's own pick, 400px out
    let h3_pos = Vec2::new(400.0, 270.0); // select_throw_target's pick once charge clamps, 300px out

    let players = vec![
        make_player("h_keeper", Team::Home, true, keeper_pos.x, keeper_pos.y),
        make_player("h2", Team::Home, false, h2_pos.x, h2_pos.y),
        make_player("h3", Team::Home, false, h3_pos.x, h3_pos.y),
        make_player("h4", Team::Home, false, 900.0, 50.0), // filler: nowhere near anything
        make_player("h5", Team::Home, false, 900.0, 500.0), // filler: nowhere near anything
        make_player("a_keeper", Team::Away, true, 940.0, 270.0),
        make_player("a1", Team::Away, false, 900.0, 270.0),
        // 28px from h3: under both KEEPER_SAFE_DIST (60px) and
        // THROW_MIN_OPEN (30px), hard-excluding h3 from
        // `commit_keeper_pass_intent`'s scan entirely.
        make_player("a2", Team::Away, false, 420.0, 290.0),
        make_player("a3", Team::Away, false, 850.0, 100.0),
        make_player("a4", Team::Away, false, 850.0, 440.0),
    ];
    let keeper_idx = 1; // h_keeper=1, h2=2, h3=3, h4=4, h5=5

    let mut state = base_state(players, Some(keeper_idx), keeper_pos);
    for player in &mut state.players {
        player.move_speed = 0.01;
    }

    let mut observed_holding_charge = false;
    let mut pass_event_receiver: Option<String> = None;

    for _ in 0..25 {
        step_once(&mut state, &tune);
        let keeper = &state.players[(keeper_idx - 1) as usize];
        if keeper.pass_charge > 0.12 {
            observed_holding_charge = true;
        }
        if let Some(evt) = state.events.iter().find(|e| e.kind == MatchEventKind::Pass) {
            pass_event_receiver = evt.player.clone();
            let receiver = state
                .players
                .iter()
                .find(|p| p.team == Team::Home && p.receive_timer > 0.0)
                .map(|p| p.id.clone());
            pass_event_receiver = receiver.or(pass_event_receiver);
            break;
        }
    }

    assert!(
        observed_holding_charge,
        "the AI keeper must actually hold: pass_charge should exceed 0.12 on an intermediate \
         tick, not release instantly"
    );
    assert_eq!(
        pass_event_receiver.as_deref(),
        Some("h3"),
        "select_throw_target must decide the receiver at release, not \
         commit_keeper_pass_intent's own pick (h2)"
    );
}

// ---------------------------------------------------------------------
// Losing possession mid-charge clears pass_intent and pass_charge.
// ---------------------------------------------------------------------

/// A carrier with a committed, still-charging pass intent that loses the
/// ball (simulated here by forcing possession away directly, the same
/// observable state a tackle or an interception leaves behind) must not
/// carry the stale target, threshold or accumulated charge into whatever
/// happens next. `update_ball`'s per-tick sweep resets `charge`,
/// `pass_charge` and `pass_target` for anyone who isn't the ball owner
/// (`#531`); `pass_intent` now resets alongside them.
#[test]
fn losing_possession_mid_charge_clears_pass_intent_and_pass_charge() {
    let mut tune = fast_charge_tuning(3.0);
    tune.set("PASS_RANGE_MAX", 300.0);

    let carrier_pos = Vec2::new(100.0, 270.0);
    let h2_pos = Vec2::new(500.0, 270.0);

    let players = vec![
        make_player("h_keeper", Team::Home, true, 20.0, 50.0),
        make_player("h1", Team::Home, false, carrier_pos.x, carrier_pos.y),
        make_player("h2", Team::Home, false, h2_pos.x, h2_pos.y),
        make_player("h3", Team::Home, false, 900.0, 50.0),
        make_player("h4", Team::Home, false, 900.0, 500.0),
        make_player("a_keeper", Team::Away, true, 940.0, 270.0),
        make_player("a1", Team::Away, false, 40.0, 270.0),
        make_player("a2", Team::Away, false, 800.0, 100.0),
        make_player("a3", Team::Away, false, 800.0, 440.0),
        make_player("a4", Team::Away, false, 800.0, 250.0),
    ];
    let carrier_idx = 2;

    let mut state = base_state(players, Some(carrier_idx), carrier_pos);
    for player in &mut state.players {
        player.move_speed = 0.01;
    }

    // Charge for a few ticks -- enough to confirm the intent actually
    // committed and started holding -- then rip the ball away mid-charge.
    for _ in 0..3 {
        step_once(&mut state, &tune);
    }
    let carrier = &state.players[(carrier_idx - 1) as usize];
    assert_eq!(
        carrier.pass_intent.stage,
        PassIntentStage::Charging,
        "the fixture must actually be mid-charge for this test to prove anything"
    );
    assert!(
        carrier.pass_charge > 0.0,
        "some charge must have accumulated"
    );

    // Simulate a lost ball: the same observable transition a tackle or an
    // interception leaves (`s.owner` no longer this player), without
    // depending on any particular contest mechanic actually landing one.
    state.owner = None;
    step_once(&mut state, &tune);

    let carrier = &state.players[(carrier_idx - 1) as usize];
    assert_eq!(
        carrier.pass_intent.stage,
        PassIntentStage::Idle,
        "pass_intent must reset the tick possession is lost"
    );
    assert_eq!(carrier.pass_intent.target_player, None);
    assert_eq!(carrier.pass_charge, 0.0, "pass_charge must reset too");
    assert_eq!(carrier.pass_target, None);
}

// ---------------------------------------------------------------------
// A rollback boundary landing mid-charge resimulates cleanly.
// ---------------------------------------------------------------------

/// `pass_intent` is the first multi-tick AI-retained state this codebase
/// has had to survive a rollback boundary (`combat_intent`'s own
/// materializations are within-tick edges, never a stored countdown that
/// spans a resimulation). Correctness by construction --
/// `PassIntentState` is a plain `Clone`-derived field on `MatchPlayer`,
/// `match_snapshot::restore` returns a native struct clone rather than a
/// lossy wire decode, and `pass_intent::materialize` is a pure function of
/// already-snapshotted `pass_charge`/`pass_intent` with no clock or
/// closure state -- is not the same claim as this test, which drives an
/// actual boundary: capture a snapshot mid-charge, discard the live state,
/// restore an independent copy from that exact snapshot (what a rollback
/// correction does when the boundary it lands on requires re-simulating
/// forward), and confirm the restored copy resolves the SAME pass, at the
/// same tick, exactly once, with the intent left clean afterward -- no
/// strand (an intent stuck `Charging` forever) and no duplicate (two
/// `Pass` events for one commit).
#[test]
fn a_rollback_boundary_mid_charge_resimulates_to_exactly_one_release_no_strand_no_duplicate() {
    let mut tune = fast_charge_tuning(3.0);
    tune.set("PASS_RANGE_MAX", 300.0);

    let carrier_pos = Vec2::new(100.0, 270.0);
    let h2_pos = Vec2::new(500.0, 270.0); // ai_pass_options' pick, 400px out (clamps the charge)
    let h3_pos = Vec2::new(395.0, 320.0); // the cone's actual receiver once charge clamps

    let players = vec![
        make_player("h_keeper", Team::Home, true, 20.0, 50.0),
        make_player("h1", Team::Home, false, carrier_pos.x, carrier_pos.y),
        make_player("h2", Team::Home, false, h2_pos.x, h2_pos.y),
        make_player("h3", Team::Home, false, h3_pos.x, h3_pos.y),
        make_player("h4", Team::Home, false, 900.0, 50.0),
        make_player("a_keeper", Team::Away, true, 940.0, 270.0),
        make_player("a1", Team::Away, false, 40.0, 270.0),
        make_player("a2", Team::Away, false, 395.0, 335.0),
        make_player("a3", Team::Away, false, 800.0, 100.0),
        make_player("a4", Team::Away, false, 800.0, 440.0),
    ];
    let carrier_idx = 2;

    let mut initial_state = base_state(players, Some(carrier_idx), carrier_pos);
    for player in &mut initial_state.players {
        player.move_speed = 0.01;
    }

    // The "predicted" timeline: step forward far enough to land mid-charge
    // (verified below), the same way a peer's own local prediction runs
    // ahead of an eventual authoritative correction.
    let mut predicted = initial_state;
    const MID_CHARGE_TICKS: i64 = 10;
    for _ in 0..MID_CHARGE_TICKS {
        step_once(&mut predicted, &tune);
    }
    let carrier = &predicted.players[(carrier_idx - 1) as usize];
    assert_eq!(
        carrier.pass_intent.stage,
        PassIntentStage::Charging,
        "the fixture must actually land mid-charge for this test to prove anything"
    );
    assert!(carrier.pass_charge > 0.0 && carrier.pass_charge < 1.0);
    assert!(
        !predicted
            .events
            .iter()
            .any(|e| e.kind == MatchEventKind::Pass),
        "must not have released yet"
    );

    // The rollback boundary: capture the mid-charge state, then throw the
    // predicted timeline away and restore an INDEPENDENT copy from the
    // captured snapshot -- exactly what a correction lands as, and the
    // only path `pass_intent` has to prove it survives (a resimulation
    // that instead restored the pre-charge `initial_boundary` and replayed
    // forward would only reprove determinism, not the restore path).
    let mid_charge_boundary = match_snapshot::capture(&predicted, None);
    let (mut resimulated, _) = match_snapshot::restore(&mid_charge_boundary);
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(&resimulated, None)),
        match_snapshot::hash(&mid_charge_boundary),
        "the restored copy must be bit-identical to what was captured"
    );

    // Continue both the (dropped) predicted timeline and the resimulated
    // one forward, in lockstep, from the same boundary.
    let mut predicted_pass_ticks: Vec<i64> = Vec::new();
    let mut resimulated_pass_ticks: Vec<i64> = Vec::new();
    for tick in 0..25 {
        step_once(&mut predicted, &tune);
        if predicted
            .events
            .iter()
            .any(|e| e.kind == MatchEventKind::Pass)
        {
            predicted_pass_ticks.push(tick);
        }
        step_once(&mut resimulated, &tune);
        if resimulated
            .events
            .iter()
            .any(|e| e.kind == MatchEventKind::Pass)
        {
            resimulated_pass_ticks.push(tick);
        }
    }

    assert_eq!(
        predicted_pass_ticks.len(),
        1,
        "the predicted timeline must release exactly once"
    );
    assert_eq!(
        resimulated_pass_ticks, predicted_pass_ticks,
        "resimulating from the mid-charge boundary must release at the identical tick -- no \
         strand (never releasing) and no duplicate (releasing twice)"
    );

    let final_predicted = &predicted.players[(carrier_idx - 1) as usize];
    let final_resimulated = &resimulated.players[(carrier_idx - 1) as usize];
    for (label, p) in [
        ("predicted", final_predicted),
        ("resimulated", final_resimulated),
    ] {
        assert_eq!(
            p.pass_intent.stage,
            PassIntentStage::Idle,
            "{label}: pass_intent must not be stranded mid-charge after release"
        );
        assert_eq!(p.pass_charge, 0.0, "{label}: pass_charge must reset");
    }
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(&predicted, None)),
        match_snapshot::hash(&match_snapshot::capture(&resimulated, None)),
        "the two timelines must converge bit-for-bit after the release"
    );
}

// ---------------------------------------------------------------------
// A real tackle mid-charge dispossesses the AI and releases no pass.
// ---------------------------------------------------------------------

/// #531's headline claim -- that the AI now pays the same interception
/// exposure during a hold a human always paid -- otherwise rests on code
/// inspection (`attempt_steals` never reads the owner's producer type,
/// `pass_intent`, or `pass_charge`; only the challenger's own state) plus
/// indirect corroboration from `tests/keeper_shadow_classifier.rs`'s moved
/// counts and `outfield_ai_baseline`'s moved `turnovers_per_min`. This
/// drives the property directly: a real standing-poke tackle, through the
/// same `attempt_steals` a human's own dash always goes through -- not a
/// simulated loss (`losing_possession_mid_charge_clears_pass_intent_and_pass_charge`
/// above forces `s.owner = None` by hand, which proves the reset wiring but
/// not that a tackle can actually land mid-charge in the first place).
///
/// The challenger is parked on `s.controlled` with `human_controlled: true`
/// so its single `dash` press reaches `move_human_player`'s standing-poke
/// branch -- the exact path a real player's tackle button takes -- while
/// the AI carrier, a different (away) player index, stays fully
/// AI-decided throughout.
#[test]
fn a_real_tackle_mid_charge_dispossesses_the_ai_and_releases_no_pass() {
    let mut tune = fast_charge_tuning(3.0); // default PASS_RANGE_MIN/MAX: a
    // 200px pass needs several real ticks to charge at the registered
    // rate ceiling, leaving a genuine window to tackle into.
    //
    // #489: a standing poke is no longer instant -- it charges, then
    // executes, then resolves. Driven to its declared floor here (0.0 for
    // the charge, ACTION_TACKLE_COMMIT's registered min of 0.05s for the
    // commit -- `Tuning::set` clamps into range, so 0.0 would silently
    // become 0.05 anyway) so this fixture still isolates "does a REAL
    // tackle clear pass_intent" rather than becoming a test of tackle
    // timing, which gc-sim/src/action_slot.rs and the knob-contract tests
    // already cover.
    tune.set("ACTION_TACKLE_FULL_CHARGE", 0.0);
    tune.set("ACTION_TACKLE_COMMIT", 0.05);

    let carrier_pos = Vec2::new(500.0, 270.0);
    let target_pos = Vec2::new(700.0, 270.0);
    // 30px from the carrier/ball: inside STAND_REACH (34px, species_reach
    // is 0 for the "none" verb every fixture player here uses), outside
    // both players' collision radii (12px each) so no push-apart physics
    // disturbs the fixture before the tackle is thrown.
    let challenger_pos = Vec2::new(470.0, 270.0);

    let players = vec![
        make_player("h_keeper", Team::Home, true, 20.0, 50.0),
        make_player(
            "h_challenger",
            Team::Home,
            false,
            challenger_pos.x,
            challenger_pos.y,
        ),
        make_player("h3", Team::Home, false, 900.0, 50.0),
        make_player("h4", Team::Home, false, 900.0, 150.0),
        make_player("h5", Team::Home, false, 900.0, 250.0),
        make_player("a_keeper", Team::Away, true, 940.0, 270.0),
        make_player("a_carrier", Team::Away, false, carrier_pos.x, carrier_pos.y),
        make_player("a_target", Team::Away, false, target_pos.x, target_pos.y),
        make_player("a3", Team::Away, false, 100.0, 450.0),
        make_player("a4", Team::Away, false, 100.0, 100.0),
    ];
    // h_keeper=1, h_challenger=2, h3=3, h4=4, h5=5, a_keeper=6,
    // a_carrier=7, a_target=8, a3=9, a4=10.
    let carrier_idx = 7;
    let challenger_idx = 2;

    let mut state = base_state(players, Some(carrier_idx), carrier_pos);
    state.human_controlled = true;
    state.controlled = challenger_idx;
    for player in &mut state.players {
        player.move_speed = 0.01;
    }

    // Let the AI commit and hold a genuine charge first -- the challenger
    // presses nothing yet.
    let mut observed_charging = false;
    for _ in 0..3 {
        step_once(&mut state, &tune);
        let carrier = &state.players[(carrier_idx - 1) as usize];
        if carrier.pass_intent.stage == PassIntentStage::Charging && carrier.pass_charge > 0.0 {
            observed_charging = true;
        }
    }
    assert!(
        observed_charging,
        "the fixture must actually be mid-charge before the tackle for this test to prove \
         anything"
    );
    assert!(
        !state.events.iter().any(|e| e.kind == MatchEventKind::Pass),
        "must not have released yet"
    );

    // The real tackle: a standing poke, committed on this tick through the
    // same input path a human's dash button always uses. #489: pressing
    // dash only COMMITS the charge; at the floor tuning above it releases
    // into its executing phase this same tick and takes a further
    // ACTION_TACKLE_COMMIT (0.05s, ~3 ticks at 60Hz) to resolve -- input is
    // never read again after the press (#489's "input loss does not
    // cancel"), so the follow-up ticks are neutral input on purpose.
    sim_match::step(
        &mut state,
        DT,
        StepInput::Legacy(match_snapshot::MatchInput {
            dash: true,
            ..match_snapshot::MatchInput::default()
        }),
        None,
        &tune,
    );
    for _ in 0..5 {
        step_once(&mut state, &tune);
    }

    assert_eq!(
        state.owner, None,
        "a successful poke tackle knocks the ball loose -- win_ball unconditionally \
         clears s.owner on any successful tackle, sliding or standing"
    );
    let carrier = &state.players[(carrier_idx - 1) as usize];
    assert_eq!(
        carrier.pass_intent.stage,
        PassIntentStage::Idle,
        "the dispossessed carrier's intent must reset the same tick"
    );
    assert_eq!(carrier.pass_charge, 0.0);

    // A few more neutral ticks: the tackled carrier must never reclaim the
    // ball and resolve the charge it was interrupted mid-way through.
    let mut pass_ever_fired = false;
    for _ in 0..30 {
        step_once(&mut state, &tune);
        if state.events.iter().any(|e| e.kind == MatchEventKind::Pass) {
            pass_ever_fired = true;
        }
    }
    assert!(
        !pass_ever_fired,
        "the interrupted charge must never resolve into a release"
    );
}
