//! Pure aerial-contact geometry and outcome resolution. This module knows no
//! match state or rendering; `sim.match` gathers candidates and applies
//! results.
//!
//! [`resolve_play`] and everything it calls read and mutate
//! [`crate::match_snapshot::MatchState`]/[`crate::match_snapshot::MatchPlayer`]/
//! [`crate::match_snapshot::MatchInput`] directly, rather than a narrow local
//! view — the functions here need nearly the entire shape anyway, so a
//! separate duplicate would buy nothing. The one wrinkle: `match_snapshot`
//! indexes `players` with **one-based** `controlled`/`owner` fields (this
//! crate's player-identity convention — see `match_snapshot`'s and
//! `r#match`'s module docs), while this module's own loops walk
//! `MatchState::players` with ordinary Rust `enumerate()` — zero-based, like
//! any `Vec`. The two never conflict: a `Vec` index and a "one-based player
//! identity" are different axes, and the *only* place this module compares
//! one against the other is [`is_human_player`], which converts explicitly.
//!
//! - [`crate::tuning::Tuning`] is an explicit owned value, never a shared
//!   mutable global (AGENTS.md §3 forbids stray global state). Every
//!   function here that needs a tuning knob takes an explicit `&Tuning`
//!   parameter.
//! - `move` is a Rust keyword, so [`crate::match_snapshot::MatchInput::r#move`]
//!   uses a raw identifier — the same convention `gc_sim::r#match` itself
//!   uses for the module name (see `lib.rs`).

use crate::match_snapshot::{
    MatchEvent, MatchEventKind, MatchInput, MatchPlayer, MatchState, Team,
};
use crate::species;
use crate::tuning::Tuning;
use gc_core::deterministic_math;
use gc_core::rng;
use gc_core::vec2::Vec2;

/// What a player is attempting against the ball in the air.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AerialIntent {
    /// Receiving/settling a dropping ball.
    Receive,
    /// A first-time strike at goal (volley or header).
    Strike,
    /// An acrobatic (bicycle-kick) attempt.
    Acrobatic,
}

/// The physical technique used to make contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AerialStyle {
    /// Standing leg control of a low, dropping ball.
    LegControl,
    /// Chest control of a mid-height ball.
    ChestControl,
    /// A first-time volley.
    Volley,
    /// A header.
    Header,
    /// An acrobatic bicycle kick.
    Bicycle,
}

/// The result of a resolved aerial contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AerialOutcome {
    /// A clean, well-executed contact.
    Clean,
    /// A heavy, mishit contact.
    Heavy,
    /// No contact at all.
    Miss,
}

/// The situation an aerial contact is evaluated against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AerialContext {
    /// Ball ground position.
    pub ball_pos: Vec2,
    /// Ball horizontal velocity.
    pub ball_vel: Vec2,
    /// Ball height above the pitch.
    pub ball_z: f64,
    /// Ball vertical velocity.
    pub ball_vz: f64,
    /// Player position.
    pub player_pos: Vec2,
    /// Player velocity.
    pub player_vel: Vec2,
    /// Player facing direction.
    pub facing: Vec2,
    /// Player move speed, px/s.
    pub move_speed: f64,
    /// Contact skill for the style being evaluated.
    pub skill: f64,
    /// Player strength, normalized 0..1.
    pub strength: f64,
    /// Distance to the nearest opponent, in pixels.
    pub opponent_distance: f64,
    /// Whether the player anticipated this ball (running onto a pass).
    pub anticipated: bool,
    /// Player instability, normalized 0..1.
    pub instability: f64,
    /// Extra contact reach, in pixels.
    pub extra_reach: f64,
    /// Extra jump lift, in pixels.
    pub extra_lift: f64,
}

/// A feasible aerial contact and its evaluated difficulty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AerialContact {
    /// The technique used.
    pub style: AerialStyle,
    /// Whether the player leaves the ground for this contact.
    pub jumping: bool,
    /// Required jump lift, in pixels.
    pub jump_lift: f64,
    /// Jump lift as a fraction of the style's maximum, 0..1.
    pub jump_ratio: f64,
    /// Evaluated difficulty, 0..1.
    pub difficulty: f64,
    /// Effective reach for this contact, in pixels.
    pub reach: f64,
    /// Stretch distance as a fraction of reach, 0..1.
    pub reach_ratio: f64,
}

/// The resolved outcome of one aerial contact attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AerialResolution {
    /// The contact that was resolved.
    pub contact: AerialContact,
    /// The outcome.
    pub outcome: AerialOutcome,
    /// The advanced RNG state, to be threaded back into the caller's state.
    pub rng: u32,
    /// Angular error applied to the resulting direction.
    pub angle_error: f64,
    /// Weight (power) error applied to the resulting strike.
    pub weight_error: f64,
    /// Probability contact was made at all.
    pub contact_probability: f64,
    /// Probability the contact was clean, given contact was made.
    pub clean_probability: f64,
}

/// Per-style geometry and base difficulty.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AerialStyleConfig {
    style: AerialStyle,
    min_z: f64,
    max_z: f64,
    max_jump: f64,
    reach: f64,
    base_difficulty: f64,
}

const LEG_CONTROL: AerialStyleConfig = AerialStyleConfig {
    style: AerialStyle::LegControl,
    min_z: 18.0,
    max_z: 42.0,
    max_jump: 22.0,
    reach: 28.0,
    base_difficulty: 0.0,
};
const CHEST_CONTROL: AerialStyleConfig = AerialStyleConfig {
    style: AerialStyle::ChestControl,
    min_z: 34.0,
    max_z: 62.0,
    max_jump: 24.0,
    reach: 26.0,
    base_difficulty: 0.05,
};
const VOLLEY: AerialStyleConfig = AerialStyleConfig {
    style: AerialStyle::Volley,
    min_z: 18.0,
    max_z: 38.0,
    max_jump: 16.0,
    reach: 30.0,
    base_difficulty: 0.12,
};
const HEADER: AerialStyleConfig = AerialStyleConfig {
    style: AerialStyle::Header,
    min_z: 44.0,
    max_z: 72.0,
    max_jump: 30.0,
    reach: 30.0,
    base_difficulty: 0.06,
};
const BICYCLE: AerialStyleConfig = AerialStyleConfig {
    style: AerialStyle::Bicycle,
    min_z: 38.0,
    max_z: 68.0,
    max_jump: 24.0,
    reach: 24.0,
    base_difficulty: 0.28,
};

const ALL_STYLES: [AerialStyleConfig; 5] = [LEG_CONTROL, CHEST_CONTROL, VOLLEY, HEADER, BICYCLE];

const ANTICIPATION_BONUS: f64 = 0.08;
const PRESSURE_RADIUS: f64 = 64.0;
const JUMP_POSE_THRESHOLD: f64 = 4.0;
const BICYCLE_BEHIND_COS: f64 = 0.15;
const BICYCLE_OVERHEAD_DIST: f64 = 8.0;
const HUMAN_ASSIST_REACH: f64 = 12.0;
const AERIAL_LOCK_TIME: f64 = 0.1;
const AERIAL_RECEIVE_TIME: f64 = 0.75;
const AERIAL_CD: f64 = 0.5;
const RECOVERY_RECEIVE: f64 = 0.18;
const RECOVERY_STAND: f64 = 0.22;
const RECOVERY_JUMP: f64 = 0.35;
const RECOVERY_BICYCLE: f64 = 0.6;
const BICYCLE_SPEED: f64 = 1.4;

// The grounded first-touch shot (#623): the volley verb evaluated at the
// moment the collection check would hand a designated receiver the ball.
// `FIRST_TIME_REACH` deliberately exceeds the collection radius that decides
// the moment (POSSESS_DIST, 22), so a receiver who is close enough to trap is
// always close enough to swing — the branch never half-fires on geometry.
const FIRST_TIME_BASE_DIFFICULTY: f64 = 0.10;
const FIRST_TIME_REACH: f64 = 26.0;
/// Shot-speed multiplier of a grounded first-time shot; the volley's grounded
/// sibling (`AerialMatchConfig::volley_speed`, 1.3), a touch under it because
/// the striker keeps both feet.
const FIRST_TIME_SPEED: f64 = 1.15;
/// Fraction of the arriving pass's pace carried into the shot: a one-timer
/// off a driven ball is faster than one off a dying roll — and the driven
/// ball is also *harder* (relative pace feeds difficulty), which is the
/// risk/reward that makes the verb situational rather than dominant.
const FIRST_TIME_PACE_CARRY: f64 = 0.2;

/// The height above which NO style can touch the ball, even at a full jump —
/// a flight that stays above this over an opponent simply cannot be attacked.
/// (Species lift bonuses come on top; leave a margin when using this.)
#[must_use]
pub fn max_touch_z() -> f64 {
    let mut max_z = 0.0_f64;
    for cfg in &ALL_STYLES {
        max_z = max_z.max(cfg.max_z + cfg.max_jump);
    }
    max_z
}

fn clamp(value: f64, low: f64, high: f64) -> f64 {
    value.max(low).min(high)
}

fn normalize(value: f64, easy: f64, hard: f64) -> f64 {
    clamp((value - easy) / (hard - easy), 0.0, 1.0)
}

fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}

fn ball_facing_cos(context: &AerialContext) -> f64 {
    let to_ball = context.ball_pos.sub(context.player_pos);
    if to_ball.length() <= 1.0 {
        return 0.0;
    }
    dot(context.facing.normalized(), to_ball.normalized())
}

fn bicycle_alignment_valid(context: &AerialContext) -> bool {
    let distance = context.player_pos.dist(context.ball_pos);
    distance <= BICYCLE_OVERHEAD_DIST || ball_facing_cos(context) <= BICYCLE_BEHIND_COS
}

fn build_contact(context: &AerialContext, config: &AerialStyleConfig) -> Option<AerialContact> {
    if context.ball_z < config.min_z {
        return None;
    }
    if config.style == AerialStyle::Bicycle && !bicycle_alignment_valid(context) {
        return None;
    }

    let max_jump = config.max_jump + context.extra_lift;
    let jump_lift = (context.ball_z - config.max_z).max(0.0);
    if jump_lift > max_jump {
        return None;
    }

    let reach = config.reach + context.extra_reach;
    let distance = context.player_pos.dist(context.ball_pos);
    if distance > reach {
        return None;
    }

    let jump_ratio = if max_jump > 0.0 {
        jump_lift / max_jump
    } else {
        0.0
    };
    let reach_ratio = if reach > 0.0 { distance / reach } else { 0.0 };
    let relative_pace = context.ball_vel.sub(context.player_vel).length();
    let horizontal_difficulty = normalize(relative_pace, 80.0, 600.0);
    let drop_difficulty = normalize((-context.ball_vz).max(0.0), 80.0, 500.0);
    let facing_cos = ball_facing_cos(context);
    let alignment_error = if config.style == AerialStyle::Bicycle {
        // Ideal contact is overhead to moderately behind, around cosine -0.5.
        clamp((facing_cos + 0.5).abs() / 1.5, 0.0, 1.0)
    } else {
        clamp((1.0 - facing_cos) * 0.5, 0.0, 1.0)
    };
    let pressure = clamp(1.0 - context.opponent_distance / PRESSURE_RADIUS, 0.0, 1.0);
    let anticipation = if context.anticipated {
        ANTICIPATION_BONUS
    } else {
        0.0
    };
    let difficulty = clamp(
        config.base_difficulty
            + 0.24 * reach_ratio * reach_ratio
            + 0.18 * horizontal_difficulty
            + 0.12 * drop_difficulty
            + 0.16 * jump_ratio
            + 0.10 * alignment_error
            + 0.10 * clamp(context.instability, 0.0, 1.0)
            + 0.10 * pressure
            - anticipation,
        0.0,
        1.0,
    );

    Some(AerialContact {
        style: config.style,
        jumping: config.style == AerialStyle::Bicycle || jump_lift > JUMP_POSE_THRESHOLD,
        jump_lift,
        jump_ratio,
        difficulty,
        reach,
        reach_ratio,
    })
}

/// Every feasible contact for `intent` in this context, in style-priority
/// order (not sorted by difficulty).
#[must_use]
pub fn contacts(context: &AerialContext, intent: AerialIntent) -> Vec<AerialContact> {
    let mut out = Vec::new();
    match intent {
        AerialIntent::Receive => {
            if let Some(c) = build_contact(context, &LEG_CONTROL) {
                out.push(c);
            }
            if let Some(c) = build_contact(context, &CHEST_CONTROL) {
                out.push(c);
            }
        }
        AerialIntent::Strike => {
            if let Some(c) = build_contact(context, &VOLLEY) {
                out.push(c);
            }
            if let Some(c) = build_contact(context, &HEADER) {
                out.push(c);
            }
        }
        AerialIntent::Acrobatic => {
            if let Some(c) = build_contact(context, &BICYCLE) {
                out.push(c);
            }
            if out.is_empty() {
                if let Some(c) = build_contact(context, &VOLLEY) {
                    out.push(c);
                }
                if let Some(c) = build_contact(context, &HEADER) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// The easiest (lowest-difficulty) feasible contact for `intent`, or `None`
/// if nothing is reachable.
#[must_use]
pub fn best_contact(context: &AerialContext, intent: AerialIntent) -> Option<AerialContact> {
    let mut best: Option<AerialContact> = None;
    for contact in contacts(context, intent) {
        best = match best {
            None => Some(contact),
            Some(b) if contact.difficulty < b.difficulty => Some(contact),
            Some(b) => Some(b),
        };
    }
    best
}

/// Resolve one aerial contact attempt against the RNG stream. Draws exactly
/// four rolls, in this fixed order, regardless of outcome — callers on the
/// determinism path must thread `rng` through unconditionally.
#[must_use]
pub fn resolve(
    context: &AerialContext,
    contact: &AerialContact,
    rng_state: u32,
) -> AerialResolution {
    let (rng_state, roll_contact) = rng::roll(rng_state);
    let (rng_state, roll_clean) = rng::roll(rng_state);
    let (rng_state, roll_angle) = rng::roll(rng_state);
    let (rng_state, roll_weight) = rng::roll(rng_state);

    let margin = clamp(context.skill, 0.0, 1.0) - contact.difficulty;
    let contact_probability = clamp(0.82 + 0.35 * margin, 0.30, 0.995);
    let clean_probability = clamp(0.58 + 0.60 * margin, 0.08, 0.97);
    let outcome = if roll_contact >= contact_probability {
        AerialOutcome::Miss
    } else if roll_clean < clean_probability {
        AerialOutcome::Clean
    } else {
        AerialOutcome::Heavy
    };

    let acrobatic = contact.style == AerialStyle::Bicycle;
    let (max_angle, max_weight) = match outcome {
        AerialOutcome::Clean => (
            if acrobatic { 0.14 } else { 0.06 },
            if acrobatic { 0.12 } else { 0.08 },
        ),
        AerialOutcome::Heavy => (
            if acrobatic { 0.9 } else { 0.55 },
            if acrobatic { 0.6 } else { 0.45 },
        ),
        AerialOutcome::Miss => (0.0, 0.0),
    };

    AerialResolution {
        contact: *contact,
        outcome,
        rng: rng_state,
        angle_error: (roll_angle * 2.0 - 1.0) * max_angle,
        weight_error: (roll_weight * 2.0 - 1.0) * max_weight,
        contact_probability,
        clean_probability,
    }
}

/// How attractive it is for a player to claim this contact over another
/// candidate's, higher is more attractive.
#[must_use]
pub fn claim_score(
    context: &AerialContext,
    contact: &AerialContact,
    intent_bonus: f64,
    jump_edge: f64,
    jitter: f64,
) -> f64 {
    let position_quality = clamp(1.0 - contact.reach_ratio, 0.0, 1.0);
    0.45 * position_quality
        + 0.35 * clamp(context.skill, 0.0, 1.0)
        + 0.10 * clamp(context.strength, 0.0, 1.0)
        + intent_bonus
        + jump_edge
        + clamp(jitter, -1.0, 1.0) * 0.04
}

// --- Match glue -------------------------------------------------------
//
// Everything below reads/mutates match state, via `match_snapshot`'s
// canonical `MatchState`/`MatchPlayer`/`MatchInput`/`MatchEvent` — see the
// module doc.

/// A neutral (no input) `MatchInput`, used when a player has no live input
/// for the frame. `aerial_strike`/`aerial_acrobatic` are explicit
/// `Some(false)` rather than left unset, so a neutral input is
/// unambiguously "not attempting an aerial strike" rather than "unknown."
#[must_use]
fn neutral_input() -> MatchInput {
    MatchInput {
        aerial_strike: Some(false),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

/// Tunable match constants [`resolve_play`] needs but does not own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AerialMatchConfig {
    /// Ball height below which it counts as grounded.
    pub ground_grab_height: f64,
    /// How far ahead of the receiver a settled touch is placed.
    pub stick_ahead: f64,
    /// Downward acceleration used by reception's landing-time solve.
    pub gravity: f64,
    /// Seconds before the ball can be picked up after a strike.
    pub release_cd: f64,
    /// Outgoing ball speed for a defensive header clearance.
    pub clear_header_speed: f64,
    /// Volley shot-speed multiplier.
    pub volley_speed: f64,
}

/// A resolved candidate for this frame's aerial contact.
struct MatchAerialCandidate {
    index: usize,
    context: AerialContext,
    contact: AerialContact,
    intent: AerialIntent,
    score: f64,
}

fn is_human_player(s: &MatchState, player_idx: usize) -> bool {
    if s.slot_mode {
        return s
            .slot_for_player
            .get(player_idx)
            .copied()
            .flatten()
            .is_some();
    }
    // `s.controlled` is one-based (`match_snapshot`'s convention); `player_idx`
    // is an ordinary zero-based `Vec` position — see the module doc.
    s.human_controlled && (player_idx as i64 + 1) == s.controlled
}

/// Whether `input.aerial_strike`, or the fallback `jockey || dash`, requests
/// a first-time strike this frame.
#[must_use]
pub fn strike_requested(input: &MatchInput) -> bool {
    if let Some(v) = input.aerial_strike {
        return v;
    }
    input.jockey || input.dash
}

/// Whether `input.aerial_acrobatic`, or the fallback `lob && strike_requested`,
/// requests an acrobatic attempt this frame.
#[must_use]
pub fn acrobatic_requested(input: &MatchInput) -> bool {
    if let Some(v) = input.aerial_acrobatic {
        return v;
    }
    input.lob && strike_requested(input)
}

fn nearest_opponent(s: &MatchState, player: &MatchPlayer) -> f64 {
    let mut nearest = f64::INFINITY;
    for opponent in &s.players {
        if opponent.team != player.team {
            nearest = nearest.min(player.pos.dist(opponent.pos));
        }
    }
    nearest
}

fn style_skill(player: &MatchPlayer, style: AerialStyle) -> f64 {
    match style {
        AerialStyle::LegControl | AerialStyle::ChestControl => player.first_touch,
        AerialStyle::Header => player.header_skill,
        AerialStyle::Volley => player.volley_skill,
        AerialStyle::Bicycle => player.bicycle_skill,
    }
}

fn make_match_context(
    s: &MatchState,
    player_idx: usize,
    skill: f64,
    tune: &Tuning,
) -> AerialContext {
    let player = &s.players[player_idx];
    let human_assist = if is_human_player(s, player_idx) {
        HUMAN_ASSIST_REACH.min(tune.value("AERIAL_ASSIST"))
    } else {
        0.0
    };
    let mut instability = clamp(player.run_vel.length() / player.move_speed, 0.0, 1.0) * 0.6;
    if player.sprinting {
        instability = (instability + 0.25).min(1.0);
    }
    AerialContext {
        ball_pos: s.ball,
        ball_vel: s.ball_vel,
        ball_z: s.ball_z,
        ball_vz: s.ball_vz,
        player_pos: player.pos,
        player_vel: player.vel,
        facing: player.facing,
        move_speed: player.move_speed,
        skill,
        strength: player.strength,
        opponent_distance: nearest_opponent(s, player),
        anticipated: player.receive_timer > 0.0,
        instability,
        extra_reach: human_assist + species::jump_reach(player.owned_verb),
        extra_lift: species::jump_lift(player.owned_verb),
    }
}

fn contact_for_intent(
    s: &MatchState,
    player_idx: usize,
    intent: AerialIntent,
    tune: &Tuning,
) -> (Option<AerialContext>, Option<AerialContact>) {
    let player = &s.players[player_idx];
    let mut context = make_match_context(s, player_idx, player.first_touch, tune);
    let contact = best_contact(&context, intent);
    match contact {
        None => (None, None),
        Some(c) => {
            context.skill = style_skill(player, c.style);
            (Some(context), Some(c))
        }
    }
}

fn own_third(field_w: f64, team: Team, pos_x: f64) -> bool {
    match team {
        Team::Home => pos_x < field_w * 0.33,
        Team::Away => pos_x > field_w * 0.67,
    }
}

fn match_intent(
    s: &MatchState,
    player_idx: usize,
    inputs: &[Option<MatchInput>],
    tune: &Tuning,
) -> AerialIntent {
    let player = &s.players[player_idx];
    if is_human_player(s, player_idx) {
        let input = inputs
            .get(player_idx)
            .cloned()
            .flatten()
            .unwrap_or_else(neutral_input);
        if strike_requested(&input) {
            return if acrobatic_requested(&input) {
                AerialIntent::Acrobatic
            } else {
                AerialIntent::Strike
            };
        }
        return AerialIntent::Receive;
    }

    let goal = match player.team {
        Team::Home => s.goal_away,
        Team::Away => s.goal_home,
    };
    let goal_line_x = match player.team {
        Team::Home => goal.x,
        Team::Away => goal.x + goal.w,
    };
    let goal_line = Vec2::new(goal_line_x, goal.y + goal.h / 2.0);
    if own_third(s.field.w, player.team, player.pos.x)
        || player.pos.dist(goal_line) <= tune.value("AI_HEADER_RANGE")
    {
        return AerialIntent::Strike;
    }
    AerialIntent::Receive
}

fn choose_candidate(
    s: &MatchState,
    inputs: &[Option<MatchInput>],
    ineligible: Option<&[bool]>,
    tune: &Tuning,
) -> Option<MatchAerialCandidate> {
    let mut best: Option<MatchAerialCandidate> = None;
    for (i, player) in s.players.iter().enumerate() {
        let is_ineligible = ineligible
            .and_then(|arr| arr.get(i).copied())
            .unwrap_or(false);
        if is_ineligible
            || player.is_keeper
            || player.header_cd > 0.0
            || player.aerial_recovery > 0.0
            || player.stun_timer > 0.0
            || player.slide_timer > 0.0
            || player.dodge_timer > 0.0
        {
            continue;
        }

        let mut intent = match_intent(s, i, inputs, tune);
        let (mut context, mut contact) = contact_for_intent(s, i, intent, tune);

        if intent == AerialIntent::Strike && !is_human_player(s, i) {
            let (acro_context, acro_contact) =
                contact_for_intent(s, i, AerialIntent::Acrobatic, tune);
            if let (Some(ac), Some(acc)) = (acro_context, acro_contact)
                && acc.style == AerialStyle::Bicycle
            {
                let acro_margin = ac.skill - acc.difficulty;
                let normal_margin = match (context, contact) {
                    (Some(c), Some(cc)) => c.skill - cc.difficulty,
                    _ => f64::NEG_INFINITY,
                };
                if acro_margin > normal_margin {
                    intent = AerialIntent::Acrobatic;
                    context = Some(ac);
                    contact = Some(acc);
                }
            }
        }

        if let (Some(context), Some(contact)) = (context, contact) {
            let mut intent_bonus = if player.receive_timer > 0.0 {
                0.12
            } else {
                0.0
            };
            if is_human_player(s, i) && intent != AerialIntent::Receive {
                intent_bonus += 0.08;
            }
            let score = claim_score(&context, &contact, intent_bonus, 0.0, 0.0);
            let replace = match &best {
                None => true,
                Some(b) => {
                    score > b.score || (score == b.score && player.id < s.players[b.index].id)
                }
            };
            if replace {
                best = Some(MatchAerialCandidate {
                    index: i,
                    context,
                    contact,
                    intent,
                    score,
                });
            }
        }
    }
    best
}

fn rotate_vector(vector: Vec2, angle: f64) -> Vec2 {
    let (ca, sa) = deterministic_math::cos_sin(angle);
    Vec2::new(vector.x * ca - vector.y * sa, vector.x * sa + vector.y * ca)
}

fn begin_action(
    player: &mut MatchPlayer,
    contact: &AerialContact,
    outcome: AerialOutcome,
    intent: AerialIntent,
) {
    let recovery = if contact.style == AerialStyle::Bicycle {
        RECOVERY_BICYCLE
    } else if intent == AerialIntent::Receive {
        if contact.jumping {
            RECOVERY_JUMP
        } else {
            RECOVERY_RECEIVE
        }
    } else if contact.jumping {
        RECOVERY_JUMP
    } else {
        RECOVERY_STAND
    };
    player.header_cd = AERIAL_CD;
    player.aerial_timer = recovery;
    player.aerial_style = Some(contact.style);
    player.aerial_outcome = Some(outcome);
    player.aerial_jump = contact.jump_ratio;
    player.aerial_recovery = recovery;
}

fn apply_reception(
    s: &mut MatchState,
    candidate: &MatchAerialCandidate,
    inputs: &[Option<MatchInput>],
    resolution: &AerialResolution,
    config: &AerialMatchConfig,
) {
    let idx = candidate.index;
    let input = inputs
        .get(idx)
        .cloned()
        .flatten()
        .unwrap_or_else(neutral_input);

    s.events.push(MatchEvent {
        kind: MatchEventKind::Reception,
        x: s.ball.x,
        y: s.ball.y,
        player: Some(s.players[idx].id.clone()),
        save_style: None,
        style: Some(candidate.contact.style),
        outcome: Some(resolution.outcome),
        jumping: Some(candidate.contact.jumping),
        difficulty: Some(candidate.contact.difficulty),
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });

    if resolution.outcome == AerialOutcome::Miss {
        s.players[idx].receive_timer = 0.0;
        s.players[idx].receive_target = None;
        return;
    }

    for i in 0..s.players.len() {
        if i != idx {
            s.players[i].receive_timer = 0.0;
            s.players[i].receive_target = None;
        }
    }
    s.players[idx].receive_timer = s.players[idx].receive_timer.max(AERIAL_RECEIVE_TIME);

    let mut aim = s.players[idx].facing;
    if is_human_player(s, idx) && (input.r#move.x != 0.0 || input.r#move.y != 0.0) {
        aim = input.r#move.normalized();
    }
    let mut duration = if candidate.contact.style == AerialStyle::ChestControl {
        0.22
    } else {
        0.12
    };
    if resolution.outcome == AerialOutcome::Heavy {
        duration *= 1.35;
    }
    let player = &s.players[idx];
    let target = player
        .pos
        .add(player.vel.scale(duration))
        .add(aim.scale(config.stick_ahead));
    let travel = rotate_vector(target.sub(s.ball), resolution.angle_error);
    let weight = clamp(1.0 + resolution.weight_error, 0.45, 1.45);
    s.ball_vel = travel.scale(weight / duration);
    s.ball_vz = (0.5 * config.gravity * duration * duration - s.ball_z) / duration;
    s.ball_spin = 0.0;
}

/// Where an attacking strike is aimed: a human's held stick direction if
/// any, otherwise the far half of the opposing goal mouth, biased away
/// from the side the keeper stands on.
fn attacking_strike_target(s: &MatchState, idx: usize, input: &MatchInput) -> Vec2 {
    let player_team = s.players[idx].team;
    let player_pos = s.players[idx].pos;
    if is_human_player(s, idx) && (input.r#move.x != 0.0 || input.r#move.y != 0.0) {
        return player_pos.add(input.r#move.normalized().scale(240.0));
    }
    let goal = match player_team {
        Team::Home => s.goal_away,
        Team::Away => s.goal_home,
    };
    let keeper = s
        .players
        .iter()
        .find(|o| o.team != player_team && o.is_keeper);
    let vbias = match keeper {
        Some(k) if k.pos.y < goal.y + goal.h / 2.0 => 0.85,
        Some(_) => -0.85,
        None => 0.85,
    };
    let goal_x = if player_team == Team::Home {
        goal.x
    } else {
        goal.x + goal.w
    };
    let half = goal.h / 2.0 - 8.0;
    Vec2::new(goal_x, goal.y + goal.h / 2.0 + vbias * half)
}

fn apply_strike(
    s: &mut MatchState,
    candidate: &MatchAerialCandidate,
    inputs: &[Option<MatchInput>],
    resolution: &AerialResolution,
    config: &AerialMatchConfig,
    tune: &Tuning,
) {
    let idx = candidate.index;
    let input = inputs
        .get(idx)
        .cloned()
        .flatten()
        .unwrap_or_else(neutral_input);
    let style = candidate.contact.style;
    assert!(
        matches!(
            style,
            AerialStyle::Header | AerialStyle::Volley | AerialStyle::Bicycle
        ),
        "strike style"
    );
    let kind = match style {
        AerialStyle::Header => MatchEventKind::Header,
        AerialStyle::Volley => MatchEventKind::Volley,
        _ => MatchEventKind::Bicycle,
    };
    s.events.push(MatchEvent {
        kind,
        x: s.ball.x,
        y: s.ball.y,
        player: Some(s.players[idx].id.clone()),
        save_style: None,
        style: Some(style),
        outcome: Some(resolution.outcome),
        jumping: Some(candidate.contact.jumping),
        difficulty: Some(candidate.contact.difficulty),
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    for other in &mut s.players {
        other.receive_timer = 0.0;
        other.receive_target = None;
    }
    if resolution.outcome == AerialOutcome::Miss {
        return;
    }

    let player_team = s.players[idx].team;
    let player_pos = s.players[idx].pos;
    let defensive = own_third(s.field.w, player_team, player_pos.x) && !is_human_player(s, idx);
    let target = if defensive {
        player_pos.add(Vec2::new(
            if player_team == Team::Home {
                240.0
            } else {
                -240.0
            },
            if s.ball.y < s.field.h / 2.0 {
                -72.0
            } else {
                72.0
            },
        ))
    } else {
        attacking_strike_target(s, idx, &input)
    };

    let direction = rotate_vector(target.sub(player_pos).normalized(), resolution.angle_error);
    let shot_speed = s.players[idx].shot_speed;
    let (mut speed, vz) = if defensive {
        (
            config.clear_header_speed,
            if resolution.outcome == AerialOutcome::Clean {
                260.0
            } else {
                340.0
            },
        )
    } else if style == AerialStyle::Header {
        (
            shot_speed * tune.value("HEADER_SPEED"),
            if resolution.outcome == AerialOutcome::Clean {
                -40.0
            } else {
                100.0
            },
        )
    } else if style == AerialStyle::Volley {
        (
            shot_speed * config.volley_speed,
            if resolution.outcome == AerialOutcome::Clean {
                40.0
            } else {
                260.0 + resolution.angle_error.abs() * 300.0
            },
        )
    } else {
        (
            shot_speed * BICYCLE_SPEED,
            if resolution.outcome == AerialOutcome::Clean {
                30.0
            } else {
                360.0 + resolution.angle_error.abs() * 240.0
            },
        )
    };
    if resolution.outcome == AerialOutcome::Heavy {
        speed *= 0.65;
    }
    speed *= clamp(1.0 + resolution.weight_error, 0.4, 1.35);
    s.ball_vel = direction.scale(speed);
    s.ball_vz = vz;
    s.ball_spin = 0.0;
    s.pickup_cd = config.release_cd * 0.6;
}

/// Resolve this frame's aerial play, if any: pick the best-placed eligible
/// candidate, resolve the contact against `s.rng`, apply the result to the
/// ball, and record an event. Returns whether the ball's trajectory changed
/// (`false` on no candidate or a miss).
pub fn resolve_play(
    s: &mut MatchState,
    inputs: &[Option<MatchInput>],
    config: &AerialMatchConfig,
    ineligible: Option<&[bool]>,
    tune: &Tuning,
) -> bool {
    if s.ball_z <= config.ground_grab_height
        || s.ball_vz >= 0.0
        || s.pickup_cd > 0.0
        || s.aerial_lock > 0.0
    {
        return false;
    }
    let candidate = match choose_candidate(s, inputs, ineligible, tune) {
        Some(c) => c,
        None => return false,
    };
    let resolution = resolve(&candidate.context, &candidate.contact, s.rng);
    s.rng = resolution.rng;
    begin_action(
        &mut s.players[candidate.index],
        &candidate.contact,
        resolution.outcome,
        candidate.intent,
    );
    s.aerial_lock = AERIAL_LOCK_TIME;
    if candidate.intent == AerialIntent::Receive {
        apply_reception(s, &candidate, inputs, &resolution, config);
    } else {
        apply_strike(s, &candidate, inputs, &resolution, config, tune);
    }
    resolution.outcome != AerialOutcome::Miss
}

/// Whether this receiver wants to strike the arriving ground pass first
/// time: a human holds the strike button; an AI receiver takes the shot
/// when within `AI_FIRST_TOUCH_RANGE` of the centre of the opposing goal line.
fn first_touch_requested(
    s: &MatchState,
    idx: usize,
    input: Option<&MatchInput>,
    tune: &Tuning,
) -> bool {
    if is_human_player(s, idx) {
        return match input {
            Some(input) => strike_requested(input),
            None => false,
        };
    }
    let player = &s.players[idx];
    let goal = match player.team {
        Team::Home => s.goal_away,
        Team::Away => s.goal_home,
    };
    let goal_line_x = match player.team {
        Team::Home => goal.x,
        Team::Away => goal.x + goal.w,
    };
    let goal_line = Vec2::new(goal_line_x, goal.y + goal.h / 2.0);
    player.pos.dist(goal_line) <= tune.value("AI_FIRST_TOUCH_RANGE")
}

/// Resolve a grounded first-touch shot (#623), if this receiver wants one:
/// called by collection at the exact moment it would otherwise grant the
/// designated receiver (`receive_timer > 0`) plain possession. Returns
/// whether an attempt was resolved — `true` means the collection grant must
/// be skipped this tick, INCLUDING on a miss, where the swing whiffs over
/// the ball and it runs on untouched.
///
/// The attempt is the aerial strike verb with a grounded style band: same
/// four-roll [`resolve`] against `volley_skill`, same Clean/Heavy/Miss
/// consequences, no charge state and no possession transfer anywhere —
/// which is what keeps #531's owner-only charge invariant intact.
pub fn resolve_first_touch_shot(
    s: &mut MatchState,
    idx: usize,
    input: Option<MatchInput>,
    config: &AerialMatchConfig,
    tune: &Tuning,
) -> bool {
    {
        let player = &s.players[idx];
        // `header_cd` is deliberately NOT a gate here, unlike
        // `choose_candidate`'s aerial eligibility: a pass that is dinked to
        // a receiver's feet often draws one airborne swing on the way down,
        // and that attempt's 0.5 s cooldown must not turn the ball landing
        // a beat later into a forced plain trap -- the grounded swing is
        // its own instant action, paced by `aerial_recovery` (the animation
        // actually in progress) rather than by the aerial verb's cooldown.
        if player.is_keeper
            || player.receive_timer <= 0.0
            || player.aerial_recovery > 0.0
            || player.stun_timer > 0.0
            || player.slide_timer > 0.0
            || player.dodge_timer > 0.0
        {
            return false;
        }
    }
    if !first_touch_requested(s, idx, input.as_ref(), tune) {
        return false;
    }

    let ft_config = AerialStyleConfig {
        style: AerialStyle::Volley,
        min_z: 0.0,
        max_z: config.ground_grab_height,
        max_jump: 0.0,
        reach: FIRST_TIME_REACH,
        base_difficulty: FIRST_TIME_BASE_DIFFICULTY,
    };
    let skill = style_skill(&s.players[idx], AerialStyle::Volley);
    let context = make_match_context(s, idx, skill, tune);
    let contact = match build_contact(&context, &ft_config) {
        Some(c) => c,
        None => return false,
    };
    let resolution = resolve(&context, &contact, s.rng);
    s.rng = resolution.rng;
    begin_action(
        &mut s.players[idx],
        &contact,
        resolution.outcome,
        AerialIntent::Strike,
    );
    s.aerial_lock = AERIAL_LOCK_TIME;
    s.events.push(MatchEvent {
        kind: MatchEventKind::FirstTouchShot,
        x: s.ball.x,
        y: s.ball.y,
        player: Some(s.players[idx].id.clone()),
        save_style: None,
        style: Some(AerialStyle::Volley),
        outcome: Some(resolution.outcome),
        jumping: Some(contact.jumping),
        difficulty: Some(contact.difficulty),
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    // Swinging consumes the receiver's designation either way: hit or
    // whiff, this ball is no longer theirs to trap (mirrors apply_strike).
    for other in &mut s.players {
        other.receive_timer = 0.0;
    }
    if resolution.outcome == AerialOutcome::Miss {
        // The whiff lets the ball run through the swing before anyone —
        // including an opponent — can gather it.
        s.pickup_cd = config.release_cd * 0.5;
        return true;
    }

    let input = input.unwrap_or_else(neutral_input);
    let player_pos = s.players[idx].pos;
    let target = attacking_strike_target(s, idx, &input);
    let direction = rotate_vector(target.sub(player_pos).normalized(), resolution.angle_error);
    let incoming_pace = s.ball_vel.length();
    let mut speed =
        s.players[idx].shot_speed * FIRST_TIME_SPEED + incoming_pace * FIRST_TIME_PACE_CARRY;
    if resolution.outcome == AerialOutcome::Heavy {
        speed *= 0.65;
    }
    speed *= clamp(1.0 + resolution.weight_error, 0.4, 1.35);
    let vz = if resolution.outcome == AerialOutcome::Clean {
        20.0
    } else {
        200.0 + resolution.angle_error.abs() * 240.0
    };
    s.ball_vel = direction.scale(speed);
    s.ball_vz = vz;
    s.ball_spin = 0.0;
    s.pickup_cd = config.release_cd * 0.6;
    true
}
