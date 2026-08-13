//! Tests for `gc_sim::locomotion` — the per-tick locomotion primitive.
//!
//! Four of these are the issue's acceptance criteria stated as assertions, and
//! each one is written to hold *structurally* rather than at the shipped
//! defaults, because every value under test is a registered knob a sweep may
//! move tomorrow:
//!
//! - [`a_reversal_decelerates_through_zero_and_never_flips_sign`] walks the
//!   velocity tick by tick rather than checking the endpoints, because "ends
//!   up going the other way" is true of the instant flip this change exists to
//!   remove.
//! - [`facing_converges_without_overshoot_at_every_tunable_extreme`] runs the
//!   corners of the facing knobs' declared ranges, not just their defaults.
//!   An easing rule that oscillates only at `max` is an easing rule that
//!   oscillates.
//! - [`strafe_and_backpedal_move_off_the_facing_line`] and
//!   [`carry_contexts_differ_from_their_empty_handed_counterparts`] pin the
//!   two claims the context table would otherwise make silently.

use gc_core::vec2::Vec2;
use gc_data::locomotion::{CONTEXT_KNOBS, LocoContext};
use gc_data::tunables::{SIM_TUNABLES, Tier};
use gc_sim::locomotion::{self, Command, FacingIntent, Kinematics};
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;
/// A mid-table outfielder: pace 7 (`stats::move_speed` → 200 px/s), and the
/// 0..1 strength/dribble scalars a `MatchPlayer` retains.
const BASE_SPEED: f64 = 200.0;
const STRENGTH: f64 = 0.6;
const DRIBBLE: f64 = 0.65;

fn tune() -> Tuning {
    Tuning::new()
}

fn profile_for(ctx: LocoContext, tune: &Tuning) -> locomotion::Profile {
    let stats = locomotion::stats(BASE_SPEED, STRENGTH, DRIBBLE, tune);
    locomotion::profile(ctx, BASE_SPEED, stats, tune)
}

fn knob(id: &str) -> &'static gc_data::tunables::TunableDef {
    SIM_TUNABLES
        .iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("{id} must be registered"))
}

fn signed_angle(from: Vec2, to: Vec2) -> f64 {
    let from = from.normalized();
    let to = to.normalized();
    (from.x * to.y - from.y * to.x).atan2(from.x * to.x + from.y * to.y)
}

fn moving(vel: Vec2, facing: FacingIntent) -> Command {
    Command {
        vel,
        carrying: false,
        sprinting: false,
        facing,
    }
}

// ---------------------------------------------------------------------------
// Reversal
// ---------------------------------------------------------------------------

#[test]
fn a_reversal_decelerates_through_zero_and_never_flips_sign() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    // At pace, running right, commanded hard left.
    let mut k = Kinematics {
        run_vel: Vec2::new(BASE_SPEED, 0.0),
        facing: Vec2::new(1.0, 0.0),
    };
    let cmd = moving(Vec2::new(-BASE_SPEED, 0.0), FacingIntent::Movement);

    let mut previous_speed = k.run_vel.length();
    let mut ticks_to_rest = None;
    for tick in 0..600 {
        let next = locomotion::step(k, &cmd, &profile, DT, &tune);
        let speed = next.run_vel.length();
        if ticks_to_rest.is_none() {
            assert!(
                speed <= previous_speed,
                "tick {tick}: speed rose to {speed} from {previous_speed} while still reversing \
                 -- the brake phase must be monotonic"
            );
            assert!(
                next.run_vel.x >= 0.0,
                "tick {tick}: velocity x went to {} while still travelling right -- a reversal \
                 must pass THROUGH zero, never across it",
                next.run_vel.x
            );
            if speed == 0.0 {
                ticks_to_rest = Some(tick);
            }
        }
        previous_speed = speed;
        k = next;
    }

    let rest = ticks_to_rest.expect("the body must come to rest before committing to the reverse");
    assert!(
        rest > 0,
        "coming to rest on the first tick would be an instant stop, not a deceleration"
    );
    assert!(
        k.run_vel.x < 0.0,
        "after the pivot the body must actually be travelling the commanded way, got {:?}",
        k.run_vel
    );

    // The brake really is the DECEL rate and not the accel rate: separate
    // rates is the whole point, so assert the two are distinguishable here.
    let expected_rest = (BASE_SPEED / profile.decel / DT).ceil() as i32;
    assert!(
        (rest - expected_rest).abs() <= 1,
        "braking took {rest} ticks; the context's decel rate predicts {expected_rest}"
    );
}

#[test]
fn a_reversal_is_slower_than_the_same_turn_from_rest() {
    // The player-facing claim: committing to a direction costs something.
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    let cmd = moving(Vec2::new(-BASE_SPEED, 0.0), FacingIntent::Movement);

    let travel = |start: Vec2| {
        let mut k = Kinematics {
            run_vel: start,
            facing: Vec2::new(1.0, 0.0),
        };
        let mut x = 0.0;
        for _ in 0..90 {
            k = locomotion::step(k, &cmd, &profile, DT, &tune);
            x += k.run_vel.x * DT;
        }
        x
    };

    let from_pace = travel(Vec2::new(BASE_SPEED, 0.0));
    let from_rest = travel(Vec2::new(0.0, 0.0));
    assert!(
        from_rest < from_pace,
        "reversing at pace must cover LESS ground leftward than starting from rest: \
         at pace {from_pace}, from rest {from_rest}"
    );
}

// ---------------------------------------------------------------------------
// Facing
// ---------------------------------------------------------------------------

#[test]
fn facing_converges_without_overshoot_at_every_tunable_extreme() {
    // Every knob that touches the facing ease, at both ends of its declared
    // range, crossed with the worst-case turn (a full 180 degrees) and with
    // one that starts inside the ease-out window.
    let facing_knobs = [
        "LOCO_BASE_TURN",
        "LOCO_FACE_TURN_MULT",
        "LOCO_TURN_EASE_DEG",
        "LOCO_FACE_EASE_FLOOR",
        "LOCO_RUN_TURN_MULT",
        "LOCO_TECHNIQUE_CURVE_LO",
        "LOCO_TECHNIQUE_CURVE_HI",
    ];
    let starts = [180.0_f64, 90.0, 19.0, 1.0, -180.0, -19.0];

    for id in facing_knobs {
        let def = knob(id);
        for value in [def.min, def.max] {
            let mut tune = tune();
            tune.set(id, value);
            assert_eq!(
                tune.value(id),
                value,
                "{id} must accept its own declared bound"
            );
            let profile = profile_for(LocoContext::Run, &tune);
            for start_deg in starts {
                let want = Vec2::new(1.0, 0.0);
                let start = start_deg.to_radians();
                let mut k = Kinematics {
                    run_vel: Vec2::new(0.0, 0.0),
                    facing: Vec2::new(start.cos(), start.sin()),
                };
                let cmd = moving(Vec2::new(0.0, 0.0), FacingIntent::Toward(want));
                let mut remaining = signed_angle(k.facing, want).abs();
                // Sign stability is only meaningful once the body is off the
                // antipode. At exactly 180 degrees the two arcs are the same
                // length, `+pi` and `-pi` name the same direction, and which
                // side of atan2's branch cut the first step lands on is not
                // an overshoot. `remaining` still has to fall monotonically
                // through that tick, which is the property that matters, and
                // it is asserted unconditionally below.
                const SETTLED: f64 = 3.0;
                let mut settled_sign: Option<f64> = None;
                let mut landed = None;
                for tick in 0..2400 {
                    k = locomotion::step(k, &cmd, &profile, DT, &tune);
                    let next = signed_angle(k.facing, want);
                    assert!(
                        next.abs() <= remaining + 1e-12,
                        "{id}={value}, start {start_deg} deg, tick {tick}: remaining angle grew \
                         from {remaining} to {} -- that is overshoot",
                        next.abs()
                    );
                    if next != 0.0 && next.abs() < SETTLED {
                        match settled_sign {
                            None => settled_sign = Some(next.signum()),
                            Some(sign) => assert!(
                                next.signum() == sign,
                                "{id}={value}, start {start_deg} deg, tick {tick}: the remaining \
                                 angle changed sign -- the facing crossed its target and is \
                                 oscillating"
                            ),
                        }
                    }
                    remaining = next.abs();
                    if remaining == 0.0 && landed.is_none() {
                        landed = Some(tick);
                    }
                }
                assert!(
                    landed.is_some(),
                    "{id}={value}, start {start_deg} deg: facing never reached its target in 40 \
                     seconds -- an ease-out with no floor asymptotes forever"
                );
            }
        }
    }
}

#[test]
fn facing_rotates_at_a_bounded_rate_rather_than_snapping() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    let want = Vec2::new(-1.0, 0.0);
    let k = Kinematics {
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(1.0, 0.0),
    };
    let next = locomotion::step(
        k,
        &moving(Vec2::new(0.0, 0.0), FacingIntent::Toward(want)),
        &profile,
        DT,
        &tune,
    );
    let turned = signed_angle(k.facing, next.facing).abs();
    assert!(
        turned > 0.0,
        "a facing target must actually move the facing"
    );
    assert!(
        turned <= profile.face_rate * DT + 1e-12,
        "one tick turned {turned} rad, above the context's bounded rate of {}",
        profile.face_rate * DT
    );
    assert!(
        turned < std::f64::consts::PI - 1e-9,
        "a 180 degree target must not be reached in one tick -- that is the snap this replaced"
    );
}

#[test]
fn holding_facing_leaves_it_untouched_while_the_body_moves() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    let mut k = Kinematics {
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(0.0, 1.0),
    };
    let cmd = moving(Vec2::new(BASE_SPEED, 0.0), FacingIntent::Hold);
    for _ in 0..120 {
        k = locomotion::step(k, &cmd, &profile, DT, &tune);
    }
    assert_eq!(
        k.facing,
        Vec2::new(0.0, 1.0),
        "Hold must not rotate the facing at all"
    );
    assert!(
        k.run_vel.x > 0.0,
        "...while the body still moved: facing and movement are independent targets"
    );
}

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

#[test]
fn every_context_is_reachable_from_the_resolver() {
    let tune = tune();
    let forward = Vec2::new(1.0, 0.0);
    let sideways = Vec2::new(0.0, 1.0);
    let backward = Vec2::new(-1.0, 0.0);
    let cases: [(LocoContext, f64, bool, bool, Vec2); 7] = [
        (LocoContext::Jog, 0.3, false, false, forward),
        (LocoContext::Run, 1.0, false, false, forward),
        (LocoContext::Sprint, 1.0, false, true, forward),
        (LocoContext::Carry, 1.0, true, false, forward),
        (LocoContext::SprintCarry, 1.0, true, true, forward),
        (LocoContext::Strafe, 1.0, false, false, sideways),
        (LocoContext::Backpedal, 1.0, false, false, backward),
    ];
    for (want, throttle, carrying, sprinting, move_dir) in cases {
        let got = locomotion::resolve(throttle, carrying, sprinting, move_dir, forward, &tune);
        assert_eq!(
            got,
            want,
            "throttle {throttle}, carrying {carrying}, sprinting {sprinting}, moving {move_dir:?} \
             must resolve as {}",
            want.wire_str()
        );
    }
}

#[test]
fn geometry_beats_possession_when_classifying_a_context() {
    // A carrier backing away from where it is looking is backpedalling. That
    // is a fact about the body, not about the ball, and it is only expressible
    // because facing is an independent target.
    let tune = tune();
    let got = locomotion::resolve(
        1.0,
        true,
        true,
        Vec2::new(-1.0, 0.0),
        Vec2::new(1.0, 0.0),
        &tune,
    );
    assert_eq!(got, LocoContext::Backpedal);
}

#[test]
fn strafe_and_backpedal_move_off_the_facing_line() {
    let tune = tune();
    let facing = Vec2::new(1.0, 0.0);
    for (ctx, command, description) in [
        (LocoContext::Strafe, Vec2::new(0.0, BASE_SPEED), "strafe"),
        (
            LocoContext::Backpedal,
            Vec2::new(-BASE_SPEED, 0.0),
            "backpedal",
        ),
    ] {
        let profile = profile_for(ctx, &tune);
        let mut k = Kinematics {
            run_vel: Vec2::new(0.0, 0.0),
            facing,
        };
        let cmd = moving(command, FacingIntent::Toward(facing));
        for _ in 0..120 {
            k = locomotion::step(k, &cmd, &profile, DT, &tune);
        }
        assert!(
            k.run_vel.length() > 1.0,
            "{description}: the body must actually be moving"
        );
        // Signed, not absolute: a backpedal is ANTI-parallel to facing, which
        // an absolute alignment would happily call "aligned".
        let alignment =
            k.run_vel.normalized().x * k.facing.x + k.run_vel.normalized().y * k.facing.y;
        assert!(
            alignment < 0.99,
            "{description}: movement came out along the facing line (alignment {alignment}) -- \
             the two targets collapsed into one"
        );
        assert_eq!(
            k.facing.normalized(),
            facing,
            "{description}: facing must have stayed on its own target"
        );
    }

    // And the two are distinguishable from each other, not one behavior with
    // two names: a strafe is broadly across the facing, a backpedal against it.
    let strafe = locomotion::resolve(1.0, false, false, Vec2::new(0.0, 1.0), facing, &tune);
    let backpedal = locomotion::resolve(1.0, false, false, Vec2::new(-1.0, 0.0), facing, &tune);
    assert_ne!(strafe, backpedal);
}

#[test]
fn carry_contexts_differ_from_their_empty_handed_counterparts() {
    let tune = tune();
    // Same command, same ticks, four profiles: only the context differs.
    let travelled = |ctx: LocoContext| {
        let profile = profile_for(ctx, &tune);
        let mut k = Kinematics {
            run_vel: Vec2::new(0.0, 0.0),
            facing: Vec2::new(1.0, 0.0),
        };
        let cmd = moving(Vec2::new(BASE_SPEED, 0.0), FacingIntent::Movement);
        let mut distance = 0.0;
        for _ in 0..180 {
            k = locomotion::step(k, &cmd, &profile, DT, &tune);
            distance += k.run_vel.length() * DT;
        }
        (distance, k.run_vel.length())
    };

    let (run_distance, run_top) = travelled(LocoContext::Run);
    let (carry_distance, carry_top) = travelled(LocoContext::Carry);
    let (sprint_distance, sprint_top) = travelled(LocoContext::Sprint);
    let (sprint_carry_distance, sprint_carry_top) = travelled(LocoContext::SprintCarry);

    assert!(
        carry_top < run_top && carry_distance < run_distance,
        "carrying must cost speed: carry {carry_top}/{carry_distance} vs run {run_top}/{run_distance}"
    );
    assert!(
        sprint_carry_top < sprint_top && sprint_carry_distance < sprint_distance,
        "sprinting with the ball must cost speed against a free sprint: sprint_carry \
         {sprint_carry_top}/{sprint_carry_distance} vs sprint {sprint_top}/{sprint_distance}"
    );
    assert!(
        sprint_carry_top > carry_top,
        "sprinting with the ball must still beat carrying at a run"
    );

    // The difference is not only top speed: turning with the ball is heavier.
    let carry = profile_for(LocoContext::Carry, &tune);
    let run = profile_for(LocoContext::Run, &tune);
    assert!(
        carry.turn_rate < run.turn_rate,
        "a carrier must turn wider than the same player empty-handed"
    );
}

// ---------------------------------------------------------------------------
// Parametric derivation
// ---------------------------------------------------------------------------

#[test]
fn profiles_are_derived_from_stats_rather_than_authored_per_character() {
    let tune = tune();
    // Two players, same context, differing only in their stat block.
    let quick = locomotion::stats(260.0, 0.2, 0.2, &tune);
    let deliberate = locomotion::stats(100.0, 0.9, 0.9, &tune);
    let quick_profile = locomotion::profile(LocoContext::Run, 260.0, quick, &tune);
    let deliberate_profile = locomotion::profile(LocoContext::Run, 100.0, deliberate, &tune);

    assert!(
        quick_profile.top_speed > deliberate_profile.top_speed,
        "pace must reach top speed"
    );
    assert!(
        quick_profile.accel_run > deliberate_profile.accel_run,
        "pace must reach acceleration"
    );
    assert!(
        deliberate_profile.decel > quick_profile.decel,
        "strength must reach deceleration"
    );
    assert!(
        deliberate_profile.turn_rate > quick_profile.turn_rate,
        "technique must reach the angular rate -- a fast, low-technique player is wide in the \
         corners without anyone authoring that"
    );
}

#[test]
fn acceleration_and_deceleration_are_separate_rates() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    assert!(
        profile.decel != profile.accel_run,
        "the shipped defaults must not make the two rates coincide, or nothing tests the split"
    );

    let from_rest = locomotion::step(
        Kinematics {
            run_vel: Vec2::new(0.0, 0.0),
            facing: Vec2::new(1.0, 0.0),
        },
        &moving(Vec2::new(BASE_SPEED, 0.0), FacingIntent::Movement),
        &profile,
        DT,
        &tune,
    );
    assert!(
        (from_rest.run_vel.length() - profile.accel_start * DT).abs() < 1e-9,
        "the first tick from rest must use the standing-start rate"
    );

    let braking = locomotion::step(
        Kinematics {
            run_vel: Vec2::new(BASE_SPEED, 0.0),
            facing: Vec2::new(1.0, 0.0),
        },
        &moving(Vec2::new(0.0, 0.0), FacingIntent::Movement),
        &profile,
        DT,
        &tune,
    );
    assert!(
        (braking.run_vel.length() - (BASE_SPEED - profile.decel * DT)).abs() < 1e-9,
        "an empty command must brake at the decel rate"
    );
}

#[test]
fn a_turn_at_pace_traces_an_arc_instead_of_cutting_the_chord() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    let mut k = Kinematics {
        run_vel: Vec2::new(BASE_SPEED, 0.0),
        facing: Vec2::new(1.0, 0.0),
    };
    let cmd = moving(Vec2::new(0.0, BASE_SPEED), FacingIntent::Movement);
    let first = locomotion::step(k, &cmd, &profile, DT, &tune);
    let turned = signed_angle(k.run_vel, first.run_vel).abs();
    assert!(
        turned > 0.0 && turned <= profile.turn_rate * 6.0 * DT,
        "a 90 degree turn at pace must be a bounded rotation, not a jump: turned {turned} rad"
    );

    for _ in 0..60 {
        k = locomotion::step(k, &cmd, &profile, DT, &tune);
    }
    assert!(
        k.run_vel.length() > BASE_SPEED * 0.9,
        "the body must hold pace THROUGH the arc; the old vector nudge shed speed on every turn, \
         got {}",
        k.run_vel.length()
    );
}

#[test]
fn a_turn_at_rest_is_free() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    let k = Kinematics {
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(1.0, 0.0),
    };
    let next = locomotion::step(
        k,
        &moving(Vec2::new(-BASE_SPEED, 0.0), FacingIntent::Movement),
        &profile,
        DT,
        &tune,
    );
    assert!(
        next.run_vel.x < 0.0,
        "a body at rest owes nothing to momentum and may leave in any direction"
    );
}

// ---------------------------------------------------------------------------
// Arrival
// ---------------------------------------------------------------------------

#[test]
fn arrival_speed_is_the_speed_a_body_can_still_stop_from() {
    let tune = tune();
    let profile = profile_for(LocoContext::Run, &tune);
    let distance = 40.0;
    let cap = locomotion::arrival_speed(distance, profile.decel);
    assert!(cap > 0.0);

    // Travel from `distance` away at exactly the cap and brake immediately:
    // the body must stop at or before the target, never past it.
    let mut k = Kinematics {
        run_vel: Vec2::new(cap, 0.0),
        facing: Vec2::new(1.0, 0.0),
    };
    let mut travelled = 0.0;
    for _ in 0..600 {
        k = locomotion::step(
            k,
            &moving(Vec2::new(0.0, 0.0), FacingIntent::Movement),
            &profile,
            DT,
            &tune,
        );
        travelled += k.run_vel.length() * DT;
    }
    assert!(
        travelled <= distance,
        "braking from the arrival cap overshot by {}",
        travelled - distance
    );
    assert_eq!(locomotion::arrival_speed(0.0, profile.decel), 0.0);
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

#[test]
fn every_context_knob_is_a_registered_tier_one_scalar() {
    // The context table names knobs by string. A typo would resolve to a
    // panic at the first tick of a real match rather than here, so check the
    // whole table against the registry up front.
    assert_eq!(
        CONTEXT_KNOBS.len(),
        LocoContext::ALL.len(),
        "every context needs exactly one knob row"
    );
    let tune = tune();
    for row in CONTEXT_KNOBS {
        for id in [row.top_mult, row.accel_mult, row.decel_mult, row.turn_mult] {
            let def = knob(id);
            assert_eq!(
                def.tier,
                Tier::Sim,
                "{id} must be a sim-affecting scalar, not presentation"
            );
            assert!(
                def.min < def.max,
                "{id} must declare a range a sweep can move it through"
            );
            assert!(!def.unit.is_empty(), "{id} must declare a unit");
            // And it must actually resolve through the live registry, which
            // is what the primitive reads.
            let value = tune.value(id);
            assert!(
                value.is_finite(),
                "{id} must resolve to a finite value through Tuning"
            );
        }
    }
}

#[test]
fn locomotion_knob_ids_survive_the_tuning_blob_round_trip() {
    // The trap this test exists for: `Tuning::deserialize` accepts only
    // `[A-Za-z0-9_]` in a key and SKIPS malformed lines silently, and
    // `knob_contract` perturbs a knob through exactly that path. A dotted id
    // would look correctly registered in the F1 panel and the sweep while
    // every knob-moves-metric assertion reported it as decoration.
    let mut ids: Vec<&'static str> = Vec::new();
    for row in CONTEXT_KNOBS {
        ids.extend([row.top_mult, row.accel_mult, row.decel_mult, row.turn_mult]);
    }
    ids.extend(
        SIM_TUNABLES
            .iter()
            .filter(|d| d.id.starts_with("LOCO_"))
            .map(|d| d.id),
    );
    for id in ids {
        let def = knob(id);
        let nudged = (def.default + def.step).min(def.max).max(def.min);
        assert!(
            nudged != def.default,
            "{id}: its own step must move it, or the perturbation below proves nothing"
        );
        let mut tune = tune();
        tune.deserialize(&format!("{id}={nudged}"));
        assert_eq!(
            tune.value(id),
            nudged,
            "{id} did not survive a Tuning blob round trip -- a knob whose id the parser drops is \
             silently unreachable from every knob-moves-metric test"
        );
    }
}
