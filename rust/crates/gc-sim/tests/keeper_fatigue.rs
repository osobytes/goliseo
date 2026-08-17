//! Tests for the keeper save-fatigue pool and its catch band (#490).
//!
//! The load-bearing one is
//! [`reach_verdicts_are_identical_at_a_full_pool_and_an_empty_one`]: #490's
//! first acceptance criterion is an INVARIANT, not a feature — "fatigue never
//! affects whether the keeper can reach a shot, only whether they can hold
//! it" — and an invariant with no test is a comment. A tired keeper who
//! started letting reachable balls past would read to a player as the keeper
//! becoming bad rather than the attack earning something, which is the exact
//! failure this whole design avoids.
//!
//! `attempt_save` and `resolve_pending_save` are private, so these tests drive
//! them the way every other `match.rs` behaviour test does: through the public
//! [`sim_match::step`], reading back events and player state.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{
    self, MatchEventKind, MatchInput, MatchState, PitchSize, SavePending, Team,
};
use gc_sim::tuning::Tuning;

fn new_match_seeded(seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: None,
    })
}

fn step(s: &mut MatchState, dt: f64, tune: &Tuning) {
    sim_match::step(s, dt, StepInput::Legacy(MatchInput::default()), None, tune);
}

/// Index (1-based) of the first keeper on `team`.
fn keeper_index(s: &MatchState, team: Team) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == team && p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no keeper for {team:?}");
}

/// What one shot's SAVE ATTEMPT resolved to, split into the two decisions
/// #490 insists stay independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Attempt {
    /// The REACH decision: did the keeper get to the ball at all. Fatigue
    /// must never change this.
    reached: bool,
    /// The HOLD decision: catch or parry, once reached. Fatigue owns this.
    kind: Option<SavePending>,
    /// Whether the beaten keeper got fingertips to it.
    tipped: bool,
}

/// Park every outfield player in the far corner so nothing but the keeper can
/// touch the ball, and hand the away keeper a grounded shot that crosses its
/// line `dive_dist` pixels to one side.
///
/// The shot is grounded (`z = 0`, `vz = 0`) so the on-target height test can
/// never be what varies between two runs of the same geometry.
fn setup_shot(s: &mut MatchState, dive_dist: f64) -> i64 {
    let ki = keeper_index(s, Team::Away);
    s.players[(ki - 1) as usize].pos = Vec2::new(880.0, 270.0);
    let mut slot = 0.0;
    for p in &mut s.players {
        if !p.is_keeper {
            p.pos = Vec2::new(40.0 + slot * 30.0, 40.0);
            slot += 1.0;
        }
    }
    s.owner = None;
    s.pickup_cd = 0.0;
    s.block_grace = 0.0;
    // vx = 500 px/s over the 90 px to the keeper's line is t = 0.18 s, so a
    // y-velocity of `dive_dist / 0.18` puts the crossing exactly `dive_dist`
    // off the keeper's own y.
    let vx = 500.0;
    let t = (880.0 - 790.0) / vx;
    s.ball = Vec2::new(790.0, 270.0);
    s.ball_vel = Vec2::new(vx, dive_dist / t);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    s.ball_spin = 0.0;
    ki
}

/// Run one shot geometry at one pool level and report the two decisions
/// separately.
fn attempt(dive_dist: f64, fatigue: f64, tune: &Tuning) -> Attempt {
    let mut s = new_match_seeded(7.0);
    let ki = setup_shot(&mut s, dive_dist);
    s.players[(ki - 1) as usize].keeper_fatigue = fatigue;

    let mut kind = None;
    let mut tipped = false;
    // 30 ticks is well past the 0.18 s flight: long enough for the commit,
    // the queued dive and the contact resolution, short enough that the
    // regeneration rate cannot lift an emptied pool anywhere near the catch
    // threshold (4 points/s x 0.5 s = 2 points).
    for _ in 0..30 {
        step(&mut s, 1.0 / 60.0, tune);
        if let Some(pending) = s.players[(ki - 1) as usize].save_pending {
            kind = Some(pending);
        }
        if s.events.iter().any(|e| e.kind == MatchEventKind::Tip) {
            tipped = true;
        }
        if s.events.iter().any(|e| e.kind == MatchEventKind::Catch)
            || s.events.iter().any(|e| e.kind == MatchEventKind::Parry)
        {
            break;
        }
    }
    Attempt {
        reached: kind.is_some(),
        kind,
        tipped,
    }
}

/// Dive distances as a fraction of the keeper's own reach, spanning "straight
/// at them" through "fingertips" to "nowhere near".
fn geometries(s: &MatchState) -> Vec<f64> {
    let reach = s.players[(keeper_index(s, Team::Away) - 1) as usize].reach;
    [
        0.0, 0.15, 0.3, 0.45, 0.6, 0.75, 0.9, 1.0, 1.05, 1.2, 1.5, 2.0,
    ]
    .iter()
    .map(|f| f * reach)
    .collect()
}

// ---------------------------------------------------------------------
// Acceptance criterion 1: fatigue gates HOLDING, never REACHING.
// ---------------------------------------------------------------------

#[test]
fn reach_verdicts_are_identical_at_a_full_pool_and_an_empty_one() {
    let tune = Tuning::new();
    let max = tune.value("KEEPER_FATIGUE_MAX");
    let probe = new_match_seeded(7.0);

    let mut full = Vec::new();
    let mut empty = Vec::new();
    for dive_dist in geometries(&probe) {
        full.push((dive_dist, attempt(dive_dist, max, &tune)));
        empty.push((dive_dist, attempt(dive_dist, 0.0, &tune)));
    }

    for ((d, f), (_, e)) in full.iter().zip(empty.iter()) {
        assert_eq!(
            f.reached, e.reached,
            "an empty fatigue pool changed whether the keeper REACHED a shot \
             {d:.1}px off its line: full={f:?} empty={e:?}. Fatigue must gate \
             holding only -- see this module's doc."
        );
        assert_eq!(
            f.tipped, e.tipped,
            "an empty fatigue pool changed whether a BEATEN keeper got \
             fingertips to a shot {d:.1}px off its line: full={f:?} empty={e:?}"
        );
    }

    // The invariant above is only worth anything if the pool does something
    // at all: a fatigue value nothing reads would satisfy it trivially.
    assert!(
        full.iter().any(|(_, a)| a.kind == Some(SavePending::Catch)),
        "a full pool caught nothing, so this fixture proves nothing: {full:?}"
    );
    assert!(
        empty
            .iter()
            .all(|(_, a)| a.kind != Some(SavePending::Catch)),
        "an empty pool still produced a clean catch, so the catch band is \
         not gating: {empty:?}"
    );
    // And the fixture must actually span both sides of the reach decision,
    // or "identical verdicts" would only mean "identical in one case".
    assert!(
        full.iter().any(|(_, a)| a.reached) && full.iter().any(|(_, a)| !a.reached),
        "the geometry sweep never crossed the reach boundary: {full:?}"
    );
}

// ---------------------------------------------------------------------
// The catch band's two independent gates.
// ---------------------------------------------------------------------

#[test]
fn an_emptied_pool_turns_a_catch_into_a_parry_that_leaves_the_ball_live() {
    let tune = Tuning::new();
    let mut s = new_match_seeded(7.0);
    let ki = setup_shot(&mut s, 0.0); // straight at the keeper: a certain catch
    s.players[(ki - 1) as usize].keeper_fatigue = 0.0;

    let mut resolved = false;
    for _ in 0..30 {
        step(&mut s, 1.0 / 60.0, &tune);
        if s.events.iter().any(|e| e.kind == MatchEventKind::Parry) {
            resolved = true;
            break;
        }
        assert!(
            !s.events.iter().any(|e| e.kind == MatchEventKind::Catch),
            "an empty pool must not permit a clean catch"
        );
    }
    assert!(resolved, "the save never resolved at all");

    // #490's whole point: the forced parry is a real save producing a real,
    // playable ball -- not a keeper who let it in.
    assert_ne!(
        s.owner,
        Some(ki),
        "a parry must not hand the keeper possession"
    );
    assert!(
        s.ball_vel.length() > 0.0,
        "the parried ball must be live, not dead at the keeper's feet"
    );
    assert_eq!(s.score.away, 0, "and it must not have been a goal");
}

#[test]
fn the_power_ceiling_forces_a_parry_even_at_a_full_pool() {
    // The band's second, independent half: shot pace, not fatigue. Drop the
    // ceiling under the fixture's own shot speed and the same dead-centre
    // shot that a fresh keeper catches must now be parried.
    let mut tune = Tuning::new();
    tune.set("KEEPER_CATCH_POWER_CEILING", 300.0);
    let max = tune.value("KEEPER_FATIGUE_MAX");

    let mut s = new_match_seeded(7.0);
    let ki = setup_shot(&mut s, 0.0);
    s.players[(ki - 1) as usize].keeper_fatigue = max;

    let mut parried = false;
    for _ in 0..30 {
        step(&mut s, 1.0 / 60.0, &tune);
        assert!(
            !s.events.iter().any(|e| e.kind == MatchEventKind::Catch),
            "a shot above the power ceiling must not be caught, however fresh \
             the keeper is"
        );
        if s.events.iter().any(|e| e.kind == MatchEventKind::Parry) {
            parried = true;
            break;
        }
    }
    assert!(parried, "the save never resolved at all");

    // The control: the identical shot at the shipped ceiling IS caught, so
    // the assertion above is about the ceiling and not about the geometry.
    let control = Tuning::new();
    let mut s = new_match_seeded(7.0);
    let ki = setup_shot(&mut s, 0.0);
    s.players[(ki - 1) as usize].keeper_fatigue = max;
    let mut caught = false;
    for _ in 0..30 {
        step(&mut s, 1.0 / 60.0, &control);
        if s.events.iter().any(|e| e.kind == MatchEventKind::Catch) {
            caught = true;
            break;
        }
    }
    assert!(
        caught,
        "the control shot must be catchable, or the ceiling test is vacuous"
    );
}

// ---------------------------------------------------------------------
// Regeneration is a per-SECOND rate, integrated per tick.
// ---------------------------------------------------------------------

#[test]
fn regeneration_is_a_per_second_rate_not_a_per_tick_increment() {
    // #490 is explicit that this must not be a per-tick constant, because
    // that shape silently redefines the recovery rate the moment the tick
    // rate changes. So: recover the same WALL-CLOCK second at two different
    // tick rates and require the same pool.
    let tune = Tuning::new();
    let regen = tune.value("KEEPER_FATIGUE_REGEN");
    let start = 10.0;

    let mut fine = new_match_seeded(7.0);
    let ki = keeper_index(&fine, Team::Away);
    fine.players[(ki - 1) as usize].keeper_fatigue = start;
    for _ in 0..120 {
        step(&mut fine, 1.0 / 120.0, &tune);
    }

    let mut coarse = new_match_seeded(7.0);
    coarse.players[(ki - 1) as usize].keeper_fatigue = start;
    for _ in 0..30 {
        step(&mut coarse, 1.0 / 30.0, &tune);
    }

    let a = fine.players[(ki - 1) as usize].keeper_fatigue;
    let b = coarse.players[(ki - 1) as usize].keeper_fatigue;
    assert!(
        (a - b).abs() < 1e-9,
        "one second of recovery must not depend on the tick rate: \
         120 Hz gave {a}, 30 Hz gave {b}"
    );
    assert!(
        (a - (start + regen)).abs() < 1e-9,
        "one second must recover exactly KEEPER_FATIGUE_REGEN ({regen}) points, \
         got {}",
        a - start
    );
}

#[test]
fn the_pool_never_leaves_its_registered_bounds() {
    let tune = Tuning::new();
    let max = tune.value("KEEPER_FATIGUE_MAX");
    let mut s = new_match_seeded(7.0);
    let ki = keeper_index(&s, Team::Away);

    // Regeneration cannot overfill.
    s.players[(ki - 1) as usize].keeper_fatigue = max;
    for _ in 0..300 {
        step(&mut s, 1.0 / 60.0, &tune);
    }
    assert!(s.players[(ki - 1) as usize].keeper_fatigue <= max);

    // And a match-long run of real play never drives any keeper negative:
    // an emptied pool stays empty rather than buying the keeper a debt.
    let mut s = new_match_seeded(31.0);
    for _ in 0..1800 {
        step(&mut s, 1.0 / 60.0, &tune);
        for p in &s.players {
            assert!(
                p.keeper_fatigue >= 0.0 && p.keeper_fatigue <= max,
                "{} left the pool bounds at {}",
                p.id,
                p.keeper_fatigue
            );
        }
    }
}

#[test]
fn a_keeper_starts_full_and_an_outfield_player_carries_no_pool() {
    let tune = Tuning::new();
    let s = new_match_seeded(7.0);
    let max = tune.value("KEEPER_FATIGUE_MAX");
    for p in &s.players {
        if p.is_keeper {
            assert_eq!(
                p.keeper_fatigue, max,
                "{} should start on a full pool",
                p.id
            );
        } else {
            assert_eq!(
                p.keeper_fatigue, 0.0,
                "{} is not a keeper and should carry no pool",
                p.id
            );
        }
    }
}

// ---------------------------------------------------------------------
// Ordinary snapshotted sim state.
// ---------------------------------------------------------------------

#[test]
fn the_pool_round_trips_through_a_snapshot_and_is_diffed() {
    let mut state = new_match_seeded(7.0);
    let ki = keeper_index(&state, Team::Away);
    state.players[(ki - 1) as usize].keeper_fatigue = 37.5;

    let snapshot = match_snapshot::capture(&state, None);
    let (restored, _) = match_snapshot::restore(&snapshot);
    assert_eq!(
        restored.players[(ki - 1) as usize].keeper_fatigue,
        37.5,
        "the pool must survive a capture/restore round trip"
    );

    // And a pool that silently stopped being compared is the failure mode the
    // rollback tooling exists to catch, so the difference must be REPORTED,
    // not merely survived.
    let mut moved = state.clone();
    moved.players[(ki - 1) as usize].keeper_fatigue = 12.5;
    let other = match_snapshot::capture(&moved, None);
    let difference = match_snapshot::first_difference(&snapshot, &other)
        .expect("a changed fatigue pool must produce a reported difference");
    assert!(
        difference.path.ends_with(".keeper_fatigue"),
        "the reported difference should name the field, got {}",
        difference.path
    );
}

// ---------------------------------------------------------------------
// The three costs are ordered, and every knob is registered in range.
// ---------------------------------------------------------------------

#[test]
fn catch_costs_more_than_parry_which_costs_more_than_a_deflection() {
    let tune = Tuning::new();
    let catch = tune.value("KEEPER_COST_CATCH");
    let parry = tune.value("KEEPER_COST_PARRY");
    let deflect = tune.value("KEEPER_COST_DEFLECT");
    assert!(
        catch > parry && parry > deflect,
        "#490's cost ordering is catch > parry > deflect, got \
         catch={catch} parry={parry} deflect={deflect}"
    );
}

#[test]
fn every_fatigue_knob_is_registered_with_a_real_range() {
    for id in [
        "KEEPER_FATIGUE_MAX",
        "KEEPER_FATIGUE_REGEN",
        "KEEPER_COST_CATCH",
        "KEEPER_COST_PARRY",
        "KEEPER_COST_DEFLECT",
        "KEEPER_CATCH_THRESHOLD",
        "KEEPER_CATCH_POWER_CEILING",
    ] {
        let def = gc_data::tunables::SIM_TUNABLES
            .iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        assert!(
            def.min < def.default && def.default < def.max,
            "{id} should default strictly inside its own range, not on a fence: \
             {} in [{}, {}]",
            def.default,
            def.min,
            def.max
        );
        assert!(!def.unit.is_empty(), "{id} must declare a unit");
        assert!(def.step > 0.0, "{id} must declare a nudge step");
    }
    // The catch band must be REACHABLE from a full pool, or it is decoration
    // by construction.
    let tune = Tuning::new();
    assert!(
        tune.value("KEEPER_CATCH_THRESHOLD") < tune.value("KEEPER_FATIGUE_MAX"),
        "a threshold at or above the pool size would put the keeper in the \
         catch band permanently"
    );
}
