//! Canonical start-of-tick snapshots for `sim::match`. The explicit field
//! lists in this module are both the copy contract and the canonical
//! serialization order: adding simulation state must fail capture/tests
//! until this module is consciously versioned.
//!
//! Design note: this module declares the canonical, fully typed
//! [`MatchState`] / [`MatchPlayer`] / [`MatchEvent`] shapes — not a narrow
//! read-only view like `metrics`/`bot`. It has to: this module's entire job
//! is to capture/restore/hash *all* of `MatchState`'s shape, so a narrowed
//! view would defeat the module's purpose. Downstream modules in this layer
//! (`combat_snapshot`, `combat_identity`, `combat_observation`,
//! `combat_policy`, `combat`, `slot_input`) reuse `MatchState`/`MatchPlayer`/
//! `MatchInput` from here rather than declaring their own views — as does
//! `gc_sim::r#match` and `gc_sim::aerial`, whose own local view needed
//! nearly this entire shape (see that module's doc). `bot` and `metrics`
//! stay on their own narrow views: each touches a small, genuinely
//! different slice of match state, and giving that slice its own name next
//! to its one consumer (`bot::BotMatchView`, `metrics::MetricsMatchView`,
//! ...) avoids a shared module that would gain nothing but a longer import
//! path, since the two shapes share almost no fields.
//!
//! ## `mark_unsupported` — a deliberate design choice
//!
//! "This exact `MatchState` value may no longer be captured without its
//! combat companion" is tracked as a field **on** the value:
//! [`MatchState::unsupported_reason`], rather than via some out-of-band
//! identity-keyed table — Rust values have no identity independent of their
//! content, and faking one would mean smuggling `Rc`/interior mutability
//! into a crate that otherwise treats sim state as plain, `Clone`-able
//! data. `unsupported_reason` is never part of [`MATCH_FIELD_COUNT`]'s wire
//! contract — [`append_state`] does not touch it — so it cannot affect a
//! hash or a wire byte.

use crate::aerial::{AerialOutcome, AerialStyle};
use crate::combat_feasibility::Team as FeasibilityTeam;
use crate::combat_snapshot::{self, CanonicalScalar, CombatMatchState, CombatSnapshotWriter};
use crate::input_frame::{self, InputOwnership, SlotId};
use crate::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use crate::outfield_decision::{self, OutfieldDecisionState};
use crate::outfield_press::{self, OutfieldPressState};
use crate::possession_transition::{self, PossessionTransitionState, TransitionWindows};
use gc_core::fnv1a64;
use gc_core::vec2::Vec2;
use gc_data::formations;
use gc_data::species::SimVerb;
use gc_data::tactics::MarkingConfig;

/// Soccer-only snapshot format version.
pub const VERSION: i64 = 11;
/// Combat-companion snapshot format version.
pub const COMBAT_VERSION: i64 = 13;

/// Fixed fixture size: five players per side, ten total.
pub const PLAYER_COUNT: usize = 10;

/// A fixture side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    /// The home side.
    Home,
    /// The away side.
    Away,
}

impl Team {
    /// This team's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            Team::Home => "home",
            Team::Away => "away",
        }
    }

    /// Map to [`crate::combat_feasibility::Team`], the local team type the
    /// combat-feasibility layer's own narrow view uses.
    #[must_use]
    pub fn to_feasibility(self) -> FeasibilityTeam {
        match self {
            Team::Home => FeasibilityTeam::Home,
            Team::Away => FeasibilityTeam::Away,
        }
    }
}

/// A value paired per fixture side. The key set is exactly the two teams,
/// so this small struct replaces a map rather than reaching for a banned
/// hash table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ByTeam<T> {
    /// The home side's value.
    pub home: T,
    /// The away side's value.
    pub away: T,
}

impl<T: Clone> ByTeam<T> {
    /// This team's value.
    #[must_use]
    pub fn get(&self, team: Team) -> T {
        match team {
            Team::Home => self.home.clone(),
            Team::Away => self.away.clone(),
        }
    }
}

/// Pitch dimensions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitchSize {
    /// Width, px.
    pub w: f64,
    /// Height, px.
    pub h: f64,
}

/// An axis-aligned rectangle (a goal mouth).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// A committed save verdict awaiting ball contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePending {
    /// A clean catch.
    Catch,
    /// A parried deflection.
    Parry,
}

/// The payload captured at a shot/punt windup's commit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindupShot {
    /// Shot direction.
    pub dir: Vec2,
    /// Shot speed.
    pub speed: f64,
    /// Vertical launch speed.
    pub vz: f64,
    /// Lateral curve/spin.
    pub spin: f64,
    /// Ground or lofted.
    pub shot_type: KeeperShotType,
}

/// One player in the fixture. Field order is [`PLAYER_FIELD_COUNT`]'s
/// canonical serialization order — see the module-level `PLAYER_FIELDS`
/// walk mirrored by [`append_player`].
#[derive(Clone, Debug, PartialEq)]
pub struct MatchPlayer {
    /// Stable player identity.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Fixture side.
    pub team: Team,
    /// Current position.
    pub pos: Vec2,
    /// Realized velocity (px/s) from last tick's movement; AI prediction source.
    pub vel: Vec2,
    /// Locomotion velocity (px/s); accel/decel toward desired each tick.
    pub run_vel: Vec2,
    /// Facing direction.
    pub facing: Vec2,
    /// Authored formation anchor.
    pub anchor: Vec2,
    /// Showcase species identity.
    pub species_id: String,
    /// This species' currently owned simulation verb.
    pub owned_verb: SimVerb,
    /// Base locomotion speed, px/s.
    pub move_speed: f64,
    /// Shot pace.
    pub shot_speed: f64,
    /// 0..1 ball control.
    pub dribble: f64,
    /// 0..1, used by aerial contests.
    pub strength: f64,
    /// 0..1 aerial reception quality.
    pub first_touch: f64,
    /// 0..1 header contact quality.
    pub header_skill: f64,
    /// 0..1 volley contact quality.
    pub volley_skill: f64,
    /// 0..1 bicycle-kick contact quality.
    pub bicycle_skill: f64,
    /// 0..1 cadence refresh rate derived from the canonical stats.
    pub scan_rate: f64,
    /// 0..1 carrier-choice sharpness derived from mental.
    pub composure: f64,
    /// Serializable retained AI intent/cadence.
    pub outfield_decision: OutfieldDecisionState,
    /// Whether this player is the keeper.
    pub is_keeper: bool,
    /// Collision radius, px.
    pub radius: f64,
    /// AI tackle cooldown (seconds until it can challenge again).
    pub dash_cd: f64,
    /// Seconds until a juke is ready again.
    pub dodge_cd: f64,
    /// Seconds of juke (sidestep + tackle immunity) remaining.
    pub dodge_timer: f64,
    /// Sidestep direction for the active juke.
    pub dodge_dir: Vec2,
    /// Keeper dive/save radius in px (0 for outfield).
    pub reach: f64,
    /// Keeper clean-catch factor 0..1 (0 for outfield).
    pub handling: f64,
    /// Positioning depth in px (0 for outfield).
    pub keeper_aggression: f64,
    /// 0..1 shot-reading quality (0 for outfield).
    pub keeper_anticipation: f64,
    /// Visible positioning/reaction state.
    pub keeper_state: KeeperBehaviorState,
    /// Seconds left in the current timed state.
    pub keeper_state_timer: f64,
    /// State captured at the latest shot release.
    pub keeper_release_state: Option<KeeperBehaviorState>,
    /// Normalized locomotion captured at shot release.
    pub keeper_release_motion: f64,
    /// Latest inbound shot type.
    pub keeper_release_kind: Option<KeeperShotType>,
    /// Line depth captured at shot release.
    pub keeper_release_depth: f64,
    /// Seconds the pre-release set/lean has been visible (0 = inactive).
    pub keeper_set: f64,
    /// Seconds of keeper dive lunge remaining.
    pub dive_timer: f64,
    /// Unit direction of the active dive.
    pub dive_dir: Vec2,
    /// Countdown until a queued dive launches (synced to ball arrival).
    pub dive_delay: f64,
    /// Intercept point the dive converges on (movement stops there).
    pub dive_target: Option<Vec2>,
    /// Seconds of the post-dive push-back-up pose remaining.
    pub keeper_get_up_timer: f64,
    /// Seconds a keeper holds the ball before distributing.
    pub hold_timer: f64,
    /// Keeper took a teammate's pass with the feet (no hands: dribbles, tackleable).
    pub feet_ball: bool,
    /// Seconds of an active slide tackle remaining.
    pub slide_timer: f64,
    /// Locked travel direction of the slide.
    pub slide_dir: Vec2,
    /// Current slide speed (px/s), decays over the slide.
    pub slide_vel: f64,
    /// Seconds of an active standing-tackle poke window remaining.
    pub tackle_timer: f64,
    /// Recovery before another tackle/slide can start.
    pub tackle_cd: f64,
    /// Seconds knocked off balance (slowed, can't tackle).
    pub stun_timer: f64,
    /// Keeper gather/reach pose remaining (visual).
    pub grab_timer: f64,
    /// Keeper release/throw pose remaining (visual).
    pub throw_timer: f64,
    /// Seconds this player is running onto an incoming pass.
    pub receive_timer: f64,
    /// 0..1 stamina tank for sprinting.
    pub sprint_meter: f64,
    /// Seconds a full tank lasts (stamina-derived).
    pub sprint_dur: f64,
    /// Currently sprint-boosted (hysteresis state).
    pub sprinting: bool,
    /// Committed save verdict awaiting ball contact.
    pub save_pending: Option<SavePending>,
    /// Backstop countdown to force-resolve a pending save.
    pub save_timer: f64,
    /// Shot x-velocity at commit (sign flip = deflected, dive whiffs).
    pub save_vx: f64,
    /// Deterministic presentation style for the active attempt.
    pub save_style: Option<SaveStyle>,
    /// One-shot guard for the current released shot.
    pub save_tip_emitted: bool,
    /// AI first-touch control window (no pressured pass until settled).
    pub settle_timer: f64,
    /// Cooldown between aerial (header/volley) attempts.
    pub header_cd: f64,
    /// Transient aerial pose timer.
    pub aerial_timer: f64,
    /// Physical technique used, if mid-attempt.
    pub aerial_style: Option<AerialStyle>,
    /// Resolved aerial outcome, if any.
    pub aerial_outcome: Option<AerialOutcome>,
    /// 0..1 required lift for rendering.
    pub aerial_jump: f64,
    /// Movement/action recovery after an aerial attempt.
    pub aerial_recovery: f64,
    /// This player's shot/punt charge, 0..1.
    pub charge: f64,
    /// This player's pass-range charge, 0..1.
    pub pass_charge: f64,
    /// Player this owner would pass to if released now.
    pub pass_target: Option<i64>,
    /// Seconds until a pending shot/punt releases (0 = none).
    pub windup_timer: f64,
    /// Payload captured at commit.
    pub windup_shot: Option<WindupShot>,
    /// Seconds of active jockey stance remaining (grants bonus poke reach).
    pub jockey_timer: f64,
}

/// The closed, versioned `MatchEvent.kind` vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchEventKind {
    /// A shot at goal.
    Shot,
    /// A pass.
    Pass,
    /// A dribble touch.
    Touch,
    /// A tackle attempt.
    Tackle,
    /// A press commit: the carrier took a heavy touch.
    PressCommitHeavyTouch,
    /// A press commit: the ball is exposed.
    PressCommitExposedBall,
    /// A press commit: covered by a teammate.
    PressCommitCover,
    /// A press commit: box desperation.
    PressCommitBoxDesperation,
    /// A press commit: discipline slipped below threshold.
    PressCommitLowDiscipline,
    /// A gameplay-AI combat commit: carrier contest.
    CombatCommitCarrierContest,
    /// A gameplay-AI combat commit: carrier protection.
    CombatCommitCarrierProtection,
    /// A gameplay-AI combat commit: loose-ball contest.
    CombatCommitLooseBallContest,
    /// A gameplay-AI combat commit: passing-lane or shot denial.
    CombatCommitPassingLaneOrShotDenial,
    /// A gameplay-AI combat commit: recovery punish.
    CombatCommitRecoveryPunish,
    /// A gameplay-AI combat commit: unattributed off-ball.
    CombatCommitUnattributedOffBall,
    /// A keeper catch.
    Catch,
    /// A keeper parry.
    Parry,
    /// A keeper tip.
    Tip,
    /// A keeper claim.
    Claim,
    /// A body block.
    Block,
    /// A header contact.
    Header,
    /// A volley contact.
    Volley,
    /// A bicycle-kick contact.
    Bicycle,
    /// A pass reception.
    Reception,
    /// A dodge/juke.
    Juke,
}

impl MatchEventKind {
    /// This kind's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        use MatchEventKind::*;
        match self {
            Shot => "shot",
            Pass => "pass",
            Touch => "touch",
            Tackle => "tackle",
            PressCommitHeavyTouch => "press_commit_heavy_touch",
            PressCommitExposedBall => "press_commit_exposed_ball",
            PressCommitCover => "press_commit_cover",
            PressCommitBoxDesperation => "press_commit_box_desperation",
            PressCommitLowDiscipline => "press_commit_low_discipline",
            CombatCommitCarrierContest => "combat_commit_carrier_contest",
            CombatCommitCarrierProtection => "combat_commit_carrier_protection",
            CombatCommitLooseBallContest => "combat_commit_loose_ball_contest",
            CombatCommitPassingLaneOrShotDenial => "combat_commit_passing_lane_or_shot_denial",
            CombatCommitRecoveryPunish => "combat_commit_recovery_punish",
            CombatCommitUnattributedOffBall => "combat_commit_unattributed_off_ball",
            Catch => "catch",
            Parry => "parry",
            Tip => "tip",
            Claim => "claim",
            Block => "block",
            Header => "header",
            Volley => "volley",
            Bicycle => "bicycle",
            Reception => "reception",
            Juke => "juke",
        }
    }
}

/// One frame's discrete action notification, for the renderer's juice
/// layer (flashes, trails). Positions are world space; `player` is the
/// actor's id (`None` for ball-only events).
#[derive(Clone, Debug, PartialEq)]
pub struct MatchEvent {
    /// Event kind.
    pub kind: MatchEventKind,
    /// World x position.
    pub x: f64,
    /// World y position.
    pub y: f64,
    /// The actor's player id, if any.
    pub player: Option<String>,
    /// Save presentation style, for a save event.
    pub save_style: Option<SaveStyle>,
    /// Aerial style, for an aerial event.
    pub style: Option<AerialStyle>,
    /// Aerial outcome, for an aerial event.
    pub outcome: Option<AerialOutcome>,
    /// Whether the keeper was jumping.
    pub jumping: Option<bool>,
    /// Save difficulty.
    pub difficulty: Option<f64>,
    /// Shot type, for a keeper-relevant event.
    pub shot_type: Option<KeeperShotType>,
    /// Keeper behavior state at the moment of this event.
    pub keeper_state: Option<KeeperBehaviorState>,
    /// Keeper line depth the strike was released from.
    pub keeper_depth: Option<f64>,
    /// Whether a strike was on target.
    pub on_target: Option<bool>,
}

/// The complete, versioned start-of-tick match state.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchState {
    /// Pitch dimensions.
    pub field: PitchSize,
    /// Left goal; away scores here.
    pub goal_home: Rect,
    /// Right goal; home scores here.
    pub goal_away: Rect,
    /// Every fixture player (home indices 0..5, away 5..10, zero-based).
    pub players: Vec<MatchPlayer>,
    /// Ball ground position (height is `ball_z`).
    pub ball: Vec2,
    /// Ball horizontal velocity (ground plane).
    pub ball_vel: Vec2,
    /// Height above the pitch (0 = on the ground).
    pub ball_z: f64,
    /// Vertical velocity (+ = rising).
    pub ball_vz: f64,
    /// One-based index into `players`; `None` if loose.
    pub owner: Option<i64>,
    /// One-based index of the selected home player; valid even when every
    /// player is AI.
    pub controlled: i64,
    /// Whether `controlled` takes human-input branches.
    pub human_controlled: bool,
    /// Match score.
    pub score: ByTeam<i64>,
    /// Seconds remaining.
    pub time_left: f64,
    /// Goal cap.
    pub max_goals: i64,
    /// Whether the match has finished.
    pub finished: bool,
    /// Seconds until a loose ball becomes collectable again.
    pub pickup_cd: f64,
    /// Chasers per team (tactic-driven).
    pub press: ByTeam<i64>,
    /// Off-ball scheme per team.
    pub marking: ByTeam<MarkingConfig>,
    /// Previous marking assignment (hysteresis). One-based target player
    /// index per one-based source player index (`None` = unmarked).
    pub marks: ByTeam<Vec<Option<i64>>>,
    /// Team-owned stable presser state.
    pub outfield_press: ByTeam<OutfieldPressState>,
    /// Tactic counterpress/counterattack seconds.
    pub transition_windows: TransitionWindows,
    /// Possession memory driving the transition phases.
    pub transition: PossessionTransitionState,
    /// Authoritative role lookup; roles derive by ordinal.
    pub formation: ByTeam<String>,
    /// Lateral curve applied to the loose ball.
    pub ball_spin: f64,
    /// Seeded PRNG state (`core::rng`): same seed = same match.
    pub rng: u32,
    /// Body-blocking re-enabled when this hits 0 (set on release).
    pub block_grace: f64,
    /// Seconds until this loose ball can take another aerial contact.
    pub aerial_lock: f64,
    /// Seconds left of post-restart defensive hold (0 = press resumes).
    pub kickoff_hold: f64,
    /// Discrete actions this frame.
    pub events: Vec<MatchEvent>,
    /// True when the fixture uses stable eight-slot `InputFrame` routing.
    pub slot_mode: bool,
    /// Stable fixture slot-to-player identity.
    pub input_ownership: Option<InputOwnership>,
    /// Canonical slot index (one-based position 1..=8) -> one-based
    /// `MatchState` player index.
    pub slot_players: Vec<Option<i64>>,
    /// One-based outfielder player index -> canonical slot index.
    pub slot_for_player: Vec<Option<i64>>,
    /// `InputFrame` tick expected by the next slot-mode step.
    pub input_tick: i64,
    /// See the module doc's `mark_unsupported` section. Never serialized.
    pub unsupported_reason: Option<String>,
}

fn is_finite(value: f64) -> bool {
    value.is_finite()
}

/// One tick's aggregate desired action for one player: `MatchInput`.
/// Translated from a canonical `InputSample` by [`crate::slot_input`], or
/// produced directly by a human/bot input driver. Ephemeral per-tick data —
/// never part of [`MatchState`]'s serialized/hashed shape, so it lives here
/// only because every module in this layer that needs the match-shaped
/// types needs this one too (one shared declaration rather than a
/// fourth/fifth view).
///
/// `move` is a Rust keyword, so [`MatchInput::r#move`] uses a raw
/// identifier — the same convention `gc_sim::r#match` itself uses for the
/// module name (see `lib.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct MatchInput {
    /// Controlled player's desired direction.
    pub r#move: Vec2,
    /// Fire the shot (released this frame).
    pub shoot: bool,
    /// Shoot key currently down (builds charge).
    pub shoot_held: bool,
    /// Release the pass (fired on key release).
    pub pass: bool,
    /// Pass key currently down (builds pass range).
    pub pass_held: bool,
    /// Hand control to the outfielder nearest the ball.
    pub switch: bool,
    /// Tackle attempt (slide when moving fast, poke when slow).
    pub dash: bool,
    /// Sidestep juke with brief tackle immunity.
    pub dodge: bool,
    /// Loft modifier: chip a shot / lob a pass over a defender.
    pub lob: bool,
    /// Hold to sprint (drains the sprint meter).
    pub sprint: bool,
    /// Hold Space off the ball: slow shadow stance, bonus poke reach on
    /// release.
    pub jockey: bool,
    /// Abstract first-time strike intent. `None` means "unset":
    /// [`crate::aerial::strike_requested`] falls back to `jockey || dash`.
    /// `Some` — set deliberately (`sim::bot`) or derived from input bits
    /// (`sim::slot_input`) — is authoritative and skips the fallback. This
    /// is the one field pair in `MatchInput` where `None` and `Some(false)`
    /// are NOT interchangeable; every other field is a plain,
    /// always-populated `bool`.
    pub aerial_strike: Option<bool>,
    /// Abstract bicycle/acrobatic intent. `None` falls back to
    /// `lob && strike_requested` — see [`crate::aerial::acrobatic_requested`]
    /// and [`MatchInput::aerial_strike`]'s doc.
    pub aerial_acrobatic: Option<bool>,
    /// Equipment control is physically held.
    pub equipment_held: bool,
    /// Equipment press edge for this fixed tick.
    pub equipment_pressed: bool,
    /// Equipment release edge for this fixed tick.
    pub equipment_released: bool,
}

// ---------------------------------------------------------------------
// number_bytes / canonical encoder
// ---------------------------------------------------------------------

/// Decompose `x` into `(mantissa, exponent)` such that
/// `x == mantissa * 2^exponent` and `0.5 <= mantissa < 1` (mirrors
/// `math.frexp`). Implemented via IEEE 754 bit manipulation — never a
/// transcendental — so it is bit-exact on every wasm runtime.
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let sign = bits & (1u64 << 63);
    let mut raw_exponent = ((bits >> 52) & 0x7ff) as i32;
    let mut mantissa_bits = bits & 0x000f_ffff_ffff_ffff;
    if raw_exponent == 0 {
        // Subnormal: normalize by shifting the mantissa left until the
        // implicit leading bit would land, tracking the exponent debt.
        let mut shift = 0i32;
        while mantissa_bits & 0x0010_0000_0000_0000 == 0 {
            mantissa_bits <<= 1;
            shift += 1;
        }
        mantissa_bits &= 0x000f_ffff_ffff_ffff;
        raw_exponent = 1 - shift;
    }
    let unbiased = raw_exponent - 1023;
    let result_bits = sign | (0x3fe_u64 << 52) | mantissa_bits;
    (f64::from_bits(result_bits), unbiased + 1)
}

/// Canonical text payload for one finite number: `number_bytes`.
///
/// Prints floats with an exact, portable, bit-preserving encoding rather
/// than a locale/precision-sensitive `%g`-style format: `frexp` splits the
/// value into a sign, a binary exponent, and a mantissa quantized into two
/// integer limbs (26 + 27 bits), each rendered as plain decimal digits.
/// Round-tripping through this scheme recovers the exact `f64`.
#[must_use]
pub fn number_bytes(number: f64) -> String {
    assert!(is_finite(number), "canonical numbers must be finite");
    if number == 0.0 {
        return if (1.0 / number) == f64::NEG_INFINITY {
            "Z".to_string()
        } else {
            "z".to_string()
        };
    }
    let sign = if number < 0.0 { "m" } else { "p" };
    let (mantissa, exponent) = frexp(number.abs());
    let scaled = mantissa * 67_108_864.0;
    let high = scaled.floor();
    let low = ((scaled - high) * 134_217_728.0 + 0.5).floor();
    format!("{sign}:{exponent}:{}:{}", high as i64, low as i64)
}

fn unsigned_decimal_bytes(value: i64) -> usize {
    if value < 10 {
        1
    } else if value < 100 {
        2
    } else if value < 1_000 {
        3
    } else if value < 10_000 {
        4
    } else if value < 100_000 {
        5
    } else if value < 1_000_000 {
        6
    } else if value < 10_000_000 {
        7
    } else if value < 100_000_000 {
        8
    } else if value < 1_000_000_000 {
        9
    } else if value < 10_000_000_000 {
        10
    } else if value < 100_000_000_000 {
        11
    } else if value < 1_000_000_000_000 {
        12
    } else if value < 10_000_000_000_000 {
        13
    } else if value < 100_000_000_000_000 {
        14
    } else if value < 1_000_000_000_000_000 {
        15
    } else {
        16
    }
}

fn integer_decimal_bytes(value: i64) -> usize {
    if value < 0 {
        unsigned_decimal_bytes(-value) + 1
    } else {
        unsigned_decimal_bytes(value)
    }
}

fn number_payload_bytes(number: f64) -> usize {
    assert!(is_finite(number), "canonical numbers must be finite");
    if number == 0.0 {
        return 1;
    }
    let (mantissa, exponent) = frexp(number.abs());
    let scaled = mantissa * 67_108_864.0;
    let high = scaled.floor();
    let low = ((scaled - high) * 134_217_728.0 + 0.5).floor();
    4 + integer_decimal_bytes(i64::from(exponent))
        + unsigned_decimal_bytes(high as i64)
        + unsigned_decimal_bytes(low as i64)
}

/// The canonical byte-counting/materializing encoder. Implements
/// [`CombatSnapshotWriter`] so [`crate::combat_snapshot::append`] can write
/// the combat companion through the same sink.
///
/// In counting mode (`parts: None`) no wire string is ever materialized;
/// only [`Self::bytes`] accumulates — a dual-mode encoder used by
/// [`encoded_size_canonical`] to get a byte count without paying for the
/// full materialized string.
pub struct Encoder {
    parts: Option<Vec<String>>,
    bytes: usize,
}

impl Encoder {
    fn materializing() -> Self {
        Encoder {
            parts: Some(Vec::new()),
            bytes: 0,
        }
    }

    fn counting() -> Self {
        Encoder {
            parts: None,
            bytes: 0,
        }
    }

    fn into_string(self) -> String {
        self.parts.expect("encoder was not materializing").concat()
    }
}

impl CombatSnapshotWriter for Encoder {
    fn literal(&mut self, value: &str) {
        self.bytes += value.len();
        if let Some(parts) = &mut self.parts {
            parts.push(value.to_string());
        }
    }

    fn name(&mut self, value: &str) {
        let wire = format!("k{}:{value};", value.len());
        self.literal(&wire);
    }

    fn scalar(&mut self, value: CanonicalScalar) {
        match value {
            CanonicalScalar::Nil => self.literal("z;"),
            CanonicalScalar::Bool(b) => self.literal(if b { "b1;" } else { "b0;" }),
            CanonicalScalar::Number(n) => {
                if let Some(parts) = &mut self.parts {
                    let payload = number_bytes(n);
                    self.bytes += payload.len() + 2;
                    parts.push(format!("n{payload};"));
                } else {
                    self.bytes += number_payload_bytes(n) + 2;
                }
            }
            CanonicalScalar::Text(s) => {
                if let Some(parts) = &mut self.parts {
                    let length = s.len().to_string();
                    self.bytes += length.len() + s.len() + 3;
                    parts.push(format!("s{length}:{s};"));
                } else {
                    self.bytes += unsigned_decimal_bytes(s.len() as i64) + s.len() + 3;
                }
            }
        }
    }
}

fn scalar_num(n: f64) -> CanonicalScalar {
    CanonicalScalar::Number(n)
}
fn scalar_i(n: i64) -> CanonicalScalar {
    CanonicalScalar::Number(n as f64)
}
fn scalar_opt_i(n: Option<i64>) -> CanonicalScalar {
    n.map_or(CanonicalScalar::Nil, |v| CanonicalScalar::Number(v as f64))
}
fn scalar_bool(b: bool) -> CanonicalScalar {
    CanonicalScalar::Bool(b)
}
fn scalar_opt_bool(b: Option<bool>) -> CanonicalScalar {
    b.map_or(CanonicalScalar::Nil, CanonicalScalar::Bool)
}
fn scalar_opt_num(n: Option<f64>) -> CanonicalScalar {
    n.map_or(CanonicalScalar::Nil, CanonicalScalar::Number)
}
fn scalar_str(s: &str) -> CanonicalScalar {
    CanonicalScalar::Text(s.to_string())
}
fn scalar_opt_str(s: Option<&str>) -> CanonicalScalar {
    s.map_or(CanonicalScalar::Nil, |v| {
        CanonicalScalar::Text(v.to_string())
    })
}

fn keeper_shot_type_wire(v: KeeperShotType) -> &'static str {
    match v {
        KeeperShotType::Ground => "ground",
        KeeperShotType::Chip => "chip",
    }
}
fn keeper_behavior_state_wire(v: KeeperBehaviorState) -> &'static str {
    match v {
        KeeperBehaviorState::Base => "base",
        KeeperBehaviorState::Advance => "advance",
        KeeperBehaviorState::Contain => "contain",
        KeeperBehaviorState::Set => "set",
        KeeperBehaviorState::Retreat => "retreat",
        KeeperBehaviorState::Recover => "recover",
    }
}
fn save_style_wire(v: SaveStyle) -> &'static str {
    match v {
        SaveStyle::Spread => "spread",
        SaveStyle::Central => "central",
        SaveStyle::Stretch => "stretch",
    }
}
fn aerial_style_wire(v: AerialStyle) -> &'static str {
    match v {
        AerialStyle::LegControl => "leg_control",
        AerialStyle::ChestControl => "chest_control",
        AerialStyle::Volley => "volley",
        AerialStyle::Header => "header",
        AerialStyle::Bicycle => "bicycle",
    }
}
fn aerial_outcome_wire(v: AerialOutcome) -> &'static str {
    match v {
        AerialOutcome::Clean => "clean",
        AerialOutcome::Heavy => "heavy",
        AerialOutcome::Miss => "miss",
    }
}
fn save_pending_wire(v: SavePending) -> &'static str {
    match v {
        SavePending::Catch => "catch",
        SavePending::Parry => "parry",
    }
}
fn sim_verb_wire(v: SimVerb) -> &'static str {
    match v {
        SimVerb::Jump => "jump",
        SimVerb::Collision => "collision",
        SimVerb::Burst => "burst",
        SimVerb::Dribble => "dribble",
        SimVerb::Block => "block",
        SimVerb::Link => "link",
        SimVerb::None => "none",
    }
}
fn marking_scheme_wire(v: gc_data::tactics::MarkingScheme) -> &'static str {
    use gc_data::tactics::MarkingScheme::*;
    match v {
        Zonal => "zonal",
        Man => "man",
        Hybrid => "hybrid",
    }
}

const SLOT_IDS: [&str; 8] = [
    "home_1", "home_2", "home_3", "home_4", "away_1", "away_2", "away_3", "away_4",
];

fn slot_id_wire(index_zero_based: usize) -> &'static str {
    SLOT_IDS[index_zero_based]
}

fn slot_id_index(id: SlotId) -> usize {
    match id {
        SlotId::Home1 => 0,
        SlotId::Home2 => 1,
        SlotId::Home3 => 2,
        SlotId::Home4 => 3,
        SlotId::Away1 => 4,
        SlotId::Away2 => 5,
        SlotId::Away3 => 6,
        SlotId::Away4 => 7,
    }
}

fn outfield_decision_context_wire(v: outfield_decision::OutfieldDecisionContext) -> &'static str {
    use outfield_decision::OutfieldDecisionContext::*;
    match v {
        Ineligible => "ineligible",
        Offball => "offball",
        Carrier => "carrier",
    }
}
fn outfield_intent_wire(v: outfield_decision::OutfieldIntent) -> &'static str {
    use outfield_decision::OutfieldIntent::*;
    match v {
        None => "none",
        Move => "move",
        InBehind => "in_behind",
        ComeShort => "come_short",
        HoldWidth => "hold_width",
        Shoot => "shoot",
        Cross => "cross",
        Pass => "pass",
        Dribble => "dribble",
    }
}
fn stable_press_mode_wire(v: outfield_press::StablePressMode) -> &'static str {
    use outfield_press::StablePressMode::*;
    match v {
        Inactive => "inactive",
        Contain => "contain",
        Commit => "commit",
    }
}
fn press_reason_wire(v: crate::brain::PressReason) -> &'static str {
    use crate::brain::PressReason::*;
    match v {
        HeavyTouch => "heavy_touch",
        ExposedBall => "exposed_ball",
        Cover => "cover",
        BoxDesperation => "box_desperation",
        LowDiscipline => "low_discipline",
        NoTrigger => "no_trigger",
    }
}
fn transition_team_wire(v: possession_transition::TransitionTeam) -> &'static str {
    use possession_transition::TransitionTeam::*;
    match v {
        Home => "home",
        Away => "away",
    }
}

fn append_outfield_decision<W: CombatSnapshotWriter>(w: &mut W, d: &OutfieldDecisionState) {
    w.scalar(scalar_i(i64::from(d.version)));
    w.scalar(scalar_i(i64::from(d.generation)));
    w.scalar(scalar_i(i64::from(d.rng_state)));
    w.scalar(scalar_num(d.remaining));
    w.scalar(scalar_str(outfield_decision_context_wire(d.context)));
    w.scalar(scalar_str(outfield_intent_wire(d.intent)));
    w.scalar(scalar_opt_num(d.target_x));
    w.scalar(scalar_opt_num(d.target_y));
    w.scalar(scalar_opt_i(d.target_player.map(i64::from)));
    w.scalar(scalar_opt_num(d.run_expires_at));
}

fn append_outfield_press<W: CombatSnapshotWriter>(w: &mut W, p: &OutfieldPressState) {
    w.name("version");
    w.scalar(scalar_i(i64::from(p.version)));
    w.name("presser_index");
    w.scalar(scalar_opt_i(p.presser_index.map(i64::from)));
    w.name("mode");
    w.scalar(scalar_str(stable_press_mode_wire(p.mode)));
    w.name("reason");
    w.scalar(scalar_str(press_reason_wire(p.reason)));
}

fn append_transition_window<W: CombatSnapshotWriter>(
    w: &mut W,
    c: gc_data::tactics::TransitionConfig,
) {
    w.name("counterpress");
    w.scalar(scalar_num(c.counterpress));
    w.name("counterattack");
    w.scalar(scalar_num(c.counterattack));
}

fn append_transition<W: CombatSnapshotWriter>(w: &mut W, t: &PossessionTransitionState) {
    w.name("version");
    w.scalar(scalar_i(i64::from(t.version)));
    w.name("last_team");
    w.scalar(scalar_opt_str(t.last_team.map(transition_team_wire)));
    w.name("holding_team");
    w.scalar(scalar_opt_str(t.holding_team.map(transition_team_wire)));
    w.name("hold");
    w.scalar(scalar_num(t.hold));
    w.name("turnover_team");
    w.scalar(scalar_opt_str(t.turnover_team.map(transition_team_wire)));
    w.name("elapsed");
    w.scalar(scalar_num(t.elapsed));
}

fn append_marking<W: CombatSnapshotWriter>(w: &mut W, m: &MarkingConfig) {
    w.name("scheme");
    w.scalar(scalar_str(marking_scheme_wire(m.scheme)));
    w.name("man_marks");
    w.scalar(scalar_i(m.man_marks));
    w.name("standoff");
    w.scalar(scalar_num(m.standoff));
    w.name("compactness");
    w.scalar(scalar_num(m.compactness));
    w.name("support");
    w.scalar(scalar_num(m.support));
}

fn append_ownership<W: CombatSnapshotWriter>(w: &mut W, value: &Option<InputOwnership>) {
    let Some(value) = value else {
        w.scalar(CanonicalScalar::Nil);
        return;
    };
    w.scalar(scalar_i(value.version));
    w.name("home");
    for id in &value.rosters.home {
        w.scalar(scalar_str(id));
    }
    w.name("away");
    for id in &value.rosters.away {
        w.scalar(scalar_str(id));
    }
    for assignment in &value.slots {
        w.name("slot");
        w.scalar(scalar_str(slot_id_wire(slot_id_index(assignment.slot))));
        w.name("team");
        w.scalar(scalar_str(match assignment.team {
            input_frame::Team::Home => "home",
            input_frame::Team::Away => "away",
        }));
        w.name("player_id");
        w.scalar(scalar_str(&assignment.player_id));
    }
}

fn append_sparse(w: &mut Encoder, values: &[Option<i64>]) {
    for value in values {
        w.scalar(scalar_opt_i(*value));
    }
}

fn append_player(w: &mut Encoder, player: &MatchPlayer) {
    macro_rules! field {
        ($name:literal, $value:expr) => {
            w.name($name);
            w.scalar($value);
        };
    }
    field!("id", scalar_str(&player.id));
    field!("name", scalar_str(&player.name));
    field!("team", scalar_str(player.team.wire_str()));
    w.name("pos");
    w.vec(player.pos);
    w.name("vel");
    w.vec(player.vel);
    w.name("run_vel");
    w.vec(player.run_vel);
    w.name("facing");
    w.vec(player.facing);
    w.name("anchor");
    w.vec(player.anchor);
    field!("species_id", scalar_str(&player.species_id));
    field!("owned_verb", scalar_str(sim_verb_wire(player.owned_verb)));
    field!("move_speed", scalar_num(player.move_speed));
    field!("shot_speed", scalar_num(player.shot_speed));
    field!("dribble", scalar_num(player.dribble));
    field!("strength", scalar_num(player.strength));
    field!("first_touch", scalar_num(player.first_touch));
    field!("header_skill", scalar_num(player.header_skill));
    field!("volley_skill", scalar_num(player.volley_skill));
    field!("bicycle_skill", scalar_num(player.bicycle_skill));
    field!("scan_rate", scalar_num(player.scan_rate));
    field!("composure", scalar_num(player.composure));
    w.name("outfield_decision");
    w.literal("d;");
    append_outfield_decision(w, &player.outfield_decision);
    field!("is_keeper", scalar_bool(player.is_keeper));
    field!("radius", scalar_num(player.radius));
    field!("dash_cd", scalar_num(player.dash_cd));
    field!("dodge_cd", scalar_num(player.dodge_cd));
    field!("dodge_timer", scalar_num(player.dodge_timer));
    w.name("dodge_dir");
    w.vec(player.dodge_dir);
    field!("reach", scalar_num(player.reach));
    field!("handling", scalar_num(player.handling));
    field!("keeper_aggression", scalar_num(player.keeper_aggression));
    field!(
        "keeper_anticipation",
        scalar_num(player.keeper_anticipation)
    );
    field!(
        "keeper_state",
        scalar_str(keeper_behavior_state_wire(player.keeper_state))
    );
    field!("keeper_state_timer", scalar_num(player.keeper_state_timer));
    field!(
        "keeper_release_state",
        scalar_opt_str(player.keeper_release_state.map(keeper_behavior_state_wire))
    );
    field!(
        "keeper_release_motion",
        scalar_num(player.keeper_release_motion)
    );
    field!(
        "keeper_release_kind",
        scalar_opt_str(player.keeper_release_kind.map(keeper_shot_type_wire))
    );
    field!(
        "keeper_release_depth",
        scalar_num(player.keeper_release_depth)
    );
    field!("keeper_set", scalar_num(player.keeper_set));
    field!("dive_timer", scalar_num(player.dive_timer));
    w.name("dive_dir");
    w.vec(player.dive_dir);
    field!("dive_delay", scalar_num(player.dive_delay));
    w.name("dive_target");
    if let Some(target) = player.dive_target {
        w.literal("v;");
        w.vec(target);
    } else {
        w.scalar(CanonicalScalar::Nil);
    }
    field!(
        "keeper_get_up_timer",
        scalar_num(player.keeper_get_up_timer)
    );
    field!("hold_timer", scalar_num(player.hold_timer));
    field!("feet_ball", scalar_bool(player.feet_ball));
    field!("slide_timer", scalar_num(player.slide_timer));
    w.name("slide_dir");
    w.vec(player.slide_dir);
    field!("slide_vel", scalar_num(player.slide_vel));
    field!("tackle_timer", scalar_num(player.tackle_timer));
    field!("tackle_cd", scalar_num(player.tackle_cd));
    field!("stun_timer", scalar_num(player.stun_timer));
    field!("grab_timer", scalar_num(player.grab_timer));
    field!("throw_timer", scalar_num(player.throw_timer));
    field!("receive_timer", scalar_num(player.receive_timer));
    field!("sprint_meter", scalar_num(player.sprint_meter));
    field!("sprint_dur", scalar_num(player.sprint_dur));
    field!("sprinting", scalar_bool(player.sprinting));
    field!(
        "save_pending",
        scalar_opt_str(player.save_pending.map(save_pending_wire))
    );
    field!("save_timer", scalar_num(player.save_timer));
    field!("save_vx", scalar_num(player.save_vx));
    field!(
        "save_style",
        scalar_opt_str(player.save_style.map(save_style_wire))
    );
    field!("save_tip_emitted", scalar_bool(player.save_tip_emitted));
    field!("settle_timer", scalar_num(player.settle_timer));
    field!("header_cd", scalar_num(player.header_cd));
    field!("aerial_timer", scalar_num(player.aerial_timer));
    field!(
        "aerial_style",
        scalar_opt_str(player.aerial_style.map(aerial_style_wire))
    );
    field!(
        "aerial_outcome",
        scalar_opt_str(player.aerial_outcome.map(aerial_outcome_wire))
    );
    field!("aerial_jump", scalar_num(player.aerial_jump));
    field!("aerial_recovery", scalar_num(player.aerial_recovery));
    field!("charge", scalar_num(player.charge));
    field!("pass_charge", scalar_num(player.pass_charge));
    field!("pass_target", scalar_opt_i(player.pass_target));
    field!("windup_timer", scalar_num(player.windup_timer));
    w.name("windup_shot");
    if let Some(shot) = &player.windup_shot {
        w.literal("w;");
        w.name("dir");
        w.vec(shot.dir);
        w.name("speed");
        w.scalar(scalar_num(shot.speed));
        w.name("vz");
        w.scalar(scalar_num(shot.vz));
        w.name("spin");
        w.scalar(scalar_num(shot.spin));
        w.name("shot_type");
        w.scalar(scalar_str(keeper_shot_type_wire(shot.shot_type)));
    } else {
        w.scalar(CanonicalScalar::Nil);
    }
    field!("jockey_timer", scalar_num(player.jockey_timer));
}

fn append_event(w: &mut Encoder, event: &MatchEvent) {
    macro_rules! field {
        ($name:literal, $value:expr) => {
            w.name($name);
            w.scalar($value);
        };
    }
    field!("kind", scalar_str(event.kind.wire_str()));
    field!("x", scalar_num(event.x));
    field!("y", scalar_num(event.y));
    field!("player", scalar_opt_str(event.player.as_deref()));
    field!(
        "save_style",
        scalar_opt_str(event.save_style.map(save_style_wire))
    );
    field!("style", scalar_opt_str(event.style.map(aerial_style_wire)));
    field!(
        "outcome",
        scalar_opt_str(event.outcome.map(aerial_outcome_wire))
    );
    field!("jumping", scalar_opt_bool(event.jumping));
    field!("difficulty", scalar_opt_num(event.difficulty));
    field!(
        "shot_type",
        scalar_opt_str(event.shot_type.map(keeper_shot_type_wire))
    );
    field!(
        "keeper_state",
        scalar_opt_str(event.keeper_state.map(keeper_behavior_state_wire))
    );
    field!("keeper_depth", scalar_opt_num(event.keeper_depth));
    field!("on_target", scalar_opt_bool(event.on_target));
}

/// Serialize `state` (and `combat_state` when `version` is
/// [`COMBAT_VERSION`]) into `encoder` in the canonical field order.
fn append_state(
    encoder: &mut Encoder,
    version: i64,
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
) {
    encoder.literal("GCMS;");
    encoder.scalar(scalar_i(version));
    macro_rules! field {
        ($name:literal, $value:expr) => {
            encoder.name($name);
            encoder.scalar($value);
        };
    }
    encoder.name("field");
    encoder.scalar(scalar_num(state.field.w));
    encoder.scalar(scalar_num(state.field.h));
    encoder.name("goal_home");
    for v in [
        state.goal_home.x,
        state.goal_home.y,
        state.goal_home.w,
        state.goal_home.h,
    ] {
        encoder.scalar(scalar_num(v));
    }
    encoder.name("goal_away");
    for v in [
        state.goal_away.x,
        state.goal_away.y,
        state.goal_away.w,
        state.goal_away.h,
    ] {
        encoder.scalar(scalar_num(v));
    }
    encoder.name("players");
    encoder.scalar(scalar_i(state.players.len() as i64));
    for player in &state.players {
        append_player(encoder, player);
    }
    encoder.name("ball");
    encoder.vec(state.ball);
    encoder.name("ball_vel");
    encoder.vec(state.ball_vel);
    field!("ball_z", scalar_num(state.ball_z));
    field!("ball_vz", scalar_num(state.ball_vz));
    field!("owner", scalar_opt_i(state.owner));
    field!("controlled", scalar_i(state.controlled));
    field!("human_controlled", scalar_bool(state.human_controlled));
    encoder.name("score");
    encoder.scalar(scalar_i(state.score.home));
    encoder.scalar(scalar_i(state.score.away));
    field!("time_left", scalar_num(state.time_left));
    field!("max_goals", scalar_i(state.max_goals));
    field!("finished", scalar_bool(state.finished));
    field!("pickup_cd", scalar_num(state.pickup_cd));
    encoder.name("press");
    encoder.scalar(scalar_i(state.press.home));
    encoder.scalar(scalar_i(state.press.away));
    encoder.name("marking");
    append_marking(encoder, &state.marking.home);
    append_marking(encoder, &state.marking.away);
    encoder.name("marks");
    append_sparse(encoder, &state.marks.home);
    append_sparse(encoder, &state.marks.away);
    encoder.name("outfield_press");
    append_outfield_press(encoder, &state.outfield_press.home);
    append_outfield_press(encoder, &state.outfield_press.away);
    encoder.name("transition_windows");
    append_transition_window(encoder, state.transition_windows.home);
    append_transition_window(encoder, state.transition_windows.away);
    encoder.name("transition");
    append_transition(encoder, &state.transition);
    encoder.name("formation");
    encoder.scalar(scalar_str(&state.formation.home));
    encoder.scalar(scalar_str(&state.formation.away));
    field!("ball_spin", scalar_num(state.ball_spin));
    field!("rng", scalar_i(i64::from(state.rng)));
    field!("block_grace", scalar_num(state.block_grace));
    field!("aerial_lock", scalar_num(state.aerial_lock));
    field!("kickoff_hold", scalar_num(state.kickoff_hold));
    encoder.name("events");
    encoder.scalar(scalar_i(state.events.len() as i64));
    for event in &state.events {
        append_event(encoder, event);
    }
    field!("slot_mode", scalar_bool(state.slot_mode));
    encoder.name("input_ownership");
    append_ownership(encoder, &state.input_ownership);
    encoder.name("slot_players");
    append_sparse(encoder, &state.slot_players);
    encoder.name("slot_for_player");
    append_sparse(encoder, &state.slot_for_player);
    field!("input_tick", scalar_i(state.input_tick));

    if version == COMBAT_VERSION {
        encoder.name("combat");
        encoder.literal("c;");
        combat_snapshot::append(
            encoder,
            combat_state.expect("combat version requires a combat companion"),
        );
    }
}

fn encode_snapshot(
    version: i64,
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
) -> String {
    let mut encoder = Encoder::materializing();
    append_state(&mut encoder, version, state, combat_state);
    encoder.into_string()
}

fn encoded_snapshot_size(
    version: i64,
    state: &MatchState,
    combat_state: Option<&CombatMatchState>,
) -> usize {
    let mut encoder = Encoder::counting();
    append_state(&mut encoder, version, state, combat_state);
    encoder.bytes
}

// ---------------------------------------------------------------------
// The snapshot envelope
// ---------------------------------------------------------------------

/// One captured/restored `(MatchState, CombatMatchState?)` pair, versioned.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSnapshot {
    /// [`VERSION`] or [`COMBAT_VERSION`].
    pub version: i64,
    /// The soccer state.
    pub state: MatchState,
    /// The combat companion, present exactly when `version ==
    /// COMBAT_VERSION`.
    pub combat: Option<CombatMatchState>,
}

/// See the module doc's `mark_unsupported` section.
pub fn mark_unsupported(state: &mut MatchState, reason: impl Into<String>) {
    let reason = reason.into();
    assert!(!reason.is_empty(), "snapshot rejection reason is required");
    state.unsupported_reason = Some(reason);
}

fn assert_snapshot_supported(state: &MatchState, combat_state: Option<&CombatMatchState>) {
    if let Some(reason) = &state.unsupported_reason {
        assert!(combat_state.is_some(), "{reason}");
    }
}

/// Validate the outfield-decision run-intent cross-field invariants that no
/// type can express: a run is legal only for an eligible, uncommitted,
/// same-team outfielder while the ball is in ordinary attacking play, and
/// at most two runs are live per team at once.
fn validate_run_relations(state: &MatchState) {
    let now = -state.time_left;
    let mut run_counts = ByTeam {
        home: 0i64,
        away: 0i64,
    };
    for (index, player) in state.players.iter().enumerate() {
        let decision = &player.outfield_decision;
        if !outfield_decision::is_run_intent(decision.intent) {
            continue;
        }
        let path = format!("state.players.{}.outfield_decision", index + 1);
        assert!(!player.is_keeper, "{path} keeper cannot own a run");
        let owner_index = state
            .owner
            .unwrap_or_else(|| panic!("{path} run requires a ball owner"));
        assert!(
            owner_index as usize != index + 1,
            "{path} owner cannot retain a run"
        );
        let owner = state
            .players
            .get(owner_index as usize - 1)
            .unwrap_or_else(|| panic!("{path} run requires a ball owner"));
        assert!(
            !owner.is_keeper && owner.team == player.team,
            "{path} run requires same-team outfield possession"
        );
        assert!(
            state.kickoff_hold <= 0.0 && !state.finished,
            "{path} run is invalid outside ordinary attack"
        );
        assert!(
            state.slot_for_player[index].is_none(),
            "{path} fixed-slot player cannot own a run"
        );
        assert!(
            !state.human_controlled || state.controlled as usize != index + 1,
            "{path} human-controlled player cannot own a run"
        );
        assert!(
            player.stun_timer <= 0.0
                && player.slide_timer <= 0.0
                && player.tackle_timer <= 0.0
                && player.dodge_timer <= 0.0
                && player.jockey_timer <= 0.0
                && player.windup_timer <= 0.0
                && player.windup_shot.is_none()
                && player.aerial_timer <= 0.0
                && player.aerial_recovery <= 0.0
                && player.receive_timer <= 0.0,
            "{path} run conflicts with a soccer commitment"
        );
        let target_x = decision.target_x.expect("run intent has a target");
        let target_y = decision.target_y.expect("run intent has a target");
        assert!(
            target_x >= 0.0
                && target_x <= state.field.w
                && target_y >= 0.0
                && target_y <= state.field.h,
            "{path} run target is outside the field"
        );
        let expires_at = decision.run_expires_at.expect("run intent has an expiry");
        assert!(expires_at > now, "{path} run expiry must be future");
        assert!(
            expires_at <= now + outfield_decision::RUN_LIFETIME_SECONDS,
            "{path} run expiry exceeds the fixed lifetime"
        );
        match player.team {
            Team::Home => run_counts.home += 1,
            Team::Away => run_counts.away += 1,
        }
        assert!(
            run_counts.get(player.team) <= 2,
            "state.outfield_decision team cannot retain more than two active runs"
        );
    }
}

fn validate_press_eligibility(state: &MatchState) {
    for team in [Team::Home, Team::Away] {
        let press_state = state.outfield_press.get(team);
        let Some(presser_index) = press_state.presser_index else {
            continue;
        };
        let path = format!("state.outfield_press.{}", team.wire_str());
        let presser = state
            .players
            .get(presser_index as usize - 1)
            .unwrap_or_else(|| panic!("{path} presser is out of bounds"));
        assert!(
            presser.team == team,
            "{path} presser belongs to the wrong team"
        );
        assert!(!presser.is_keeper, "{path} presser cannot be a keeper");
        assert!(
            state.slot_for_player[presser_index as usize - 1].is_none(),
            "{path} presser cannot own a fixed input slot"
        );
        assert!(
            !state.human_controlled || state.controlled != i64::from(presser_index),
            "{path} presser cannot be human-controlled"
        );
    }
}

fn validate_formations(state: &MatchState) {
    for team in [Team::Home, Team::Away] {
        let id = state.formation.get(team);
        assert!(
            formations::get(&id).is_some(),
            "state.formation.{} must identify an authored formation",
            team.wire_str()
        );
    }
}

fn validate_transition_liveness(state: &MatchState) {
    if possession_transition::active(&state.transition) {
        assert!(
            !state.finished,
            "state.transition cannot stay live after the match has finished"
        );
        assert!(
            state.transition.elapsed
                < possession_transition::window_limit(&state.transition, state.transition_windows),
            "state.transition outlived both tactic windows"
        );
    }
}

/// Validate a [`MatchState`]'s structural invariants that no Rust type
/// alone can express. The cross-field business invariants — run legality,
/// press-presser eligibility, transition liveness, and authored-formation
/// existence — are checked; a plain shape/type mismatch cannot happen here
/// because the Rust type system already rules it out.
pub fn validate(state: &MatchState) {
    assert_eq!(
        state.players.len(),
        PLAYER_COUNT,
        "state.players has the wrong length"
    );
    // Sparse for the same reason `marks` is, below: only present keys need
    // to fall in `1..=SLOT_COUNT`; a non-slot-mode match legitimately
    // leaves this short. Requiring density here made `capture` panic on
    // any non-slot-mode fixture — a real bug this length check must not
    // reintroduce.
    assert!(
        state.slot_players.len() <= input_frame::SLOT_COUNT as usize,
        "state.slot_players has more entries than slots"
    );
    assert_eq!(
        state.slot_for_player.len(),
        state.players.len(),
        "state.slot_for_player has the wrong length"
    );
    // `marks` is a *sparse* man-marking table: it clears to empty whenever a
    // team has no active assignment, which is the common case (attacking,
    // loose-ball and build-up all clear it), and the wire encoding pads it
    // to `#players` at encode time. Requiring density here rejected
    // legitimate states: `match::new` followed immediately by `capture`
    // panicked. Assert the invariant that actually holds instead.
    for team_marks in [&state.marks.home, &state.marks.away] {
        assert!(
            team_marks.len() <= state.players.len(),
            "state.marks has more entries than players"
        );
    }
    let _ = possession_transition::copy_state(&state.transition);
    // Each player's decision state and each team's press state get run
    // through their own validating copy. Those copies assert structural
    // coherence — version, and the mode/presser/reason pairing — which the
    // relational checks below do not: `validate_run_relations` skips any
    // player whose intent is not a run, and `validate_press_eligibility`
    // only checks an already-present presser. The copy is discarded; the
    // assertion inside it is the point, exactly as for `transition` above.
    for player in &state.players {
        let _ = outfield_decision::copy_state(&player.outfield_decision);
    }
    let _ = outfield_press::copy_state(&state.outfield_press.home);
    let _ = outfield_press::copy_state(&state.outfield_press.away);
    validate_run_relations(state);
    validate_press_eligibility(state);
    validate_formations(state);
    validate_transition_liveness(state);
}

/// Pad a sparse `marks` table out to one entry per player.
///
/// `MatchState` keeps `marks` sparse and this crate densifies at encode
/// time (the wire encoding always writes `count` scalars, a sentinel for
/// absent). The Rust representation is a dense `Vec<Option<i64>>`, so the
/// padding happens here instead — on the way into the snapshot, which is
/// the only place the density is observable.
fn densify_marks(state: &mut MatchState) {
    let count = state.players.len();
    state.marks.home.resize(count, None);
    state.marks.away.resize(count, None);
    // `slot_players` is sparse in the same way and pads to SLOT_COUNT, not to
    // the roster size.
    state
        .slot_players
        .resize(input_frame::SLOT_COUNT as usize, None);
}

/// Assert that no player with an active run intent is simultaneously committed
/// in combat.
///
/// Guards against a genuinely contradictory state: a player simultaneously
/// mid-run and mid-commitment in combat. `validate_run_relations` does not
/// cover this — it only checks run state against the match, never against
/// the companion combat state — so this assertion is the only thing that
/// would catch that contradiction before it gets hashed or restored. Guards
/// both [`capture`] and [`restore`].
fn assert_combat_run_relations(state: &MatchState, combat_state: &CombatMatchState, path: &str) {
    for (index, player) in state.players.iter().enumerate() {
        if outfield_decision::is_run_intent(player.outfield_decision.intent) {
            let runtime = &combat_state.players[index];
            assert!(
                runtime.phase == crate::combat_feasibility::CombatActionPhase::Ready
                    && runtime.forced_ticks <= 0,
                "{path}.players.{} combat commitment conflicts with an active run",
                index + 1
            );
        }
    }
}

/// Capture a validated, defensively-copied [`MatchSnapshot`] of `state`
/// (and `combat_state`, when present).
#[must_use]
pub fn capture(state: &MatchState, combat_state: Option<&CombatMatchState>) -> MatchSnapshot {
    assert_snapshot_supported(state, combat_state);
    validate(state);
    let mut copied_state = state.clone();
    densify_marks(&mut copied_state);
    if let Some(combat_state) = combat_state {
        let copied_combat = combat_snapshot::copy(combat_state, &copied_state);
        assert_combat_run_relations(&copied_state, &copied_combat, "combat");
        MatchSnapshot {
            version: COMBAT_VERSION,
            state: copied_state,
            combat: Some(copied_combat),
        }
    } else {
        MatchSnapshot {
            version: VERSION,
            state: copied_state,
            combat: None,
        }
    }
}

/// Internal ownership-transfer copy for a [`MatchState`] previously
/// produced by a validated snapshot or by `sim::match`. Arbitrary/external
/// states must use [`capture`] so schema drift and malformed values still
/// fail loudly.
#[must_use]
pub fn capture_owned(state: &MatchState, combat_state: Option<&CombatMatchState>) -> MatchSnapshot {
    assert_snapshot_supported(state, combat_state);
    let mut copied_state = state.clone();
    densify_marks(&mut copied_state);
    if let Some(combat_state) = combat_state {
        if copied_state.slot_mode {
            assert_eq!(
                combat_state.tick, copied_state.input_tick,
                "combat tick must match input tick"
            );
        }
        MatchSnapshot {
            version: COMBAT_VERSION,
            state: copied_state,
            combat: Some(combat_snapshot::copy_owned(combat_state)),
        }
    } else {
        MatchSnapshot {
            version: VERSION,
            state: copied_state,
            combat: None,
        }
    }
}

/// Validate and restore a [`MatchSnapshot`] into its `(MatchState,
/// CombatMatchState?)` pair.
#[must_use]
pub fn restore(snapshot: &MatchSnapshot) -> (MatchState, Option<CombatMatchState>) {
    assert!(
        snapshot.version == VERSION || snapshot.version == COMBAT_VERSION,
        "unsupported match snapshot version"
    );
    validate(&snapshot.state);
    let mut state = snapshot.state.clone();
    if snapshot.version == VERSION {
        assert!(
            snapshot.combat.is_none(),
            "soccer-only snapshots cannot contain combat state"
        );
        return (state, None);
    }
    let combat_source = snapshot
        .combat
        .as_ref()
        .expect("combat snapshot companion is required");
    let combat_state = combat_snapshot::copy(combat_source, &state);
    assert_combat_run_relations(&state, &combat_state, "snapshot.combat");
    mark_unsupported(
        &mut state,
        "combat-active match snapshots require their CombatMatchState companion",
    );
    (state, Some(combat_state))
}

/// Internal restore for a canonical snapshot already owned by rollback.
/// Public and external snapshots must use [`restore`] for full validation.
#[must_use]
pub fn restore_owned(snapshot: &MatchSnapshot) -> (MatchState, Option<CombatMatchState>) {
    assert!(
        snapshot.version == VERSION || snapshot.version == COMBAT_VERSION,
        "unsupported owned match snapshot version"
    );
    let mut state = snapshot.state.clone();
    if snapshot.version == VERSION {
        assert!(
            snapshot.combat.is_none(),
            "owned soccer-only snapshot cannot contain combat state"
        );
        return (state, None);
    }
    let combat_source = snapshot
        .combat
        .as_ref()
        .expect("owned combat snapshot companion is required");
    let combat_state = combat_snapshot::copy_owned(combat_source);
    mark_unsupported(
        &mut state,
        "combat-active match snapshots require their CombatMatchState companion",
    );
    (state, Some(combat_state))
}

/// Encode a snapshot's canonical wire form. Restores first, so schema drift
/// and malformed values still fail loudly.
#[must_use]
pub fn encode(snapshot: &MatchSnapshot) -> String {
    let (state, combat_state) = restore(snapshot);
    encode_snapshot(snapshot.version, &state, combat_state.as_ref())
}

/// Encode a snapshot just returned by [`capture`], or an equally canonical
/// owned copy, without repeating its full restore/validation pass.
#[must_use]
pub fn encode_canonical(snapshot: &MatchSnapshot) -> String {
    assert!(
        snapshot.version == VERSION || snapshot.version == COMBAT_VERSION,
        "unsupported canonical snapshot version"
    );
    encode_snapshot(snapshot.version, &snapshot.state, snapshot.combat.as_ref())
}

/// Count the exact canonical wire bytes for an owned snapshot without
/// materializing its wire string.
#[must_use]
pub fn encoded_size_canonical(snapshot: &MatchSnapshot) -> usize {
    assert!(
        snapshot.version == VERSION || snapshot.version == COMBAT_VERSION,
        "unsupported canonical snapshot version"
    );
    encoded_snapshot_size(snapshot.version, &snapshot.state, snapshot.combat.as_ref())
}

/// Hash a snapshot's canonical wire form with FNV-1a-64.
#[must_use]
pub fn hash(snapshot: &MatchSnapshot) -> String {
    fnv1a64::hash(encode(snapshot).as_bytes())
}

/// Hash a snapshot just returned by [`capture`], or an equally canonical
/// owned copy, without repeating its restore/validation pass.
#[must_use]
pub fn hash_canonical(snapshot: &MatchSnapshot) -> String {
    fnv1a64::hash(encode_canonical(snapshot).as_bytes())
}

// ---------------------------------------------------------------------
// Structural diff
// ---------------------------------------------------------------------

/// One leaf disagreement between two [`MatchSnapshot`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSnapshotDifference {
    /// Dotted path to the differing field.
    pub path: String,
    /// The left/expected value, rendered for display.
    pub expected: String,
    /// The right/actual value, rendered for display.
    pub actual: String,
}

fn diff(
    path: impl Into<String>,
    expected: impl std::fmt::Debug,
    actual: impl std::fmt::Debug,
) -> MatchSnapshotDifference {
    MatchSnapshotDifference {
        path: path.into(),
        expected: format!("{expected:?}"),
        actual: format!("{actual:?}"),
    }
}

fn same_f64(a: f64, b: f64) -> bool {
    if a != b {
        return false;
    }
    if a == 0.0 && b == 0.0 {
        return (1.0 / a) == (1.0 / b);
    }
    true
}

macro_rules! diff_num {
    ($path:expr, $a:expr, $b:expr) => {
        if !same_f64($a, $b) {
            return Some(diff($path, $a, $b));
        }
    };
}

macro_rules! diff_eq {
    ($path:expr, $a:expr, $b:expr) => {
        if $a != $b {
            return Some(diff($path, &$a, &$b));
        }
    };
}

fn diff_vec2(path: String, a: Vec2, b: Vec2) -> Option<MatchSnapshotDifference> {
    diff_num!(format!("{path}.x"), a.x, b.x);
    diff_num!(format!("{path}.y"), a.y, b.y);
    None
}

fn diff_outfield_decision(
    path: &str,
    a: &OutfieldDecisionState,
    b: &OutfieldDecisionState,
) -> Option<MatchSnapshotDifference> {
    diff_eq!(format!("{path}.version"), a.version, b.version);
    diff_eq!(format!("{path}.generation"), a.generation, b.generation);
    diff_eq!(format!("{path}.rng_state"), a.rng_state, b.rng_state);
    diff_num!(format!("{path}.remaining"), a.remaining, b.remaining);
    diff_eq!(format!("{path}.context"), a.context, b.context);
    diff_eq!(format!("{path}.intent"), a.intent, b.intent);
    diff_eq!(format!("{path}.target_x"), a.target_x, b.target_x);
    diff_eq!(format!("{path}.target_y"), a.target_y, b.target_y);
    diff_eq!(
        format!("{path}.target_player"),
        a.target_player,
        b.target_player
    );
    diff_eq!(
        format!("{path}.run_expires_at"),
        a.run_expires_at,
        b.run_expires_at
    );
    None
}

fn diff_windup_shot(
    path: &str,
    a: Option<&WindupShot>,
    b: Option<&WindupShot>,
) -> Option<MatchSnapshotDifference> {
    match (a, b) {
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => Some(diff(path, a.is_some(), b.is_some())),
        (Some(a), Some(b)) => {
            if let Some(d) = diff_vec2(format!("{path}.dir"), a.dir, b.dir) {
                return Some(d);
            }
            diff_num!(format!("{path}.speed"), a.speed, b.speed);
            diff_num!(format!("{path}.vz"), a.vz, b.vz);
            diff_num!(format!("{path}.spin"), a.spin, b.spin);
            diff_eq!(format!("{path}.shot_type"), a.shot_type, b.shot_type);
            None
        }
    }
}

#[allow(clippy::cognitive_complexity)]
fn diff_player(path: &str, a: &MatchPlayer, b: &MatchPlayer) -> Option<MatchSnapshotDifference> {
    diff_eq!(format!("{path}.id"), a.id, b.id);
    diff_eq!(format!("{path}.name"), a.name, b.name);
    diff_eq!(format!("{path}.team"), a.team, b.team);
    if let Some(d) = diff_vec2(format!("{path}.pos"), a.pos, b.pos) {
        return Some(d);
    }
    if let Some(d) = diff_vec2(format!("{path}.vel"), a.vel, b.vel) {
        return Some(d);
    }
    if let Some(d) = diff_vec2(format!("{path}.run_vel"), a.run_vel, b.run_vel) {
        return Some(d);
    }
    if let Some(d) = diff_vec2(format!("{path}.facing"), a.facing, b.facing) {
        return Some(d);
    }
    if let Some(d) = diff_vec2(format!("{path}.anchor"), a.anchor, b.anchor) {
        return Some(d);
    }
    diff_eq!(format!("{path}.species_id"), a.species_id, b.species_id);
    diff_eq!(format!("{path}.owned_verb"), a.owned_verb, b.owned_verb);
    diff_num!(format!("{path}.move_speed"), a.move_speed, b.move_speed);
    diff_num!(format!("{path}.shot_speed"), a.shot_speed, b.shot_speed);
    diff_num!(format!("{path}.dribble"), a.dribble, b.dribble);
    diff_num!(format!("{path}.strength"), a.strength, b.strength);
    diff_num!(format!("{path}.first_touch"), a.first_touch, b.first_touch);
    diff_num!(
        format!("{path}.header_skill"),
        a.header_skill,
        b.header_skill
    );
    diff_num!(
        format!("{path}.volley_skill"),
        a.volley_skill,
        b.volley_skill
    );
    diff_num!(
        format!("{path}.bicycle_skill"),
        a.bicycle_skill,
        b.bicycle_skill
    );
    diff_num!(format!("{path}.scan_rate"), a.scan_rate, b.scan_rate);
    diff_num!(format!("{path}.composure"), a.composure, b.composure);
    if let Some(d) = diff_outfield_decision(
        &format!("{path}.outfield_decision"),
        &a.outfield_decision,
        &b.outfield_decision,
    ) {
        return Some(d);
    }
    diff_eq!(format!("{path}.is_keeper"), a.is_keeper, b.is_keeper);
    diff_num!(format!("{path}.radius"), a.radius, b.radius);
    diff_num!(format!("{path}.dash_cd"), a.dash_cd, b.dash_cd);
    diff_num!(format!("{path}.dodge_cd"), a.dodge_cd, b.dodge_cd);
    diff_num!(format!("{path}.dodge_timer"), a.dodge_timer, b.dodge_timer);
    if let Some(d) = diff_vec2(format!("{path}.dodge_dir"), a.dodge_dir, b.dodge_dir) {
        return Some(d);
    }
    diff_num!(format!("{path}.reach"), a.reach, b.reach);
    diff_num!(format!("{path}.handling"), a.handling, b.handling);
    diff_num!(
        format!("{path}.keeper_aggression"),
        a.keeper_aggression,
        b.keeper_aggression
    );
    diff_num!(
        format!("{path}.keeper_anticipation"),
        a.keeper_anticipation,
        b.keeper_anticipation
    );
    diff_eq!(
        format!("{path}.keeper_state"),
        a.keeper_state,
        b.keeper_state
    );
    diff_num!(
        format!("{path}.keeper_state_timer"),
        a.keeper_state_timer,
        b.keeper_state_timer
    );
    diff_eq!(
        format!("{path}.keeper_release_state"),
        a.keeper_release_state,
        b.keeper_release_state
    );
    diff_num!(
        format!("{path}.keeper_release_motion"),
        a.keeper_release_motion,
        b.keeper_release_motion
    );
    diff_eq!(
        format!("{path}.keeper_release_kind"),
        a.keeper_release_kind,
        b.keeper_release_kind
    );
    diff_num!(
        format!("{path}.keeper_release_depth"),
        a.keeper_release_depth,
        b.keeper_release_depth
    );
    diff_num!(format!("{path}.keeper_set"), a.keeper_set, b.keeper_set);
    diff_num!(format!("{path}.dive_timer"), a.dive_timer, b.dive_timer);
    if let Some(d) = diff_vec2(format!("{path}.dive_dir"), a.dive_dir, b.dive_dir) {
        return Some(d);
    }
    diff_num!(format!("{path}.dive_delay"), a.dive_delay, b.dive_delay);
    match (a.dive_target, b.dive_target) {
        (None, None) => {}
        (Some(_), None) | (None, Some(_)) => {
            return Some(diff(
                format!("{path}.dive_target"),
                a.dive_target.is_some(),
                b.dive_target.is_some(),
            ));
        }
        (Some(at), Some(bt)) => {
            if let Some(d) = diff_vec2(format!("{path}.dive_target"), at, bt) {
                return Some(d);
            }
        }
    }
    diff_num!(
        format!("{path}.keeper_get_up_timer"),
        a.keeper_get_up_timer,
        b.keeper_get_up_timer
    );
    diff_num!(format!("{path}.hold_timer"), a.hold_timer, b.hold_timer);
    diff_eq!(format!("{path}.feet_ball"), a.feet_ball, b.feet_ball);
    diff_num!(format!("{path}.slide_timer"), a.slide_timer, b.slide_timer);
    if let Some(d) = diff_vec2(format!("{path}.slide_dir"), a.slide_dir, b.slide_dir) {
        return Some(d);
    }
    diff_num!(format!("{path}.slide_vel"), a.slide_vel, b.slide_vel);
    diff_num!(
        format!("{path}.tackle_timer"),
        a.tackle_timer,
        b.tackle_timer
    );
    diff_num!(format!("{path}.tackle_cd"), a.tackle_cd, b.tackle_cd);
    diff_num!(format!("{path}.stun_timer"), a.stun_timer, b.stun_timer);
    diff_num!(format!("{path}.grab_timer"), a.grab_timer, b.grab_timer);
    diff_num!(format!("{path}.throw_timer"), a.throw_timer, b.throw_timer);
    diff_num!(
        format!("{path}.receive_timer"),
        a.receive_timer,
        b.receive_timer
    );
    diff_num!(
        format!("{path}.sprint_meter"),
        a.sprint_meter,
        b.sprint_meter
    );
    diff_num!(format!("{path}.sprint_dur"), a.sprint_dur, b.sprint_dur);
    diff_eq!(format!("{path}.sprinting"), a.sprinting, b.sprinting);
    diff_eq!(
        format!("{path}.save_pending"),
        a.save_pending,
        b.save_pending
    );
    diff_num!(format!("{path}.save_timer"), a.save_timer, b.save_timer);
    diff_num!(format!("{path}.save_vx"), a.save_vx, b.save_vx);
    diff_eq!(format!("{path}.save_style"), a.save_style, b.save_style);
    diff_eq!(
        format!("{path}.save_tip_emitted"),
        a.save_tip_emitted,
        b.save_tip_emitted
    );
    diff_num!(
        format!("{path}.settle_timer"),
        a.settle_timer,
        b.settle_timer
    );
    diff_num!(format!("{path}.header_cd"), a.header_cd, b.header_cd);
    diff_num!(
        format!("{path}.aerial_timer"),
        a.aerial_timer,
        b.aerial_timer
    );
    diff_eq!(
        format!("{path}.aerial_style"),
        a.aerial_style,
        b.aerial_style
    );
    diff_eq!(
        format!("{path}.aerial_outcome"),
        a.aerial_outcome,
        b.aerial_outcome
    );
    diff_num!(format!("{path}.aerial_jump"), a.aerial_jump, b.aerial_jump);
    diff_num!(
        format!("{path}.aerial_recovery"),
        a.aerial_recovery,
        b.aerial_recovery
    );
    diff_num!(format!("{path}.charge"), a.charge, b.charge);
    diff_num!(format!("{path}.pass_charge"), a.pass_charge, b.pass_charge);
    diff_eq!(format!("{path}.pass_target"), a.pass_target, b.pass_target);
    diff_num!(
        format!("{path}.windup_timer"),
        a.windup_timer,
        b.windup_timer
    );
    if let Some(d) = diff_windup_shot(
        &format!("{path}.windup_shot"),
        a.windup_shot.as_ref(),
        b.windup_shot.as_ref(),
    ) {
        return Some(d);
    }
    diff_num!(
        format!("{path}.jockey_timer"),
        a.jockey_timer,
        b.jockey_timer
    );
    None
}

fn diff_event(path: &str, a: &MatchEvent, b: &MatchEvent) -> Option<MatchSnapshotDifference> {
    diff_eq!(format!("{path}.kind"), a.kind, b.kind);
    diff_num!(format!("{path}.x"), a.x, b.x);
    diff_num!(format!("{path}.y"), a.y, b.y);
    diff_eq!(format!("{path}.player"), a.player, b.player);
    diff_eq!(format!("{path}.save_style"), a.save_style, b.save_style);
    diff_eq!(format!("{path}.style"), a.style, b.style);
    diff_eq!(format!("{path}.outcome"), a.outcome, b.outcome);
    diff_eq!(format!("{path}.jumping"), a.jumping, b.jumping);
    diff_eq!(format!("{path}.difficulty"), a.difficulty, b.difficulty);
    diff_eq!(format!("{path}.shot_type"), a.shot_type, b.shot_type);
    diff_eq!(
        format!("{path}.keeper_state"),
        a.keeper_state,
        b.keeper_state
    );
    diff_eq!(
        format!("{path}.keeper_depth"),
        a.keeper_depth,
        b.keeper_depth
    );
    diff_eq!(format!("{path}.on_target"), a.on_target, b.on_target);
    None
}

fn diff_input_ownership(
    path: &str,
    a: Option<&InputOwnership>,
    b: Option<&InputOwnership>,
) -> Option<MatchSnapshotDifference> {
    match (a, b) {
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => Some(diff(path, a.is_some(), b.is_some())),
        (Some(a), Some(b)) => {
            diff_eq!(format!("{path}.version"), a.version, b.version);
            for (i, (x, y)) in a.rosters.home.iter().zip(&b.rosters.home).enumerate() {
                diff_eq!(format!("{path}.rosters.home.{}", i + 1), x, y);
            }
            for (i, (x, y)) in a.rosters.away.iter().zip(&b.rosters.away).enumerate() {
                diff_eq!(format!("{path}.rosters.away.{}", i + 1), x, y);
            }
            for (i, (x, y)) in a.slots.iter().zip(&b.slots).enumerate() {
                diff_eq!(format!("{path}.slots.{}.slot", i + 1), x.slot, y.slot);
                diff_eq!(format!("{path}.slots.{}.team", i + 1), x.team, y.team);
                diff_eq!(
                    format!("{path}.slots.{}.player_id", i + 1),
                    x.player_id,
                    y.player_id
                );
            }
            None
        }
    }
}

fn first_difference_canonical_inner(
    a: &MatchSnapshot,
    b: &MatchSnapshot,
) -> Option<MatchSnapshotDifference> {
    diff_eq!("version", a.version, b.version);
    let (sa, sb) = (&a.state, &b.state);

    diff_num!("state.field.w", sa.field.w, sb.field.w);
    diff_num!("state.field.h", sa.field.h, sb.field.h);
    diff_num!("state.goal_home.x", sa.goal_home.x, sb.goal_home.x);
    diff_num!("state.goal_home.y", sa.goal_home.y, sb.goal_home.y);
    diff_num!("state.goal_home.w", sa.goal_home.w, sb.goal_home.w);
    diff_num!("state.goal_home.h", sa.goal_home.h, sb.goal_home.h);
    diff_num!("state.goal_away.x", sa.goal_away.x, sb.goal_away.x);
    diff_num!("state.goal_away.y", sa.goal_away.y, sb.goal_away.y);
    diff_num!("state.goal_away.w", sa.goal_away.w, sb.goal_away.w);
    diff_num!("state.goal_away.h", sa.goal_away.h, sb.goal_away.h);

    diff_eq!("state.players.length", sa.players.len(), sb.players.len());
    for (index, (pa, pb)) in sa.players.iter().zip(&sb.players).enumerate() {
        if let Some(d) = diff_player(&format!("state.players.{}", index + 1), pa, pb) {
            return Some(d);
        }
    }

    if let Some(d) = diff_vec2("state.ball".to_string(), sa.ball, sb.ball) {
        return Some(d);
    }
    if let Some(d) = diff_vec2("state.ball_vel".to_string(), sa.ball_vel, sb.ball_vel) {
        return Some(d);
    }
    diff_num!("state.ball_z", sa.ball_z, sb.ball_z);
    diff_num!("state.ball_vz", sa.ball_vz, sb.ball_vz);
    diff_eq!("state.owner", sa.owner, sb.owner);
    diff_eq!("state.controlled", sa.controlled, sb.controlled);
    diff_eq!(
        "state.human_controlled",
        sa.human_controlled,
        sb.human_controlled
    );
    diff_eq!("state.score.home", sa.score.home, sb.score.home);
    diff_eq!("state.score.away", sa.score.away, sb.score.away);
    diff_num!("state.time_left", sa.time_left, sb.time_left);
    diff_eq!("state.max_goals", sa.max_goals, sb.max_goals);
    diff_eq!("state.finished", sa.finished, sb.finished);
    diff_num!("state.pickup_cd", sa.pickup_cd, sb.pickup_cd);
    diff_eq!("state.press.home", sa.press.home, sb.press.home);
    diff_eq!("state.press.away", sa.press.away, sb.press.away);

    diff_eq!(
        "state.marking.home.scheme",
        sa.marking.home.scheme,
        sb.marking.home.scheme
    );
    diff_eq!(
        "state.marking.home.man_marks",
        sa.marking.home.man_marks,
        sb.marking.home.man_marks
    );
    diff_num!(
        "state.marking.home.standoff",
        sa.marking.home.standoff,
        sb.marking.home.standoff
    );
    diff_num!(
        "state.marking.home.compactness",
        sa.marking.home.compactness,
        sb.marking.home.compactness
    );
    diff_num!(
        "state.marking.home.support",
        sa.marking.home.support,
        sb.marking.home.support
    );
    diff_eq!(
        "state.marking.away.scheme",
        sa.marking.away.scheme,
        sb.marking.away.scheme
    );
    diff_eq!(
        "state.marking.away.man_marks",
        sa.marking.away.man_marks,
        sb.marking.away.man_marks
    );
    diff_num!(
        "state.marking.away.standoff",
        sa.marking.away.standoff,
        sb.marking.away.standoff
    );
    diff_num!(
        "state.marking.away.compactness",
        sa.marking.away.compactness,
        sb.marking.away.compactness
    );
    diff_num!(
        "state.marking.away.support",
        sa.marking.away.support,
        sb.marking.away.support
    );

    for (index, (x, y)) in sa.marks.home.iter().zip(&sb.marks.home).enumerate() {
        diff_eq!(format!("state.marks.home.{}", index + 1), x, y);
    }
    for (index, (x, y)) in sa.marks.away.iter().zip(&sb.marks.away).enumerate() {
        diff_eq!(format!("state.marks.away.{}", index + 1), x, y);
    }

    diff_eq!(
        "state.outfield_press.home.version",
        sa.outfield_press.home.version,
        sb.outfield_press.home.version
    );
    diff_eq!(
        "state.outfield_press.home.presser_index",
        sa.outfield_press.home.presser_index,
        sb.outfield_press.home.presser_index
    );
    diff_eq!(
        "state.outfield_press.home.mode",
        sa.outfield_press.home.mode,
        sb.outfield_press.home.mode
    );
    diff_eq!(
        "state.outfield_press.home.reason",
        sa.outfield_press.home.reason,
        sb.outfield_press.home.reason
    );
    diff_eq!(
        "state.outfield_press.away.version",
        sa.outfield_press.away.version,
        sb.outfield_press.away.version
    );
    diff_eq!(
        "state.outfield_press.away.presser_index",
        sa.outfield_press.away.presser_index,
        sb.outfield_press.away.presser_index
    );
    diff_eq!(
        "state.outfield_press.away.mode",
        sa.outfield_press.away.mode,
        sb.outfield_press.away.mode
    );
    diff_eq!(
        "state.outfield_press.away.reason",
        sa.outfield_press.away.reason,
        sb.outfield_press.away.reason
    );

    diff_num!(
        "state.transition_windows.home.counterpress",
        sa.transition_windows.home.counterpress,
        sb.transition_windows.home.counterpress
    );
    diff_num!(
        "state.transition_windows.home.counterattack",
        sa.transition_windows.home.counterattack,
        sb.transition_windows.home.counterattack
    );
    diff_num!(
        "state.transition_windows.away.counterpress",
        sa.transition_windows.away.counterpress,
        sb.transition_windows.away.counterpress
    );
    diff_num!(
        "state.transition_windows.away.counterattack",
        sa.transition_windows.away.counterattack,
        sb.transition_windows.away.counterattack
    );

    diff_eq!(
        "state.transition.version",
        sa.transition.version,
        sb.transition.version
    );
    diff_eq!(
        "state.transition.last_team",
        sa.transition.last_team,
        sb.transition.last_team
    );
    diff_eq!(
        "state.transition.holding_team",
        sa.transition.holding_team,
        sb.transition.holding_team
    );
    diff_num!(
        "state.transition.hold",
        sa.transition.hold,
        sb.transition.hold
    );
    diff_eq!(
        "state.transition.turnover_team",
        sa.transition.turnover_team,
        sb.transition.turnover_team
    );
    diff_num!(
        "state.transition.elapsed",
        sa.transition.elapsed,
        sb.transition.elapsed
    );

    diff_eq!("state.formation.home", sa.formation.home, sb.formation.home);
    diff_eq!("state.formation.away", sa.formation.away, sb.formation.away);

    diff_num!("state.ball_spin", sa.ball_spin, sb.ball_spin);
    diff_eq!("state.rng", sa.rng, sb.rng);
    diff_num!("state.block_grace", sa.block_grace, sb.block_grace);
    diff_num!("state.aerial_lock", sa.aerial_lock, sb.aerial_lock);
    diff_num!("state.kickoff_hold", sa.kickoff_hold, sb.kickoff_hold);

    diff_eq!("state.events.length", sa.events.len(), sb.events.len());
    for (index, (ea, eb)) in sa.events.iter().zip(&sb.events).enumerate() {
        if let Some(d) = diff_event(&format!("state.events.{}", index + 1), ea, eb) {
            return Some(d);
        }
    }

    if let Some(d) = diff_input_ownership(
        "state.input_ownership",
        sa.input_ownership.as_ref(),
        sb.input_ownership.as_ref(),
    ) {
        return Some(d);
    }

    for (index, (x, y)) in sa.slot_players.iter().zip(&sb.slot_players).enumerate() {
        diff_eq!(format!("state.slot_players.{}", index + 1), x, y);
    }
    for (index, (x, y)) in sa
        .slot_for_player
        .iter()
        .zip(&sb.slot_for_player)
        .enumerate()
    {
        diff_eq!(format!("state.slot_for_player.{}", index + 1), x, y);
    }
    diff_eq!("state.input_tick", sa.input_tick, sb.input_tick);

    combat_snapshot::first_difference(a.combat.as_ref(), b.combat.as_ref(), "combat").map(|d| {
        MatchSnapshotDifference {
            path: d.path,
            expected: d.expected,
            actual: d.actual,
        }
    })
}

/// The first structural disagreement between two [`MatchSnapshot`]s,
/// restoring and re-capturing both first so schema drift and canonicalized
/// values are what get compared.
#[must_use]
pub fn first_difference(
    left: &MatchSnapshot,
    right: &MatchSnapshot,
) -> Option<MatchSnapshotDifference> {
    let (left_state, left_combat) = restore(left);
    let (right_state, right_combat) = restore(right);
    let a = capture(&left_state, left_combat.as_ref());
    let b = capture(&right_state, right_combat.as_ref());
    first_difference_canonical_inner(&a, &b)
}

/// Compare snapshots already returned by [`capture`], or equally canonical
/// owned copies, without repeating their full restore/capture validation
/// pass.
#[must_use]
pub fn first_difference_canonical(
    left: &MatchSnapshot,
    right: &MatchSnapshot,
) -> Option<MatchSnapshotDifference> {
    first_difference_canonical_inner(left, right)
}
