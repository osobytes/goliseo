//! Adversarial coverage for the grounded first-touch shot's AFTERMATH
//! (#623), the class of failure that shipped broken three times because the
//! original tests asserted the MECHANISM (the event fired, `s.owner` stayed
//! `None`) at one tick instead of the OUTCOME a player watches unfold over
//! the following second. `the_striker_does_not_block_their_own_first_touch`
//! in `first_touch.rs` proved one instance of this (the striker's own body
//! eating the release one tick later); this file attacks the same "did
//! anyone actually watch it play out" gap along five different axes: a
//! clean release's flight, a whiff's ball, a nearby opponent's body, a
//! near-goal keeper duel, and a nearby teammate's body.
//!
//! House patterns reused verbatim from `first_touch.rs`: `new_match_seeded`,
//! `stage_arrival`, `strike_input` (jockey + aerial_strike together — the
//! real shape a held space bar produces through `slot_input`), and the
//! seed-scan-with-escape-hatch idiom for outcome-specific cases (a maxed
//! `volley_skill` makes `Clean` overwhelmingly likely per seed, so the scan
//! pins the first seed that lands it and panics if none in the range does,
//! so a probability regression can't pass silently).

use gc_core::vec2::Vec2;
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

/// First home outfielder, one-based. Deterministic across seeds: roster
/// order comes from team data, not rng.
fn home_outfielder(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no home outfielder");
}

/// Second home outfielder, one-based — a teammate distinct from whichever
/// player `home_outfielder` designates as the receiver.
fn second_home_outfielder(s: &MatchState) -> i64 {
    let first = home_outfielder(s);
    for (i, p) in s.players.iter().enumerate() {
        let idx = (i + 1) as i64;
        if p.team == Team::Home && !p.is_keeper && idx != first {
            return idx;
        }
    }
    panic!("fixture has no second home outfielder");
}

/// First away outfielder, one-based.
fn away_outfielder(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Away && !p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no away outfielder");
}

/// The away team's keeper, one-based.
fn away_keeper(s: &MatchState) -> i64 {
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Away && p.is_keeper {
            return (i + 1) as i64;
        }
    }
    panic!("fixture has no away keeper");
}

/// Stage the moment collection fires: `receiver` (one-based) is the
/// designated receiver of a pass arriving at `ball_speed` from the east,
/// already inside possession reach, everyone else parked far away.
/// Identical to `first_touch.rs`'s fixture so this file exercises the exact
/// same wiring, not a hand-rolled variant.
fn stage_arrival(s: &mut MatchState, receiver: i64, receiver_pos: Vec2, ball_speed: f64) {
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
    s.ball = receiver_pos.add(Vec2::new(10.0, 0.0));
    s.ball_vel = Vec2::new(-ball_speed, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    s.ball_spin = 0.0;
}

/// The exact shape `slot_input::to_match_input` produces while the ACTION
/// button is held off the ball: jockey AND aerial_strike together, plus the
/// held aim. Tests use this rather than a bare `aerial_strike` so the cases
/// exercise the wiring a real space-bar hold goes through.
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

fn block_by(s: &MatchState, id: &str) -> bool {
    s.events
        .iter()
        .any(|e| e.kind == MatchEventKind::Block && e.player.as_deref() == Some(id))
}

fn any_save_event(s: &MatchState) -> bool {
    s.events.iter().any(|e| {
        matches!(
            e.kind,
            MatchEventKind::Catch | MatchEventKind::Parry | MatchEventKind::Tip
        )
    })
}

const RECEIVER_POS: Vec2 = Vec2 { x: 700.0, y: 200.0 };

// ---------------------------------------------------------------------
// 1. Clean first touch: the shot must actually go somewhere and stay gone.
// ---------------------------------------------------------------------

#[test]
fn a_clean_first_touch_leaves_the_striker_behind_for_the_next_45_ticks() {
    // The exact shape of the play-tested bug: an event that "fired
    // correctly" at tick 0 is worthless if the ball drifts back into the
    // striker's own body a moment later. Track real distance-from-striker
    // over 45 ticks, not just the flag at the moment of release.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
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
            panic!("the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let striker_id = s.players[(receiver - 1) as usize].id.clone();

        let mut prev_dist = s.ball.dist(s.players[(receiver - 1) as usize].pos);
        for tick in 0..45 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert!(
                !block_by(&s, &striker_id),
                "the striker's own body must never block their own release (tick {tick})"
            );
            if tick < 20 {
                // Bounded to the near term deliberately: this held input
                // also drives the striker's own run direction (north, the
                // same as the aim), so late in the window it is entirely
                // legitimate for them to catch back up with a shot that
                // has bounced off the touchline -- that is a player
                // chasing their own strike, not the sim re-gluing the ball
                // to them. The interesting property is the immediate
                // aftermath, not the eventual re-contest.
                assert_ne!(
                    s.owner,
                    Some(receiver),
                    "the striker must not re-collect the ball they just struck this soon \
                     after releasing it (tick {tick})"
                );
            }
            let dist = s.ball.dist(s.players[(receiver - 1) as usize].pos);
            if tick < 15 {
                // Small epsilon for floating-point noise in a per-tick
                // Euler step, not a real tolerance for the ball drifting
                // back in.
                assert!(
                    dist >= prev_dist - 1e-6,
                    "the ball must keep moving away from the striker in the first 15 ticks \
                     (tick {tick}: {dist:.3} < previous {prev_dist:.3})"
                );
            }
            prev_dist = dist;
        }
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

// ---------------------------------------------------------------------
// 2. Whiffed first touch: the ball must not become permanently untouchable.
// ---------------------------------------------------------------------

#[test]
fn a_whiffed_first_touch_ball_gets_collected_by_somebody_within_60_ticks() {
    // The whiff (Miss) lets the ball "run through the swing" per the
    // mechanism comment in `resolve_first_touch_shot` -- but a mechanism
    // test stops at "the ball kept its velocity". A player watching this
    // needs the ball to eventually belong to SOMEONE, not roll forever in a
    // permanently-ungrabbable state. Park a teammate 60px down the ball's
    // own line (well inside the roll before it decays to a stop) so there
    // is always a body positioned to prove the ball became collectable
    // again once the whiff's own short `pickup_cd` clears.
    for seed in 0..60 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 340.0);
        s.players[(receiver - 1) as usize].volley_skill = 0.0;
        let waiting = second_home_outfielder(&s);
        s.players[(waiting - 1) as usize].pos = Vec2::new(640.0, 200.0);
        sim_match::set_controlled_player(&mut s, receiver);
        let tune = Tuning::new();
        sim_match::step(
            &mut s,
            DT,
            StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
            None,
            &tune,
        );
        let Some(event) = first_touch_event(&s) else {
            panic!("the attempt itself must fire regardless of skill");
        };
        if event.outcome != Some(AerialOutcome::Miss) {
            continue;
        }
        assert_eq!(s.owner, None, "a whiff must not become a trap");
        assert!(
            s.ball_vel.x < 0.0,
            "the whiffed ball keeps rolling on its own original line"
        );

        let mut collected_tick = None;
        for tick in 0..60 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(Vec2::new(0.0, -1.0))),
                None,
                &tune,
            );
            if s.owner.is_some() {
                collected_tick = Some(tick);
                break;
            }
        }
        assert!(
            collected_tick.is_some(),
            "the whiffed ball must not stay permanently untouchable for 60 ticks \
             (final owner={:?}, ball={:?}, ball_vel={:?})",
            s.owner,
            s.ball,
            s.ball_vel
        );
        return;
    }
    panic!("no seed in 0..60 produced a Miss at volley skill 0.0");
}

// ---------------------------------------------------------------------
// 3. Nearby bodies on the aim line: grace covers adjacent, not distant.
// ---------------------------------------------------------------------

#[test]
fn an_opponent_thirty_px_down_the_aim_line_does_not_eat_the_immediate_release() {
    // Mirrors `the_striker_does_not_block_their_own_first_touch`'s
    // release-grace fix, but for a FOREIGN body standing right where the
    // shot has to pass through -- a marker tight to the striker at the
    // moment of the touch. `block_grace` is a blanket time gate on the
    // whole body-block rule, not a self-only exemption, so it must cover
    // this body too for the shot's first moments.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        let opponent = away_outfielder(&s);
        s.players[(opponent - 1) as usize].pos = RECEIVER_POS.add(aim.scale(30.0));
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
            panic!("the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let opponent_id = s.players[(opponent - 1) as usize].id.clone();
        let launch_speed = s.ball_vel.length();

        // BLOCK_GRACE is 0.08s ~= 5 ticks at 60Hz; give a little headroom
        // and watch through tick 10.
        for tick in 0..10 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            assert!(
                !block_by(&s, &opponent_id),
                "a body 30px down the aim line must not eat the shot in its opening ticks \
                 (tick {tick})"
            );
        }
        assert!(
            s.ball_vel.length() > launch_speed * 0.5,
            "the shot must still be carrying real pace after clearing the nearby body"
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

#[test]
fn a_defender_250px_down_the_aim_line_interacts_with_the_ball_rather_than_letting_it_pass_through()
{
    // Far enough that release grace has long since expired by the time the
    // ball arrives: the block rule (or the defender simply collecting a
    // slowed ball) must engage. The failure this guards against is a shot
    // that sails through a body's collision radius leaving no trace at all
    // -- no Block, no change of hands, nothing -- which reads to a player
    // as the ball teleporting through a defender's chest.
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        let defender = away_outfielder(&s);
        let defender_start = RECEIVER_POS.add(aim.scale(250.0));
        s.players[(defender - 1) as usize].pos = defender_start;
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
            panic!("the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let defender_id = s.players[(defender - 1) as usize].id.clone();

        let mut closest = s.ball.dist(s.players[(defender - 1) as usize].pos);
        let mut interacted = false;
        for _ in 0..90 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            let d = s.ball.dist(s.players[(defender - 1) as usize].pos);
            closest = closest.min(d);
            if block_by(&s, &defender_id) || s.owner == Some(defender) || any_save_event(&s) {
                interacted = true;
                break;
            }
        }
        assert!(
            closest < 40.0,
            "test geometry sanity: the ball must actually have come near the defender \
             (closest approach {closest:.1}px)"
        );
        assert!(
            interacted,
            "a defender the shot's path runs straight through must block it or take it, \
             not let it pass through untouched (closest approach {closest:.1}px)"
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

// ---------------------------------------------------------------------
// 4. Shot at the near goal, keeper alone at home: the world must reach a
//    terminal outcome, not evaporate.
// ---------------------------------------------------------------------

#[test]
fn a_first_touch_at_the_near_goal_with_only_the_keeper_home_reaches_a_terminal_outcome() {
    let aim = Vec2::new(1.0, 0.0); // toward the away goal at x = field.w
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        let shoot_from = Vec2::new(s.field.w - 200.0, s.field.h / 2.0);
        stage_arrival(&mut s, receiver, shoot_from, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        // stage_arrival parks every away player, keeper included, at
        // midfield -- undo that for the keeper alone, so the only thing
        // standing between this shot and the net is exactly what the test
        // name promises.
        let keeper = away_keeper(&s);
        let keeper_row = s.players[(keeper - 1) as usize].pos.y;
        s.players[(keeper - 1) as usize].pos = Vec2::new(s.field.w - 15.0, keeper_row);
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
            panic!("the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }
        let home_score_before = s.score.home;

        let mut terminal = false;
        for _ in 0..60 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
            let goal_scored = s.score.home != home_score_before;
            let keeper_event = s.events.iter().any(|e| {
                matches!(
                    e.kind,
                    MatchEventKind::Catch
                        | MatchEventKind::Parry
                        | MatchEventKind::Tip
                        | MatchEventKind::Claim
                )
            });
            let out_of_play = s.ball.x < -5.0
                || s.ball.x > s.field.w + 5.0
                || s.ball.y < -5.0
                || s.ball.y > s.field.h + 5.0;
            if goal_scored || keeper_event || out_of_play || s.owner.is_some() {
                terminal = true;
                break;
            }
        }
        assert!(
            terminal,
            "a near-goal first touch with only the keeper home must reach a terminal \
             outcome within 60 ticks -- not just evaporate (final ball={:?}, owner={:?}, \
             score={:?})",
            s.ball, s.owner, s.score
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}

// ---------------------------------------------------------------------
// 5. A teammate on the aim line: no glued oscillation between two bodies.
// ---------------------------------------------------------------------

#[test]
fn a_teammate_forty_px_down_the_aim_line_does_not_leave_the_ball_oscillating() {
    let aim = Vec2::new(0.0, -1.0);
    for seed in 0..40 {
        let mut s = new_match_seeded(seed as f64, None);
        let receiver = home_outfielder(&s);
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;
        let teammate = second_home_outfielder(&s);
        let teammate_start = RECEIVER_POS.add(aim.scale(40.0));
        s.players[(teammate - 1) as usize].pos = teammate_start;
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
            panic!("the staged arrival must produce the attempt");
        };
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }

        for _ in 0..30 {
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(strike_input(aim)),
                None,
                &tune,
            );
        }
        let clear_of_teammate = s.ball.dist(teammate_start) > 80.0;
        assert!(
            s.owner.is_some() || clear_of_teammate,
            "30 ticks on, the ball must have resolved -- either someone owns it or it has \
             plainly moved on from the teammate, not still glued oscillating next to them \
             (owner={:?}, ball={:?}, dist_from_teammate_start={:.1})",
            s.owner,
            s.ball,
            s.ball.dist(teammate_start)
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean first touch at volley skill 1.0");
}
