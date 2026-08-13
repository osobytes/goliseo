//! Tests for the keeper save commit ([`gc_sim::r#match`]'s private
//! `attempt_save`) consuming [`gc_sim::ball_prediction::BallPredictor`]
//! instead of a hand-rolled, gravity-only quadratic (#486, sliced from
//! #490).
//!
//! `attempt_save` is private, so these tests drive it the same way every
//! other `match.rs` behavior test does: through the public
//! [`sim_match::step`] entry point, reading back events and player state.

use gc_core::vec2::Vec2;
use gc_sim::ball_prediction::{BallPredictionConfig, BallPredictor};
use gc_sim::r#match::{self as sim_match, GRAVITY_PX, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
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

fn no_input() -> MatchInput {
    MatchInput::default()
}

fn step(s: &mut MatchState, dt: f64, tune: &Tuning) {
    sim_match::step(s, dt, StepInput::Legacy(no_input()), None, tune);
}

fn has_event(s: &MatchState, kind: MatchEventKind) -> bool {
    s.events.iter().any(|e| e.kind == kind)
}

/// Index of the first player on `team` with `is_keeper`.
fn keeper_index(s: &MatchState, team: Team) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == team && p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no keeper for {team:?}");
}

// ---------------------------------------------------------------------
// The `None` path: an unresolvable query defers, it never guesses.
// ---------------------------------------------------------------------

/// A grounded shot decaying under friction so close to the point where it
/// would die entirely (`keeper::travel_time`'s `ratio >= 0.95` cutoff) that
/// its time-to-arrival is real (`Some`) but exceeds
/// `predict.max_horizon` (2.0s) — the one gap between the two: `eta` only
/// asks "does the friction decay ever get there", `position_at_time` asks
/// "does the buffer's bounded horizon reach that far". `FRICTION` is 1.2
/// px/s (ground) and `-ln(1-ratio)/FRICTION` grows without bound as
/// `ratio -> 0.95`, so a shot placed at `ratio ~= 0.93` sits inside that
/// gap: `travel_time` returns `Some(~2.2s)`, well past the 2.0s horizon.
///
/// Everyone stays clear of the shot line so nothing but `attempt_save`
/// can touch the ball.
fn setup_borderline_slow_shot(s: &mut MatchState) -> i64 {
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
    // dxa = 100 (inside SAVE_ZONE=130), speed chosen so
    // ratio = dxa * FRICTION / speed = 100 * 1.2 / 129.03 ~= 0.930,
    // giving eta = -ln(1 - 0.930) / 1.2 ~= 2.22s (Some, but > 2.0s horizon).
    s.ball = Vec2::new(780.0, 270.0);
    s.ball_vel = Vec2::new(129.03, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    ki
}

#[test]
fn a_query_that_cannot_resolve_inside_the_horizon_defers_the_commit_instead_of_guessing() {
    let tune = Tuning::new();
    let mut s = new_match_seeded(11.0);
    let ki = setup_borderline_slow_shot(&mut s);

    step(&mut s, 1.0 / 60.0, &tune);

    // The old gravity-only quadratic never returned `None`: for a grounded
    // ball (z=0, vz=0) it is `-0.5 * GRAVITY * tz * tz`, strictly negative
    // for any tz > 0, which trivially clears both the crossbar and
    // keeper-air-grab upper bounds. So the old code committed to a verdict
    // immediately, however far away `tz` actually was. The predictor-backed
    // version must not: it has nothing authoritative to say about a instant
    // 2.2 real seconds out, so it must not have committed on this first
    // eligible tick.
    let keeper = &s.players[(ki - 1) as usize];
    assert!(
        keeper.save_pending.is_none(),
        "a query beyond the horizon must not produce a save verdict"
    );
    assert_eq!(
        keeper.dive_timer, 0.0,
        "no dive should have launched off an unresolved query"
    );
    assert_eq!(
        keeper.dive_delay, 0.0,
        "no dive should even be queued off an unresolved query"
    );
    assert!(!has_event(&s, MatchEventKind::Catch));
    assert!(!has_event(&s, MatchEventKind::Parry));
    assert_eq!(s.score.away, 0, "and certainly no goal was adjudicated");
}

#[test]
fn the_deferred_shot_still_resolves_once_it_closes_inside_the_horizon() {
    let tune = Tuning::new();
    let mut s = new_match_seeded(11.0);
    let ki = setup_borderline_slow_shot(&mut s);

    // `attempt_save` runs every live tick and recomputes `t`/`eta` from the
    // ball's CURRENT position and speed each time, not from the tick-0
    // snapshot. As the ball closes on the keeper's line under friction
    // decay, `tz` only shrinks, so the deferral above must be temporary:
    // give it the run of a full match and the shot must still be resolved
    // one way or another well before it would ever go dead or stray from
    // the fixture's tight geometry.
    let mut resolved = false;
    for _ in 0..600 {
        step(&mut s, 1.0 / 60.0, &tune);
        let keeper = &s.players[(ki - 1) as usize];
        if keeper.save_pending.is_some()
            || keeper.dive_timer > 0.0
            || keeper.dive_delay > 0.0
            || has_event(&s, MatchEventKind::Catch)
            || has_event(&s, MatchEventKind::Parry)
            || s.score.away > 0
        {
            resolved = true;
            break;
        }
    }
    assert!(
        resolved,
        "a query that starts unresolvable must still resolve once tz \
         closes inside predict.max_horizon, rather than being locked out \
         for the rest of the shot's flight"
    );
}

// ---------------------------------------------------------------------
// The real, sampled trajectory disagrees with the deleted formula.
// ---------------------------------------------------------------------

/// Direct evidence that `attempt_save`'s new contact-height query
/// ([`BallPredictor::position_at_time`]) answers with what the ball will
/// actually do — including the ground bounce — rather than with the
/// deleted `s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz` closed
/// form, in a case engineered so the two visibly disagree about whether the
/// keeper could even reach the ball.
///
/// A ball driven down at `(z0=120, vz0=-500)` — a firm downward header or
/// half-volley, not a free drop — lands within the first quarter second
/// (empirically, at the live 60Hz step's resolution: tick 13, `t ~= 0.217s`)
/// and the real step function's `BOUNCE = 0.55` restitution sends it back
/// up, cresting a second arc that carries it back through `z ~= 66.4px` on
/// the way up at `tz = 0.45s` (tick 27). 66.4 sits strictly between
/// `KEEPER_AIR_GRAB` (60) and `CROSSBAR` (70) — too high for the keeper's
/// hands, but not a chip over the bar either. (These are empirical, not
/// hand-solved from the continuous ballistic equations: the discrete
/// 60Hz step can overshoot the ground by a fraction of a tick before the
/// `z <= 0.0` clamp fires, so the exact post-bounce apex differs slightly
/// from what a continuous solve predicts — which is itself part of why a
/// hand-rolled closed form drifts from what the live simulation does.)
///
/// The deleted formula never modeled the bounce at all: fed the same
/// `(z0, vz0)` and the same `tz`, it keeps integrating straight through
/// the ground and reports a large negative height — which the on-target
/// check's upper bounds (`z_cross < CROSSBAR && z_cross <= KEEPER_AIR_GRAB`)
/// would have accepted as trivially "reachable". A keeper reading the old
/// formula would commit to (and, depending on the RNG roll, possibly
/// "catch") a ball that has actually bounced up out of its hands' reach.
#[test]
fn the_predictor_reports_the_real_post_bounce_height_where_the_old_formula_went_unboundedly_negative()
 {
    let s = new_match_seeded(1.0);
    let mut ball = s.clone();
    ball.ball = Vec2::new(480.0, 270.0);
    ball.ball_vel = Vec2::new(0.0, 0.0);
    ball.ball_z = 120.0;
    ball.ball_vz = -500.0;
    ball.owner = None;

    let tz = 0.45_f64;
    let mut predictor = BallPredictor::new(BallPredictionConfig::default());
    let sample = predictor
        .position_at_time(&ball, tz)
        .expect("well inside the 2.0s horizon");

    let old_formula = ball.ball_z + ball.ball_vz * tz - 0.5 * GRAVITY_PX * tz * tz;

    assert!(
        sample.z > 60.0 && sample.z < 70.0,
        "the real post-bounce height should sit between KEEPER_AIR_GRAB \
         and CROSSBAR, got {}",
        sample.z
    );
    assert!(
        old_formula < 0.0,
        "the deleted gravity-only quadratic should have gone negative \
         (never modeled the bounce), got {old_formula}"
    );
    assert!(
        old_formula <= 60.0,
        "the deleted formula would have wrongly cleared \
         KEEPER_AIR_GRAB (60), got {old_formula}"
    );
}
