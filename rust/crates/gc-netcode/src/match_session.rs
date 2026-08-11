//! `match_session` builds the online match request: everything one peer
//! needs to start playing, derived only from the frozen manifest and the
//! frozen slot assignment.
//! Nothing here is local: two peers holding the same freeze build
//! byte-identical requests, including boundary zero — the driver's rollback
//! session is seeded from this snapshot on every peer, so a difference here
//! would be an immediate desync rather than a cosmetic disagreement.
//!
//! It draws nothing, opens no transport, and advances no simulation.
//!
//! # Ownership, switching, and the keeper
//!
//! A human owns a *set* of canonical outfield slots — four in 1v1, two in
//! 2v2, one in 4v4 — and controls one of them at a time. Switching inside
//! that frozen set reuses the shipped single-player rules, intersected with
//! the owned set by `coordinator::next_live_slot`. In 4v4 the owned set is a
//! singleton, so every branch of that rule returns the slot already live and
//! switching is inert — a *consequence* of the general rule, not a mode
//! branch anywhere in this file.
//!
//! **Keeper control stays disabled in every mode.** Keepers are slotless and
//! AI-only, so a human's owned set can never contain one. [`KEEPER_CONTROL`]
//! records the decision in one place.
//!
//! # `initial_snapshot` is a parameter, not something this module builds
//!
//! Building a boundary-zero snapshot means calling `gc_sim::r#match::new({
//! home, away, ..., input_ownership })`, where `home`/`away` are
//! `gc-data`-authored `TeamData` and `input_ownership` is a
//! `gc_sim::input_frame::InputOwnership` built from real `PlayerData`. Per
//! `crate::match_manifest`'s module doc comment, this crate has no `gc-data`
//! dependency, so `gc_sim::r#match::new` cannot be called from here at all:
//! its `NewMatchOptions` takes `&gc_data::teams::TeamData` directly, and this
//! module has no way to even name that type.
//!
//! [`request`] therefore takes the already-captured boundary-zero
//! [`gc_sim::match_snapshot::MatchSnapshot`] as a parameter instead of
//! building it. The caller — a layer that *does* have `gc-data` — builds it
//! with `r#match::new` (`input_ownership` set) followed by
//! `match_snapshot::capture`, and hands it in. [`request`] still asserts the
//! two invariants that recipe must produce (`state.slot_mode`,
//! `state.input_tick == 0`), so a caller that gets the recipe wrong still
//! fails loudly here rather than producing a quietly-wrong match.
//!
//! This also splits [`request`] into [`resolve_identity`] (every fallible
//! check: contracts, freeze/manifest identity agreement, owned-set shape,
//! opening live slot — none of it needs `initial_snapshot` or `content` at
//! all) and [`finish`] (infallible: attach the caller-supplied snapshot and
//! content, asserting the two invariants above). [`request`] is the two
//! composed into one entry point for real callers. The split exists because
//! `tests/match_session.rs` cannot build a `MatchSnapshot` without `gc-data`
//! (see that file), and [`resolve_identity`] is where all of this module's
//! actually-interesting, previously-untested logic lives — gating its test
//! coverage on an unavailable dependency would be a worse trade than the
//! split.

use gc_sim::input_frame::SlotId;
use gc_sim::{fixed_clock, input_tape, match_snapshot};

use crate::coordinator::{self, Freeze, Role, SlotDriver};
use crate::live_slot;
use crate::match_manifest::{self, OnlineMatchContent};
use crate::protocol::{self, MatchMode, Value};

/// Keeper control is off in every online mode. See the module doc comment.
pub const KEEPER_CONTROL: bool = false;

/// Inputs to [`request`]/[`resolve_identity`].
#[derive(Clone, Debug, PartialEq)]
pub struct RequestOptions {
    /// This peer's session role.
    pub role: Role,
    /// This peer's stable identity.
    pub peer_id: String,
    /// The frozen, agreed manifest.
    pub manifest: Value,
    /// The frozen ownership generation.
    pub freeze: Freeze,
}

/// Every fallible outcome of [`resolve_identity`], carrying enough to
/// [`finish`] once a caller supplies `initial_snapshot`/`content`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedIdentity {
    /// This peer's session role.
    pub role: Role,
    /// This peer's stable identity.
    pub peer_id: String,
    /// The frozen, agreed manifest.
    pub manifest: Value,
    /// The frozen ownership generation.
    pub freeze: Freeze,
    /// How the eight canonical slots are shared.
    pub mode: MatchMode,
    /// This peer's frozen owned set, canonical order.
    pub owned: Vec<SlotId>,
    /// Opening live slot.
    pub live: SlotId,
    /// Who materializes each canonical slot's input row, canonical order.
    pub slot_drivers: Vec<SlotDriver>,
    /// Explicit combat-bearing `MatchSnapshot` contract.
    pub snapshot_version: i64,
    /// Explicit combat-bearing input tape contract.
    pub tape_version: i64,
}

/// `OnlineMatchRequest`: one peer's complete online match request.
#[derive(Clone, Debug, PartialEq)]
pub struct OnlineMatchRequest {
    /// This peer's session role.
    pub role: Role,
    /// This peer's stable identity.
    pub peer_id: String,
    /// The frozen, agreed manifest.
    pub manifest: Value,
    /// The frozen ownership generation.
    pub freeze: Freeze,
    /// How the eight canonical slots are shared.
    pub mode: MatchMode,
    /// This peer's frozen owned set, canonical order.
    pub owned: Vec<SlotId>,
    /// Opening live slot.
    pub live: SlotId,
    /// Who materializes each canonical slot's input row, canonical order
    /// (`coordinator::slot_drivers`'s own convention: index `i` is the
    /// driver for `input_frame::slot(i + 1)`).
    pub slot_drivers: Vec<SlotDriver>,
    /// The home side's resolved content.
    pub home: match_manifest::TeamContent,
    /// The away side's resolved content.
    pub away: match_manifest::TeamContent,
    /// The resolved arena id.
    pub arena_id: String,
    /// Always `true`: the online match always selects the combat-bearing
    /// contracts explicitly.
    pub combat_enabled: bool,
    /// Explicit combat-bearing `MatchSnapshot` contract.
    pub snapshot_version: i64,
    /// Explicit combat-bearing input tape contract.
    pub tape_version: i64,
    /// Simulation RNG seed.
    pub seed: i64,
    /// Match duration, in seconds, derived from the frozen tick budget.
    pub duration: f64,
    /// Goal cap.
    pub max_goals: i64,
    /// The frozen session's first authority input tick.
    pub first_input_tick: i64,
    /// Boundary zero.
    pub initial_snapshot: match_snapshot::MatchSnapshot,
}

fn manifest_int(manifest: &Value, field: &str) -> Option<i64> {
    manifest.get(field).and_then(Value::as_int)
}

fn manifest_str<'a>(manifest: &'a Value, field: &str) -> Option<&'a str> {
    manifest.get(field).and_then(Value::as_str)
}

fn identity_agrees(freeze: &Freeze, manifest: &Value) -> std::result::Result<(), String> {
    let manifest_id = protocol::manifest_id(manifest);
    if manifest_id != freeze.manifest_id {
        return Err("the frozen manifest digest does not match the manifest".to_string());
    }
    if manifest_int(manifest, "seed") != Some(freeze.seed) {
        return Err("the freeze and the manifest disagree about seed".to_string());
    }
    if manifest_int(manifest, "tick_rate") != Some(freeze.tick_rate) {
        return Err("the freeze and the manifest disagree about tick_rate".to_string());
    }
    if manifest_int(manifest, "duration_ticks") != Some(freeze.duration_ticks) {
        return Err("the freeze and the manifest disagree about duration_ticks".to_string());
    }
    if manifest_int(manifest, "max_goals") != Some(freeze.max_goals) {
        return Err("the freeze and the manifest disagree about max_goals".to_string());
    }
    if manifest_str(manifest, "content_id") != Some(freeze.content_id.as_str()) {
        return Err("the freeze and the manifest disagree about content_id".to_string());
    }
    if manifest_str(manifest, "tuning_id") != Some(freeze.tuning_id.as_str()) {
        return Err("the freeze and the manifest disagree about tuning_id".to_string());
    }
    if manifest_str(manifest, "combat_rules_id") != Some(freeze.combat_rules_id.as_str()) {
        return Err("the freeze and the manifest disagree about combat_rules_id".to_string());
    }
    if manifest_str(manifest, "gameplay_ai_policy_id")
        != Some(freeze.gameplay_ai_policy_id.as_str())
    {
        return Err("the freeze and the manifest disagree about gameplay_ai_policy_id".to_string());
    }
    if manifest_str(manifest, "combat_status") != Some(freeze.combat_status.as_str()) {
        return Err("the freeze and the manifest disagree about combat_status".to_string());
    }
    if manifest_str(manifest, "match_mode") != Some(freeze.match_mode.wire_str()) {
        return Err("the freeze and the manifest disagree about match_mode".to_string());
    }
    Ok(())
}

/// The online match always selects the combat-bearing contracts explicitly.
/// The protocol already refuses any other version, but stating it here means
/// the boundary-zero snapshot carries a `CombatMatchState` because this flow
/// asked for one, not because a default happened to line up.
fn contracts_agree(manifest: &Value) -> std::result::Result<(), String> {
    if manifest_int(manifest, "snapshot_version") != Some(match_snapshot::COMBAT_VERSION) {
        return Err("the online match requires the combat-bearing snapshot contract".to_string());
    }
    if manifest_int(manifest, "tape_version") != Some(input_tape::COMBAT_VERSION) {
        return Err("the online match requires the combat-bearing tape contract".to_string());
    }
    if manifest_int(manifest, "tick_rate") != Some(fixed_clock::TICK_RATE as i64) {
        return Err("the manifest tick rate is not this build's fixed tick rate".to_string());
    }
    Ok(())
}

fn owned_slots(freeze: &Freeze, peer_id: &str) -> std::result::Result<Vec<SlotId>, String> {
    let shape = protocol::match_mode_shape(freeze.match_mode);
    let owned_wire = protocol::owned_slots(&freeze.assignments, peer_id);
    if owned_wire.is_empty() {
        // A spectating peer is not a supported OMP-3 shape: every seated
        // human owns a block, and a peer with no block would author nothing
        // while the partition still expected it to.
        return Err("the freeze seats no slots for this peer".to_string());
    }
    if owned_wire.len() as i64 != shape.slots_per_human {
        return Err("the frozen owned set is the wrong size for the match mode".to_string());
    }
    let declared = freeze.owned.get(peer_id);
    let declared = match declared {
        Some(declared) if declared.len() == owned_wire.len() => declared,
        _ => return Err("the freeze does not record this peer's owned set".to_string()),
    };
    let mut owned = Vec::with_capacity(owned_wire.len());
    for (index, slot_wire) in owned_wire.iter().enumerate() {
        let slot = protocol::slot_id_from_wire(slot_wire).ok_or_else(|| {
            "the frozen owned set names a slot outside the canonical eight".to_string()
        })?;
        if declared[index] != slot {
            return Err("the frozen owned set disagrees with the frozen assignments".to_string());
        }
        owned.push(slot);
    }
    Ok(owned)
}

/// Every fallible check `game.online.match_session.request` runs, before it
/// ever touches `initial_snapshot`/`content`: contracts this build accepts,
/// freeze/manifest identity agreement, the owned set's shape for the frozen
/// mode, and that the opening live slot is inside that owned set. Returns
/// `Err` for anything a caller is meant to handle.
pub fn resolve_identity(options: RequestOptions) -> std::result::Result<ResolvedIdentity, String> {
    contracts_agree(&options.manifest)?;
    identity_agrees(&options.freeze, &options.manifest)?;
    let owned = owned_slots(&options.freeze, &options.peer_id)?;
    let live = options
        .freeze
        .live
        .get(&options.peer_id)
        .copied()
        .ok_or_else(|| "the freeze records no opening live slot for this peer".to_string())?;
    if !owned.contains(&live) {
        return Err("the opening live slot is outside the frozen owned set".to_string());
    }
    let slot_drivers = coordinator::slot_drivers(&options.freeze, Some(&options.freeze.live));
    Ok(ResolvedIdentity {
        role: options.role,
        peer_id: options.peer_id,
        mode: options.freeze.match_mode,
        owned,
        live,
        slot_drivers,
        snapshot_version: manifest_int(&options.manifest, "snapshot_version").unwrap(),
        tape_version: manifest_int(&options.manifest, "tape_version").unwrap(),
        manifest: options.manifest,
        freeze: options.freeze,
    })
}

/// Attaches the caller-built `initial_snapshot`/`content` to an already
/// [`resolve_identity`]d request. Infallible: everything that can fail
/// already did, in [`resolve_identity`].
///
/// # Panics
///
/// Panics if `initial_snapshot` is not a slot-mode, boundary-zero snapshot;
/// see the module doc comment for why building it is the caller's job
/// rather than this module's.
#[must_use]
pub fn finish(
    identity: ResolvedIdentity,
    initial_snapshot: match_snapshot::MatchSnapshot,
    content: OnlineMatchContent,
) -> OnlineMatchRequest {
    let (state, _combat) = match_snapshot::restore(&initial_snapshot);
    assert!(
        state.slot_mode,
        "an online match requires a slot-mode simulation"
    );
    assert!(
        state.input_tick == 0,
        "an online match starts at boundary zero"
    );

    OnlineMatchRequest {
        role: identity.role,
        peer_id: identity.peer_id,
        mode: identity.mode,
        owned: identity.owned,
        live: identity.live,
        slot_drivers: identity.slot_drivers,
        home: content.home,
        away: content.away,
        arena_id: content.arena_id,
        combat_enabled: true,
        snapshot_version: identity.snapshot_version,
        tape_version: identity.tape_version,
        seed: identity.freeze.seed,
        duration: identity.freeze.duration_ticks as f64 / identity.freeze.tick_rate as f64,
        max_goals: identity.freeze.max_goals,
        first_input_tick: identity.freeze.first_input_tick,
        initial_snapshot,
        manifest: identity.manifest,
        freeze: identity.freeze,
    }
}

/// `match_session.request`: build one peer's complete online match request.
/// Returns `Err` for anything a caller is meant to handle — see
/// [`resolve_identity`], which runs every fallible check before this
/// function ever touches `initial_snapshot`/`content`.
///
/// # Panics
///
/// See [`finish`].
pub fn request(
    options: RequestOptions,
    initial_snapshot: match_snapshot::MatchSnapshot,
    content: OnlineMatchContent,
) -> std::result::Result<OnlineMatchRequest, String> {
    let identity = resolve_identity(options)?;
    Ok(finish(identity, initial_snapshot, content))
}

/// The match player index a slot drives, for presentation only. Slot routing
/// in the simulation is `slot_players`; this is what the camera follows and
/// what the HUD names, and it can never widen who a peer may author.
///
/// # Panics
///
/// Panics if `slot` is unmapped in `state`: every canonical slot must have
/// a player mapped for a valid boundary snapshot, so an unmapped slot here
/// is a programmer error, not external input.
#[must_use]
pub fn player_index(state: &match_snapshot::MatchState, slot: SlotId) -> i64 {
    let index = live_slot::slot_index(slot);
    state.slot_players[(index - 1) as usize].expect("canonical slot is unmapped")
}

/// True when the slot a human is live on is currently carrying the ball. The
/// offline switch rule ("K without the ball switches") reads exactly this.
#[must_use]
pub fn carrying(state: &match_snapshot::MatchState, slot: Option<SlotId>) -> bool {
    match (slot, state.owner) {
        (Some(slot), Some(owner)) => owner == player_index(state, slot),
        _ => false,
    }
}
