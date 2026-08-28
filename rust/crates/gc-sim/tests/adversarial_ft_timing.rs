//! Adversarial tests for the grounded first-touch shot (#623) at the SIM
//! INPUT BOUNDARY: exactly what `aerial_strike: Option<bool>` looks like on
//! the exact tick collection would otherwise fire, and the ticks around it.
//!
//! The TS slot layer sits a 15-frame tap buffer in front of this, but the
//! sim itself only ever sees a per-tick `Some(true)`/`Some(false)`, and
//! `resolve_first_touch_shot` is invoked from exactly one place — the
//! collection block, on the exact tick the ball enters reach — so the
//! contract this file pins is: what the sim does with a same-tick flip, a
//! pathological every-other-tick flicker, an off-topic modifier bit held
//! alongside strike, an aim pointed at the striker's own goal, and a stale
//! cooldown left behind by an earlier swing.
//!
//! Every assertion is aftermath-shaped per the house rule: past the event
//! tick, the same held input is kept and the following ticks are inspected
//! for ball position/velocity/ownership/events, not just an event flag at
//! one instant.

use gc_core::vec2::Vec2;
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;
const RECEIVER_POS: Vec2 = Vec2 { x: 700.0, y: 200.0 };

// ---------------------------------------------------------------------
// House patterns, duplicated verbatim in shape from
// `rust/crates/gc-sim/tests/first_touch.rs` (each test file is its own
// compiled binary; there is no shared `tests/common` module in this crate).
// `stage_incoming` generalizes `stage_arrival` with a configurable offset
// so the timing cases below get a real multi-tick flight to flip input on,
// instead of the 10px "already in reach on tick one" fixture.
// ---------------------------------------------------------------------

fn new_match_seeded(seed: f64, human_controlled: Option<bool>) -> MatchState {
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

/// Stage an incoming pass at `receiver` (one-based), `offset` px east of
/// `receiver_pos`, travelling west at `ball_speed`, with everyone else
/// parked far away. Generalizes `first_touch.rs`'s `stage_arrival` (fixed
/// `offset = 10.0`, i.e. already in reach on the very first tick) so the
/// timing cases below can give the ball a real multi-tick flight to flip
/// input during.
fn stage_incoming(
    s: &mut MatchState,
    receiver: i64,
    receiver_pos: Vec2,
    offset: f64,
    ball_speed: f64,
) {
    for (i, p) in s.players.iter_mut().enumerate() {
        let row = 40.0 + 30.0 * i as f64;
        p.pos = if p.team == Team::Home {
            Vec2::new(90.0, row)
        } else {
            Vec2::new(480.0, row) // far midfield: no pressure on the receiver
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
    s.ball = receiver_pos.add(Vec2::new(offset, 0.0));
    s.ball_vel = Vec2::new(-ball_speed, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    s.ball_spin = 0.0;
}

fn stage_arrival(s: &mut MatchState, receiver: i64, receiver_pos: Vec2, ball_speed: f64) {
    stage_incoming(s, receiver, receiver_pos, 10.0, ball_speed);
}

/// The exact shape a held space bar produces: jockey AND aerial_strike
/// together, plus the held aim.
fn strike_input(aim: Vec2) -> MatchInput {
    MatchInput {
        r#move: aim,
        jockey: true,
        aerial_strike: Some(true),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

/// A fully released input: strike explicitly `Some(false)`, matching what a
/// slot layer sends on an actual button-up rather than an unset field.
fn released_input() -> MatchInput {
    MatchInput {
        aerial_strike: Some(false),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

fn first_touch_event(s: &MatchState) -> Option<&gc_sim::match_snapshot::MatchEvent> {
    s.events
        .iter()
        .find(|e| e.kind == MatchEventKind::FirstTouchShot)
}

/// Run a fresh staged arrival holding strike RELEASED the entire way,
/// returning the 1-based tick on which the ball is trapped — i.e. the exact
/// tick `resolve_first_touch_shot` would instead have been invoked on, had
/// strike been held. The outer collection loop only reaches the resolver
/// once the ball is already within `POSSESS_DIST` of the designated
/// receiver (see `match.rs`'s collection block), so no branch before that
/// tick depends on the held input at all — the ball's trajectory up to and
/// including this tick is identical no matter what a caller holds. That is
/// exactly what lets every case below aim a single input flip at a tick
/// measured independently of what it then holds.
fn find_collection_tick(seed: f64, receiver_pos: Vec2, offset: f64, ball_speed: f64) -> usize {
    let mut s = new_match_seeded(seed, None);
    let tune = Tuning::new();
    let receiver = home_outfielder(&s);
    stage_incoming(&mut s, receiver, receiver_pos, offset, ball_speed);
    sim_match::set_controlled_player(&mut s, receiver);
    for tick in 1..=600 {
        sim_match::step(&mut s, DT, StepInput::Legacy(released_input()), None, &tune);
        assert_eq!(
            first_touch_event(&s),
            None,
            "strike was held released throughout; nothing should ever fire"
        );
        if s.owner == Some(receiver) {
            return tick;
        }
    }
    panic!("the staged pass never arrived within 10 simulated seconds");
}

// ---------------------------------------------------------------------
// 1 & 2: the exact-tick flip, from both directions.
// ---------------------------------------------------------------------

#[test]
fn strike_released_on_the_exact_collection_tick_does_not_fire() {
    // A player who let go of space a frame early (or the buffer expiring
    // right at the worst moment) must get the ordinary trap, not a shot
    // fired off a stale press. Pin the negative explicitly: this is the
    // sim-side contract the TS tap buffer is built on top of.
    let seed = 5.0;
    let offset = 220.0;
    let tune = Tuning::new();
    let speed = gc_sim::passing::speed_for(offset, &tune);
    let collect_tick = find_collection_tick(seed, RECEIVER_POS, offset, speed);
    assert!(
        collect_tick > 3,
        "fixture must give this timing case real ticks to work with (got {collect_tick})"
    );

    let mut s = new_match_seeded(seed, None);
    let receiver = home_outfielder(&s);
    stage_incoming(&mut s, receiver, RECEIVER_POS, offset, speed);
    sim_match::set_controlled_player(&mut s, receiver);
    let aim = Vec2::new(0.0, -1.0);
    for tick in 1..collect_tick {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );
        assert_eq!(
            first_touch_event(&s),
            None,
            "tick {tick} is still in flight, out of reach"
        );
        assert_eq!(s.owner, None);
    }
    sim_match::step(&mut s, DT, StepInput::Legacy(released_input()), None, &tune);
    assert_eq!(
        first_touch_event(&s),
        None,
        "strike released exactly on the collection tick must not fire the shot"
    );
    assert_eq!(
        s.owner,
        Some(receiver),
        "with strike not held at the moment of collection, the pass is trapped normally"
    );

    // Aftermath: a second's worth of doing nothing must not somehow undo
    // the trap or conjure a late shot.
    for _ in 0..30 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(MatchInput::default()),
            None,
            &tune,
        );
        assert_eq!(first_touch_event(&s), None);
        assert_eq!(
            s.owner,
            Some(receiver),
            "possession must not evaporate after a normal trap"
        );
    }
}

#[test]
fn strike_pressed_exactly_on_the_collection_tick_still_fires() {
    // The mirror case: strike was NOT held during the flight at all and is
    // pressed for the first time on the exact tick collection resolves.
    // The verb must still arm — a player who times their press perfectly
    // is not penalized for not having held it the whole flight.
    let seed = 5.0;
    let offset = 220.0;
    let tune = Tuning::new();
    let speed = gc_sim::passing::speed_for(offset, &tune);
    let collect_tick = find_collection_tick(seed, RECEIVER_POS, offset, speed);
    assert!(
        collect_tick > 3,
        "fixture must give this case real ticks (got {collect_tick})"
    );

    let mut s = new_match_seeded(seed, None);
    let receiver = home_outfielder(&s);
    stage_incoming(&mut s, receiver, RECEIVER_POS, offset, speed);
    sim_match::set_controlled_player(&mut s, receiver);
    for tick in 1..collect_tick {
        sim_match::step(&mut s, DT, StepInput::Legacy(released_input()), None, &tune);
        assert_eq!(
            first_touch_event(&s),
            None,
            "tick {tick} is still in flight"
        );
        assert_eq!(s.owner, None);
    }
    let aim = Vec2::new(0.0, -1.0);
    sim_match::step(
        &mut s,
        DT,
        StepInput::Legacy(strike_input(aim)),
        None,
        &tune,
    );
    if first_touch_event(&s).is_none() {
        panic!("strike pressed exactly on the collection tick must fire the shot");
    }
    assert_eq!(s.owner, None, "a first-touch shot never grants possession");

    // Aftermath: the ball must actually be travelling from a real strike,
    // not just carrying an event flag, and must never re-fire or revert to
    // a grant afterward.
    let start = s.ball;
    let mut fires = 1;
    for _ in 0..30 {
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );
        if first_touch_event(&s).is_some() {
            fires += 1;
        }
        assert_eq!(
            s.owner, None,
            "the shot must not resolve into a late grant either"
        );
    }
    assert_eq!(
        fires, 1,
        "a single press-on-collection must produce exactly one shot, not a re-fire"
    );
    assert!(
        s.ball.dist(start) > 30.0,
        "the ball must actually have travelled after the strike, not just flagged an event"
    );
}

// ---------------------------------------------------------------------
// 3: pathological every-other-tick autofire through the whole arrival.
// ---------------------------------------------------------------------

#[test]
fn a_flickering_strike_fires_iff_true_on_the_collection_tick_and_never_double_fires() {
    // Autofire / a failing switch can hand the sim `true, false, true,
    // false, ...` every tick for the whole flight. The contract is exactly
    // the per-tick read the collection block performs: the outcome is
    // decided ENTIRELY by whichever value lands on the collection tick
    // itself, and whichever way it goes, the attempt can only ever resolve
    // once — the ball is not there to be swung at twice.
    let seed = 5.0;
    let offset = 220.0;
    let tune = Tuning::new();
    let speed = gc_sim::passing::speed_for(offset, &tune);
    let collect_tick = find_collection_tick(seed, RECEIVER_POS, offset, speed);
    assert!(
        collect_tick > 3,
        "fixture must give this case real ticks (got {collect_tick})"
    );

    // start_true=true holds strike on odd ticks; start_true=false holds it
    // on even ticks. Pick whichever phase lands `true` on the collection
    // tick for run A, and the opposite phase (so it lands `false` there)
    // for run B.
    let true_hits_collect_tick_when_start_true = collect_tick % 2 == 1;

    let run = |start_true: bool| -> (MatchState, i64, Vec<usize>) {
        let mut s = new_match_seeded(seed, None);
        let receiver = home_outfielder(&s);
        stage_incoming(&mut s, receiver, RECEIVER_POS, offset, speed);
        sim_match::set_controlled_player(&mut s, receiver);
        let mut fired = Vec::new();
        for tick in 1..=(collect_tick + 20) {
            let held = if start_true {
                tick % 2 == 1
            } else {
                tick % 2 == 0
            };
            let input = if held {
                strike_input(Vec2::new(0.0, -1.0))
            } else {
                released_input()
            };
            sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &tune);
            if first_touch_event(&s).is_some() {
                fired.push(tick);
            }
            if s.owner == Some(receiver) {
                break; // trapped normally; nothing further can happen to it
            }
        }
        (s, receiver, fired)
    };

    // Run A: strike lands `true` on the collection tick.
    let (s_a, receiver_a, fired_a) = run(true_hits_collect_tick_when_start_true);
    assert_eq!(
        fired_a,
        vec![collect_tick],
        "with strike true on the collection tick, the attempt must fire there and nowhere else"
    );
    assert_eq!(
        s_a.owner, None,
        "a first-touch shot never grants possession"
    );

    // Run B: strike lands `false` on the collection tick (opposite phase).
    let (s_b, receiver_b, fired_b) = run(!true_hits_collect_tick_when_start_true);
    assert!(
        fired_b.is_empty(),
        "with strike false on the collection tick, the flicker before and after it must never fire"
    );
    assert_eq!(
        s_b.owner,
        Some(receiver_b),
        "false on the collection tick traps normally, whatever the flicker did earlier"
    );
    let _ = receiver_a;
}

// ---------------------------------------------------------------------
// 4: an unrelated modifier bit held alongside strike.
// ---------------------------------------------------------------------

#[test]
fn holding_the_acrobatic_modifier_on_a_grounded_pass_still_resolves_a_grounded_volley() {
    // A held space+lob ("bicycle") combo landing on a ball that is rolling
    // on the ground, not dropping out of the air, is an ordinary pad
    // accident (both face buttons under one thumb, or a buffered lob still
    // latched from the previous ball). `resolve_first_touch_shot` hardcodes
    // `AerialStyle::Volley` and never reads `aerial_acrobatic` at all — this
    // pins that the ignored bit really is inert, not merely unread by
    // accident, and that building a would-be bicycle contact against a
    // ball with no height never panics.
    for seed in 0..20_i64 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        sim_match::set_controlled_player(&mut s, receiver);
        let tune = Tuning::new();
        let input = MatchInput {
            r#move: Vec2::new(0.0, -1.0),
            jockey: true,
            aerial_strike: Some(true),
            aerial_acrobatic: Some(true),
            ..MatchInput::default()
        };
        sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &tune);
        let Some(event) = first_touch_event(&s) else {
            panic!("seed {seed}: the attempt must still fire with the acrobatic bit also held");
        };
        assert_eq!(
            event.style,
            Some(AerialStyle::Volley),
            "seed {seed}: a grounded first touch is a volley, never a bicycle, \
             regardless of the acrobatic bit"
        );
        assert_eq!(s.owner, None);

        // Aftermath: keep holding the exact same accidental combo and
        // confirm nothing degenerates over the following second.
        for _ in 0..30 {
            sim_match::step(&mut s, DT, StepInput::Legacy(input), None, &tune);
            assert!(
                !s.events
                    .iter()
                    .any(|e| e.style == Some(AerialStyle::Bicycle)),
                "seed {seed}: no bicycle-styled event may ever appear from a grounded pass"
            );
        }
        assert!(
            s.ball_vel.length().is_finite() && s.ball.x.is_finite() && s.ball.y.is_finite(),
            "seed {seed}: the ball must remain in a sane numeric state"
        );
    }
}

// ---------------------------------------------------------------------
// 5: aiming at the striker's own goal.
// ---------------------------------------------------------------------

#[test]
fn aiming_the_strike_at_the_receivers_own_goal_follows_the_aim_and_does_not_crash() {
    // Design fact, not a bug: nothing about the first-touch verb stops a
    // player from holding the stick toward their OWN goal while striking —
    // a deliberate (or panicked) backward clear that could even end up an
    // own goal. The contract under test is narrower than "is this wise":
    // the shot must faithfully follow that aim like any other aim, and the
    // sim must not choke on a target behind the striker's own back.
    let aim = Vec2::new(-1.0, 0.0); // Home's own goal sits at low x (goal_home).
    for seed in 0..40_i64 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
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
        let Some(event) = first_touch_event(&s) else {
            panic!("seed {seed}: the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let dir = s.ball_vel.normalized();
        assert!(
            dir.x * aim.x + dir.y * aim.y > 0.995,
            "seed {seed}: a clean first touch must follow the held aim even toward the \
             striker's own goal (got {dir:?})"
        );
        assert!(
            dir.x < 0.0,
            "seed {seed}: aiming backward must actually send the ball backward, not toward \
             the opponent's goal by default"
        );

        // Aftermath: the ball must keep carrying backward under its own
        // momentum, and every value along the way must stay finite — no
        // NaN or infinity from aiming "the wrong way".
        let start_x = s.ball.x;
        for _ in 0..15 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert!(
                s.ball.x.is_finite() && s.ball.y.is_finite() && s.ball_vel.x.is_finite(),
                "seed {seed}: the ball must remain in a sane numeric state"
            );
        }
        assert!(
            s.ball.x < start_x,
            "seed {seed}: the ball must actually have carried backward over the following \
             half second, not just launched that way for one tick"
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

// ---------------------------------------------------------------------
// 6: a stale cooldown left behind by an earlier swing.
// ---------------------------------------------------------------------

#[test]
fn a_stale_header_cooldown_from_an_earlier_swing_does_not_block_a_later_one() {
    // A give-and-go: the receiver's first swing leaves `header_cd` sitting
    // at the full `AERIAL_CD` (0.5s) behind, even though its own
    // `aerial_recovery` (0.22s, `RECOVERY_STAND`) — the animation actually
    // in progress — has already run out. `resolve_first_touch_shot`'s own
    // doc comment says `header_cd` is deliberately not a gate here; this
    // pins that a SECOND real attempt, moments later once the designation
    // is freshly re-armed by a teammate's return ball, actually gets to
    // fire instead of silently degrading into a forced trap.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40_i64 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
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
        let Some(event) = first_touch_event(&s) else {
            panic!("seed {seed}: the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        {
            let rp = &mut s.players[(receiver - 1) as usize];
            assert!(
                rp.header_cd > 0.0,
                "seed {seed}: sanity — the first swing must leave a live header_cd behind"
            );
            // "Moments later": the swing's own recovery animation has
            // finished, but its (much longer) cooldown has not — the exact
            // stale state the doc comment calls out.
            rp.aerial_recovery = 0.0;
        }
        // The teammate immediately gives it back: a second arrival at the
        // SAME receiver, designation freshly re-armed.
        stage_incoming(&mut s, receiver, RECEIVER_POS, 10.0, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        assert!(
            s.players[(receiver - 1) as usize].header_cd > 0.0,
            "seed {seed}: sanity — the re-stage must not itself have reset header_cd, \
             or this test would not exercise the stale-cooldown case at all"
        );
        sim_match::set_controlled_player(&mut s, receiver);
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );
        if first_touch_event(&s).is_none() {
            panic!(
                "seed {seed}: a stale header_cd must not block a second grounded swing once \
                 that swing's own aerial_recovery has cleared"
            );
        }
        assert_eq!(
            s.owner, None,
            "seed {seed}: the second first-touch shot must not grant possession either"
        );

        // Aftermath: keep holding, and confirm no THIRD swing fires out of
        // nothing — there is no live designation left to fire from.
        for _ in 0..20 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert_eq!(
                first_touch_event(&s),
                None,
                "seed {seed}: no further swing should fire without a further designation"
            );
        }
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}
