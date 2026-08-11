//! Human-proxy input driver for the controlled slot in headless matches.
//! Produces a `MatchInput` per frame the way a *predictable mediocre* player
//! would: decisions refresh only every reaction window (not every frame),
//! aim carries noise, and heuristics are deliberately simple. Balance
//! results are only comparable under the same bot — see
//! `docs/design/fun_metrics.md`.
//!
//! This is a SEPARATELY IDENTIFIED human-proxy population, not the gameplay
//! AI. It emits no equipment intent: every `equipment_*` field is
//! deliberately `false`. `gameplay_ai/combat/v1` lives in
//! `sim.combat_policy` and drives the match AI and the declared fixed-slot
//! bot fills; this driver may only gain combat capability behind its own
//! explicit policy id, and must never be reported as the gameplay AI by
//! accident.
//!
//! [`BotMatchView`]/[`BotPlayerView`] are a deliberately narrow read
//! interface onto match state, exactly the fields [`input`] and its helpers
//! touch — a human-proxy input driver has no business seeing keeper release
//! timers, tactic state, or the rest of `MatchState`. [`crate::headless::to_bot_view`]
//! builds one from a real `MatchState`.
//!
//! [`input`]'s *output*, by contrast, is not narrow — every `MatchInput`
//! field this driver produces is one the real simulation reads — so it
//! returns [`crate::match_snapshot::MatchInput`] directly rather than a
//! local mirror.
//!
//! [`crate::tuning::Tuning`] is an explicit `&Tuning` parameter, never a
//! shared global (AGENTS.md §3; see [`crate::tuning`]'s doc).
//!
//! `move` is a Rust keyword, so
//! [`crate::match_snapshot::MatchInput::r#move`] and [`BotState::r#move`]
//! use a raw identifier — the same convention `gc_sim::r#match` itself uses
//! for the module name (see `lib.rs`).

use crate::match_snapshot::MatchInput;
use crate::tuning::Tuning;
use gc_core::rng;
use gc_core::vec2::Vec2;

const REACTION: f64 = 0.2; // seconds between decisions (human-ish latency)
const AIM_NOISE: f64 = 0.15; // radians of uniform aim wobble per decision
const SHOT_CHARGE_HOLD: f64 = 0.35; // seconds of shoot_held before releasing
const PASS_CHARGE_HOLD: f64 = 0.22; // short hold reaches a deliberate mid/long outlet
const SHOOT_EAGERNESS: f64 = 1.15; // shoots a bit outside the AI's own range
const PRESSURE_PANIC: f64 = 1.3; // passes when pressed at this x the AI's radius
const JUKE_REACT_DIST: f64 = 64.0; // sidestep a defender who has committed inside this range
const LONG_OUTLET_DIST: f64 = 240.0; // charge and sometimes loft passes beyond this distance
const AERIAL_STRIKE_Z: f64 = 18.0; // first-time intent begins above the ground-control band
const AERIAL_STRIKE_DIST: f64 = 84.0; // mirrors the match's readable aerial anticipation window
const DEFEND_JOCKEY_DIST: f64 = 70.0; // shadow instead of chase inside this range
const POKE_DIST: f64 = 34.0; // attempt the standing tackle inside this range
const SPRINT_MIN_METER: f64 = 0.5; // only sprint on a half-full tank
const CHASE_LEAD: f64 = 0.15; // seconds of ball-velocity lead when chasing

/// A fixture side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    /// Home side.
    Home,
    /// Away side.
    Away,
}

/// The minimal per-player surface [`input`] and its helpers need (README
/// §5.1 end state 2 — see the module doc).
#[derive(Clone, Debug, PartialEq)]
pub struct BotPlayerView {
    /// Fixture side.
    pub team: Team,
    /// Whether this player is the keeper.
    pub is_keeper: bool,
    /// Current position.
    pub pos: Vec2,
    /// Seconds of an active standing-tackle poke window remaining.
    pub tackle_timer: f64,
    /// Seconds of an active slide tackle remaining.
    pub slide_timer: f64,
    /// Seconds until a juke is ready again.
    pub dodge_cd: f64,
    /// 0..1 stamina tank for sprinting.
    pub sprint_meter: f64,
}

/// Pitch dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// Width, px.
    pub w: f64,
    /// Height, px.
    pub h: f64,
}

/// The minimal match-state surface [`input`] and its helpers need (README
/// §5.1 end state 2 — see the module doc).
#[derive(Clone, Debug, PartialEq)]
pub struct BotMatchView {
    /// Every player in the fixture.
    pub players: Vec<BotPlayerView>,
    /// Pitch dimensions.
    pub field: Field,
    /// Index (0-based) of the player this bot drives.
    pub controlled: usize,
    /// Index (0-based) of the ball owner, if the ball is owned.
    pub owner: Option<usize>,
    /// Ball ground position.
    pub ball: Vec2,
    /// Ball horizontal velocity.
    pub ball_vel: Vec2,
    /// Ball height above the pitch.
    pub ball_z: f64,
    /// Ball vertical velocity.
    pub ball_vz: f64,
}

/// Construction options for [`new`].
#[derive(Clone, Copy, Debug, Default)]
pub struct BotOptions {
    /// PRNG seed. Defaults to `1` when `None`.
    pub seed: Option<f64>,
    /// Seconds between decisions. Defaults to [`REACTION`] when `None`.
    pub reaction: Option<f64>,
}

/// Live state for one bot-driven controlled slot.
#[derive(Clone, Debug, PartialEq)]
pub struct BotState {
    /// Own PRNG state: never touches the match's.
    pub rng: u32,
    /// Seconds between decisions.
    pub reaction: f64,
    /// Countdown to the next decision.
    pub decide_t: f64,
    /// Committed move direction (held between decisions). `move` is a Rust
    /// keyword — see the module doc.
    pub r#move: Vec2,
    /// Committed sprint intent (held between decisions).
    pub sprint: bool,
    /// Committed jockey intent (held between decisions).
    pub jockey: bool,
    /// One-shot: consumed by the next frame.
    pub dash: bool,
    /// One-shot carrier juke.
    pub dodge: bool,
    /// One-shot pass.
    pub pass: bool,
    /// Modifier for the queued one-shot action.
    pub lob: bool,
    /// >0: holding shoot, release when it expires.
    pub charge_t: f64,
    /// >0: holding pass, release when it expires.
    pub pass_charge_t: f64,
    /// Loft modifier queued for the pending charged pass.
    pub pass_lob: bool,
    /// Loft modifier queued for the pending charged shot.
    pub shot_lob: bool,
    /// Queued abstract first-time strike intent.
    pub aerial_strike: bool,
    /// Queued abstract bicycle/acrobatic intent.
    pub aerial_acrobatic: bool,
}

/// A fresh bot state for one controlled slot.
#[must_use]
pub fn new(opts: BotOptions) -> BotState {
    BotState {
        rng: rng::seed(opts.seed.unwrap_or(1.0) * 7919.0 + 17.0),
        reaction: opts.reaction.unwrap_or(REACTION),
        decide_t: 0.0,
        r#move: Vec2::new(0.0, 0.0),
        sprint: false,
        jockey: false,
        dash: false,
        dodge: false,
        pass: false,
        lob: false,
        charge_t: 0.0,
        pass_charge_t: 0.0,
        pass_lob: false,
        shot_lob: false,
        aerial_strike: false,
        aerial_acrobatic: false,
    }
}

/// Uniform sample in `[0, 1)` from the bot's own stream.
fn roll(b: &mut BotState) -> f64 {
    let (state, v) = rng::roll(b.rng);
    b.rng = state;
    v
}

/// `dir` rotated by up to +/- `AIM_NOISE` radians: the bot never aims
/// perfectly.
fn noisy(b: &mut BotState, dir: Vec2) -> Vec2 {
    let a = (roll(b) * 2.0 - 1.0) * AIM_NOISE;
    let (c, s) = (a.cos(), a.sin());
    Vec2::new(dir.x * c - dir.y * s, dir.x * s + dir.y * c)
}

fn nearest_opponent(s: &BotMatchView, from: Vec2) -> (f64, Option<&BotPlayerView>) {
    let mut best = f64::INFINITY;
    let mut who = None;
    for p in &s.players {
        if p.team == Team::Away && !p.is_keeper {
            let d = from.dist(p.pos);
            if d < best {
                best = d;
                who = Some(p);
            }
        }
    }
    (best, who)
}

/// The teammate the bot would pick out: most advanced open outfielder in
/// passing range (crude — the sim's own aim cone does the real target
/// pick).
fn best_outlet<'a>(
    s: &'a BotMatchView,
    me: &BotPlayerView,
    tune: &Tuning,
) -> Option<&'a BotPlayerView> {
    let mut best = f64::NEG_INFINITY;
    let mut who = None;
    for (i, p) in s.players.iter().enumerate() {
        if p.team == Team::Home && !p.is_keeper && i != s.controlled {
            let d = me.pos.dist(p.pos);
            if d <= tune.value("PASS_RANGE_MAX") {
                let (space, _) = nearest_opponent(s, p.pos);
                let value = p.pos.x + space * 0.5; // advanced and unmarked
                if value > best {
                    best = value;
                    who = Some(p);
                }
            }
        }
    }
    who
}

/// Decide a fresh intent (called once per reaction window).
fn decide(b: &mut BotState, s: &BotMatchView, tune: &Tuning) {
    let me = &s.players[s.controlled];
    let goal = Vec2::new(s.field.w, s.field.h / 2.0);
    b.sprint = false;
    b.jockey = false;
    b.dash = false;
    b.dodge = false;
    b.pass = false;
    b.lob = false;
    b.aerial_strike = false;
    b.aerial_acrobatic = false;
    b.shot_lob = false;
    b.pass_lob = false;

    if s.owner == Some(s.controlled) {
        if me.is_keeper {
            b.pass = true; // distribute at the first opportunity
            b.r#move = Vec2::new(0.0, 0.0);
            return;
        }
        let me_pos = me.pos;
        let me_dodge_cd = me.dodge_cd;
        let to_goal = goal.sub(me_pos);
        let (pressure, defender) = nearest_opponent(s, me_pos);
        let defender_committed =
            defender.is_some_and(|d| d.tackle_timer > 0.0 || d.slide_timer > 0.0);
        if defender_committed && pressure <= JUKE_REACT_DIST && me_dodge_cd <= 0.0 && roll(b) < 0.35
        {
            // A tool-using human reacts to the defender, not a random timer:
            // sidestep the committed poke, then reconsider on the next beat.
            b.r#move = noisy(b, to_goal.normalized());
            b.dodge = true;
            return;
        }
        if to_goal.length() < tune.value("AI_SHOOT_RANGE") * SHOOT_EAGERNESS {
            // Line up and charge: face the goal while holding the shot.
            b.r#move = noisy(b, to_goal.normalized());
            b.charge_t = SHOT_CHARGE_HOLD;
            b.shot_lob = roll(b) < 0.15; // occasional chip keeps the loft verb represented
        } else if pressure < tune.value("AI_PASS_PRESSURE") * PRESSURE_PANIC {
            let outlet = best_outlet(s, me, tune);
            if let Some(outlet) = outlet {
                let outlet_pos = outlet.pos;
                b.r#move = noisy(b, outlet_pos.sub(me_pos).normalized());
                let outlet_dist = me_pos.dist(outlet_pos);
                b.lob = outlet_dist >= LONG_OUTLET_DIST && roll(b) < 0.35;
                if outlet_dist >= LONG_OUTLET_DIST {
                    b.pass_charge_t = PASS_CHARGE_HOLD;
                    b.pass_lob = b.lob;
                } else {
                    b.pass = true;
                }
            } else {
                b.r#move = noisy(b, to_goal.normalized()); // nowhere to go: push on
            }
        } else {
            b.r#move = noisy(b, to_goal.normalized());
            b.sprint = me.sprint_meter > SPRINT_MIN_METER;
        }
        return;
    }

    if s.owner.is_none() {
        // Loose ball: run at where it is going, not where it is.
        let aim = s.ball.add(s.ball_vel.scale(CHASE_LEAD));
        let me_pos = me.pos;
        let diff = aim.sub(me_pos);
        b.r#move = if diff.length() > 1.0 {
            noisy(b, diff.normalized())
        } else {
            Vec2::new(0.0, 0.0)
        };
        b.sprint = diff.length() > 120.0 && me.sprint_meter > SPRINT_MIN_METER;
        if s.ball_z >= AERIAL_STRIKE_Z && s.ball_vz < 0.0 && diff.length() <= AERIAL_STRIKE_DIST {
            b.aerial_strike = true;
            b.aerial_acrobatic = s.ball_z >= 45.0 && roll(b) < 0.08;
        }
        return;
    }

    let owner_idx = s.owner.expect("owner is Some in this branch");
    let owner = &s.players[owner_idx];
    if owner.team == Team::Home {
        // A teammate has it: offer a forward option ahead of the carrier.
        let owner_pos = owner.pos;
        let me_pos = me.pos;
        let spot = Vec2::new(
            (owner_pos.x + 140.0).min(s.field.w - 60.0),
            me_pos.y * 0.7 + s.ball.y * 0.3,
        );
        let diff = spot.sub(me_pos);
        b.r#move = if diff.length() > 12.0 {
            noisy(b, diff.normalized())
        } else {
            Vec2::new(0.0, 0.0)
        };
        return;
    }

    // Opponent has it: get goal-side of the ball, contain, poke when close.
    let home_goal = Vec2::new(0.0, s.field.h / 2.0);
    let me_pos = me.pos;
    let d = me_pos.dist(s.ball);
    if d < POKE_DIST && roll(b) < 0.5 {
        b.dash = true; // commit the tackle (only sometimes: humans hesitate)
    }
    let aim = if d < DEFEND_JOCKEY_DIST {
        s.ball
    } else {
        s.ball.add(home_goal.sub(s.ball).normalized().scale(30.0))
    };
    let diff = aim.sub(me_pos);
    b.r#move = if diff.length() > 1.0 {
        noisy(b, diff.normalized())
    } else {
        Vec2::new(0.0, 0.0)
    };
    b.jockey = d < DEFEND_JOCKEY_DIST && !b.dash;
    b.sprint = d > 150.0 && s.players[s.controlled].sprint_meter > SPRINT_MIN_METER;
}

/// Produce this frame's input; call once per match tick with the same `dt`.
pub fn input(b: &mut BotState, s: &BotMatchView, dt: f64, tune: &Tuning) -> MatchInput {
    // Finish an in-flight charged shot before considering anything else.
    if b.charge_t > 0.0 {
        b.charge_t -= dt;
        if s.owner != Some(s.controlled) {
            b.charge_t = 0.0; // lost the ball mid-charge: abort
        } else if b.charge_t > 0.0 {
            return MatchInput {
                r#move: b.r#move,
                shoot: false,
                shoot_held: true,
                pass: false,
                pass_held: false,
                switch: false,
                dash: false,
                dodge: false,
                lob: b.shot_lob,
                sprint: false,
                jockey: false,
                aerial_strike: Some(false),
                aerial_acrobatic: Some(false),
                equipment_held: false,
                equipment_pressed: false,
                equipment_released: false,
            };
        } else {
            b.decide_t = b.reaction; // shot away: give it a beat before reacting
            return MatchInput {
                r#move: b.r#move,
                shoot: true,
                shoot_held: false,
                pass: false,
                pass_held: false,
                switch: false,
                dash: false,
                dodge: false,
                lob: b.shot_lob,
                sprint: false,
                jockey: false,
                aerial_strike: Some(false),
                aerial_acrobatic: Some(false),
                equipment_held: false,
                equipment_pressed: false,
                equipment_released: false,
            };
        }
    }

    // Deliberate long pass: hold to select range, then release with the same
    // aim and optional loft modifier. Losing the ball aborts the action.
    if b.pass_charge_t > 0.0 {
        b.pass_charge_t -= dt;
        if s.owner != Some(s.controlled) {
            b.pass_charge_t = 0.0;
            b.pass_lob = false;
        } else if b.pass_charge_t > 0.0 {
            return MatchInput {
                r#move: b.r#move,
                shoot: false,
                shoot_held: false,
                pass: false,
                pass_held: true,
                switch: false,
                dash: false,
                dodge: false,
                lob: b.pass_lob,
                sprint: false,
                jockey: false,
                aerial_strike: Some(false),
                aerial_acrobatic: Some(false),
                equipment_held: false,
                equipment_pressed: false,
                equipment_released: false,
            };
        } else {
            b.decide_t = b.reaction;
            let lob = b.pass_lob;
            b.pass_lob = false;
            return MatchInput {
                r#move: b.r#move,
                shoot: false,
                shoot_held: false,
                pass: true,
                pass_held: false,
                switch: false,
                dash: false,
                dodge: false,
                lob,
                sprint: false,
                jockey: false,
                aerial_strike: Some(false),
                aerial_acrobatic: Some(false),
                equipment_held: false,
                equipment_pressed: false,
                equipment_released: false,
            };
        }
    }

    b.decide_t -= dt;
    if b.decide_t <= 0.0 {
        b.decide_t = b.reaction * (0.75 + roll(b) * 0.5); // jittered cadence
        decide(b, s, tune);
    }

    let (pass, dash, dodge, lob) = (b.pass, b.dash, b.dodge, b.lob);
    // one-frame actions
    b.pass = false;
    b.dash = false;
    b.dodge = false;
    b.lob = false;
    MatchInput {
        r#move: b.r#move,
        shoot: false,
        shoot_held: false,
        pass,
        pass_held: false,
        switch: false,
        dash,
        dodge,
        lob,
        sprint: b.sprint,
        jockey: b.jockey,
        aerial_strike: Some(b.aerial_strike),
        aerial_acrobatic: Some(b.aerial_acrobatic),
        equipment_held: false,
        equipment_pressed: false,
        equipment_released: false,
    }
}
