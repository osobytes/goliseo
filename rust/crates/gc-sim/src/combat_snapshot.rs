//! The versioned, canonical `CombatMatchState` record: its shape, its
//! validating copy, its append-to-wire contract, and its structural diff.
//!
//! Design note: [`copy`] takes `&crate::match_snapshot::MatchState` directly
//! rather than declaring a narrow local view. Unlike `metrics`/`bot`/`aerial`,
//! whose narrow views exist because they only ever need a small slice of
//! match state, this crate's `match_snapshot` module's entire job is to
//! capture/restore/hash *all* of `MatchState`'s shape, so it already is the
//! canonical, fully-typed `MatchState`/`MatchPlayer` record. Reusing it here
//! instead of inventing another view keeps the ledger from growing.

use crate::combat_feasibility::CombatActionPhase;
use crate::combat_intent::{self, CombatIntentState};
use crate::match_snapshot::MatchState;
use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::loadouts;

/// 3 adds the typed request outcome/reason and the encounter terminal to the
/// event row. Every combat boundary hash and every combat tape therefore
/// moves, which is exactly the build skew the version guard exists to
/// reject.
pub const VERSION: i64 = 3;

const MAX_INTEGER: i64 = 2_147_483_647;

/// A canonical wire scalar: exactly the four value kinds the snapshot wire
/// format supports (`z;`, `b0;`/`b1;`, `n...;`, `s...;`). Owned here because
/// [`CombatSnapshotWriter`] needs it; [`crate::match_snapshot`]'s canonical
/// encoder (the only implementor) reuses this type rather than duplicating
/// it.
#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalScalar {
    /// Absent value (`z;`).
    Nil,
    /// A boolean (`b0;`/`b1;`).
    Bool(bool),
    /// A finite number (`n...;`).
    Number(f64),
    /// A string (`s<len>:...;`).
    Text(String),
}

/// Callback sink [`append`] writes a [`CombatMatchState`] into. Implemented
/// by [`crate::match_snapshot`]'s canonical encoder, which owns the wire
/// format and byte-counting variant.
pub trait CombatSnapshotWriter {
    /// Append a literal wire fragment verbatim.
    fn literal(&mut self, value: &str);
    /// Append a field name.
    fn name(&mut self, value: &str);
    /// Append a canonical scalar value.
    fn scalar(&mut self, value: CanonicalScalar);
    /// Append a 2D vector as its two scalar components.
    fn vec(&mut self, value: Vec2) {
        self.scalar(CanonicalScalar::Number(value.x));
        self.scalar(CanonicalScalar::Number(value.y));
    }
}

/// A row's current combat action phase. Re-exported alias of
/// [`crate::combat_feasibility::CombatActionPhase`] — same closed
/// vocabulary (`ready`, `windup`, `active`, `aim`, `guard`, `recovery`), one
/// declaration.
pub type CombatPhase = CombatActionPhase;

/// A player's forced (stagger/knockback) disable state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatForcedState {
    /// A brief interruption with no meaningful displacement.
    Stagger,
    /// An interruption with a meaningful displacement.
    Knockback,
}

/// The closed, versioned event-kind vocabulary. Order is the contract's own
/// and is the declared tie break wherever several reasons hold at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatEventKind {
    /// A physical equipment request was rejected.
    RequestRejected,
    /// A physical equipment request was accepted; opens a source sequence.
    Commit,
    /// A ranged projectile spawned.
    ProjectileSpawn,
    /// A ranged projectile's lifetime elapsed, or it left the pitch.
    ProjectileExpire,
    /// A melee or projectile action made contact.
    Contact,
    /// Contact spilled the ball loose.
    BallSpill,
    /// A target entered a forced (stagger/knockback) state.
    Forced,
    /// A guarded contact recoiled the source.
    GuardRecoil,
    /// A melee window ran its course without a contact.
    Miss,
    /// An open encounter was interrupted by a forced state.
    Interrupted,
    /// An open encounter was cancelled by a kickoff restart.
    Cancelled,
    /// An open encounter was closed by full time.
    MatchTerminated,
}

impl CombatEventKind {
    /// This kind's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            CombatEventKind::RequestRejected => "request_rejected",
            CombatEventKind::Commit => "commit",
            CombatEventKind::ProjectileSpawn => "projectile_spawn",
            CombatEventKind::ProjectileExpire => "projectile_expire",
            CombatEventKind::Contact => "contact",
            CombatEventKind::BallSpill => "ball_spill",
            CombatEventKind::Forced => "forced",
            CombatEventKind::GuardRecoil => "guard_recoil",
            CombatEventKind::Miss => "miss",
            CombatEventKind::Interrupted => "interrupted",
            CombatEventKind::Cancelled => "cancelled",
            CombatEventKind::MatchTerminated => "match_terminated",
        }
    }
}

/// A landed contact's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatContactResult {
    /// The contact hit and applied its unguarded outcome.
    Hit,
    /// The contact hit a target already forced, extending the forced state.
    Extended,
    /// The contact was guarded.
    Guarded,
    /// The target was immune (post-hit immunity window).
    Immune,
    /// A dominant contact on the same target superseded this one.
    Superseded,
}

impl CombatContactResult {
    /// This result's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            CombatContactResult::Hit => "hit",
            CombatContactResult::Extended => "extended",
            CombatContactResult::Guarded => "guarded",
            CombatContactResult::Immune => "immune",
            CombatContactResult::Superseded => "superseded",
        }
    }

    /// This contact result's lawful terminal (`CONTACT_TERMINALS`). An
    /// extended contact is a landed contact that lengthened an existing
    /// forced state, so `hit` is the only lawful terminal it can carry —
    /// there is no `extended` member in [`CombatEncounterTerminal`].
    #[must_use]
    pub fn terminal(self) -> CombatEncounterTerminal {
        match self {
            CombatContactResult::Hit | CombatContactResult::Extended => {
                CombatEncounterTerminal::Hit
            }
            CombatContactResult::Guarded => CombatEncounterTerminal::Guarded,
            CombatContactResult::Immune => CombatEncounterTerminal::Immune,
            CombatContactResult::Superseded => CombatEncounterTerminal::Superseded,
        }
    }
}

/// A physical equipment request's typed acceptance outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatRequestOutcome {
    /// The request opened a source sequence.
    Accepted,
    /// The request was refused.
    Rejected,
}

impl CombatRequestOutcome {
    /// This outcome's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            CombatRequestOutcome::Accepted => "accepted",
            CombatRequestOutcome::Rejected => "rejected",
        }
    }
}

/// The closed rejection-reason vocabulary for a refused equipment request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatRequestRejectionReason {
    /// The requester is a keeper, or has no combat loadout.
    ProtectedKeeperOrNoLoadout,
    /// Requests are refused during a kickoff hold.
    KickoffHold,
    /// A soccer action already claims priority this tick.
    SoccerCommitment,
    /// The requester is mid-aerial or its recovery.
    AerialStateOrRecovery,
    /// The requester is in a forced (stagger/knockback) state.
    ForcedState,
    /// The requester already has an action committed.
    AlreadyCommitted,
    /// The requester's action is still cooling down.
    Cooldown,
    /// The request has no equipment-press edge to answer.
    MissingPressEdge,
    /// The input itself is malformed.
    MalformedInput,
}

impl CombatRequestRejectionReason {
    /// This reason's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            CombatRequestRejectionReason::ProtectedKeeperOrNoLoadout => {
                "protected_keeper_or_no_loadout"
            }
            CombatRequestRejectionReason::KickoffHold => "kickoff_hold",
            CombatRequestRejectionReason::SoccerCommitment => "soccer_commitment",
            CombatRequestRejectionReason::AerialStateOrRecovery => "aerial_state_or_recovery",
            CombatRequestRejectionReason::ForcedState => "forced_state",
            CombatRequestRejectionReason::AlreadyCommitted => "already_committed",
            CombatRequestRejectionReason::Cooldown => "cooldown",
            CombatRequestRejectionReason::MissingPressEdge => "missing_press_edge",
            CombatRequestRejectionReason::MalformedInput => "malformed_input",
        }
    }
}

/// The closed terminal vocabulary an accepted encounter closes under,
/// exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatEncounterTerminal {
    /// A melee window ran its course without a contact.
    Miss,
    /// A ranged projectile's lifetime or pitch bound was reached.
    Expire,
    /// The encounter's contact was guarded.
    Guarded,
    /// The encounter's contact hit an immune target.
    Immune,
    /// The encounter's contact was superseded.
    Superseded,
    /// The encounter's contact hit (including an extension).
    Hit,
    /// The encounter was interrupted by a forced state.
    Interrupted,
    /// The encounter was cancelled by a kickoff restart.
    Cancelled,
    /// The encounter was closed by full time.
    MatchTerminated,
}

impl CombatEncounterTerminal {
    /// This terminal's canonical wire string.
    #[must_use]
    pub fn wire_str(self) -> &'static str {
        match self {
            CombatEncounterTerminal::Miss => "miss",
            CombatEncounterTerminal::Expire => "expire",
            CombatEncounterTerminal::Guarded => "guarded",
            CombatEncounterTerminal::Immune => "immune",
            CombatEncounterTerminal::Superseded => "superseded",
            CombatEncounterTerminal::Hit => "hit",
            CombatEncounterTerminal::Interrupted => "interrupted",
            CombatEncounterTerminal::Cancelled => "cancelled",
            CombatEncounterTerminal::MatchTerminated => "match_terminated",
        }
    }
}

/// This action family's canonical wire id (`"unarmed"`, `"guard"`,
/// `"light_melee"`, `"ranged"`). `gc-data`'s [`ActionFamilyId`] carries no
/// such string itself, so this mapping lives here and is reused by every
/// module in this layer that serializes a family id onto the wire.
#[must_use]
pub fn action_family_wire_id(id: ActionFamilyId) -> &'static str {
    match id {
        ActionFamilyId::Unarmed => "unarmed",
        ActionFamilyId::Guard => "guard",
        ActionFamilyId::LightMelee => "light_melee",
        ActionFamilyId::Ranged => "ranged",
    }
}

/// One player's retained combat runtime.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatPlayerState {
    /// This player's fixed combat loadout id, or `None` without one.
    pub loadout_id: Option<String>,
    /// This player's mechanical action family, or `None` without a loadout.
    pub family_id: Option<ActionFamilyId>,
    /// Current combat action phase.
    pub phase: CombatPhase,
    /// Ticks remaining in the current phase.
    pub phase_ticks: i64,
    /// Ticks remaining before this player's action can commit again.
    pub cooldown_ticks: i64,
    /// The open encounter's source sequence, if any.
    pub source_sequence: Option<i64>,
    /// Whether the current melee action has already contacted.
    pub contacted: bool,
    /// Whether a ranged/guard release has latched.
    pub release_latched: bool,
    /// Whether the equipment control is currently held.
    pub control_held: bool,
    /// Whether the current ranged action has spawned its projectile.
    pub projectile_spawned: bool,
    /// The current forced (stagger/knockback) state, if any.
    pub forced_state: Option<CombatForcedState>,
    /// Ticks remaining in the current forced state.
    pub forced_ticks: i64,
    /// Ticks remaining a fresh forced state may still be chained within.
    pub chain_ticks: i64,
    /// Ticks remaining in the post-hit immunity window.
    pub immunity_ticks: i64,
    /// Retained gameplay-AI combat intent ([`crate::combat_intent`]).
    pub intent: CombatIntentState,
}

/// One in-flight ranged projectile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombatProjectile {
    /// Always [`ActionFamilyId::Ranged`].
    pub family_id: ActionFamilyId,
    /// The firing player's canonical index (one-based).
    pub source_index: i64,
    /// The firing action's source sequence.
    pub source_sequence: i64,
    /// Current position.
    pub pos: Vec2,
    /// Unit travel direction.
    pub dir: Vec2,
    /// Ticks remaining before this projectile expires.
    pub remaining_ticks: i64,
}

/// One discrete combat event row.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatEvent {
    /// Event kind.
    pub kind: CombatEventKind,
    /// The input tick this event is attributed to.
    pub tick: i64,
    /// The action family involved, if any.
    pub family_id: Option<ActionFamilyId>,
    /// The acting player's canonical index, if any.
    pub source_index: Option<i64>,
    /// The targeted player's canonical index, if any.
    pub target_index: Option<i64>,
    /// The involved encounter's source sequence, if any.
    pub source_sequence: Option<i64>,
    /// The contact result, set on contact rows only.
    pub result: Option<CombatContactResult>,
    /// The request outcome, set on the two request rows only.
    pub outcome: Option<CombatRequestOutcome>,
    /// The rejection reason, set on a rejected request only.
    pub reason: Option<CombatRequestRejectionReason>,
    /// The terminal this row closes its sequence under, if any.
    pub terminal: Option<CombatEncounterTerminal>,
    /// World x position this event is attributed at.
    pub x: f64,
    /// World y position this event is attributed at.
    pub y: f64,
    /// Ticks the target is interrupted for, on a forced row.
    pub interruption_ticks: Option<i64>,
    /// Px the target is displaced, on a forced/guard-recoil row.
    pub displacement_px: Option<f64>,
}

/// The complete, versioned combat companion to one tick's `MatchState`.
#[derive(Clone, Debug, PartialEq)]
pub struct CombatMatchState {
    /// Always [`VERSION`].
    pub version: i64,
    /// The tick this state describes.
    pub tick: i64,
    /// Every fixture player's stable id, in canonical player order.
    pub player_ids: Vec<String>,
    /// Every fixture player's combat runtime, in canonical player order.
    pub players: Vec<CombatPlayerState>,
    /// Every in-flight projectile.
    pub projectiles: Vec<CombatProjectile>,
    /// This tick's discrete combat events.
    pub events: Vec<CombatEvent>,
    /// The next source sequence a commit will be assigned.
    pub next_source_sequence: i64,
}

fn is_bounded_non_negative(value: i64) -> bool {
    (0..=MAX_INTEGER).contains(&value)
}

fn is_bounded_positive(value: i64) -> bool {
    (1..=MAX_INTEGER).contains(&value)
}

fn validate_player(path: &str, player: &CombatPlayerState) {
    assert!(
        is_bounded_non_negative(player.phase_ticks),
        "{path}.phase_ticks must be a bounded non-negative integer"
    );
    assert!(
        is_bounded_non_negative(player.cooldown_ticks),
        "{path}.cooldown_ticks must be a bounded non-negative integer"
    );
    if let Some(sequence) = player.source_sequence {
        assert!(
            is_bounded_positive(sequence),
            "{path}.source_sequence must be a bounded positive integer"
        );
    }
    assert!(
        is_bounded_non_negative(player.forced_ticks),
        "{path}.forced_ticks must be a bounded non-negative integer"
    );
    assert!(
        is_bounded_non_negative(player.chain_ticks),
        "{path}.chain_ticks must be a bounded non-negative integer"
    );
    assert!(
        is_bounded_non_negative(player.immunity_ticks),
        "{path}.immunity_ticks must be a bounded non-negative integer"
    );
    if let Some(loadout_id) = &player.loadout_id {
        let loadout =
            loadouts::get(loadout_id).unwrap_or_else(|| panic!("{path}.loadout_id is unknown"));
        assert_eq!(
            Some(loadout.family_id),
            player.family_id,
            "{path} loadout/family pair is inconsistent"
        );
    } else {
        assert!(
            player.family_id.is_none(),
            "{path} family requires a loadout"
        );
    }
    let _ = combat_intent::copy_state(&player.intent);
}

fn validate_projectile(path: &str, projectile: &CombatProjectile, player_count: usize) {
    assert_eq!(
        projectile.family_id,
        ActionFamilyId::Ranged,
        "{path}.family_id is unknown"
    );
    assert!(
        is_bounded_positive(projectile.source_index)
            && (projectile.source_index as usize) <= player_count,
        "{path}.source_index must be a valid player index"
    );
    assert!(
        is_bounded_positive(projectile.source_sequence),
        "{path}.source_sequence must be a bounded positive integer"
    );
    assert!(
        is_bounded_positive(projectile.remaining_ticks),
        "{path}.remaining_ticks must be positive"
    );
    assert!(projectile.pos.x.is_finite(), "{path}.pos.x must be finite");
    assert!(projectile.pos.y.is_finite(), "{path}.pos.y must be finite");
    assert!(projectile.dir.x.is_finite(), "{path}.dir.x must be finite");
    assert!(projectile.dir.y.is_finite(), "{path}.dir.y must be finite");
}

fn validate_event(path: &str, event: &CombatEvent, player_count: usize) {
    let tick = event.tick;
    assert!(
        is_bounded_non_negative(tick),
        "{path}.tick must be a bounded non-negative integer"
    );
    assert_eq!(
        event.reason.is_some(),
        event.outcome == Some(CombatRequestOutcome::Rejected),
        "{path} pairs a rejection reason with a rejected request"
    );
    assert!(event.x.is_finite(), "{path}.x must be finite");
    assert!(event.y.is_finite(), "{path}.y must be finite");
    if let Some(interruption_ticks) = event.interruption_ticks {
        assert!(
            is_bounded_non_negative(interruption_ticks),
            "{path}.interruption_ticks must be a bounded non-negative integer"
        );
    }
    if let Some(displacement_px) = event.displacement_px {
        assert!(
            displacement_px.is_finite(),
            "{path}.displacement_px must be finite"
        );
    }
    if let Some(source_index) = event.source_index {
        assert!(
            is_bounded_positive(source_index) && (source_index as usize) <= player_count,
            "{path}.source_index must be a valid player index"
        );
    }
    if let Some(target_index) = event.target_index {
        assert!(
            is_bounded_positive(target_index) && (target_index as usize) <= player_count,
            "{path}.target_index must be a valid player index"
        );
    }
    if let Some(source_sequence) = event.source_sequence {
        assert!(
            is_bounded_positive(source_sequence),
            "{path}.source_sequence must be a bounded positive integer"
        );
    }
}

/// Validate `source` against `match_state`'s fixture and return a
/// defensively copied, canonical [`CombatMatchState`].
///
/// Structural shape checks (field presence, scalar kind) are unnecessary
/// here — the type system already guarantees them (ARCHITECTURE.md §3 rule 7
/// / the same principle `input_frame`'s module doc documents). The numeric-bound
/// and cross-field invariants are kept.
#[must_use]
pub fn copy(source: &CombatMatchState, match_state: &MatchState) -> CombatMatchState {
    assert_eq!(source.version, VERSION, "combat.version is unsupported");
    assert_eq!(
        source.player_ids.len(),
        match_state.players.len(),
        "combat.player_ids has the wrong length"
    );
    assert_eq!(
        source.players.len(),
        match_state.players.len(),
        "combat.players has the wrong length"
    );
    let next_source_sequence = source.next_source_sequence;
    assert!(
        is_bounded_non_negative(source.tick),
        "combat.tick must be a bounded non-negative integer"
    );
    assert!(
        is_bounded_positive(next_source_sequence),
        "combat.next_source_sequence must be positive"
    );
    if match_state.slot_mode {
        assert_eq!(
            source.tick, match_state.input_tick,
            "combat.tick must match state.input_tick"
        );
    }

    let mut player_ids = Vec::with_capacity(match_state.players.len());
    let mut players = Vec::with_capacity(match_state.players.len());
    for (index, player) in match_state.players.iter().enumerate() {
        let player_id = &source.player_ids[index];
        assert!(!player_id.is_empty(), "combat.player_ids is invalid");
        assert_eq!(
            player_id, &player.id,
            "combat.player_ids does not match the fixture"
        );
        player_ids.push(player_id.clone());
        let path = format!("combat.players.{}", index + 1);
        validate_player(&path, &source.players[index]);
        let copied = source.players[index].clone();
        if player.is_keeper {
            assert!(
                copied.loadout_id.is_none(),
                "{path} keeper cannot have a combat loadout"
            );
        }
        players.push(copied);
    }

    let mut projectiles = Vec::with_capacity(source.projectiles.len());
    for (index, projectile) in source.projectiles.iter().enumerate() {
        let path = format!("combat.projectiles.{}", index + 1);
        validate_projectile(&path, projectile, match_state.players.len());
        projectiles.push(*projectile);
    }

    let mut events = Vec::with_capacity(source.events.len());
    for (index, event) in source.events.iter().enumerate() {
        let path = format!("combat.events.{}", index + 1);
        validate_event(&path, event, match_state.players.len());
        assert_eq!(
            event.tick,
            source.tick - 1,
            "{path}.tick must name the causal input tick"
        );
        events.push(event.clone());
    }

    CombatMatchState {
        version: VERSION,
        tick: source.tick,
        player_ids,
        players,
        projectiles,
        events,
        next_source_sequence,
    }
}

/// Internal ownership-transfer copy for a [`CombatMatchState`] previously
/// produced by a validated snapshot or by [`crate::combat`]. Rollback owns
/// states that have already crossed [`copy`]'s validating boundary, so this
/// is a defensive clone without repeating the allowlist/bound checks —
/// `Clone` already gives byte-for-byte deep ownership, so no field-by-field
/// walk is needed to preserve that contract.
#[must_use]
pub fn copy_owned(source: &CombatMatchState) -> CombatMatchState {
    source.clone()
}

/// Serialize a [`CombatMatchState`] into `writer` in the canonical field
/// order (`STATE_FIELDS`).
pub fn append<W: CombatSnapshotWriter>(writer: &mut W, combat_state: &CombatMatchState) {
    writer.literal("GCCS;");
    writer.name("version");
    writer.scalar(CanonicalScalar::Number(combat_state.version as f64));
    writer.name("tick");
    writer.scalar(CanonicalScalar::Number(combat_state.tick as f64));
    writer.name("player_ids");
    writer.scalar(CanonicalScalar::Number(combat_state.player_ids.len() as f64));
    for player_id in &combat_state.player_ids {
        writer.scalar(CanonicalScalar::Text(player_id.clone()));
    }
    writer.name("players");
    writer.scalar(CanonicalScalar::Number(combat_state.players.len() as f64));
    for player in &combat_state.players {
        append_player(writer, player);
    }
    writer.name("projectiles");
    writer.scalar(CanonicalScalar::Number(
        combat_state.projectiles.len() as f64
    ));
    for projectile in &combat_state.projectiles {
        append_projectile(writer, projectile);
    }
    writer.name("events");
    writer.scalar(CanonicalScalar::Number(combat_state.events.len() as f64));
    for event in &combat_state.events {
        append_event(writer, event);
    }
    writer.name("next_source_sequence");
    writer.scalar(CanonicalScalar::Number(
        combat_state.next_source_sequence as f64,
    ));
}

fn scalar_opt_i64(value: Option<i64>) -> CanonicalScalar {
    value.map_or(CanonicalScalar::Nil, |v| CanonicalScalar::Number(v as f64))
}

fn append_player<W: CombatSnapshotWriter>(writer: &mut W, player: &CombatPlayerState) {
    writer.name("loadout_id");
    writer.scalar(
        player
            .loadout_id
            .clone()
            .map_or(CanonicalScalar::Nil, CanonicalScalar::Text),
    );
    writer.name("family_id");
    writer.scalar(player.family_id.map_or(CanonicalScalar::Nil, |id| {
        CanonicalScalar::Text(action_family_wire_id(id).to_string())
    }));
    writer.name("phase");
    writer.scalar(CanonicalScalar::Text(
        phase_wire_str(player.phase).to_string(),
    ));
    writer.name("phase_ticks");
    writer.scalar(CanonicalScalar::Number(player.phase_ticks as f64));
    writer.name("cooldown_ticks");
    writer.scalar(CanonicalScalar::Number(player.cooldown_ticks as f64));
    writer.name("source_sequence");
    writer.scalar(scalar_opt_i64(player.source_sequence));
    writer.name("contacted");
    writer.scalar(CanonicalScalar::Bool(player.contacted));
    writer.name("release_latched");
    writer.scalar(CanonicalScalar::Bool(player.release_latched));
    writer.name("control_held");
    writer.scalar(CanonicalScalar::Bool(player.control_held));
    writer.name("projectile_spawned");
    writer.scalar(CanonicalScalar::Bool(player.projectile_spawned));
    writer.name("forced_state");
    writer.scalar(player.forced_state.map_or(CanonicalScalar::Nil, |state| {
        CanonicalScalar::Text(
            match state {
                CombatForcedState::Stagger => "stagger",
                CombatForcedState::Knockback => "knockback",
            }
            .to_string(),
        )
    }));
    writer.name("forced_ticks");
    writer.scalar(CanonicalScalar::Number(player.forced_ticks as f64));
    writer.name("chain_ticks");
    writer.scalar(CanonicalScalar::Number(player.chain_ticks as f64));
    writer.name("immunity_ticks");
    writer.scalar(CanonicalScalar::Number(player.immunity_ticks as f64));
    writer.name("intent");
    writer.literal("i;");
    writer.scalar(CanonicalScalar::Text(
        match player.intent.stage {
            combat_intent::CombatIntentStage::Idle => "idle",
            combat_intent::CombatIntentStage::Release => "release",
            combat_intent::CombatIntentStage::Hold => "hold",
        }
        .to_string(),
    ));
    writer.scalar(CanonicalScalar::Number(player.intent.hold_ticks as f64));
    writer.scalar(CanonicalScalar::Text(
        intent_reason_wire_str(player.intent.reason).to_string(),
    ));
    writer.scalar(scalar_opt_i64(player.intent.target_player));
}

fn intent_reason_wire_str(reason: combat_intent::CombatDecisionReason) -> &'static str {
    use combat_intent::CombatDecisionReason as R;
    match reason {
        R::None => "none",
        R::Decline => "decline",
        R::CarrierContest => "carrier_contest",
        R::CarrierProtection => "carrier_protection",
        R::LooseBallContest => "loose_ball_contest",
        R::PassingLaneOrShotDenial => "passing_lane_or_shot_denial",
        R::RecoveryPunish => "recovery_punish",
        R::UnattributedOffBall => "unattributed_off_ball",
    }
}

/// This phase's canonical wire string.
#[must_use]
pub fn phase_wire_str(phase: CombatPhase) -> &'static str {
    match phase {
        CombatPhase::Ready => "ready",
        CombatPhase::Windup => "windup",
        CombatPhase::Active => "active",
        CombatPhase::Aim => "aim",
        CombatPhase::Guard => "guard",
        CombatPhase::Recovery => "recovery",
    }
}

fn append_projectile<W: CombatSnapshotWriter>(writer: &mut W, projectile: &CombatProjectile) {
    writer.name("family_id");
    writer.scalar(CanonicalScalar::Text(
        action_family_wire_id(projectile.family_id).to_string(),
    ));
    writer.name("source_index");
    writer.scalar(CanonicalScalar::Number(projectile.source_index as f64));
    writer.name("source_sequence");
    writer.scalar(CanonicalScalar::Number(projectile.source_sequence as f64));
    writer.name("pos");
    writer.vec(projectile.pos);
    writer.name("dir");
    writer.vec(projectile.dir);
    writer.name("remaining_ticks");
    writer.scalar(CanonicalScalar::Number(projectile.remaining_ticks as f64));
}

fn append_event<W: CombatSnapshotWriter>(writer: &mut W, event: &CombatEvent) {
    writer.name("kind");
    writer.scalar(CanonicalScalar::Text(event.kind.wire_str().to_string()));
    writer.name("tick");
    writer.scalar(CanonicalScalar::Number(event.tick as f64));
    writer.name("family_id");
    writer.scalar(event.family_id.map_or(CanonicalScalar::Nil, |id| {
        CanonicalScalar::Text(action_family_wire_id(id).to_string())
    }));
    writer.name("source_index");
    writer.scalar(scalar_opt_i64(event.source_index));
    writer.name("target_index");
    writer.scalar(scalar_opt_i64(event.target_index));
    writer.name("source_sequence");
    writer.scalar(scalar_opt_i64(event.source_sequence));
    writer.name("result");
    writer.scalar(event.result.map_or(CanonicalScalar::Nil, |r| {
        CanonicalScalar::Text(r.wire_str().to_string())
    }));
    writer.name("outcome");
    writer.scalar(event.outcome.map_or(CanonicalScalar::Nil, |o| {
        CanonicalScalar::Text(o.wire_str().to_string())
    }));
    writer.name("reason");
    writer.scalar(event.reason.map_or(CanonicalScalar::Nil, |r| {
        CanonicalScalar::Text(r.wire_str().to_string())
    }));
    writer.name("terminal");
    writer.scalar(event.terminal.map_or(CanonicalScalar::Nil, |t| {
        CanonicalScalar::Text(t.wire_str().to_string())
    }));
    writer.name("x");
    writer.scalar(CanonicalScalar::Number(event.x));
    writer.name("y");
    writer.scalar(CanonicalScalar::Number(event.y));
    writer.name("interruption_ticks");
    writer.scalar(scalar_opt_i64(event.interruption_ticks));
    writer.name("displacement_px");
    writer.scalar(
        event
            .displacement_px
            .map_or(CanonicalScalar::Nil, CanonicalScalar::Number),
    );
}

/// One leaf disagreement between two [`CombatMatchState`]s (or the two
/// snapshots that contain them).
#[derive(Clone, Debug, PartialEq)]
pub struct CombatSnapshotDifference {
    /// Dotted path to the differing field.
    pub path: String,
    /// The left/expected value, rendered for display.
    pub expected: String,
    /// The right/actual value, rendered for display.
    pub actual: String,
}

fn difference(
    path: impl Into<String>,
    expected: impl Into<String>,
    actual: impl Into<String>,
) -> CombatSnapshotDifference {
    CombatSnapshotDifference {
        path: path.into(),
        expected: expected.into(),
        actual: actual.into(),
    }
}

/// The first structural disagreement between two optional combat states, if
/// any. `same_scalar` distinguishes `+0.0` from `-0.0`
/// (`1 / left == 1 / right` when both compare equal to zero); ordinary
/// `PartialEq` on `f64` does not, so every numeric comparison below goes
/// through [`same_f64`] explicitly.
#[must_use]
pub fn first_difference(
    left: Option<&CombatMatchState>,
    right: Option<&CombatMatchState>,
    path: &str,
) -> Option<CombatSnapshotDifference> {
    match (left, right) {
        (None, None) => None,
        (Some(_), None) => Some(difference(path, "present", "nil")),
        (None, Some(_)) => Some(difference(path, "nil", "present")),
        (Some(left), Some(right)) => first_difference_present(left, right, path),
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

fn first_difference_present(
    left: &CombatMatchState,
    right: &CombatMatchState,
    path: &str,
) -> Option<CombatSnapshotDifference> {
    if left.version != right.version {
        return Some(difference(
            format!("{path}.version"),
            left.version.to_string(),
            right.version.to_string(),
        ));
    }
    if left.tick != right.tick {
        return Some(difference(
            format!("{path}.tick"),
            left.tick.to_string(),
            right.tick.to_string(),
        ));
    }
    if left.player_ids.len() != right.player_ids.len() {
        return Some(difference(
            format!("{path}.player_ids.length"),
            left.player_ids.len().to_string(),
            right.player_ids.len().to_string(),
        ));
    }
    for (index, (a, b)) in left.player_ids.iter().zip(&right.player_ids).enumerate() {
        if a != b {
            return Some(difference(
                format!("{path}.player_ids.{}", index + 1),
                a.clone(),
                b.clone(),
            ));
        }
    }
    if left.players.len() != right.players.len() {
        return Some(difference(
            format!("{path}.players.length"),
            left.players.len().to_string(),
            right.players.len().to_string(),
        ));
    }
    for (index, (a, b)) in left.players.iter().zip(&right.players).enumerate() {
        if let Some(diff) = compare_player(a, b, &format!("{path}.players.{}", index + 1)) {
            return Some(diff);
        }
    }
    if left.projectiles.len() != right.projectiles.len() {
        return Some(difference(
            format!("{path}.projectiles.length"),
            left.projectiles.len().to_string(),
            right.projectiles.len().to_string(),
        ));
    }
    for (index, (a, b)) in left.projectiles.iter().zip(&right.projectiles).enumerate() {
        if let Some(diff) = compare_projectile(a, b, &format!("{path}.projectiles.{}", index + 1)) {
            return Some(diff);
        }
    }
    if left.events.len() != right.events.len() {
        return Some(difference(
            format!("{path}.events.length"),
            left.events.len().to_string(),
            right.events.len().to_string(),
        ));
    }
    for (index, (a, b)) in left.events.iter().zip(&right.events).enumerate() {
        if let Some(diff) = compare_event(a, b, &format!("{path}.events.{}", index + 1)) {
            return Some(diff);
        }
    }
    if left.next_source_sequence != right.next_source_sequence {
        return Some(difference(
            format!("{path}.next_source_sequence"),
            left.next_source_sequence.to_string(),
            right.next_source_sequence.to_string(),
        ));
    }
    None
}

fn compare_player(
    a: &CombatPlayerState,
    b: &CombatPlayerState,
    path: &str,
) -> Option<CombatSnapshotDifference> {
    if a.loadout_id != b.loadout_id {
        return Some(difference(
            format!("{path}.loadout_id"),
            format!("{:?}", a.loadout_id),
            format!("{:?}", b.loadout_id),
        ));
    }
    if a.family_id != b.family_id {
        return Some(difference(
            format!("{path}.family_id"),
            format!("{:?}", a.family_id),
            format!("{:?}", b.family_id),
        ));
    }
    if a.phase != b.phase {
        return Some(difference(
            format!("{path}.phase"),
            phase_wire_str(a.phase),
            phase_wire_str(b.phase),
        ));
    }
    if a.phase_ticks != b.phase_ticks {
        return Some(difference(
            format!("{path}.phase_ticks"),
            a.phase_ticks.to_string(),
            b.phase_ticks.to_string(),
        ));
    }
    if a.cooldown_ticks != b.cooldown_ticks {
        return Some(difference(
            format!("{path}.cooldown_ticks"),
            a.cooldown_ticks.to_string(),
            b.cooldown_ticks.to_string(),
        ));
    }
    if a.source_sequence != b.source_sequence {
        return Some(difference(
            format!("{path}.source_sequence"),
            format!("{:?}", a.source_sequence),
            format!("{:?}", b.source_sequence),
        ));
    }
    if a.contacted != b.contacted {
        return Some(difference(
            format!("{path}.contacted"),
            a.contacted.to_string(),
            b.contacted.to_string(),
        ));
    }
    if a.release_latched != b.release_latched {
        return Some(difference(
            format!("{path}.release_latched"),
            a.release_latched.to_string(),
            b.release_latched.to_string(),
        ));
    }
    if a.control_held != b.control_held {
        return Some(difference(
            format!("{path}.control_held"),
            a.control_held.to_string(),
            b.control_held.to_string(),
        ));
    }
    if a.projectile_spawned != b.projectile_spawned {
        return Some(difference(
            format!("{path}.projectile_spawned"),
            a.projectile_spawned.to_string(),
            b.projectile_spawned.to_string(),
        ));
    }
    if a.forced_state != b.forced_state {
        return Some(difference(
            format!("{path}.forced_state"),
            format!("{:?}", a.forced_state),
            format!("{:?}", b.forced_state),
        ));
    }
    if a.forced_ticks != b.forced_ticks {
        return Some(difference(
            format!("{path}.forced_ticks"),
            a.forced_ticks.to_string(),
            b.forced_ticks.to_string(),
        ));
    }
    if a.chain_ticks != b.chain_ticks {
        return Some(difference(
            format!("{path}.chain_ticks"),
            a.chain_ticks.to_string(),
            b.chain_ticks.to_string(),
        ));
    }
    if a.immunity_ticks != b.immunity_ticks {
        return Some(difference(
            format!("{path}.immunity_ticks"),
            a.immunity_ticks.to_string(),
            b.immunity_ticks.to_string(),
        ));
    }
    let ip = format!("{path}.intent");
    if a.intent.stage != b.intent.stage {
        return Some(difference(
            format!("{ip}.stage"),
            format!("{:?}", a.intent.stage),
            format!("{:?}", b.intent.stage),
        ));
    }
    if a.intent.hold_ticks != b.intent.hold_ticks {
        return Some(difference(
            format!("{ip}.hold_ticks"),
            a.intent.hold_ticks.to_string(),
            b.intent.hold_ticks.to_string(),
        ));
    }
    if a.intent.reason != b.intent.reason {
        return Some(difference(
            format!("{ip}.reason"),
            format!("{:?}", a.intent.reason),
            format!("{:?}", b.intent.reason),
        ));
    }
    if a.intent.target_player != b.intent.target_player {
        return Some(difference(
            format!("{ip}.target_player"),
            format!("{:?}", a.intent.target_player),
            format!("{:?}", b.intent.target_player),
        ));
    }
    None
}

fn compare_projectile(
    a: &CombatProjectile,
    b: &CombatProjectile,
    path: &str,
) -> Option<CombatSnapshotDifference> {
    if a.family_id != b.family_id {
        return Some(difference(
            format!("{path}.family_id"),
            format!("{:?}", a.family_id),
            format!("{:?}", b.family_id),
        ));
    }
    if a.source_index != b.source_index {
        return Some(difference(
            format!("{path}.source_index"),
            a.source_index.to_string(),
            b.source_index.to_string(),
        ));
    }
    if a.source_sequence != b.source_sequence {
        return Some(difference(
            format!("{path}.source_sequence"),
            a.source_sequence.to_string(),
            b.source_sequence.to_string(),
        ));
    }
    if !same_f64(a.pos.x, b.pos.x) {
        return Some(difference(
            format!("{path}.pos.x"),
            a.pos.x.to_string(),
            b.pos.x.to_string(),
        ));
    }
    if !same_f64(a.pos.y, b.pos.y) {
        return Some(difference(
            format!("{path}.pos.y"),
            a.pos.y.to_string(),
            b.pos.y.to_string(),
        ));
    }
    if !same_f64(a.dir.x, b.dir.x) {
        return Some(difference(
            format!("{path}.dir.x"),
            a.dir.x.to_string(),
            b.dir.x.to_string(),
        ));
    }
    if !same_f64(a.dir.y, b.dir.y) {
        return Some(difference(
            format!("{path}.dir.y"),
            a.dir.y.to_string(),
            b.dir.y.to_string(),
        ));
    }
    if a.remaining_ticks != b.remaining_ticks {
        return Some(difference(
            format!("{path}.remaining_ticks"),
            a.remaining_ticks.to_string(),
            b.remaining_ticks.to_string(),
        ));
    }
    None
}

fn compare_event(a: &CombatEvent, b: &CombatEvent, path: &str) -> Option<CombatSnapshotDifference> {
    if a.kind != b.kind {
        return Some(difference(
            format!("{path}.kind"),
            a.kind.wire_str(),
            b.kind.wire_str(),
        ));
    }
    if a.tick != b.tick {
        return Some(difference(
            format!("{path}.tick"),
            a.tick.to_string(),
            b.tick.to_string(),
        ));
    }
    if a.family_id != b.family_id {
        return Some(difference(
            format!("{path}.family_id"),
            format!("{:?}", a.family_id),
            format!("{:?}", b.family_id),
        ));
    }
    if a.source_index != b.source_index {
        return Some(difference(
            format!("{path}.source_index"),
            format!("{:?}", a.source_index),
            format!("{:?}", b.source_index),
        ));
    }
    if a.target_index != b.target_index {
        return Some(difference(
            format!("{path}.target_index"),
            format!("{:?}", a.target_index),
            format!("{:?}", b.target_index),
        ));
    }
    if a.source_sequence != b.source_sequence {
        return Some(difference(
            format!("{path}.source_sequence"),
            format!("{:?}", a.source_sequence),
            format!("{:?}", b.source_sequence),
        ));
    }
    if a.result != b.result {
        return Some(difference(
            format!("{path}.result"),
            format!("{:?}", a.result),
            format!("{:?}", b.result),
        ));
    }
    if a.outcome != b.outcome {
        return Some(difference(
            format!("{path}.outcome"),
            format!("{:?}", a.outcome),
            format!("{:?}", b.outcome),
        ));
    }
    if a.reason != b.reason {
        return Some(difference(
            format!("{path}.reason"),
            format!("{:?}", a.reason),
            format!("{:?}", b.reason),
        ));
    }
    if a.terminal != b.terminal {
        return Some(difference(
            format!("{path}.terminal"),
            format!("{:?}", a.terminal),
            format!("{:?}", b.terminal),
        ));
    }
    if !same_f64(a.x, b.x) {
        return Some(difference(
            format!("{path}.x"),
            a.x.to_string(),
            b.x.to_string(),
        ));
    }
    if !same_f64(a.y, b.y) {
        return Some(difference(
            format!("{path}.y"),
            a.y.to_string(),
            b.y.to_string(),
        ));
    }
    if a.interruption_ticks != b.interruption_ticks {
        return Some(difference(
            format!("{path}.interruption_ticks"),
            format!("{:?}", a.interruption_ticks),
            format!("{:?}", b.interruption_ticks),
        ));
    }
    if a.displacement_px != b.displacement_px {
        return Some(difference(
            format!("{path}.displacement_px"),
            format!("{:?}", a.displacement_px),
            format!("{:?}", b.displacement_px),
        ));
    }
    None
}
