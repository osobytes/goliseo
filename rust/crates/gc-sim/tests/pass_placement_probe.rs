//! QUANTITATIVE PROBE for the reported pass-placement defect (2026-08-25):
//! "passes are led to points far away from the receiver". Runs seeded
//! AI-vs-AI matches, captures every ground-pass release through the
//! `pass_probe_*` diagnostic seam in `gc_sim::r#match`, then tracks the real
//! ball against the intended receiver tick by tick and reports aggregates.
//!
//! Run with:
//!   cargo test -p gc-sim --release --test pass_placement_probe -- --nocapture
//!
//! Originally a pure measurement harness for the diagnosis; since the
//! pass-reception rework it is ALSO the regression gate for that fix: the
//! assertions at the bottom pin conservative floors under the measured
//! recovery (led intended-receiver completion within 2 s ≥ 60%, opponent
//! runout interceptions ≤ 28%, overshoot p90 ≤ 250 px), far enough below
//! the measured values (75.9% / 18.0% / 126 px, seeds 1–24) to survive
//! balance tuning while catching a return of either root defect: the
//! receiver-lockout collision and the runout past the meeting point.

use gc_core::vec2::Vec2;
use gc_sim::fixed_clock;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, PassProbeRecord, StepInput};
use gc_sim::match_snapshot::{MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::tuning::Tuning;
use std::io::Write as _;

const DT: f64 = fixed_clock::TICK_SECONDS;
const FIELD_W: f64 = 1648.0;
const FIELD_H: f64 = 927.0;
const DURATION: f64 = 120.0;
const MATCHES: u64 = 24;
/// Outcome-classification window, seconds (the task's "sensible window").
const WINDOW_S: f64 = 2.0;
/// Hard tracking ceiling per flight, seconds.
const TRACK_MAX_S: f64 = 6.0;
/// Ball speed below which the ball counts as rolled dead, px/s.
const DEAD_SPEED: f64 = 30.0;
/// `RELEASE_CD` in match.rs: the global pickup lockout after any release.
const RELEASE_CD: f64 = 0.3;
/// `POSSESS_MAX_SPEED` in match.rs: max pace a NON-designated outfielder
/// can collect at.
const POSSESS_MAX_SPEED: f64 = 350.0;
const FRICTION: f64 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    /// Collected by the intended receiver (tick delta).
    Intended(u64),
    /// Collected by another teammate of the passer.
    Teammate(u64),
    /// Collected by an opponent.
    Opponent(u64),
    /// A goal was scored while the ball was loose.
    Goal(u64),
    /// The ball rolled dead (speed < DEAD_SPEED) with nobody collecting
    /// inside the tracking ceiling.
    Dead(u64),
    /// Still loose when tracking gave up (TRACK_MAX_S) or the match ended.
    Lost(u64),
}

impl Outcome {
    fn ticks(self) -> u64 {
        match self {
            Outcome::Intended(t)
            | Outcome::Teammate(t)
            | Outcome::Opponent(t)
            | Outcome::Goal(t)
            | Outcome::Dead(t)
            | Outcome::Lost(t) => t,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Arrival {
    /// Ticks after release when the ball first reached the aim point
    /// (closest approach) or died.
    ticks: u64,
    /// Ball -> intended receiver distance at that tick, px.
    ball_recv_d: f64,
    /// Ball speed at that tick, px/s.
    ball_speed: f64,
    /// Ball position at that tick.
    ball_pos: Vec2,
    /// Receiver position at that tick.
    recv_pos: Vec2,
    /// Whether the receiver's run velocity still pointed at the aim point.
    recv_moving_toward_aim: bool,
}

struct Flight {
    rec: PassProbeRecord,
    seed: u64,
    release_tick: u64,
    score_at_release: (i64, i64),
    prev_ball: Vec2,
    prev_d_aim: f64,
    prev_ball_speed: f64,
    prev_recv_pos: Vec2,
    prev_recv_run_vel: Vec2,
    ticks: u64,
    arrival: Option<Arrival>,
    min_ball_recv_d: f64,
    min_ball_recv_ticks: u64,
    /// Ball -> aim distance when the flight closed.
    final_d_aim: f64,
    /// Farthest the ball got past the aim point, measured as
    /// dist(ball, release) - dist(aim, release), px (>= 0 only if it
    /// overshot).
    overshoot: f64,
    outcome: Option<Outcome>,
}

fn passer_team(s: &MatchState, f: &Flight) -> Team {
    s.players[(f.rec.owner_idx - 1) as usize].team
}

/// Analytic exponential-friction travel time to cover `d` px from launch
/// speed `v0`: `None` when the ball dies short of `d`.
fn travel_time(v0: f64, d: f64) -> Option<f64> {
    let x = FRICTION * d / v0;
    if x >= 1.0 {
        return None;
    }
    Some(-(1.0 - x).ln() / FRICTION)
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn pctl(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in percentile input"));
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i]
}

#[test]
fn pass_placement_probe() {
    let tune = Tuning::new();
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");

    let mut flights: Vec<Flight> = Vec::new();
    let mut superseded = 0u64;

    for seed in 1..=MATCHES {
        let mut s = sim_match::new(NewMatchOptions {
            home,
            away,
            field: PitchSize {
                w: FIELD_W,
                h: FIELD_H,
            },
            home_formation: None,
            tactic: None,
            away_tactic: None,
            duration: Some(DURATION),
            max_goals: None,
            seed: Some(seed as f64),
            players_by_id: None,
            species_by_id: None,
            showcase_players_by_id: None,
            human_controlled: Some(false),
            input_ownership: None,
        });
        sim_match::pass_probe_begin();

        let max_ticks = (DURATION / DT).ceil() as u64 + 600;
        let mut active: Option<Flight> = None;
        for tick in 0..max_ticks {
            if s.finished {
                break;
            }
            sim_match::step(
                &mut s,
                DT,
                StepInput::Legacy(MatchInput::default()),
                None,
                &tune,
            );

            // 1) Update the active flight against this tick's settled state.
            if let Some(f) = active.as_mut() {
                f.ticks = tick - f.release_tick;
                let ball = s.ball;
                let ball_speed = s.ball_vel.length();
                let d_aim = ball.dist(f.rec.aim);
                let recv = &s.players[(f.rec.target_idx - 1) as usize];
                let d_recv = ball.dist(recv.pos);
                if d_recv < f.min_ball_recv_d {
                    f.min_ball_recv_d = d_recv;
                    f.min_ball_recv_ticks = f.ticks;
                }
                let past = ball.dist(f.rec.owner_pos) - f.rec.aim.dist(f.rec.owner_pos);
                if past > f.overshoot {
                    f.overshoot = past;
                }
                if f.arrival.is_none() {
                    if d_aim > f.prev_d_aim {
                        // Closest approach to the aim point was last tick.
                        let to_aim = f.rec.aim.sub(f.prev_recv_pos);
                        let toward =
                            f.prev_recv_run_vel.x * to_aim.x + f.prev_recv_run_vel.y * to_aim.y;
                        f.arrival = Some(Arrival {
                            ticks: f.ticks.saturating_sub(1),
                            ball_recv_d: f.prev_ball.dist(f.prev_recv_pos),
                            ball_speed: f.prev_ball_speed,
                            ball_pos: f.prev_ball,
                            recv_pos: f.prev_recv_pos,
                            recv_moving_toward_aim: toward > 0.0,
                        });
                    } else if ball_speed < DEAD_SPEED {
                        let to_aim = f.rec.aim.sub(recv.pos);
                        let toward = recv.run_vel.x * to_aim.x + recv.run_vel.y * to_aim.y;
                        f.arrival = Some(Arrival {
                            ticks: f.ticks,
                            ball_recv_d: d_recv,
                            ball_speed,
                            ball_pos: ball,
                            recv_pos: recv.pos,
                            recv_moving_toward_aim: toward > 0.0,
                        });
                    }
                }
                // Outcome resolution.
                let score_now = (s.score.home, s.score.away);
                let mut close: Option<Outcome> = None;
                if score_now != f.score_at_release {
                    close = Some(Outcome::Goal(f.ticks));
                } else if s.events.iter().any(|e| {
                    e.kind == MatchEventKind::FirstTouchShot
                        && e.player.as_deref()
                            == Some(s.players[(f.rec.target_idx - 1) as usize].id.as_str())
                }) {
                    // The intended receiver striking the pass first time
                    // (#623) IS the meeting point working: the ball reached
                    // its man in collectable position and he chose to shoot
                    // it rather than settle it. Counting it as anything else
                    // would punish the reception rework for the one-timer
                    // verb succeeding -- which is exactly what happened when
                    // the verb's release stopped being eaten by the
                    // striker's own body (block grace, 2026-08-28) and this
                    // guardrail's led<=2s reading fell 75.9% -> 58.9%
                    // overnight with the verb's own per-match rate (2.2)
                    // still comfortably inside its authored band.
                    close = Some(Outcome::Intended(f.ticks));
                } else if let Some(owner) = s.owner {
                    let pteam = passer_team(&s, f);
                    close = Some(if owner == f.rec.target_idx {
                        Outcome::Intended(f.ticks)
                    } else if s.players[(owner - 1) as usize].team == pteam {
                        Outcome::Teammate(f.ticks)
                    } else {
                        Outcome::Opponent(f.ticks)
                    });
                } else if f.ticks as f64 * DT >= TRACK_MAX_S {
                    close = Some(if ball_speed < DEAD_SPEED {
                        Outcome::Dead(f.ticks)
                    } else {
                        Outcome::Lost(f.ticks)
                    });
                }
                f.prev_ball = ball;
                f.prev_d_aim = d_aim;
                f.prev_ball_speed = ball_speed;
                f.prev_recv_pos = recv.pos;
                f.prev_recv_run_vel = recv.run_vel;
                if let Some(o) = close {
                    f.outcome = Some(o);
                    f.final_d_aim = d_aim;
                    flights.push(active.take().expect("flight is active"));
                }
            }

            // 2) New releases this tick supersede whatever was in the air.
            for rec in sim_match::pass_probe_drain() {
                if let Some(mut f) = active.take() {
                    f.outcome = Some(Outcome::Lost(f.ticks));
                    f.final_d_aim = f.prev_d_aim;
                    superseded += 1;
                    flights.push(f);
                }
                let d_aim0 = s.ball.dist(rec.aim);
                let recv = &s.players[(rec.target_idx - 1) as usize];
                active = Some(Flight {
                    seed,
                    release_tick: tick,
                    score_at_release: (s.score.home, s.score.away),
                    prev_ball: s.ball,
                    prev_d_aim: d_aim0,
                    prev_ball_speed: s.ball_vel.length(),
                    prev_recv_pos: recv.pos,
                    prev_recv_run_vel: recv.run_vel,
                    ticks: 0,
                    arrival: None,
                    min_ball_recv_d: s.ball.dist(recv.pos),
                    min_ball_recv_ticks: 0,
                    final_d_aim: d_aim0,
                    overshoot: 0.0,
                    outcome: None,
                    rec,
                });
            }
        }
        if let Some(mut f) = active.take() {
            f.outcome = Some(Outcome::Lost(f.ticks));
            f.final_d_aim = f.prev_d_aim;
            flights.push(f);
        }
        sim_match::pass_probe_end();
    }

    // ---------------- CSV dump ----------------
    // Per-release CSV only on request: an unset PASS_PROBE_OUT must not
    // litter the crate directory on every gate run.
    let csv_dir = std::env::var("PASS_PROBE_OUT").ok();
    if let Some(csv_path) = csv_dir.map(|dir| format!("{dir}/pass_probe.csv"))
        && let Ok(mut fcsv) = std::fs::File::create(&csv_path)
    {
        let _ = writeln!(
            fcsv,
            "seed,release_tick,owner,target,keeper,led,lead_time,solver_travel,solver_reach,\
             launch_speed,pass_dist,lead_dist,recv_run_speed,arrival_ticks,arrival_ball_recv_d,\
             arrival_ball_speed,recv_toward_aim,outcome,outcome_ticks,min_ball_recv_d,\
             min_ball_recv_ticks,overshoot,final_d_aim,along_px,lateral_px"
        );
        for f in &flights {
            let (led, lt, ts, rs) = match f.rec.lead {
                Some(l) => (1, l.lead_time, l.travel_time, l.reach_time),
                None => (0, 0.0, f64::NAN, f64::NAN),
            };
            let (at, ad, asp, tw) = match f.arrival {
                Some(a) => (
                    a.ticks as f64,
                    a.ball_recv_d,
                    a.ball_speed,
                    i32::from(a.recv_moving_toward_aim),
                ),
                None => (f64::NAN, f64::NAN, f64::NAN, -1),
            };
            let (along, lat) = match f.arrival {
                Some(a) => {
                    let rel = a.ball_pos.sub(a.recv_pos);
                    let rv = f.rec.target_run_vel;
                    if rv.length() > 1e-9 {
                        let rd = rv.normalized();
                        let al = rel.x * rd.x + rel.y * rd.y;
                        let latv = rel.sub(rd.scale(al));
                        (al, latv.length())
                    } else {
                        (0.0, rel.length())
                    }
                }
                None => (f64::NAN, f64::NAN),
            };
            let o = match f.outcome {
                Some(Outcome::Intended(_)) => "intended",
                Some(Outcome::Teammate(_)) => "teammate",
                Some(Outcome::Opponent(_)) => "opponent",
                Some(Outcome::Goal(_)) => "goal",
                Some(Outcome::Dead(_)) => "dead",
                Some(Outcome::Lost(_)) => "lost",
                None => "open",
            };
            let _ = writeln!(
                fcsv,
                "{},{},{},{},{},{},{:.2},{:.4},{:.4},{:.1},{:.1},{:.1},{:.1},{},{:.1},{:.1},{},{},{},{:.1},{},{:.1},{:.1},{:.1},{:.1}",
                f.seed,
                f.release_tick,
                f.rec.owner_idx,
                f.rec.target_idx,
                i32::from(f.rec.target_is_keeper),
                led,
                lt,
                ts,
                rs,
                f.rec.launch_speed,
                f.rec.owner_pos.dist(f.rec.aim),
                f.rec.target_pos.dist(f.rec.aim),
                f.rec.target_run_vel.length(),
                at,
                ad,
                asp,
                tw,
                o,
                f.outcome.map_or(0, Outcome::ticks),
                f.min_ball_recv_d,
                f.min_ball_recv_ticks,
                f.overshoot,
                f.final_d_aim,
                along,
                lat
            );
        }
        println!("wrote {} rows to {csv_path}", flights.len());
    }

    // ---------------- Aggregate report ----------------
    let outfield: Vec<&Flight> = flights.iter().filter(|f| !f.rec.target_is_keeper).collect();
    let keeper_n = flights.len() - outfield.len();
    let led: Vec<&&Flight> = outfield.iter().filter(|f| f.rec.lead.is_some()).collect();
    let unled: Vec<&&Flight> = outfield.iter().filter(|f| f.rec.lead.is_none()).collect();
    let unled_gate = unled
        .iter()
        .filter(|f| f.rec.target_run_vel.length() < 60.0)
        .count();
    let unled_solver_none = unled.len() - unled_gate;

    println!("\n================ pass placement probe ================");
    println!(
        "matches={MATCHES} duration={DURATION}s  ground releases tracked={} (superseded mid-flight={superseded})",
        flights.len()
    );
    println!(
        "to outfielders={}  to keepers={} (lead solve gated off for keepers)",
        outfield.len(),
        keeper_n
    );
    println!(
        "led={} ({:.1}%)  unled={} ({:.1}%)  [unled: {} below 60 px/s gate, {} solver returned None]",
        led.len(),
        100.0 * led.len() as f64 / outfield.len().max(1) as f64,
        unled.len(),
        100.0 * unled.len() as f64 / outfield.len().max(1) as f64,
        unled_gate,
        unled_solver_none
    );

    // Lead-time distribution.
    let mut lt_counts: std::collections::BTreeMap<i64, usize> = std::collections::BTreeMap::new();
    for f in &led {
        let lt = f.rec.lead.expect("led flight has a lead").lead_time;
        *lt_counts.entry((lt * 100.0).round() as i64).or_insert(0) += 1;
    }
    print!("chosen lead_time distribution (s -> n): ");
    for (k, v) in &lt_counts {
        print!("{:.2}->{v}  ", *k as f64 / 100.0);
    }
    println!();

    let lead_dists: Vec<f64> = led
        .iter()
        .map(|f| f.rec.target_pos.dist(f.rec.aim))
        .collect();
    println!(
        "lead distance (aim - receiver@release): mean={:.1} px  p50={:.1}  p90={:.1}",
        mean(&lead_dists),
        pctl(&mut lead_dists.clone(), 0.5),
        pctl(&mut lead_dists.clone(), 0.9)
    );

    let solver_travel: Vec<f64> = led
        .iter()
        .map(|f| f.rec.lead.expect("led").travel_time)
        .collect();
    let solver_reach: Vec<f64> = led
        .iter()
        .map(|f| f.rec.lead.expect("led").reach_time)
        .collect();
    println!(
        "solver travel_time: mean={:.3}s p50={:.3}  solver reach_time: mean={:.3}s p50={:.3}",
        mean(&solver_travel),
        pctl(&mut solver_travel.clone(), 0.5),
        mean(&solver_reach),
        pctl(&mut solver_reach.clone(), 0.5)
    );
    let within_cd = |ts: &[f64]| {
        100.0 * ts.iter().filter(|t| **t < RELEASE_CD).count() as f64 / ts.len().max(1) as f64
    };
    println!(
        "led passes whose solved ball travel_time < RELEASE_CD({RELEASE_CD}s): {:.1}%",
        within_cd(&solver_travel)
    );
    let unled_travel: Vec<f64> = unled
        .iter()
        .filter_map(|f| travel_time(f.rec.launch_speed, f.rec.owner_pos.dist(f.rec.aim)))
        .collect();
    println!(
        "unled passes: analytic travel_time mean={:.3}s p50={:.3}  < RELEASE_CD: {:.1}%  (dies short of aim: {})",
        mean(&unled_travel),
        pctl(&mut unled_travel.clone(), 0.5),
        within_cd(&unled_travel),
        unled.len() - unled_travel.len()
    );

    // Outcomes.
    let bucket = |set: &[&&Flight]| {
        let mut c = [0usize; 7]; // intended2s, intended-late, teammate, opponent, goal, dead, lost/open
        for f in set {
            match f.outcome {
                Some(Outcome::Intended(t)) => {
                    if (t as f64) * DT <= WINDOW_S {
                        c[0] += 1;
                    } else {
                        c[1] += 1;
                    }
                }
                Some(Outcome::Teammate(_)) => c[2] += 1,
                Some(Outcome::Opponent(_)) => c[3] += 1,
                Some(Outcome::Goal(_)) => c[4] += 1,
                Some(Outcome::Dead(_)) => c[5] += 1,
                Some(Outcome::Lost(_)) | None => c[6] += 1,
            }
        }
        c
    };
    let show = |name: &str, set: &[&&Flight]| {
        let c = bucket(set);
        let n = set.len().max(1) as f64;
        println!(
            "{name}: n={}  intended<=2s={:.1}%  intended-late={:.1}%  other-teammate={:.1}%  opponent={:.1}%  goal={:.1}%  rolled-dead={:.1}%  lost>6s={:.1}%",
            set.len(),
            100.0 * c[0] as f64 / n,
            100.0 * c[1] as f64 / n,
            100.0 * c[2] as f64 / n,
            100.0 * c[3] as f64 / n,
            100.0 * c[4] as f64 / n,
            100.0 * c[5] as f64 / n,
            100.0 * c[6] as f64 / n
        );
    };
    show("LED  ", &led);
    show("UNLED", &unled);

    // Time to first possession.
    let tposs: Vec<f64> = outfield
        .iter()
        .filter_map(|f| match f.outcome {
            Some(Outcome::Intended(t) | Outcome::Teammate(t) | Outcome::Opponent(t)) => {
                Some(t as f64 * DT)
            }
            _ => None,
        })
        .collect();
    println!(
        "time release -> first possession: mean={:.2}s p50={:.2}s p90={:.2}s (n={})",
        mean(&tposs),
        pctl(&mut tposs.clone(), 0.5),
        pctl(&mut tposs.clone(), 0.9),
        tposs.len()
    );

    // Arrival geometry.
    let arr = |set: &[&&Flight], name: &str| {
        let ds: Vec<f64> = set
            .iter()
            .filter_map(|f| f.arrival.map(|a| a.ball_recv_d))
            .collect();
        let sp: Vec<f64> = set
            .iter()
            .filter_map(|f| f.arrival.map(|a| a.ball_speed))
            .collect();
        let at: Vec<f64> = set
            .iter()
            .filter_map(|f| f.arrival.map(|a| a.ticks as f64 * DT))
            .collect();
        let hot = 100.0 * sp.iter().filter(|v| **v > POSSESS_MAX_SPEED).count() as f64
            / sp.len().max(1) as f64;
        let in_cd =
            100.0 * at.iter().filter(|v| **v < RELEASE_CD).count() as f64 / at.len().max(1) as f64;
        let toward = 100.0
            * set
                .iter()
                .filter_map(|f| f.arrival)
                .filter(|a| a.recv_moving_toward_aim)
                .count() as f64
            / set.iter().filter(|f| f.arrival.is_some()).count().max(1) as f64;
        println!(
            "{name} at ball's aim-point arrival: ball<->receiver mean={:.1}px p50={:.1} p90={:.1} | ball speed mean={:.0} (> {POSSESS_MAX_SPEED} px/s: {:.1}%) | arrival < pickup lockout: {:.1}% | receiver still moving toward aim: {:.1}%",
            mean(&ds),
            pctl(&mut ds.clone(), 0.5),
            pctl(&mut ds.clone(), 0.9),
            mean(&sp),
            hot,
            in_cd,
            toward
        );
    };
    arr(&led, "LED  ");
    arr(&unled, "UNLED");

    // Miss vectors for passes the intended receiver never collected.
    let misses: Vec<&&Flight> = outfield
        .iter()
        .filter(|f| !matches!(f.outcome, Some(Outcome::Intended(_))))
        .collect();
    let mut ahead = 0usize;
    let mut behind = 0usize;
    let mut side = 0usize;
    let mut along_px: Vec<f64> = Vec::new();
    let mut lat_px: Vec<f64> = Vec::new();
    for f in &misses {
        let Some(a) = f.arrival else { continue };
        let rel = a.ball_pos.sub(a.recv_pos);
        let rv = f.rec.target_run_vel;
        if rv.length() > 1e-9 {
            let rd = rv.normalized();
            let al = rel.x * rd.x + rel.y * rd.y;
            let latv = rel.sub(rd.scale(al)).length();
            along_px.push(al);
            lat_px.push(latv);
            if latv > al.abs() {
                side += 1;
            } else if al > 0.0 {
                ahead += 1;
            } else {
                behind += 1;
            }
        } else {
            side += 1;
            lat_px.push(rel.length());
        }
    }
    println!(
        "misses (not collected by intended, n={}): ahead-of-run={} behind-run={} off-to-side={} | along-run mean={:.1}px p50={:.1} | lateral mean={:.1}px p50={:.1}",
        misses.len(),
        ahead,
        behind,
        side,
        mean(&along_px),
        pctl(&mut along_px.clone(), 0.5),
        mean(&lat_px),
        pctl(&mut lat_px.clone(), 0.5)
    );

    // Overshoot: how far past the aim point the ball travelled.
    let over: Vec<f64> = outfield.iter().map(|f| f.overshoot).collect();
    let closest: Vec<f64> = outfield.iter().map(|f| f.min_ball_recv_d).collect();
    println!(
        "ball overshoot past aim point: mean={:.1}px p50={:.1} p90={:.1} | closest ball<->receiver ever: mean={:.1}px p50={:.1} p90={:.1}",
        mean(&over),
        pctl(&mut over.clone(), 0.5),
        pctl(&mut over.clone(), 0.9),
        mean(&closest),
        pctl(&mut closest.clone(), 0.5),
        pctl(&mut closest.clone(), 0.9)
    );

    // Observed vs solver-predicted ball arrival for led passes.
    let dtt: Vec<f64> = led
        .iter()
        .filter_map(|f| {
            f.arrival
                .map(|a| a.ticks as f64 * DT - f.rec.lead.expect("led").travel_time)
        })
        .collect();
    println!(
        "led passes, observed aim-arrival minus solver travel_time: mean={:.3}s p50={:.3}s",
        mean(&dtt),
        pctl(&mut dtt.clone(), 0.5)
    );
    println!("======================================================\n");

    assert!(
        flights.len() >= 50,
        "probe saw only {} ground releases; harness is broken",
        flights.len()
    );

    // ------------------------------------------------------------------
    // Regression floor (pass-reception rework). The run is seed-pinned and
    // deterministic, so these are exact for a given build; the margins
    // below the measured values (led<=2s 75.9%, opponent 18.0%, overshoot
    // p90 126 px on seeds 1-24) leave room for balance tuning without
    // letting the two root defects silently return: the receiver-lockout
    // collision (led completion collapsed to 32%) and the runout
    // (overshoot p90 466 px, a third of passes ending at an opponent 287
    // px past the aim).
    // ------------------------------------------------------------------
    let led_counts = bucket(&led);
    let led_n = led.len().max(1) as f64;
    assert!(
        led.len() >= 100,
        "the seeds must produce a meaningful led sample, got {}",
        led.len()
    );
    let led_2s = led_counts[0] as f64 / led_n;
    assert!(
        led_2s >= 0.60,
        "led passes collected by their intended receiver within {WINDOW_S}s fell to \
         {:.1}% — the reception rework's floor is 60% (measured 75.9%)",
        100.0 * led_2s
    );
    let led_opp = led_counts[3] as f64 / led_n;
    assert!(
        led_opp <= 0.28,
        "led passes ending at an opponent rose to {:.1}% — the runout regression \
         ceiling is 28% (measured 18.0%)",
        100.0 * led_opp
    );
    let led_open = led_counts[6] as f64 / led_n;
    assert!(
        led_open <= 0.02,
        "led passes with no resolution at all rose to {:.1}%",
        100.0 * led_open
    );
    let over_p90 = pctl(&mut over.clone(), 0.9);
    assert!(
        over_p90 <= 250.0,
        "ball overshoot past the aim point p90 rose to {over_p90:.0} px — the \
         runout regression ceiling is 250 (measured 126)"
    );
}
