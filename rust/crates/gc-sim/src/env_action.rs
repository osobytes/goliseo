//! An action is the same abstract intent a human or a #112 bot expresses: a
//! continuous move vector plus named held actions and one-shot edges. It is
//! quantized into the canonical [`InputSample`] that `sim::match` already
//! consumes, so a policy cannot address resolver internals, teleport, set
//! ownership or outcomes, skip recovery or cooldowns, or act between fixed
//! ticks. Anything a policy could want that is not in this contract simply
//! does not exist as an input.
//!
//! Legality masks are derived from an observation, not from `MatchState`. A
//! mask built from a representative view therefore cannot contain privileged
//! legality by construction; a mask built from the privileged profile is
//! tagged as such.
//!
//! This is on the wire format: [`to_sample`]/[`from_sample`] feed and read
//! the same [`InputSample`] that crosses the network and rollback resim, so
//! ARCHITECTURE.md §3 rule 3's wire exception applies — no index or tag here is
//! renumbered for internal convenience.
//!
//! [`validate`] is the sole entry point that accepts genuinely untyped
//! external input; every other function here is typed against the
//! normalized [`EnvSlotAction`].

use crate::input_frame::{self, EdgeAction, HeldAction, InputSample, InputSampleOptions};
use indexmap::IndexMap;

/// The env action module version.
pub const VERSION: i64 = 1;

/// The eight canonical held actions, in canonical order.
pub const HELD_ACTIONS: [HeldAction; 8] = [
    HeldAction::Shoot,
    HeldAction::Pass,
    HeldAction::Sprint,
    HeldAction::Jockey,
    HeldAction::Lob,
    HeldAction::AerialStrike,
    HeldAction::AerialAcrobatic,
    HeldAction::Equipment,
];

/// The seven canonical one-tick edge actions, in canonical order.
pub const EDGE_ACTIONS: [EdgeAction; 7] = [
    EdgeAction::Shoot,
    EdgeAction::Pass,
    EdgeAction::Switch,
    EdgeAction::Dash,
    EdgeAction::Dodge,
    EdgeAction::EquipmentPressed,
    EdgeAction::EquipmentReleased,
];

const MOVE_EPSILON: f64 = 1e-9;

/// Failure reasons an env-action operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvActionErrorCode {
    /// The data violates a structural or type invariant.
    Malformed,
    /// A move vector lies outside the unit disc.
    MoveOutOfRange,
    /// A held-action key names something outside [`HELD_ACTIONS`].
    UnknownHeldAction,
    /// An edge-action key names something outside [`EDGE_ACTIONS`].
    UnknownEdgeAction,
    /// The action is well-formed but a mask does not permit it this tick.
    UnavailableAction,
}

/// An expected, recoverable env-action failure (ARCHITECTURE.md §3 rule 5): actions
/// come from a policy (external input), so an illegal action is a
/// recoverable rejection with a machine-readable reason, never a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvActionError {
    /// Machine-readable failure reason.
    pub code: EnvActionErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl EnvActionError {
    fn new(code: EnvActionErrorCode, message: impl Into<String>) -> Self {
        EnvActionError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EnvActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EnvActionError {}

/// Result alias for fallible env-action operations.
pub type Result<T> = std::result::Result<T, EnvActionError>;

fn failure<T>(code: EnvActionErrorCode, message: impl Into<String>) -> Result<T> {
    Err(EnvActionError::new(code, message))
}

fn wrap<T>(code: EnvActionErrorCode, result: input_frame::Result<T>) -> Result<T> {
    result.map_err(|e| EnvActionError::new(code, e.message))
}

// ---------------------------------------------------------------------------
// Raw, not-yet-validated external input.
//
// `validate` receives a policy's action before anything about its shape is
// known. A typed Rust struct cannot represent "an extra unknown field" or
// "a value of the wrong type", so this small untyped-table shape stands in
// for that one boundary. Everything downstream of `validate` works with the
// fully typed `EnvSlotAction`.
// ---------------------------------------------------------------------------

/// One field's value in a [`RawAction`]/[`RawTable`], before validation
/// confirms its type.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    /// A numeric field (`f64` per ARCHITECTURE.md §3 rule 1).
    Number(f64),
    /// A boolean field.
    Bool(bool),
    /// A nested table.
    Table(RawTable),
}

/// A raw table of named fields, order-independent — nothing here depends on
/// key order.
pub type RawTable = IndexMap<String, RawValue>;

/// The untyped payload offered to [`validate`]: either a table of fields, or
/// something else entirely.
#[derive(Clone, Debug, PartialEq)]
pub enum RawAction {
    /// A table of fields.
    Table(RawTable),
    /// Anything that is not a table (a bare string, number, etc).
    Other,
}

fn as_table(value: &RawValue) -> Option<&RawTable> {
    match value {
        RawValue::Table(table) => Some(table),
        _ => None,
    }
}

fn as_bool(value: &RawValue) -> Option<bool> {
    match value {
        RawValue::Bool(b) => Some(*b),
        _ => None,
    }
}

/// A numeric field, defaulting to zero when absent. A present-but-wrong-type
/// value becomes `NaN`, which fails the caller's next `is_finite` check.
fn number_or_zero(value: Option<&RawValue>) -> f64 {
    match value {
        None => 0.0,
        Some(RawValue::Number(n)) => *n,
        Some(_) => f64::NAN,
    }
}

fn number_equals(value: Option<&RawValue>, target: f64) -> bool {
    matches!(value, Some(RawValue::Number(n)) if *n == target)
}

// ---------------------------------------------------------------------------
// Normalized, typed shapes.
// ---------------------------------------------------------------------------

/// Which observation lens a view or mask was derived from. Declared locally
/// rather than imported from [`crate::env_config`], which declares a
/// naming-compatible type of the same name — keeping this module's
/// dependency surface no wider than [`crate::input_frame`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnvObservationProfile {
    /// One controlled slot restricted to presented, current cues.
    #[default]
    Representative,
    /// Several controlled slots, each still player-observable.
    Team,
    /// Authoritative oracle/debug profile; never a human proxy.
    Privileged,
}

/// A continuous move vector, inside the unit disc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvMove {
    /// Horizontal axis.
    pub x: f64,
    /// Vertical axis.
    pub y: f64,
}

/// Continuously held intents for one tick. Named fields rather than a
/// string-keyed map: the key set is exactly the eight entries in
/// [`HELD_ACTIONS`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvHeldActions {
    /// Shoot held.
    pub shoot: bool,
    /// Pass held.
    pub pass: bool,
    /// Sprint held.
    pub sprint: bool,
    /// Jockey held.
    pub jockey: bool,
    /// Lob held.
    pub lob: bool,
    /// Aerial strike held.
    pub aerial_strike: bool,
    /// Aerial acrobatic held.
    pub aerial_acrobatic: bool,
    /// Equipment action held.
    pub equipment: bool,
}

/// One-shot intents that fired on this tick only. Named fields rather than a
/// string-keyed map: the key set is exactly the seven entries in
/// [`EDGE_ACTIONS`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvEdgeActions {
    /// Shoot pressed this tick.
    pub shoot: bool,
    /// Pass pressed this tick.
    pub pass: bool,
    /// Switch pressed this tick. Always illegal under fixed-slot routing —
    /// kept as a field so the contract can name and reject it explicitly
    /// instead of silently dropping it.
    pub switch: bool,
    /// Dash pressed this tick.
    pub dash: bool,
    /// Dodge pressed this tick.
    pub dodge: bool,
    /// Equipment action pressed this tick.
    pub equipment_pressed: bool,
    /// Equipment action released this tick.
    pub equipment_released: bool,
}

/// A normalized, fully-typed learning-environment action for one slot: the
/// output of [`validate`]/[`from_sample`], and the input every other
/// function in this module accepts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvSlotAction {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Continuous move vector. Named `movement` rather than `move` — `move`
    /// is a Rust keyword.
    pub movement: EnvMove,
    /// Continuously held intents.
    pub held: EnvHeldActions,
    /// One-shot edges that fired this tick only.
    pub edges: EnvEdgeActions,
}

fn get_held(actions: &EnvHeldActions, action: HeldAction) -> bool {
    match action {
        HeldAction::Shoot => actions.shoot,
        HeldAction::Pass => actions.pass,
        HeldAction::Sprint => actions.sprint,
        HeldAction::Jockey => actions.jockey,
        HeldAction::Lob => actions.lob,
        HeldAction::AerialStrike => actions.aerial_strike,
        HeldAction::AerialAcrobatic => actions.aerial_acrobatic,
        HeldAction::Equipment => actions.equipment,
    }
}

fn set_held(actions: &mut EnvHeldActions, action: HeldAction, value: bool) {
    match action {
        HeldAction::Shoot => actions.shoot = value,
        HeldAction::Pass => actions.pass = value,
        HeldAction::Sprint => actions.sprint = value,
        HeldAction::Jockey => actions.jockey = value,
        HeldAction::Lob => actions.lob = value,
        HeldAction::AerialStrike => actions.aerial_strike = value,
        HeldAction::AerialAcrobatic => actions.aerial_acrobatic = value,
        HeldAction::Equipment => actions.equipment = value,
    }
}

fn get_edge(actions: &EnvEdgeActions, action: EdgeAction) -> bool {
    match action {
        EdgeAction::Shoot => actions.shoot,
        EdgeAction::Pass => actions.pass,
        EdgeAction::Switch => actions.switch,
        EdgeAction::Dash => actions.dash,
        EdgeAction::Dodge => actions.dodge,
        EdgeAction::EquipmentPressed => actions.equipment_pressed,
        EdgeAction::EquipmentReleased => actions.equipment_released,
    }
}

fn set_edge(actions: &mut EnvEdgeActions, action: EdgeAction, value: bool) {
    match action {
        EdgeAction::Shoot => actions.shoot = value,
        EdgeAction::Pass => actions.pass = value,
        EdgeAction::Switch => actions.switch = value,
        EdgeAction::Dash => actions.dash = value,
        EdgeAction::Dodge => actions.dodge = value,
        EdgeAction::EquipmentPressed => actions.equipment_pressed = value,
        EdgeAction::EquipmentReleased => actions.equipment_released = value,
    }
}

fn held_action_name(action: HeldAction) -> &'static str {
    match action {
        HeldAction::Shoot => "shoot",
        HeldAction::Pass => "pass",
        HeldAction::Sprint => "sprint",
        HeldAction::Jockey => "jockey",
        HeldAction::Lob => "lob",
        HeldAction::AerialStrike => "aerial_strike",
        HeldAction::AerialAcrobatic => "aerial_acrobatic",
        HeldAction::Equipment => "equipment",
    }
}

fn held_action_from_name(name: &str) -> Option<HeldAction> {
    HELD_ACTIONS
        .into_iter()
        .find(|action| held_action_name(*action) == name)
}

fn edge_action_name(action: EdgeAction) -> &'static str {
    match action {
        EdgeAction::Shoot => "shoot",
        EdgeAction::Pass => "pass",
        EdgeAction::Switch => "switch",
        EdgeAction::Dash => "dash",
        EdgeAction::Dodge => "dodge",
        EdgeAction::EquipmentPressed => "equipment_pressed",
        EdgeAction::EquipmentReleased => "equipment_released",
    }
}

fn edge_action_from_name(name: &str) -> Option<EdgeAction> {
    EDGE_ACTIONS
        .into_iter()
        .find(|action| edge_action_name(*action) == name)
}

/// A no-op action: no movement, nothing held, no edges.
#[must_use]
pub fn neutral() -> EnvSlotAction {
    EnvSlotAction {
        version: VERSION,
        movement: EnvMove { x: 0.0, y: 0.0 },
        held: EnvHeldActions::default(),
        edges: EnvEdgeActions::default(),
    }
}

/// Validate one slot action and return a normalized copy. Actions come from
/// a policy (external input), so an illegal action is a recoverable
/// rejection with a machine-readable reason, never an assert.
pub fn validate(action: &RawAction) -> Result<EnvSlotAction> {
    let table = match action {
        RawAction::Table(table) => table,
        RawAction::Other => {
            return failure(
                EnvActionErrorCode::Malformed,
                "action must be a table of known fields",
            );
        }
    };
    for key in table.keys() {
        if !matches!(key.as_str(), "version" | "move" | "held" | "edges") {
            return failure(
                EnvActionErrorCode::Malformed,
                "action must be a table of known fields",
            );
        }
    }
    if table.contains_key("version") && !number_equals(table.get("version"), VERSION as f64) {
        return failure(
            EnvActionErrorCode::Malformed,
            "unsupported env action version",
        );
    }

    let movement = match table.get("move") {
        None => EnvMove { x: 0.0, y: 0.0 },
        Some(value) => {
            let move_table = as_table(value).ok_or_else(|| {
                EnvActionError::new(
                    EnvActionErrorCode::Malformed,
                    "action.move must be an { x, y } table",
                )
            })?;
            for key in move_table.keys() {
                if !matches!(key.as_str(), "x" | "y") {
                    return failure(
                        EnvActionErrorCode::Malformed,
                        "action.move must be an { x, y } table",
                    );
                }
            }
            let x = number_or_zero(move_table.get("x"));
            let y = number_or_zero(move_table.get("y"));
            if !x.is_finite() || !y.is_finite() {
                return failure(
                    EnvActionErrorCode::Malformed,
                    "action.move components must be finite numbers",
                );
            }
            if x * x + y * y > 1.0 + MOVE_EPSILON {
                return failure(
                    EnvActionErrorCode::MoveOutOfRange,
                    "action.move must lie inside the unit disc",
                );
            }
            EnvMove { x, y }
        }
    };

    let mut held = EnvHeldActions::default();
    if let Some(value) = table.get("held") {
        let held_table = as_table(value).ok_or_else(|| {
            EnvActionError::new(EnvActionErrorCode::Malformed, "action.held must be a table")
        })?;
        for (key, value) in held_table {
            let Some(held_action) = held_action_from_name(key) else {
                return failure(
                    EnvActionErrorCode::UnknownHeldAction,
                    format!("unknown held action: {key}"),
                );
            };
            let Some(flag) = as_bool(value) else {
                return failure(
                    EnvActionErrorCode::Malformed,
                    format!("held action {key} must be a boolean"),
                );
            };
            set_held(&mut held, held_action, flag);
        }
    }

    let mut edges = EnvEdgeActions::default();
    if let Some(value) = table.get("edges") {
        let edges_table = as_table(value).ok_or_else(|| {
            EnvActionError::new(
                EnvActionErrorCode::Malformed,
                "action.edges must be a table",
            )
        })?;
        for (key, value) in edges_table {
            let Some(edge_action) = edge_action_from_name(key) else {
                return failure(
                    EnvActionErrorCode::UnknownEdgeAction,
                    format!("unknown edge action: {key}"),
                );
            };
            let Some(flag) = as_bool(value) else {
                return failure(
                    EnvActionErrorCode::Malformed,
                    format!("edge action {key} must be a boolean"),
                );
            };
            set_edge(&mut edges, edge_action, flag);
        }
    }

    Ok(EnvSlotAction {
        version: VERSION,
        movement,
        held,
        edges,
    })
}

/// Drop the one-shot edges, keeping held intents. Used when one action is
/// held across several fixed ticks: an edge means "this tick only", so
/// repeating it would fabricate presses the policy never made.
#[must_use]
pub fn without_edges(action: &EnvSlotAction) -> EnvSlotAction {
    EnvSlotAction {
        version: VERSION,
        movement: action.movement,
        held: action.held,
        edges: EnvEdgeActions::default(),
    }
}

/// Re-check a normalized action's version and move vector. `to_sample` and
/// `check_mask`'s `EnvSlotAction`-typed parameter can still carry an
/// out-of-range move vector or a stale version, since neither is enforced by
/// construction.
fn validate_normalized(action: &EnvSlotAction) -> Result<()> {
    if action.version != VERSION {
        return failure(
            EnvActionErrorCode::Malformed,
            "unsupported env action version",
        );
    }
    let EnvMove { x, y } = action.movement;
    if !x.is_finite() || !y.is_finite() {
        return failure(
            EnvActionErrorCode::Malformed,
            "action.move components must be finite numbers",
        );
    }
    if x * x + y * y > 1.0 + MOVE_EPSILON {
        return failure(
            EnvActionErrorCode::MoveOutOfRange,
            "action.move must lie inside the unit disc",
        );
    }
    Ok(())
}

/// Quantize a normalized action into the canonical [`InputSample`] wire
/// format. Any index or bit that ends up in the encoded output keeps its
/// `input_frame` value (ARCHITECTURE.md §3 rule 3's wire exception).
pub fn to_sample(action: &EnvSlotAction) -> Result<InputSample> {
    validate_normalized(action)?;
    let (move_x, move_y) = wrap(
        EnvActionErrorCode::MoveOutOfRange,
        input_frame::quantize_move(action.movement.x, action.movement.y),
    )?;
    let mut held_mask: i64 = 0;
    for held_action in HELD_ACTIONS {
        if get_held(&action.held, held_action) {
            held_mask |= held_action.bit();
        }
    }
    let mut edge_mask: i64 = 0;
    for edge_action in EDGE_ACTIONS {
        if get_edge(&action.edges, edge_action) {
            edge_mask |= edge_action.bit();
        }
    }
    wrap(
        EnvActionErrorCode::Malformed,
        input_frame::new_sample(InputSampleOptions {
            move_x: Some(move_x),
            move_y: Some(move_y),
            held: Some(held_mask),
            edges: Some(edge_mask),
        }),
    )
}

/// Decode a wire [`InputSample`] back into a normalized action.
pub fn from_sample(sample: &InputSample) -> Result<EnvSlotAction> {
    let (move_x, move_y) = wrap(
        EnvActionErrorCode::Malformed,
        input_frame::dequantize_move(sample),
    )?;
    let mut held = EnvHeldActions::default();
    for held_action in HELD_ACTIONS {
        let is_held = wrap(
            EnvActionErrorCode::Malformed,
            input_frame::is_held(sample, held_action),
        )?;
        set_held(&mut held, held_action, is_held);
    }
    let mut edges = EnvEdgeActions::default();
    for edge_action in EDGE_ACTIONS {
        let has_edge = wrap(
            EnvActionErrorCode::Malformed,
            input_frame::has_edge(sample, edge_action),
        )?;
        set_edge(&mut edges, edge_action, has_edge);
    }
    Ok(EnvSlotAction {
        version: VERSION,
        movement: EnvMove {
            x: move_x,
            y: move_y,
        },
        held,
        edges,
    })
}

// ---------------------------------------------------------------------------
// Legality masks.
// ---------------------------------------------------------------------------

/// One slot's equipment state, as far as [`mask`] needs to know.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvActionViewEquipment {
    /// Whether the equipped action is off cooldown and unforced.
    pub ready: bool,
}

/// The self-state fields [`mask`] reads from a slot view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnvActionViewSelf {
    /// Currently stunned (closes tackle/dodge/aerial intents).
    pub stunned: bool,
    /// Off cooldown for a header/aerial contest.
    pub header_ready: bool,
    /// Off cooldown for a standing tackle.
    pub tackle_ready: bool,
    /// Off cooldown for a dodge.
    pub dodge_ready: bool,
    /// Equipped loadout state, when the slot has a loadout.
    pub equipment: Option<EnvActionViewEquipment>,
}

/// The ball state fields [`mask`] reads from a slot view.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnvActionViewBall {
    /// Whether the ball is currently airborne.
    pub airborne: bool,
}

/// The narrow slot projection an action mask needs. It is a strict subset of
/// [`crate::env_observation`]'s `EnvSlotView`. Only `own` and `ball` are
/// read, so callers on the per-step path can build one of these instead of a
/// whole observation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnvActionView {
    /// Which observation lens this view was derived from.
    pub profile: EnvObservationProfile,
    /// Canonical input slot this view belongs to.
    pub slot: i64,
    /// The viewed slot's own equipment/readiness state.
    pub own: EnvActionViewSelf,
    /// The viewed ball state.
    pub ball: EnvActionViewBall,
}

/// Legality a normal client can know, computed only from the fields present
/// in a view. The representative and team profiles carry no private timers,
/// so the resulting mask cannot encode privileged legality; the privileged
/// profile is flagged so a report can never present it as a human-equivalent
/// mask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvActionMask {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Which observation lens this mask was derived from.
    pub profile: EnvObservationProfile,
    /// True when the mask was derived from a privileged view.
    pub privileged: bool,
    /// Canonical input slot this mask belongs to.
    pub slot: i64,
    /// Movement is always legal; kept explicit for consumers.
    pub movement: bool,
    /// Legal held actions this tick.
    pub held: EnvHeldActions,
    /// Legal edge actions this tick.
    pub edges: EnvEdgeActions,
}

/// Derive an action mask from a slot view. Fixed-slot routing gives every
/// slot exactly one player for the whole fixture, so the legacy "switch to
/// the outfielder nearest the ball" intent has no meaning here: `switch` is
/// always closed.
#[must_use]
pub fn mask(view: &EnvActionView) -> EnvActionMask {
    let equipped = view.own.equipment.is_some();
    let equipment_ready = view.own.equipment.is_some_and(|e| e.ready);
    let aerial_available = view.own.header_ready && view.ball.airborne && !view.own.stunned;
    EnvActionMask {
        version: VERSION,
        profile: view.profile,
        privileged: view.profile == EnvObservationProfile::Privileged,
        slot: view.slot,
        movement: true,
        held: EnvHeldActions {
            shoot: true,
            pass: true,
            sprint: true,
            jockey: true,
            lob: true,
            aerial_strike: aerial_available,
            aerial_acrobatic: aerial_available,
            equipment: equipped,
        },
        edges: EnvEdgeActions {
            shoot: true,
            pass: true,
            switch: false,
            dash: view.own.tackle_ready && !view.own.stunned,
            dodge: view.own.dodge_ready && !view.own.stunned,
            equipment_pressed: equipment_ready,
            equipment_released: equipped,
        },
    }
}

/// Check a validated action against a mask and name the first violation.
/// The environment reports this reason back to the caller so a policy
/// learns why an intent was refused instead of silently losing it.
pub fn check_mask(action: &EnvSlotAction, mask: &EnvActionMask) -> Result<bool> {
    validate_normalized(action)?;
    for held_action in HELD_ACTIONS {
        if get_held(&action.held, held_action) && !get_held(&mask.held, held_action) {
            return failure(
                EnvActionErrorCode::UnavailableAction,
                format!(
                    "held action {} is not available to slot {} this tick",
                    held_action_name(held_action),
                    mask.slot
                ),
            );
        }
    }
    for edge_action in EDGE_ACTIONS {
        if get_edge(&action.edges, edge_action) && !get_edge(&mask.edges, edge_action) {
            let reason = if edge_action == EdgeAction::Switch {
                format!(
                    "edge action switch does not exist under fixed-slot routing; slot {} controls one player for the whole fixture",
                    mask.slot
                )
            } else {
                format!(
                    "edge action {} is not available to slot {} this tick",
                    edge_action_name(edge_action),
                    mask.slot
                )
            };
            return failure(EnvActionErrorCode::UnavailableAction, reason);
        }
    }
    Ok(true)
}
