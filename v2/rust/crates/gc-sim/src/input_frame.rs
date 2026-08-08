//! Port of `sim/input_frame.lua`.
//!
//! Pure, versioned input records for the multiplayer-shaped simulation. This
//! module deliberately knows neither render input nor transport delivery.
//!
//! This is the wire format: its bytes go on the network *and* into rollback
//! re-simulation, so two clients must encode identically or the match
//! desyncs. [`encode`]/[`decode`] are differential-tested against the
//! reference Lua implementation (see `v2/tools/lua_reference`).
//!
//! Many of the Lua original's runtime shape checks (`has_only_fields`,
//! `is_canonical_array`, `is_integer` on values that are already integers)
//! validate that an untyped Lua table has the shape a typed language
//! guarantees at compile time. Those checks are dropped here as structurally
//! redundant; the bound/range/uniqueness checks they wrapped around are kept.

use gc_data::players::{PlayerData, Position};
use indexmap::IndexMap;

/// Frame format version. Bumping this is a breaking wire change.
pub const VERSION: i64 = 2;
/// Stable outfield input slots on the home side.
pub const HOME_SLOT_COUNT: i64 = 4;
/// Stable outfield input slots on the away side.
pub const AWAY_SLOT_COUNT: i64 = 4;
/// Total canonical input slots per frame.
pub const SLOT_COUNT: i64 = HOME_SLOT_COUNT + AWAY_SLOT_COUNT;
/// Players per fixture side, including the (unslotted) keeper.
pub const FIXTURE_TEAM_SIZE: i64 = HOME_SLOT_COUNT + 1;
/// Quantized movement axis magnitude bound.
pub const MOVE_SCALE: i64 = 127;
/// Largest representable input tick.
pub const MAX_TICK: i64 = 2_147_483_647;
/// Longest allowed player id, in bytes.
pub const MAX_PLAYER_ID_BYTES: usize = 64;
/// Compact per-sample wire size: two hex characters per sample byte.
pub const MAX_SAMPLE_WIRE_BYTES: usize = 8;
/// Longest allowed encoded frame, in bytes.
pub const MAX_WIRE_BYTES: usize = 156;

/// Held-action bit, `shoot`.
pub const HELD_SHOOT: i64 = 1;
/// Held-action bit, `pass`.
pub const HELD_PASS: i64 = 2;
/// Held-action bit, `sprint`.
pub const HELD_SPRINT: i64 = 4;
/// Held-action bit, `jockey`.
pub const HELD_JOCKEY: i64 = 8;
/// Held-action bit, `lob`.
pub const HELD_LOB: i64 = 16;
/// Held-action bit, `aerial_strike`.
pub const HELD_AERIAL_STRIKE: i64 = 32;
/// Held-action bit, `aerial_acrobatic`.
pub const HELD_AERIAL_ACROBATIC: i64 = 64;
/// Held-action bit, `equipment`.
pub const HELD_EQUIPMENT: i64 = 128;

/// Edge-action bit, `shoot`.
pub const EDGE_SHOOT: i64 = 1;
/// Edge-action bit, `pass`.
pub const EDGE_PASS: i64 = 2;
/// Edge-action bit, `switch`.
pub const EDGE_SWITCH: i64 = 4;
/// Edge-action bit, `dash`.
pub const EDGE_DASH: i64 = 8;
/// Edge-action bit, `dodge`.
pub const EDGE_DODGE: i64 = 16;
/// Edge-action bit, `equipment_pressed`.
pub const EDGE_EQUIPMENT_PRESSED: i64 = 32;
/// Edge-action bit, `equipment_released`.
pub const EDGE_EQUIPMENT_RELEASED: i64 = 64;

const MAX_HELD_MASK: i64 = 255;
const MAX_EDGE_MASK: i64 = 127;

/// One of the eight canonical held actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldAction {
    /// Shoot.
    Shoot,
    /// Pass.
    Pass,
    /// Sprint.
    Sprint,
    /// Jockey (contain, slow backpedal).
    Jockey,
    /// Lob.
    Lob,
    /// Aerial strike.
    AerialStrike,
    /// Aerial acrobatic (bicycle-kick family).
    AerialAcrobatic,
    /// Equipment action held.
    Equipment,
}

impl HeldAction {
    /// This action's bit in an [`InputSample::held`] mask.
    #[must_use]
    pub fn bit(self) -> i64 {
        match self {
            HeldAction::Shoot => HELD_SHOOT,
            HeldAction::Pass => HELD_PASS,
            HeldAction::Sprint => HELD_SPRINT,
            HeldAction::Jockey => HELD_JOCKEY,
            HeldAction::Lob => HELD_LOB,
            HeldAction::AerialStrike => HELD_AERIAL_STRIKE,
            HeldAction::AerialAcrobatic => HELD_AERIAL_ACROBATIC,
            HeldAction::Equipment => HELD_EQUIPMENT,
        }
    }
}

/// One of the seven canonical one-tick edge actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeAction {
    /// Shoot pressed this tick.
    Shoot,
    /// Pass pressed this tick.
    Pass,
    /// Switch pressed this tick.
    Switch,
    /// Dash pressed this tick.
    Dash,
    /// Dodge pressed this tick.
    Dodge,
    /// Equipment action pressed this tick.
    EquipmentPressed,
    /// Equipment action released this tick.
    EquipmentReleased,
}

impl EdgeAction {
    /// This action's bit in an [`InputSample::edges`] mask.
    #[must_use]
    pub fn bit(self) -> i64 {
        match self {
            EdgeAction::Shoot => EDGE_SHOOT,
            EdgeAction::Pass => EDGE_PASS,
            EdgeAction::Switch => EDGE_SWITCH,
            EdgeAction::Dash => EDGE_DASH,
            EdgeAction::Dodge => EDGE_DODGE,
            EdgeAction::EquipmentPressed => EDGE_EQUIPMENT_PRESSED,
            EdgeAction::EquipmentReleased => EDGE_EQUIPMENT_RELEASED,
        }
    }
}

/// A fixture side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    /// The home side.
    Home,
    /// The away side.
    Away,
}

/// One of the eight canonical input slot identities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotId {
    /// Home outfield slot 1.
    Home1,
    /// Home outfield slot 2.
    Home2,
    /// Home outfield slot 3.
    Home3,
    /// Home outfield slot 4.
    Home4,
    /// Away outfield slot 1.
    Away1,
    /// Away outfield slot 2.
    Away2,
    /// Away outfield slot 3.
    Away3,
    /// Away outfield slot 4.
    Away4,
}

/// A canonical input slot: its stable position, identity, side, and outfield
/// ordinal within that side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSlot {
    /// Canonical position in every [`InputFrame`] (one through eight).
    pub index: i64,
    /// Stable identity.
    pub id: SlotId,
    /// Fixture side.
    pub team: Team,
    /// Stable one-based outfield position within the team.
    pub outfield_index: i64,
}

const SLOT_ORDER: [InputSlot; 8] = [
    InputSlot {
        index: 1,
        id: SlotId::Home1,
        team: Team::Home,
        outfield_index: 1,
    },
    InputSlot {
        index: 2,
        id: SlotId::Home2,
        team: Team::Home,
        outfield_index: 2,
    },
    InputSlot {
        index: 3,
        id: SlotId::Home3,
        team: Team::Home,
        outfield_index: 3,
    },
    InputSlot {
        index: 4,
        id: SlotId::Home4,
        team: Team::Home,
        outfield_index: 4,
    },
    InputSlot {
        index: 5,
        id: SlotId::Away1,
        team: Team::Away,
        outfield_index: 1,
    },
    InputSlot {
        index: 6,
        id: SlotId::Away2,
        team: Team::Away,
        outfield_index: 2,
    },
    InputSlot {
        index: 7,
        id: SlotId::Away3,
        team: Team::Away,
        outfield_index: 3,
    },
    InputSlot {
        index: 8,
        id: SlotId::Away4,
        team: Team::Away,
        outfield_index: 4,
    },
];

/// Failure reasons an [`InputFrame`]/[`InputSample`]/[`InputOwnership`]
/// operation can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputFrameErrorCode {
    /// The data violates a structural or range invariant.
    Malformed,
    /// The data names a frame/ownership version this build does not support.
    UnsupportedVersion,
    /// The encoded wire would exceed [`MAX_WIRE_BYTES`].
    WireTooLarge,
}

/// An expected, recoverable input-frame failure (README rule 5.5): the
/// caller is meant to handle it, not a programmer error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFrameError {
    /// Machine-readable failure reason.
    pub code: InputFrameErrorCode,
    /// Human-readable detail.
    pub message: String,
}

impl InputFrameError {
    fn new(code: InputFrameErrorCode, message: impl Into<String>) -> Self {
        InputFrameError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InputFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InputFrameError {}

/// Result alias for fallible input-frame operations.
pub type Result<T> = std::result::Result<T, InputFrameError>;

fn failure<T>(code: InputFrameErrorCode, message: impl Into<String>) -> Result<T> {
    Err(InputFrameError::new(code, message))
}

/// One tick's quantized input from one slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct InputSample {
    /// Signed quantized horizontal axis in `[-127, 127]`.
    pub move_x: i64,
    /// Signed quantized vertical axis in `[-127, 127]`.
    pub move_y: i64,
    /// Bitmask of [`HeldAction`] values currently held for this tick.
    pub held: i64,
    /// Bitmask of [`EdgeAction`] values that occurred during this tick only.
    pub edges: i64,
}

/// Optional overrides for building an [`InputSample`]; unset fields default
/// to zero, matching [`InputSample::default`].
#[derive(Clone, Copy, Debug, Default)]
pub struct InputSampleOptions {
    /// Overrides [`InputSample::move_x`].
    pub move_x: Option<i64>,
    /// Overrides [`InputSample::move_y`].
    pub move_y: Option<i64>,
    /// Overrides [`InputSample::held`].
    pub held: Option<i64>,
    /// Overrides [`InputSample::edges`].
    pub edges: Option<i64>,
}

impl From<InputSample> for InputSampleOptions {
    fn from(sample: InputSample) -> Self {
        InputSampleOptions {
            move_x: Some(sample.move_x),
            move_y: Some(sample.move_y),
            held: Some(sample.held),
            edges: Some(sample.edges),
        }
    }
}

fn is_axis(value: i64) -> bool {
    (-MOVE_SCALE..=MOVE_SCALE).contains(&value)
}

fn is_mask(value: i64, maximum: i64) -> bool {
    (0..=maximum).contains(&value)
}

fn has_bit(mask: i64, bit: i64) -> bool {
    mask & bit != 0
}

/// Validate a sample's movement axes, masks, and equipment edge/hold
/// consistency.
pub fn validate_sample(sample: &InputSample) -> Result<()> {
    if !is_axis(sample.move_x) || !is_axis(sample.move_y) {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample movement axes must be signed 8-bit integers",
        );
    }
    if !is_mask(sample.held, MAX_HELD_MASK) {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample held mask is invalid",
        );
    }
    if !is_mask(sample.edges, MAX_EDGE_MASK) {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample edge mask is invalid",
        );
    }
    let equipment_held = has_bit(sample.held, HELD_EQUIPMENT);
    let equipment_pressed = has_bit(sample.edges, EDGE_EQUIPMENT_PRESSED);
    let equipment_released = has_bit(sample.edges, EDGE_EQUIPMENT_RELEASED);
    if (equipment_pressed && !equipment_released && !equipment_held)
        || (equipment_released && equipment_held)
    {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample equipment transition combination is invalid",
        );
    }
    Ok(())
}

/// The compact packet codec uses one explicit byte for each version-2 sample
/// component. Axes are biased into `[0, 254]`; held and edge masks retain
/// their complete canonical bytes. Lowercase hexadecimal keeps the result
/// ASCII-safe without allowing a transport to infer or repeat combat edges.
pub fn encode_sample(sample: &InputSample) -> Result<String> {
    validate_sample(sample)?;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}",
        sample.move_x + MOVE_SCALE,
        sample.move_y + MOVE_SCALE,
        sample.held,
        sample.edges
    ))
}

/// Decode a compact four-byte sample. Rejects any wire that is not the
/// canonical encoding of the sample it decodes to.
pub fn decode_sample(wire: &str) -> Result<InputSample> {
    if wire.len() != MAX_SAMPLE_WIRE_BYTES
        || !wire
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample wire must be eight lowercase hex characters encoding four bytes",
        );
    }
    let byte = |range: std::ops::Range<usize>| -> Option<i64> {
        i64::from_str_radix(&wire[range], 16).ok()
    };
    let (Some(move_x_raw), Some(move_y_raw), Some(held), Some(edges)) =
        (byte(0..2), byte(2..4), byte(4..6), byte(6..8))
    else {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample wire must be eight lowercase hex characters encoding four bytes",
        );
    };
    let sample = InputSample {
        move_x: move_x_raw - MOVE_SCALE,
        move_y: move_y_raw - MOVE_SCALE,
        held,
        edges,
    };
    validate_sample(&sample)?;
    if encode_sample(&sample)? != wire {
        return failure(
            InputFrameErrorCode::Malformed,
            "input sample wire is not canonical",
        );
    }
    Ok(sample)
}

/// Build a validated sample from optional overrides, defaulting unset fields
/// to zero.
pub fn new_sample(options: InputSampleOptions) -> Result<InputSample> {
    let sample = InputSample {
        move_x: options.move_x.unwrap_or(0),
        move_y: options.move_y.unwrap_or(0),
        held: options.held.unwrap_or(0),
        edges: options.edges.unwrap_or(0),
    };
    validate_sample(&sample)?;
    Ok(sample)
}

/// The all-zero sample: no movement, nothing held, no edges.
#[must_use]
pub fn neutral_sample() -> InputSample {
    InputSample::default()
}

/// Look up a canonical slot by its one-based position.
pub fn slot(index: i64) -> Result<InputSlot> {
    if !(1..=SLOT_COUNT).contains(&index) {
        return failure(
            InputFrameErrorCode::Malformed,
            "input slot index must be between one and eight",
        );
    }
    Ok(SLOT_ORDER[(index - 1) as usize])
}

/// The stable slot position for a team's outfield ordinal.
pub fn slot_index(team: Team, outfield_index: i64) -> Result<i64> {
    if !(1..=HOME_SLOT_COUNT).contains(&outfield_index) {
        return failure(
            InputFrameErrorCode::Malformed,
            "input team and outfield index must name a stable slot",
        );
    }
    Ok(match team {
        Team::Home => outfield_index,
        Team::Away => HOME_SLOT_COUNT + outfield_index,
    })
}

/// Quantize a raw `[-1, 1]` axis into a signed 8-bit value, saturating
/// outside that range and rounding half-away-from-zero.
pub fn quantize_axis(raw_axis: f64) -> Result<i64> {
    if !raw_axis.is_finite() {
        return failure(
            InputFrameErrorCode::Malformed,
            "raw movement axis must be finite",
        );
    }
    let clamped = raw_axis.clamp(-1.0, 1.0);
    let scaled = clamped * MOVE_SCALE as f64;
    let quantized = if scaled >= 0.0 {
        (scaled + 0.5).floor()
    } else {
        (scaled - 0.5).ceil()
    };
    if quantized == 0.0 {
        return Ok(0);
    }
    Ok(quantized as i64)
}

/// Quantize a raw `(x, y)` movement vector.
pub fn quantize_move(raw_x: f64, raw_y: f64) -> Result<(i64, i64)> {
    let move_x = quantize_axis(raw_x)?;
    let move_y = quantize_axis(raw_y)?;
    Ok((move_x, move_y))
}

/// Dequantize a signed 8-bit axis back to `[-1, 1]`.
pub fn dequantize_axis(axis: i64) -> Result<f64> {
    if !is_axis(axis) {
        return failure(
            InputFrameErrorCode::Malformed,
            "quantized movement axis must be signed 8-bit",
        );
    }
    Ok(axis as f64 / MOVE_SCALE as f64)
}

/// Dequantize a sample's movement axes back to `[-1, 1]`.
pub fn dequantize_move(sample: &InputSample) -> Result<(f64, f64)> {
    validate_sample(sample)?;
    Ok((
        sample.move_x as f64 / MOVE_SCALE as f64,
        sample.move_y as f64 / MOVE_SCALE as f64,
    ))
}

/// Whether `action` is currently held in `sample`.
pub fn is_held(sample: &InputSample, action: HeldAction) -> Result<bool> {
    validate_sample(sample)?;
    Ok(has_bit(sample.held, action.bit()))
}

/// Whether `action` fired as a one-tick edge in `sample`.
pub fn has_edge(sample: &InputSample, action: EdgeAction) -> Result<bool> {
    validate_sample(sample)?;
    Ok(has_bit(sample.edges, action.bit()))
}

/// A complete, versioned tick of input across every canonical slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputFrame {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Non-negative fixed-simulation tick.
    pub tick: i64,
    /// Exactly eight samples in canonical [`InputSlot`] order.
    pub slots: [InputSample; 8],
}

/// Validate an already-constructed frame's version, tick bound, and every
/// slot's sample.
pub fn validate(frame: &InputFrame) -> Result<()> {
    if frame.version != VERSION {
        return failure(
            InputFrameErrorCode::UnsupportedVersion,
            "unsupported input frame version",
        );
    }
    if !(0..=MAX_TICK).contains(&frame.tick) {
        return failure(
            InputFrameErrorCode::Malformed,
            "input frame tick must be a bounded non-negative integer",
        );
    }
    for (index, sample) in frame.slots.iter().enumerate() {
        validate_sample(sample).map_err(|e| {
            InputFrameError::new(e.code, format!("input slot {}: {}", index + 1, e.message))
        })?;
    }
    Ok(())
}

/// Build a validated frame. `slots` defaults every slot to
/// [`neutral_sample`] when `None`.
pub fn new(tick: i64, slots: Option<[InputSample; 8]>) -> Result<InputFrame> {
    let mut copied_slots = [InputSample::default(); 8];
    if let Some(provided) = slots {
        for (index, sample) in provided.into_iter().enumerate() {
            validate_sample(&sample)?;
            copied_slots[index] = sample;
        }
    }
    let frame = InputFrame {
        version: VERSION,
        tick,
        slots: copied_slots,
    };
    validate(&frame)?;
    Ok(frame)
}

/// A frame with every slot at [`neutral_sample`].
pub fn neutral(tick: i64) -> Result<InputFrame> {
    new(tick, None)
}

/// A defensive, re-validated copy of `frame`.
pub fn copy(frame: &InputFrame) -> Result<InputFrame> {
    validate(frame)?;
    new(frame.tick, Some(frame.slots))
}

/// The two fixture sides' rosters, by player id. Each roster has exactly
/// [`FIXTURE_TEAM_SIZE`] players, including that side's keeper.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFixtureRosters {
    /// Home side roster.
    pub home: Vec<String>,
    /// Away side roster.
    pub away: Vec<String>,
}

/// Which authored player owns one canonical input slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSlotAssignment {
    /// The slot's stable identity.
    pub slot: SlotId,
    /// The slot's fixture side.
    pub team: Team,
    /// Authored outfield `PlayerData` id; never a keeper.
    pub player_id: String,
}

/// A complete mapping of every canonical input slot to an authored,
/// non-keeper roster player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOwnership {
    /// Exactly [`VERSION`].
    pub version: i64,
    /// Explicit fixture-side roster membership.
    pub rosters: InputFixtureRosters,
    /// Exactly eight assignments in canonical [`InputSlot`] order.
    pub slots: [InputSlotAssignment; 8],
}

fn collect_roster_membership(
    roster: &[String],
    team_label: &str,
    players_by_id: &IndexMap<&str, PlayerData>,
    fixture_players: &mut Vec<String>,
) -> Result<Vec<String>> {
    if roster.len() != FIXTURE_TEAM_SIZE as usize {
        return failure(
            InputFrameErrorCode::Malformed,
            format!("{team_label} fixture roster is invalid"),
        );
    }
    let mut members = Vec::with_capacity(roster.len());
    let mut keeper_count = 0;
    for player_id in roster {
        if player_id.is_empty() {
            return failure(
                InputFrameErrorCode::Malformed,
                format!("{team_label} fixture roster has an invalid player id"),
            );
        }
        if player_id.len() > MAX_PLAYER_ID_BYTES {
            return failure(
                InputFrameErrorCode::Malformed,
                format!("{team_label} fixture roster player id is too long"),
            );
        }
        let player = players_by_id.get(player_id.as_str()).ok_or_else(|| {
            InputFrameError::new(
                InputFrameErrorCode::Malformed,
                format!("{team_label} fixture roster names an unknown player"),
            )
        })?;
        if fixture_players.contains(player_id) {
            return failure(
                InputFrameErrorCode::Malformed,
                "one roster player cannot belong to both fixture sides",
            );
        }
        fixture_players.push(player_id.clone());
        members.push(player_id.clone());
        if player.position == Position::Keeper {
            keeper_count += 1;
        }
    }
    if keeper_count != 1 {
        return failure(
            InputFrameErrorCode::Malformed,
            format!("{team_label} fixture roster needs exactly one keeper"),
        );
    }
    Ok(members)
}

struct RosterMemberships {
    home: Vec<String>,
    away: Vec<String>,
}

fn roster_memberships(
    rosters: &InputFixtureRosters,
    players_by_id: &IndexMap<&str, PlayerData>,
) -> Result<RosterMemberships> {
    let mut fixture_players = Vec::new();
    let home =
        collect_roster_membership(&rosters.home, "home", players_by_id, &mut fixture_players)?;
    let away =
        collect_roster_membership(&rosters.away, "away", players_by_id, &mut fixture_players)?;
    Ok(RosterMemberships { home, away })
}

fn slot_id_for(index: usize) -> SlotId {
    SLOT_ORDER[index].id
}

fn slot_team_for(index: usize) -> Team {
    SLOT_ORDER[index].team
}

/// Validate every slot's assignment against the fixture's rosters: canonical
/// slot/team order, non-keeper outfield players, unique ownership, and
/// side-correct membership.
pub fn validate_ownership(
    ownership: &InputOwnership,
    players_by_id: &IndexMap<&str, PlayerData>,
) -> Result<()> {
    if ownership.version != VERSION {
        return failure(
            InputFrameErrorCode::UnsupportedVersion,
            "unsupported input ownership version",
        );
    }
    let memberships = roster_memberships(&ownership.rosters, players_by_id)?;

    let mut seen_players: Vec<&str> = Vec::new();
    for index in 0..SLOT_COUNT as usize {
        let assignment = &ownership.slots[index];
        if assignment.slot != slot_id_for(index) || assignment.team != slot_team_for(index) {
            return failure(
                InputFrameErrorCode::Malformed,
                format!(
                    "input ownership slot {} violates canonical team order",
                    index + 1
                ),
            );
        }
        if assignment.player_id.is_empty() {
            return failure(
                InputFrameErrorCode::Malformed,
                format!("input ownership slot {} needs a player id", index + 1),
            );
        }
        if assignment.player_id.len() > MAX_PLAYER_ID_BYTES {
            return failure(
                InputFrameErrorCode::Malformed,
                format!("input ownership slot {} player id is too long", index + 1),
            );
        }
        if seen_players.contains(&assignment.player_id.as_str()) {
            return failure(
                InputFrameErrorCode::Malformed,
                "one roster player cannot own multiple input slots",
            );
        }
        let player = players_by_id
            .get(assignment.player_id.as_str())
            .ok_or_else(|| {
                InputFrameError::new(
                    InputFrameErrorCode::Malformed,
                    format!("input ownership slot {} names an unknown player", index + 1),
                )
            })?;
        let side_members = match assignment.team {
            Team::Home => &memberships.home,
            Team::Away => &memberships.away,
        };
        if !side_members.iter().any(|id| id == &assignment.player_id) {
            return failure(
                InputFrameErrorCode::Malformed,
                format!(
                    "input ownership slot {} binds a player from the other fixture side",
                    index + 1
                ),
            );
        }
        if player.position == Position::Keeper {
            return failure(
                InputFrameErrorCode::Malformed,
                "keepers cannot own input slots",
            );
        }
        seen_players.push(&assignment.player_id);
    }
    Ok(())
}

/// Build a validated ownership mapping from per-slot assignments and fixture
/// rosters.
pub fn new_ownership(
    assignments: &[InputSlotAssignment; 8],
    rosters: &InputFixtureRosters,
    players_by_id: &IndexMap<&str, PlayerData>,
) -> Result<InputOwnership> {
    // Validating rosters up front matches the Lua source's failure order:
    // a bad roster is reported before any slot-shape error.
    roster_memberships(rosters, players_by_id)?;
    let ownership = InputOwnership {
        version: VERSION,
        rosters: rosters.clone(),
        slots: assignments.clone(),
    };
    validate_ownership(&ownership, players_by_id)?;
    Ok(ownership)
}

/// A defensive, re-validated copy of `ownership`.
pub fn copy_ownership(
    ownership: &InputOwnership,
    players_by_id: &IndexMap<&str, PlayerData>,
) -> Result<InputOwnership> {
    validate_ownership(ownership, players_by_id)?;
    new_ownership(&ownership.slots, &ownership.rosters, players_by_id)
}

fn parse_unsigned(value: &str) -> Option<i64> {
    if value == "0" {
        return Some(0);
    }
    let bytes = value.as_bytes();
    if !bytes.is_empty() && bytes[0] != b'0' && bytes.iter().all(|b| b.is_ascii_digit()) {
        return value.parse::<i64>().ok();
    }
    None
}

fn parse_axis(value: &str) -> Option<i64> {
    if value == "0" {
        return Some(0);
    }
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value),
    };
    let bytes = digits.as_bytes();
    if !bytes.is_empty() && bytes[0] != b'0' && bytes.iter().all(|b| b.is_ascii_digit()) {
        let parsed = digits.parse::<i64>().ok()?;
        return Some(if negative { -parsed } else { parsed });
    }
    None
}

fn encode_frame_sample(sample: &InputSample) -> String {
    format!(
        "{},{},{},{}",
        sample.move_x, sample.move_y, sample.held, sample.edges
    )
}

/// Encode a frame into its pipe-delimited wire form:
/// `version|tick|move_x,move_y,held,edges|...` (one slot group per canonical
/// slot).
pub fn encode(frame: &InputFrame) -> Result<String> {
    validate(frame)?;
    let mut fields = vec![frame.version.to_string(), frame.tick.to_string()];
    for sample in &frame.slots {
        fields.push(encode_frame_sample(sample));
    }
    let wire = fields.join("|");
    if wire.len() > MAX_WIRE_BYTES {
        return failure(
            InputFrameErrorCode::WireTooLarge,
            "input frame wire exceeds the byte limit",
        );
    }
    Ok(wire)
}

/// Decode a wire-form frame, rejecting anything non-canonical.
pub fn decode(wire: &str) -> Result<InputFrame> {
    if wire.len() > MAX_WIRE_BYTES {
        return failure(
            InputFrameErrorCode::WireTooLarge,
            "input frame wire exceeds the byte limit",
        );
    }

    let fields: Vec<&str> = wire.split('|').collect();
    if fields.len() != SLOT_COUNT as usize + 2 {
        return failure(
            InputFrameErrorCode::Malformed,
            "input frame wire has invalid fields",
        );
    }
    let (Some(version), Some(tick)) = (parse_unsigned(fields[0]), parse_unsigned(fields[1])) else {
        return failure(
            InputFrameErrorCode::Malformed,
            "input frame wire version and tick must be canonical integers",
        );
    };

    let mut slots = [InputSample::default(); 8];
    for index in 0..SLOT_COUNT as usize {
        let parts: Vec<&str> = fields[index + 2].split(',').collect();
        if parts.len() != 4 {
            return failure(
                InputFrameErrorCode::Malformed,
                format!("input frame wire slot {} is invalid", index + 1),
            );
        }
        let (raw_x, raw_y, raw_held, raw_edges) = (parts[0], parts[1], parts[2], parts[3]);
        let (Some(move_x), Some(move_y), Some(held), Some(edges)) = (
            parse_axis(raw_x),
            parse_axis(raw_y),
            parse_unsigned(raw_held),
            parse_unsigned(raw_edges),
        ) else {
            return failure(
                InputFrameErrorCode::Malformed,
                format!("input frame wire slot {} is not canonical", index + 1),
            );
        };
        slots[index] = InputSample {
            move_x,
            move_y,
            held,
            edges,
        };
    }

    let frame = InputFrame {
        version,
        tick,
        slots,
    };
    validate(&frame)?;
    copy(&frame)
}
