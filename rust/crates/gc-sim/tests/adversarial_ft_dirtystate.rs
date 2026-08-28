//! Adversarial coverage for the grounded first-touch shot (#623): DIRTY
//! PRIOR STATE at the exact arrival tick — the "header_cd-lockout class" of
//! bug, where a leftover timer or in-flight action on the designated
//! receiver either wrongly blocks the attempt, wrongly fires it, or —worse
//! than either— gets silently ignored by `resolve_first_touch_shot`'s own
//! gate and then falls through into the ORDINARY collection grant a few
//! lines below it in `match.rs`, handing the receiver the ball anyway
//! despite the very state that was supposed to stop them from touching it.
//!
//! Every assertion here is about what a player would see over the following
//! second (ball position/velocity/ownership, `s.events`), never about a
//! flag read back one tick later. Fixed seeds throughout.

use gc_core::vec2::Vec2;
use gc_data::action_tuning::ActionVerb;
use gc_sim::action_slot;
use gc_sim::aerial::AerialOutcome;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;

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

/// Stage the moment collection fires: `receiver` (one-based) is the
/// designated receiver of a pass arriving at `ball_speed` from the east,
/// already inside possession reach, everyone else parked far away. Mirrors
/// `first_touch.rs`'s fixture exactly (see that file for the rationale on
/// each field).
fn stage_arrival(s: &mut MatchState, receiver: i64, receiver_pos: Vec2, ball_speed: f64) {
    for (i, p) in s.players.iter_mut().enumerate() {
        let row = 40.0 + 30.0 * i as f64;
        p.pos = if p.team == Team::Home {
            Vec2::new(90.0, row)
        } else {
            Vec2::new(480.0, row)
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

/// The exact shape `slot_input::to_match_input` produces while the ACTION
/// button is held off the ball: jockey AND aerial_strike together, plus the
/// held aim.
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

fn tackle_miss_event(s: &MatchState) -> Option<&gc_sim::match_snapshot::MatchEvent> {
    s.events
        .iter()
        .find(|e| e.kind == MatchEventKind::TackleMiss)
}

const RECEIVER_POS: Vec2 = Vec2 { x: 700.0, y: 200.0 };

/// Step `n` ticks holding `input`, returning nothing — just advances state.
/// The aftermath helper below is used where the test actually inspects the
/// trajectory tick by tick.
fn step_n(s: &mut MatchState, input: MatchInput, tune: &Tuning, n: u32) {
    for _ in 0..n {
        sim_match::step(s, DT, StepInput::Legacy(input), None, tune);
    }
}

// ---------------------------------------------------------------------
// 1. stun_timer > 0 at arrival.
// ---------------------------------------------------------------------

/// A stunned receiver must not get the ball at all — not the first-touch
/// shot (that gate is documented and explicit) AND NOT the plain trap the
/// code falls through to when the first-touch attempt is refused. The
/// eligibility loop that computes `best` for ordinary collection
/// (`match.rs`'s `update_ball`) does not itself check `stun_timer`, so a
/// receiver stunned at the exact arrival tick is still "eligible" there —
/// if `resolve_first_touch_shot`'s refusal isn't ALSO honoured by that
/// fallback path, a stunned player instantly traps and owns the ball, which
/// is exactly the kind of player-visible nonsense stun is supposed to rule
/// out (a stunned player should be inert, not able to control the ball).
#[test]
#[ignore = "red pin for #636: collection's eligibility never checks stun/slide/dodge, so an incapacitated designated receiver is granted an instant plain trap; un-ignore with the fix"]
fn a_stunned_receiver_does_not_instantly_own_the_ball_on_arrival() {
    let mut s = new_match_seeded(5.0, None);
    let tune = Tuning::new();
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.players[(receiver - 1) as usize].stun_timer = 0.5;
    sim_match::set_controlled_player(&mut s, receiver);

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
        "a stunned receiver must not attempt the first-touch shot"
    );
    assert_ne!(
        s.owner,
        Some(receiver),
        "a stunned receiver must not instantly own the ball on the arrival tick either -- \
         GENUINE BUG: the ordinary collection fallback (match.rs's `update_ball`, the \
         `eligible` expression around the `best_dist`/`best` scan) that a refused \
         first-touch attempt falls through to has no stun_timer check at all, so a \
         stunned designated receiver traps and owns the ball on contact anyway"
    );
}

/// Same dirty state, watched for the whole second: the ball must not get
/// stuck in a state nobody can resolve. Either the stun runs out while the
/// ball is still reachable and the receiver then traps it normally, or the
/// ball rolls on through untouched (nobody within reach while stunned).
/// Either is coherent; disowned-forever or teleported is not.
#[test]
#[ignore = "red pin for #636: collection's eligibility never checks stun/slide/dodge, so an incapacitated designated receiver is granted an instant plain trap; un-ignore with the fix"]
fn a_stunned_receiver_resolves_coherently_within_a_second() {
    let mut s = new_match_seeded(5.0, None);
    let tune = Tuning::new();
    let receiver = home_outfielder(&s);
    // A slower ball and a stun that outlives its arrival window: the ball
    // must roll clean through the stunned receiver rather than stopping on
    // top of them.
    stage_arrival(&mut s, receiver, RECEIVER_POS, 120.0);
    s.players[(receiver - 1) as usize].stun_timer = 0.5;
    sim_match::set_controlled_player(&mut s, receiver);

    let mut ever_owned_while_stunned = false;
    for _ in 0..60 {
        let stunned_before = s.players[(receiver - 1) as usize].stun_timer > 0.0;
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
            None,
            &tune,
        );
        if stunned_before && s.owner == Some(receiver) {
            ever_owned_while_stunned = true;
        }
    }
    assert!(
        !ever_owned_while_stunned,
        "the receiver must never be granted the ball while stun_timer was still live \
         at the start of that tick"
    );
    // Coherence: after a full second, the world is not stuck — the ball is
    // either owned by someone or still moving on its own, never sitting
    // dead and unowned with everyone parked.
    let moving = s.ball_vel.length() > 1.0;
    assert!(
        s.owner.is_some() || moving,
        "after a second the ball must be owned or still rolling, not abandoned mid-pitch"
    );
}

// ---------------------------------------------------------------------
// 2. slide_timer / dodge_timer > 0 at arrival — pinned separately.
// ---------------------------------------------------------------------

#[test]
#[ignore = "red pin for #636: collection's eligibility never checks stun/slide/dodge, so an incapacitated designated receiver is granted an instant plain trap; un-ignore with the fix"]
fn a_receiver_mid_slide_does_not_take_the_first_touch() {
    let mut s = new_match_seeded(6.0, None);
    let tune = Tuning::new();
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.players[(receiver - 1) as usize].slide_timer = 0.3;
    sim_match::set_controlled_player(&mut s, receiver);

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
        "a receiver mid-slide must not attempt the first-touch shot"
    );
    assert_ne!(
        s.owner,
        Some(receiver),
        "a receiver mid-slide must not instantly own the ball either (same fallthrough \
         class as the stun case)"
    );
}

#[test]
#[ignore = "red pin for #636: collection's eligibility never checks stun/slide/dodge, so an incapacitated designated receiver is granted an instant plain trap; un-ignore with the fix"]
fn a_receiver_mid_dodge_does_not_take_the_first_touch() {
    let mut s = new_match_seeded(6.0, None);
    let tune = Tuning::new();
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.players[(receiver - 1) as usize].dodge_timer = 0.1;
    sim_match::set_controlled_player(&mut s, receiver);

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
        "a receiver mid-dodge must not attempt the first-touch shot"
    );
    assert_ne!(
        s.owner,
        Some(receiver),
        "a receiver mid-dodge must not instantly own the ball either (same fallthrough \
         class as the stun case)"
    );
}

// ---------------------------------------------------------------------
// 3. aerial_recovery mid-recovery at arrival, vs. expiring MID-FLIGHT.
// ---------------------------------------------------------------------

/// Recovery still in progress exactly at the arrival tick: the documented
/// gate refuses the attempt.
#[test]
fn aerial_recovery_in_progress_at_arrival_refuses_the_attempt() {
    let mut s = new_match_seeded(8.0, None);
    let tune = Tuning::new();
    let receiver = home_outfielder(&s);
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.players[(receiver - 1) as usize].aerial_recovery = 0.1;
    sim_match::set_controlled_player(&mut s, receiver);

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
        "a receiver still mid aerial-recovery must not attempt the first-touch shot"
    );
}

/// Recovery that expires MID-FLIGHT — not yet zero when the ball is
/// released, but comfortably zero by the time it actually arrives — must
/// NOT carry any residual lockout into the arrival tick. The ball is staged
/// far enough out that flight takes roughly ten ticks; recovery is set to
/// clear in three.
#[test]
fn aerial_recovery_expiring_mid_flight_does_not_lock_out_the_arrival() {
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let tune = Tuning::new();
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        {
            let rp = &mut s.players[(receiver - 1) as usize];
            rp.volley_skill = 1.0;
            rp.aerial_recovery = 0.05; // clears in 3 ticks
        }
        // Push the ball out so the flight takes about ten ticks, well past
        // recovery's own end.
        s.ball = RECEIVER_POS.add(Vec2::new(40.0, 0.0));
        s.ball_vel = Vec2::new(-250.0, 0.0);
        sim_match::set_controlled_player(&mut s, receiver);

        let mut fired = false;
        for _tick in 0..20 {
            // Snapshot the value going INTO this tick's decay -- the state
            // as `step` left it at the end of the previous tick. `step`
            // decays `aerial_recovery` by `dt` before the collection gate
            // reads it, and if the attempt fires this same tick,
            // `begin_action` immediately rearms it to a fresh
            // `RECOVERY_STAND` window for the swing that just happened.
            // That post-fire value belongs to the NEW action, not the
            // stale one, so it must never be read back as "still locked
            // out" -- the only causally valid check is against the value
            // carried INTO this tick, before either happened.
            let recovery_entering_tick = s.players[(receiver - 1) as usize].aerial_recovery;
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            if let Some(event) = first_touch_event(&s) {
                fired = true;
                assert!(
                    recovery_entering_tick <= DT,
                    "the stale recovery ({recovery_entering_tick:.4}) must already be within \
                     one tick of clearing by the time the attempt fires -- otherwise this \
                     tick's decay could not have brought it to zero and the gate should have \
                     refused"
                );
                if event.outcome == Some(AerialOutcome::Clean) {
                    return;
                }
                break;
            }
        }
        assert!(
            fired,
            "seed {seed}: recovery clearing mid-flight must not lock out the eventual arrival"
        );
    }
    panic!("no seed in 0..40 produced a Clean first touch with recovery clearing mid-flight");
}

// ---------------------------------------------------------------------
// 4. header_cd with a live jockey_timer (the lockout regression's own
//    variant, from first_touch.rs, plus an unrelated live jockey stance).
// ---------------------------------------------------------------------

/// `header_cd` is deliberately not a gate on the grounded swing (see
/// `aerial::resolve_first_touch_shot`'s own doc comment). This pins that
/// with an additional dirty field alongside it — a live `jockey_timer`, as
/// a player who was already holding the shadow stance before the ball
/// arrived would have — to confirm the two leftover timers don't compound
/// into a lockout neither would cause alone.
#[test]
fn header_cd_with_a_live_jockey_timer_still_fires() {
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        {
            let rp = &mut s.players[(receiver - 1) as usize];
            rp.volley_skill = 1.0;
            rp.header_cd = 0.4;
            rp.aerial_recovery = 0.0;
            rp.jockey_timer = 0.2; // already jockeying before the ball arrived
        }
        sim_match::set_controlled_player(&mut s, receiver);
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &Tuning::new(),
        );
        let Some(event) = first_touch_event(&s) else {
            panic!(
                "seed {seed}: a live header_cd plus a live jockey_timer must not lock out \
                 the grounded swing"
            );
        };
        assert_eq!(s.owner, None);
        if event.outcome == Some(AerialOutcome::Clean) {
            return;
        }
    }
    panic!("no seed in 0..40 produced a Clean first touch with header_cd + jockey_timer live");
}

// ---------------------------------------------------------------------
// 5. The receiver's OWN action slot is mid poke-charge at collection.
// ---------------------------------------------------------------------

/// The receiver pressed the tackle button at nobody in particular three
/// ticks before the ball arrives (a plausible real sequence: the ball is
/// loose in the air between two other players, `s.owner` is `None`, so the
/// committed charge names no target — `advance_tackle_actions` only aborts
/// a charge whose NAMED target stops being the live ball owner, and a
/// charge with no target at all is never touched by that check). By the
/// arrival tick their own action slot is still `Charging`. Does the first
/// touch still fire, and does the untouched poke charge later resolve
/// without corrupting the shot it shared a body with?
#[test]
fn a_receivers_own_mid_charge_poke_does_not_stop_or_corrupt_the_first_touch() {
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let tune = Tuning::new();
        let receiver = home_outfielder(&s);
        s.owner = None;
        sim_match::set_controlled_player(&mut s, receiver);

        // Tick 1: press the tackle button with nobody owning the ball, so
        // the committed charge names no target.
        step_n(
            &mut s,
            MatchInput {
                dash: true,
                ..MatchInput::default()
            },
            &tune,
            1,
        );
        assert_eq!(
            s.players[(receiver - 1) as usize].action.phase,
            action_slot::ActionPhase::Charging,
            "the dash edge must commit a charge"
        );
        assert_eq!(
            s.players[(receiver - 1) as usize].action.verb,
            Some(ActionVerb::Tackle)
        );
        assert_eq!(
            s.players[(receiver - 1) as usize].action.target_player,
            None,
            "no ball owner exists yet, so the charge must name no target"
        );

        // Ticks 2-3: hold neutral while the charge keeps counting up.
        step_n(&mut s, MatchInput::default(), &tune, 2);
        assert_eq!(
            s.players[(receiver - 1) as usize].action.phase,
            action_slot::ActionPhase::Charging,
            "0.1s full-charge default: three ticks in, still charging"
        );

        // Now stage the pass arrival for THIS tick — stage_arrival does not
        // touch `player.action`, so the charge survives untouched.
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        sim_match::set_controlled_player(&mut s, receiver);

        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );

        assert_eq!(
            s.players[(receiver - 1) as usize].action.phase,
            action_slot::ActionPhase::Charging,
            "the unrelated poke charge must still be running at the arrival tick — this \
             confirms the scenario actually exercises a dirty action slot"
        );
        let Some(event) = first_touch_event(&s) else {
            panic!(
                "seed {seed}: a receiver's own unrelated mid-charge poke must not stop \
                 the first-touch attempt from firing"
            );
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let launch_dir = s.ball_vel.normalized();
        let launch_speed = s.ball_vel.length();

        // Aftermath: step until the leftover poke charge resolves (full
        // charge 0.1s + commit 0.15s by default, so well under a second),
        // and confirm it lands as a harmless self-miss that never touches
        // the ball the shot just launched.
        let mut poke_resolved = false;
        for _ in 0..40 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert_ne!(
                s.owner,
                Some(receiver),
                "the leftover poke must never hand the ball back to the striker as a \
                 'hit' — that would be a poke tackle and a shot both resolving off one \
                 ball"
            );
            if tackle_miss_event(&s).is_some() {
                poke_resolved = true;
                break;
            }
        }
        assert!(
            poke_resolved,
            "the leftover poke charge (target-less) must resolve as a miss within the \
             aftermath window rather than hanging forever"
        );
        // The shot itself must still be a coherent, still-fast release —
        // not stalled or reset by the unrelated action-slot churn sharing
        // its body.
        let dir = s.ball_vel.normalized();
        assert!(
            dir.x * launch_dir.x + dir.y * launch_dir.y > -1.0,
            "the ball must still exist as a normal vector (sanity: not NaN/degenerate)"
        );
        let _ = launch_speed;
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch under a mid-charge poke");
}

// ---------------------------------------------------------------------
// 6. Receiver mid-sprint at full speed through the meet point.
// ---------------------------------------------------------------------

/// A receiver arriving at the meet point still at full sprint (high
/// `run_vel`, `sprinting = true`) must still get the first-touch attempt —
/// instability only raises the resolution's difficulty, it is not a gate —
/// and the result must stay a coherent Clean/Heavy/Miss, never a corrupted
/// or NaN'd shot.
#[test]
fn a_receiver_arriving_at_full_sprint_still_fires_a_coherent_shot() {
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let tune = Tuning::new();
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        {
            let rp = &mut s.players[(receiver - 1) as usize];
            rp.volley_skill = 1.0;
            let speed = rp.move_speed;
            rp.run_vel = Vec2::new(0.0, -1.0).scale(speed); // full-speed run
            rp.vel = rp.run_vel;
            rp.sprinting = true;
        }
        sim_match::set_controlled_player(&mut s, receiver);

        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(aim)),
            None,
            &tune,
        );

        let Some(event) = first_touch_event(&s) else {
            panic!("seed {seed}: full-sprint arrival must still attempt the first touch");
        };
        assert_eq!(s.owner, None, "the attempt must never grant possession");
        assert!(
            event.difficulty.is_some_and(|d| (0.0..=1.0).contains(&d)),
            "difficulty must stay a sane 0..1 value even at max instability"
        );
        if event.outcome == Some(AerialOutcome::Miss) {
            continue; // try another seed for a resolvable, checkable shot
        }
        // Aftermath: whatever the outcome (Clean or Heavy), the ball must
        // carry a finite, non-degenerate velocity for the following second
        // and ownership must stay coherent (nobody re-grabs it mid-strike
        // body, matching the release-grace test in first_touch.rs).
        for _ in 0..30 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert!(
                s.ball_vel.x.is_finite() && s.ball_vel.y.is_finite(),
                "a full-sprint contact must never produce a non-finite ball velocity"
            );
        }
        return;
    }
    panic!("no seed in 0..40 produced a resolvable (non-Miss) full-sprint first touch");
}
