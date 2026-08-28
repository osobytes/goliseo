//! Adversarial geometric and numeric boundary tests for the first-touch
//! shot (#623): corners, touchlines, the exact grab-height edge, the exact
//! POSSESS_MAX_SPEED edge, two simultaneously "designated" receivers, and
//! the exact POSSESS_DIST reach edge.
//!
//! Every test steps well past the triggering tick (holding whatever input a
//! real player would still be holding) and asserts on what a player would
//! actually observe -- ball containment, ownership, or an eventual trap --
//! never an internal flag read at one tick in isolation. House patterns
//! (`new_match_seeded`, `stage_arrival`, `strike_input`) are lifted
//! verbatim from `tests/first_touch.rs`, adapted to the real game's pitch
//! (1648x927, see `gc_sim::headless::FIELD_W`/`FIELD_H`) because geometric
//! edge cases at the 960x540 fixture scale would not be the real edges.

use gc_core::vec2::Vec2;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;

// The real game's pitch, not the 960x540 scaled fixture `first_touch.rs`
// uses for its own, non-geometric assertions.
const FIELD_W: f64 = 1648.0;
const FIELD_H: f64 = 927.0;

// Duplicated from `gc_sim::r#match`, which keeps them private:
// `POSSESS_DIST`, `POSSESS_MAX_SPEED`, and `GOAL_DEPTH` (the net box's
// depth behind each goal line, which pins the arena's hard containment
// bound `assert_ball_in_bounds` checks below).
const POSSESS_DIST: f64 = 22.0;
const POSSESS_MAX_SPEED: f64 = 350.0;
const GOAL_DEPTH: f64 = 51.0;

/// A mid-pitch point, far from any touchline or goal-line, for the tests
/// in this file that are not themselves probing a corner/touchline edge.
const RECEIVER_POS: Vec2 = Vec2 {
    x: 1200.0,
    y: 460.0,
};

fn new_match_seeded(seed: f64, human_controlled: Option<bool>) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize {
            w: FIELD_W,
            h: FIELD_H,
        },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled,
        input_ownership: None,
    })
}

/// First home outfielder, one-based.
fn home_outfielder(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no home outfielder");
}

/// A second home outfielder, distinct from `first`, one-based.
fn second_home_outfielder(s: &MatchState, first: i64) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if idx != first && p.team == Team::Home && !p.is_keeper {
            return idx;
        }
    }
    panic!("fixture has no second home outfielder");
}

/// Stage the moment collection fires: `receiver` (one-based) is the
/// designated receiver of a pass arriving at `ball_speed` from the east,
/// already inside possession reach, everyone else parked far away. Lifted
/// verbatim from `tests/first_touch.rs`.
fn stage_arrival(s: &mut MatchState, receiver: i64, receiver_pos: Vec2, ball_speed: f64) {
    for (i, p) in s.players.iter_mut().enumerate() {
        let row = 40.0 + 30.0 * i as f64;
        p.pos = if p.team == Team::Home {
            Vec2::new(90.0, row)
        } else {
            Vec2::new(FIELD_W - 90.0, row) // far from the receiver: no pressure
        };
        p.vel = Vec2::new(0.0, 0.0);
        p.run_vel = Vec2::new(0.0, 0.0);
        p.receive_timer = 0.0;
    }
    let rp = &mut s.players[(receiver - 1) as usize];
    rp.pos = receiver_pos;
    rp.receive_timer = 1.0;
    rp.facing = Vec2::new(1.0, 0.0);
    s.owner = None;
    s.kickoff_hold = 0.0;
    s.pickup_cd = 0.0;
    s.aerial_lock = 0.0;
    s.ball = receiver_pos.add(Vec2::new(10.0, 0.0));
    s.ball_vel = Vec2::new(-ball_speed, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    s.ball_spin = 0.0;
}

/// The exact shape a held space bar produces: jockey AND aerial_strike
/// together, plus the held aim. Lifted verbatim from `tests/first_touch.rs`.
fn strike_input(aim: Vec2) -> MatchInput {
    MatchInput {
        r#move: aim,
        jockey: true,
        aerial_strike: Some(true),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

fn first_touch_event(s: &MatchState) -> Option<&gc_sim::match_snapshot::MatchEvent> {
    s.events
        .iter()
        .find(|e| e.kind == MatchEventKind::FirstTouchShot)
}

/// The arena's hard containment bound. There are deliberately TWO regimes
/// here, not one, and the naive single bound (`BALL_RADIUS` in from every
/// wall, matching `ball_flight::step`'s free-flight clamp) is WRONG:
/// `match.rs`'s "Keep a possessed ball on the pitch" clamp (around line
/// 7228, its own comment: "the clamp region is the ARENA, not the pitch")
/// deliberately lets a ball still glued to a dribbler's feet sit flush
/// against the line at `y=0`/`x=0` — a player can run right up to the
/// touchline — and, in a goal mouth, all the way to the net's own back
/// (`GOAL_DEPTH`, no ball-radius margin subtracted). Confirmed by direct
/// instrumentation: a first-touch shot fired from the corner (1600, 40)
/// and followed for 90 ticks reaches `ball.y == 0.0` on ~60% of seeds once
/// the receiver dribbles the loose ball back near the line, and holds
/// there under the exact `[0, field.h]` clamp — never below it. So the
/// bound asserted here is the UNION of both regimes, which is the only
/// one that is actually invariant over the whole match: `x` in
/// `[-GOAL_DEPTH, field.w + GOAL_DEPTH]`, `y` in `[0, field.h]`. A ball
/// outside THIS on any tick is a real tunnel, not a legitimate
/// dribble-at-the-line position.
fn assert_ball_in_bounds(s: &MatchState, tick: usize) {
    let min_x = -GOAL_DEPTH - 0.5;
    let max_x = FIELD_W + GOAL_DEPTH + 0.5;
    let min_y = -0.5;
    let max_y = FIELD_H + 0.5;
    assert!(
        s.ball.x >= min_x && s.ball.x <= max_x,
        "tick {tick}: ball tunneled past the goal-line bound on x (x={:.2}, z={:.2})",
        s.ball.x,
        s.ball_z
    );
    assert!(
        s.ball.y >= min_y && s.ball.y <= max_y,
        "tick {tick}: ball tunneled past the touchline bound on y (y={:.2})",
        s.ball.y
    );
}

// ---------------------------------------------------------------------
// 1. Corners and touchlines: the shot fires at the arena's own edges and
//    must never let the ball tunnel out.
// ---------------------------------------------------------------------

#[test]
fn a_first_touch_aimed_into_the_corner_never_tunnels_the_arena_bounds() {
    // Receiver arrives right in the away corner, ball just off their toe,
    // aimed straight INTO the corner -- the direction most likely to
    // expose an off-by-one in the touchline/back-wall clamp, since it
    // drives the ball at both walls near-simultaneously.
    let corner = Vec2::new(1600.0, 40.0);
    let aim = Vec2::new(1.0, -1.0).normalized(); // toward the (field.w, 0) corner
    let mut s = new_match_seeded(4.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, corner, 250.0);
    s.players[(receiver - 1) as usize].volley_skill = 1.0;
    sim_match::set_controlled_player(&mut s, receiver);
    let tune = Tuning::new();

    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(strike_input(aim)),
        None,
        &tune,
    );
    assert!(
        first_touch_event(&s).is_some(),
        "the staged arrival in the corner must still produce the attempt"
    );
    assert_ball_in_bounds(&s, 0);

    // Aftermath: the same held aim for a full second, watching every tick.
    for tick in 1..=60 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );
        assert_ball_in_bounds(&s, tick);
    }
}

#[test]
fn a_first_touch_on_the_touchline_aimed_along_it_never_tunnels_the_arena_bounds() {
    // Receiver sits ON the touchline (y=12, just inside the y=6 hard
    // clamp), ball arriving along the line from the east, shot aimed
    // straight into the touchline itself.
    let on_the_line = Vec2::new(824.0, 12.0);
    let aim = Vec2::new(0.0, -1.0);
    let mut s = new_match_seeded(6.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, on_the_line, 250.0);
    s.players[(receiver - 1) as usize].volley_skill = 1.0;
    sim_match::set_controlled_player(&mut s, receiver);
    let tune = Tuning::new();

    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(strike_input(aim)),
        None,
        &tune,
    );
    // Either the shot fires (Clean/Heavy/Miss) or a plain trap took it --
    // both are legal outcomes for this geometry; what must never happen is
    // the ball leaving the arena.
    assert_ball_in_bounds(&s, 0);
    for tick in 1..=60 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );
        assert_ball_in_bounds(&s, tick);
    }
}

// ---------------------------------------------------------------------
// 2. The grab-height edge and the gap above it.
// ---------------------------------------------------------------------

#[test]
fn a_ball_at_13_9_units_high_still_resolves_the_grounded_first_touch() {
    // Just inside the grounded first-touch band: `resolve_first_touch_shot`
    // reuses GROUND_GRAB_HEIGHT (14) as its `ft_config.max_z`
    // (gc-sim/src/aerial.rs). A ball settling here with a whisper of
    // downward velocity, as a dinked pass would, must still produce SOME
    // resolution on the exact tick collection would otherwise grant plain
    // possession.
    let mut s = new_match_seeded(2.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.ball_z = 13.9;
    s.ball_vz = -2.0;
    s.players[(receiver - 1) as usize].volley_skill = 1.0;
    sim_match::set_controlled_player(&mut s, receiver);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
        None,
        &Tuning::new(),
    );
    assert!(
        first_touch_event(&s).is_some(),
        "z=13.9 (just inside the grab-height band) must still fire the grounded first touch"
    );
    assert_eq!(
        s.owner, None,
        "a first-touch attempt must never grant plain possession"
    );
}

#[test]
fn a_ball_dropping_through_20_units_still_resolves_an_aerial_volley() {
    // Just inside the aerial VOLLEY style's own band (min_z 18,
    // gc-sim/src/aerial.rs::VOLLEY): a dropping ball here is above the
    // grounded first touch's max_z (14), so only the AERIAL side of the
    // verb can take it -- the same "some resolution" property, exercised
    // from the other direction.
    let mut s = new_match_seeded(2.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.ball_z = 20.0;
    s.ball_vz = -50.0; // genuinely dropping, not merely resting there
    s.players[(receiver - 1) as usize].volley_skill = 1.0;
    sim_match::set_controlled_player(&mut s, receiver);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
        None,
        &Tuning::new(),
    );
    let fired = s.events.iter().any(|e| {
        matches!(
            e.kind,
            MatchEventKind::Volley | MatchEventKind::Header | MatchEventKind::Bicycle
        )
    });
    assert!(
        fired,
        "z=20, dropping, inside the volley band, must still produce an aerial strike attempt"
    );
}

#[test]
fn a_low_apex_pass_in_the_grab_height_to_volley_gap_does_not_roll_through_untouched() {
    // Between the grounded first touch's max_z (14) and the aerial
    // VOLLEY style's min_z (18) sits a 4-unit band no style covers at
    // all -- too high to trap, too low to volley (the two tests above pin
    // each side). Gravity usually carries a ball through that gap inside
    // a single tick, so it is harmless in the general case; the
    // adversarial construction is a shallow, barely-arcing pass that
    // spends several ticks right at its apex inside the gap. This stages
    // exactly that (small downward vz starting at z=17) and watches for
    // the failure a player would actually see: the ball drifting past a
    // receiver who is holding strike and designated to take it, with
    // neither path -- grounded or aerial -- ever laying a touch on it.
    let mut s = new_match_seeded(5.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 60.0); // a gentle dink, not a driven pass
    s.ball_z = 17.0;
    s.ball_vz = -3.0;
    s.players[(receiver - 1) as usize].volley_skill = 1.0;
    sim_match::set_controlled_player(&mut s, receiver);
    let tune = Tuning::new();
    let mut resolved = false;
    for tick in 0..60 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
            None,
            &tune,
        );
        let aerial_fired = s
            .events
            .iter()
            .any(|e| matches!(e.kind, MatchEventKind::Volley | MatchEventKind::Header));
        if first_touch_event(&s).is_some() || aerial_fired || s.owner == Some(receiver) {
            resolved = true;
            break;
        }
        assert!(
            s.players[(receiver - 1) as usize].receive_timer > 0.0,
            "tick {tick}: the designation must not expire before the ball ever \
             becomes touchable by either path"
        );
    }
    assert!(
        resolved,
        "the ball must eventually be touched or trapped, not roll through the \
         grab-height/volley gap forever untouched"
    );
}

// ---------------------------------------------------------------------
// 3. The POSSESS_MAX_SPEED edge: designation, not speed, must be the gate
//    for a non-designated player holding strike.
// ---------------------------------------------------------------------

#[test]
fn a_non_designated_player_holding_strike_never_first_touches_regardless_of_pass_speed() {
    // POSSESS_MAX_SPEED (350) gates normal collection for everyone EXCEPT
    // the designated receiver (match.rs's collection eligibility:
    // `p.is_keeper || p.receive_timer > 0.0 || speed < POSSESS_MAX_SPEED`).
    // The first-touch verb has its own, unconditional gate --
    // `player.receive_timer <= 0.0` returns false in
    // `resolve_first_touch_shot` before geometry is even consulted. This
    // pins that a player holding strike with NO designation (hand-zeroed
    // here, as if the pass had already been claimed by someone else) never
    // gets the shot, at speeds straddling the collection threshold on both
    // sides.
    let tune = Tuning::new();
    for ball_speed in [POSSESS_MAX_SPEED - 1.0, POSSESS_MAX_SPEED + 1.0] {
        let mut s = new_match_seeded(13.0, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, ball_speed);
        s.players[(receiver - 1) as usize].receive_timer = 0.0; // NOT designated
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        sim_match::set_controlled_player(&mut s, receiver);
        let mut trapped = false;
        for tick in 0..90 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
                None,
                &tune,
            );
            assert_eq!(
                first_touch_event(&s),
                None,
                "tick {tick} at {ball_speed} px/s: an undesignated player must never \
                 get the first-touch shot, whichever side of POSSESS_MAX_SPEED the \
                 pass speed sits on"
            );
            if s.owner == Some(receiver) {
                trapped = true;
                break;
            }
        }
        assert!(
            trapped,
            "an undesignated player holding strike at {ball_speed} px/s must still \
             eventually trap the ball by ordinary collection"
        );
    }
}

// ---------------------------------------------------------------------
// 4. Two simultaneously "designated" receivers: exactly one resolution,
//    deterministically.
// ---------------------------------------------------------------------

#[test]
fn two_simultaneously_designated_receivers_produce_exactly_one_resolution_deterministically() {
    // A designation race that should be structurally impossible after a
    // real pass (only one player is ever marked) is staged directly here
    // to stress the SELECTION rather than trust that upstream code never
    // lets it happen: two players both carry `receive_timer > 0`, both
    // sit inside collection reach of the ball, one is the controlled
    // player holding strike and the other is AI-controlled and inside
    // AI_FIRST_TOUCH_RANGE of goal (so it too "wants" the shot).
    // Collection picks the nearest eligible candidate as a single `best`
    // (match.rs) before ever calling `resolve_first_touch_shot`, so
    // exactly one attempt should fire per tick -- this pins that, AND that
    // the pick is byte-for-byte deterministic across two identical runs.
    fn run() -> MatchState {
        let mut s = new_match_seeded(21.0, None);
        let receiver_a = home_outfielder(&s); // controlled, human, holds strike
        let receiver_b = second_home_outfielder(&s, receiver_a); // AI
        // Near the away goal line so the AI receiver's own
        // `first_touch_requested` (proximity-based) also wants the shot.
        let near_goal = Vec2::new(1350.0, 463.0);
        stage_arrival(&mut s, receiver_a, near_goal, 240.0);
        {
            let rb = &mut s.players[(receiver_b - 1) as usize];
            rb.pos = near_goal.add(Vec2::new(5.0, 5.0)); // also inside reach
            rb.receive_timer = 1.0;
            rb.volley_skill = 1.0;
        }
        s.players[(receiver_a - 1) as usize].volley_skill = 1.0;
        sim_match::set_controlled_player(&mut s, receiver_a);
        let tune = Tuning::new();
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
            None,
            &tune,
        );
        s
    }

    let a = run();
    let b = run();

    let ft_events_a: Vec<_> = a
        .events
        .iter()
        .filter(|e| e.kind == MatchEventKind::FirstTouchShot)
        .collect();
    let ft_events_b: Vec<_> = b
        .events
        .iter()
        .filter(|e| e.kind == MatchEventKind::FirstTouchShot)
        .collect();
    assert_eq!(
        ft_events_a.len(),
        1,
        "exactly one first-touch resolution must fire when two receivers are both \
         eligible in the same tick (got {})",
        ft_events_a.len()
    );
    assert_eq!(
        ft_events_a.len(),
        ft_events_b.len(),
        "the number of resolutions must be identical across two identical runs"
    );
    assert_eq!(
        ft_events_a[0].player, ft_events_b[0].player,
        "the same receiver must be picked both times"
    );
    assert_eq!(
        ft_events_a[0].outcome, ft_events_b[0].outcome,
        "the resolved outcome must be identical across two identical runs"
    );
    assert_eq!(
        a.ball, b.ball,
        "the resulting ball position must be identical across two identical runs"
    );
    assert_eq!(
        a.ball_vel, b.ball_vel,
        "the resulting ball velocity must be identical across two identical runs"
    );
    assert_eq!(
        a.owner, b.owner,
        "the resulting owner must be identical across two identical runs"
    );
}

// ---------------------------------------------------------------------
// 5. The exact POSSESS_DIST reach edge: no fire out of reach, but the
//    receive assist closes the last step and traps.
// ---------------------------------------------------------------------

#[test]
fn a_ball_one_pixel_past_possess_dist_never_fires_but_the_assist_closes_it_and_traps() {
    // The ball dies (friction/rest, not staged velocity) one pixel PAST
    // POSSESS_DIST from the designated receiver's feet: 23px against the
    // 22px reach both `resolve_first_touch_shot`'s caller (collection's
    // `best` selection) and plain collection itself require. At that
    // distance the receiver cannot be `best` this tick, so the shot must
    // never fire no matter how long strike is held -- but the receive
    // assist (`move.rs`'s "Receive assist" branch, active whenever
    // `receive_timer > 0 && s.owner.is_none()`) should walk the body the
    // last pixel and let it trap normally within a fraction of a second.
    let mut s = new_match_seeded(17.0, None);
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 0.0);
    s.ball = RECEIVER_POS.add(Vec2::new(POSSESS_DIST + 1.0, 0.0));
    s.ball_vel = Vec2::new(0.0, 0.0);
    sim_match::set_controlled_player(&mut s, receiver);
    let tune = Tuning::new();
    let mut trapped = false;
    for tick in 0..30 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(MatchInput::default()),
            None,
            &tune,
        );
        assert_eq!(
            first_touch_event(&s),
            None,
            "tick {tick}: a ball resting 1px past POSSESS_DIST must never let the \
             first-touch verb fire"
        );
        if s.owner == Some(receiver) {
            trapped = true;
            break;
        }
    }
    assert!(
        trapped,
        "the receive assist must close the last 1px and trap the ball within 30 ticks"
    );
}
