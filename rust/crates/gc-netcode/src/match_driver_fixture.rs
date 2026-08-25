//! Fixture builders for a connected, in-process online match session.
//!
//! This module builds three things: a pinned slot-mode boundary zero
//! ([`initial_snapshot`]), a frozen `CoordinatorFreeze` ([`freeze`]), and a
//! connected in-process star ([`session`]), the last built on
//! [`crate::fake_star`], a Rust in-process fake star transport written
//! against [`crate::fault_transport`]'s `StarTransportAdapter` trait.
//!
//! # Bridging the real and placeholder `CoordinatorFreeze`
//!
//! [`freeze`] returns [`crate::coordinator::Freeze`] — the real type, one to
//! one with what `coordinator::begin_countdown` produces.
//! [`crate::match_driver::MatchDriver`] itself still stores the narrower
//! `crate::match_driver::CoordinatorFreeze` placeholder documented in that
//! module (replacing it with the real type is out of scope for this file to
//! decide unilaterally). [`to_driver_freeze`]/[`to_driver_manifest`] bridge
//! the two, and [`DriverRules`] is the real
//! [`crate::match_driver::MatchDriverRules`] implementation a `MatchDriver`
//! caller injects — built by delegating to [`crate::live_slot`],
//! [`crate::coordinator`], and this file's own `canonical_host_batch`, which
//! needs manifest/assignment context [`crate::input_protocol`] deliberately
//! does not hold (see that module's own doc comment).

use crate::coordinator;
use crate::fake_star::{self, FakeStarTransport};
use crate::fault_transport::{
    StarTransportAdapter, TransportMessage, TransportMessageType, TransportRole,
};
use crate::input_protocol;
use crate::live_slot;
use crate::match_driver::{
    CoordinatorFreeze, CoordinatorSlotDriver, HostBatchError, HostBatchErrorCode, HostBatchRequest,
    LiveSlotRankEntry, LiveSlotTransition, LiveSlotTransitionOptions, MatchDriverRules,
    ProducerKind, SessionManifest, SlotAssignment,
};
use crate::protocol::{self, Value};
use crate::protocol_fixture;
use gc_data::teams;
use gc_sim::combat;
use gc_sim::combat_snapshot::CombatMatchState;
use gc_sim::input_frame::{self, SlotId};
use gc_sim::r#match::{self, NewMatchOptions};
use gc_sim::match_snapshot::{self, MatchSnapshot, MatchState, PitchSize};
use indexmap::IndexMap;

/// The fixture host's fixed peer id, duplicated locally the same way
/// `crate::match_driver` duplicates `transport_contract.HOST_PEER_ID`.
pub const HOST_PEER_ID: &str = "host";
/// The fixed countdown id every fixture session freezes under.
pub const COUNTDOWN_ID: &str = "countdown.1";
/// This fixture's default match duration, in seconds.
pub const DEFAULT_DURATION: f64 = 6.0;
/// This fixture's default simulation RNG seed.
pub const DEFAULT_SEED: f64 = 74.0;

/// This fixture's peer id for the guest seated at `index`.
#[must_use]
pub fn guest_peer_id(index: i64) -> String {
    format!("guest_{index}")
}

/// Boundary zero for the pinned combat fixture: two authored fixture teams,
/// slot mode, and (when `combat_active`) the combat snapshot schema.
///
/// `seed` overrides the pinned match seed — every peer shares one boundary
/// zero in a real session, so a differing seed is not a configuration; it is
/// the cheapest honest way to give one peer a genuinely divergent simulation
/// while every input row still agrees, which is what a desync looks like
/// from the driver's side.
///
/// # Panics
///
/// Panics if the `nebula`/`orion` fixture teams are missing from
/// [`gc_data::teams`] — a content-table invariant, never expected to fail.
#[must_use]
pub fn initial_snapshot(
    duration: Option<f64>,
    combat_active: bool,
    seed: Option<f64>,
) -> MatchSnapshot {
    let home = teams::get("nebula").expect("fixture team nebula is always authored");
    let away = teams::get("orion").expect("fixture team orion is always authored");
    let ownership = r#match::ownership_for_teams(home, away, None);
    let mut state = r#match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize {
            w: 1648.0,
            h: 927.0,
        },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(duration.unwrap_or(DEFAULT_DURATION)),
        max_goals: Some(99),
        seed: Some(seed.unwrap_or(DEFAULT_SEED)),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership),
    });
    if !combat_active {
        return match_snapshot::capture(&state, None);
    }
    let combat: CombatMatchState = combat::new_state(&mut state, None);
    match_snapshot::capture(&state, Some(&combat))
}

/// The full connected session this fixture assembles: mode, manifest,
/// frozen ownership, and every endpoint's transport.
pub struct MatchDriverFixtureSession {
    /// Which match mode this session seats.
    pub mode: protocol::MatchMode,
    /// The frozen session manifest.
    pub manifest: Value,
    /// The frozen session state.
    pub freeze: coordinator::Freeze,
    /// The host's own peer id.
    pub host_peer_id: String,
    /// Every seated guest's peer id, in seating order.
    pub guest_peer_ids: Vec<String>,
    /// The host endpoint of the in-process star.
    pub host_transport: FakeStarTransport,
    /// Peer id -> that guest's endpoint of the in-process star.
    pub guest_transports: IndexMap<String, FakeStarTransport>,
}

/// Every peer id this session seats, host first. `humans` defaults to a
/// full lobby: a short lobby is the only way a declared bot fill exists at
/// all, since at full seating every mode covers all eight slots.
///
/// # Panics
///
/// Panics if `humans` is outside `1..=` the mode's own human count.
#[must_use]
pub fn peer_ids(mode: protocol::MatchMode, humans: Option<i64>) -> Vec<String> {
    let shape = protocol::match_mode_shape(mode);
    let count = humans.unwrap_or(shape.humans);
    assert!(
        (1..=shape.humans).contains(&count),
        "the mode does not seat that many humans"
    );
    let mut ids = vec![HOST_PEER_ID.to_string()];
    for index in 1..count {
        ids.push(guest_peer_id(index));
    }
    ids
}

/// The freeze `coordinator.begin_countdown` produces, rebuilt from the same
/// public pieces: `plan_assignments` seats the humans, `slot_sources` proves
/// the seating, `owned_slots` names each owned set, and the opening live
/// slot is the first owned slot in canonical order.
///
/// # Panics
///
/// Panics if the fixture manifest, the planned assignments, or their
/// derived sources somehow fail to validate — every one of those is a
/// content/programmer invariant of this fixture, never expected to fail.
#[must_use]
pub fn freeze(
    mode: protocol::MatchMode,
    first_input_tick: Option<i64>,
    humans: Option<i64>,
) -> coordinator::Freeze {
    let manifest = protocol_fixture::manifest(Some(mode));
    let ids = peer_ids(mode, humans);
    let assignments =
        coordinator::plan_assignments(&manifest, &ids).expect("fixture assignments must be valid");
    coordinator::slot_sources(&manifest, &assignments).expect("fixture sources must be valid");
    let mut owned: IndexMap<String, Vec<SlotId>> = IndexMap::new();
    let mut live: IndexMap<String, SlotId> = IndexMap::new();
    for producer in coordinator::assignments_producers(&assignments) {
        if coordinator::producer_kind(producer) == coordinator::ProducerKind::Peer {
            let pid = coordinator::producer_id(producer);
            if !owned.contains_key(&pid) {
                let slots: Vec<SlotId> = protocol::owned_slots(&assignments, &pid)
                    .into_iter()
                    .map(|wire| protocol::slot_id_from_wire(&wire).expect("canonical owned slot"))
                    .collect();
                live.insert(pid.clone(), slots[0]);
                owned.insert(pid, slots);
            }
        }
    }
    coordinator::Freeze {
        manifest_id: protocol::manifest_id(&manifest),
        assignment_id: protocol::assignment_id(&assignments, 1),
        countdown_id: COUNTDOWN_ID.to_string(),
        first_input_tick: first_input_tick.unwrap_or(0),
        seed: coordinator::value_int(&manifest, "seed"),
        tick_rate: coordinator::value_int(&manifest, "tick_rate"),
        duration_ticks: coordinator::value_int(&manifest, "duration_ticks"),
        max_goals: coordinator::value_int(&manifest, "max_goals"),
        content_id: coordinator::value_str(&manifest, "content_id"),
        tuning_id: coordinator::value_str(&manifest, "tuning_id"),
        combat_rules_id: coordinator::value_str(&manifest, "combat_rules_id"),
        gameplay_ai_policy_id: coordinator::value_str(&manifest, "gameplay_ai_policy_id"),
        combat_status: coordinator::value_str(&manifest, "combat_status"),
        match_mode: mode,
        assignments,
        owned,
        live,
    }
}

/// One connected in-process star: a host endpoint plus one guest endpoint
/// per seated guest, all sharing a rendezvous so they can only reach each
/// other.
///
/// # Panics
///
/// Panics if constructing or linking any fixture endpoint somehow fails —
/// every step here is a fixture invariant, never expected to fail.
#[must_use]
pub fn session(
    mode: protocol::MatchMode,
    first_input_tick: Option<i64>,
    humans: Option<i64>,
) -> MatchDriverFixtureSession {
    let manifest = protocol_fixture::manifest(Some(mode));
    let freeze = freeze(mode, first_input_tick, humans);
    let ids = peer_ids(mode, humans);
    let rendezvous = FakeStarTransport::new_rendezvous();
    let mut host = FakeStarTransport::new(fake_star::FakeStarTransportOptions {
        role: TransportRole::Host,
        rendezvous: Some(rendezvous.clone()),
        ..Default::default()
    });
    host.initialize().expect("fixture host initializes");
    let mut guest_peer_ids = Vec::new();
    let mut guest_transports: IndexMap<String, FakeStarTransport> = IndexMap::new();
    for peer_id in ids.iter().skip(1) {
        guest_peer_ids.push(peer_id.clone());
        let mut guest = FakeStarTransport::new(fake_star::FakeStarTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some(peer_id.clone()),
            rendezvous: Some(rendezvous.clone()),
            ..Default::default()
        });
        guest.initialize().expect("fixture guest initializes");
        host.open_peer(peer_id)
            .expect("fixture host opens the guest slot");
        host.link(&guest).expect("fixture host links the guest");
        guest_transports.insert(peer_id.clone(), guest);
    }
    MatchDriverFixtureSession {
        mode,
        manifest,
        freeze,
        host_peer_id: HOST_PEER_ID.to_string(),
        guest_peer_ids,
        host_transport: host,
        guest_transports,
    }
}

// ---------------------------------------------------------------------------
// Bridging the real `coordinator::Freeze`/manifest to `match_driver`'s
// placeholder shapes (see module doc).
// ---------------------------------------------------------------------------

/// Converts a real [`coordinator::Freeze`] into the narrower
/// [`crate::match_driver::CoordinatorFreeze`] a [`match_driver::MatchDriver`]
/// is actually built from.
#[must_use]
pub fn to_driver_freeze(freeze: &coordinator::Freeze) -> CoordinatorFreeze {
    let assignments: [SlotAssignment; 8] = std::array::from_fn(|i| {
        let producer = coordinator::assignment_at(&freeze.assignments, i as i64 + 1);
        SlotAssignment {
            producer_kind: match coordinator::producer_kind(producer) {
                coordinator::ProducerKind::Peer => ProducerKind::Peer,
                coordinator::ProducerKind::Bot => ProducerKind::Bot,
            },
            producer_id: coordinator::producer_id(producer),
            bot_seed: coordinator::producer_bot_seed(producer),
        }
    });
    CoordinatorFreeze {
        manifest_id: freeze.manifest_id.clone(),
        first_input_tick: freeze.first_input_tick,
        seed: freeze.seed,
        assignments,
        owned: freeze.owned.clone(),
        live: freeze.live.clone(),
    }
}

/// Converts the real manifest [`Value`] into the narrower
/// [`crate::match_driver::SessionManifest`] a
/// [`match_driver::MatchDriver`] is actually built from.
#[must_use]
pub fn to_driver_manifest(manifest: &Value) -> SessionManifest {
    SessionManifest {
        session_id: coordinator::value_str(manifest, "session_id"),
    }
}

// ---------------------------------------------------------------------------
// `DriverRules`: the real `MatchDriverRules`, delegating to `live_slot`,
// `coordinator`, and this file's own `canonical_host_batch` (see module doc).
// ---------------------------------------------------------------------------

/// The real [`MatchDriverRules`] implementation: [`Self::carrier`]/
/// [`Self::transition`]/[`Self::next_live_slot`]/[`Self::slot_drivers`]
/// delegate directly to [`crate::live_slot`]/[`crate::coordinator`]; only
/// [`Self::canonical_host_batch`] is new code, because it needs manifest and
/// assignment validation that `input_protocol.rs` deliberately does not hold
/// (see that module's own doc comment).
///
/// Holds its own copy of the real manifest and freeze — the ones
/// [`crate::match_driver::MatchDriver`] itself is built from are the
/// narrower placeholder shapes ([`to_driver_freeze`]/[`to_driver_manifest`]),
/// which do not carry enough to validate a host batch against the session
/// protocol's own rules.
pub struct DriverRules {
    manifest: Value,
    freeze: coordinator::Freeze,
}

impl DriverRules {
    /// Builds the real rules environment for one frozen session.
    #[must_use]
    pub fn new(manifest: Value, freeze: coordinator::Freeze) -> Self {
        DriverRules { manifest, freeze }
    }
}

fn host_batch_error(code: HostBatchErrorCode, message: impl Into<String>) -> HostBatchError {
    HostBatchError {
        code,
        message: message.into(),
    }
}

/// A bounded opaque ASCII id: alphanumeric start, then alphanumerics plus
/// `_`, `.`, `-`. Restates `input_protocol.rs`'s private `validate_id`
/// predicate locally — see that module's doc comment for why this crate
/// duplicates small bound constants rather than importing them.
fn valid_ascii_id(value: &str, maximum: usize) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= maximum
        && bytes.iter().enumerate().all(|(index, &byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'_' | b'.' | b'-'))
        })
}

fn row_order(
    a: &input_protocol::AuthorityRow,
    b: &input_protocol::AuthorityRow,
) -> std::cmp::Ordering {
    a.tick.cmp(&b.tick).then(a.slot_index.cmp(&b.slot_index))
}

/// The same envelope check as [`input_protocol::validate_envelope`],
/// restated locally: `canonical_host_batch` reports failures as
/// [`HostBatchError`]/[`HostBatchErrorCode`] rather than
/// [`input_protocol::Error`], so it re-checks here instead of reusing that
/// function's `Result` type.
fn validate_envelope(
    packet: &input_protocol::Packet,
    envelope: &TransportMessage,
) -> std::result::Result<(), HostBatchError> {
    if envelope.kind != TransportMessageType::Input {
        return Err(host_batch_error(
            HostBatchErrorCode::Other,
            "input packet requires an input transport envelope",
        ));
    }
    if envelope.seq != packet.sequence || envelope.tick != Some(packet.transport_tick) {
        return Err(host_batch_error(
            HostBatchErrorCode::Other,
            "input packet sequence or transport tick mismatches envelope",
        ));
    }
    let expected = input_protocol::encode(packet).expect("a validated packet always encodes");
    if envelope.payload != expected {
        return Err(host_batch_error(
            HostBatchErrorCode::Other,
            "input packet bytes mismatch the transport envelope",
        ));
    }
    Ok(())
}

impl MatchDriverRules for DriverRules {
    fn carrier(&self, state: &MatchState) -> Option<SlotId> {
        live_slot::carrier(state)
    }

    fn transition(
        &self,
        state: &MatchState,
        options: LiveSlotTransitionOptions,
    ) -> LiveSlotTransition {
        let lua_transition = live_slot::transition(
            state,
            &live_slot::TransitionOptions {
                switch: options.switch,
                live: options.live,
                previous_carrier: options.previous_carrier,
            },
        );
        let ranked: Vec<LiveSlotRankEntry> = live_slot::entries(state)
            .into_iter()
            .filter(|entry| Some(entry.slot) != options.live)
            .map(|entry| LiveSlotRankEntry {
                slot: entry.slot,
                index: entry.index,
                distance_sq: entry.distance_sq,
            })
            .collect();
        LiveSlotTransition {
            switch: lua_transition.switch,
            carrier: lua_transition.carrier,
            winner: lua_transition.winner,
            ranked,
        }
    }

    fn next_live_slot(
        &self,
        owned: &[SlotId],
        current: SlotId,
        transition: &LiveSlotTransition,
    ) -> SlotId {
        let lua_transition = live_slot::LiveTransition {
            switch: transition.switch,
            carrier: transition.carrier,
            winner: transition.winner,
            ranked: transition.ranked.iter().map(|entry| entry.slot).collect(),
        };
        coordinator::next_live_slot(owned, current, &lua_transition)
    }

    fn slot_drivers(
        &self,
        _freeze: &CoordinatorFreeze,
        live: &IndexMap<String, SlotId>,
    ) -> Vec<(SlotId, CoordinatorSlotDriver)> {
        // `match_driver::CoordinatorSlotDriver` (the placeholder this trait's
        // caller declares) has no `kind: human|ai` field the way
        // `coordinator::SlotDriver` does — only `producer_id`. The closest
        // faithful encoding without inventing a field that placeholder
        // doesn't have: `Some(id)` exactly when `coordinator::slot_drivers`
        // says this slot is this tick's live *human* driver, `None` when it
        // is AI-driven (whether that AI is a declared bot fill or a peer's
        // currently-non-live owned slot). A caller that wants the coarser
        // "who owns this slot regardless of live status" reads
        // `freeze.assignments` directly instead of this diagnostic.
        let drivers = coordinator::slot_drivers(&self.freeze, Some(live));
        (1..=input_frame::SLOT_COUNT)
            .map(|index| {
                let slot = live_slot::slot_id(index);
                let producer = coordinator::assignment_at(&self.freeze.assignments, index);
                let producer_id = match drivers[(index - 1) as usize] {
                    coordinator::SlotDriver::Human => Some(coordinator::producer_id(producer)),
                    coordinator::SlotDriver::Ai => None,
                };
                (slot, CoordinatorSlotDriver { producer_id })
            })
            .collect()
    }

    fn canonical_host_batch(
        &self,
        request: HostBatchRequest<'_>,
    ) -> std::result::Result<input_protocol::Packet, HostBatchError> {
        protocol::validate_manifest(&self.manifest)
            .map_err(|err| host_batch_error(HostBatchErrorCode::Other, err.message))?;
        let manifest_id = protocol::manifest_id(&self.manifest);
        protocol::validate_assignment_manifest(&self.manifest, &self.freeze.assignments)
            .map_err(|err| host_batch_error(HostBatchErrorCode::OwnershipMismatch, err.message))?;
        if !valid_ascii_id(request.host_peer_id, protocol::MAX_PEER_ID_BYTES) {
            return Err(host_batch_error(
                HostBatchErrorCode::Other,
                "host peer id is invalid",
            ));
        }
        if !(0..=protocol::MAX_SEQUENCE).contains(&request.sequence) {
            return Err(host_batch_error(
                HostBatchErrorCode::Other,
                "host input sequence is invalid",
            ));
        }
        if !(0..=input_frame::MAX_TICK).contains(&request.transport_tick) {
            return Err(host_batch_error(
                HostBatchErrorCode::Other,
                "host batch transport tick is invalid",
            ));
        }
        if !(0..=input_frame::MAX_TICK).contains(&request.first_input_tick) {
            return Err(host_batch_error(
                HostBatchErrorCode::Other,
                "host first input tick is invalid",
            ));
        }
        // The transport contract's fixed queue-limit bound, duplicated the
        // same way `crate::match_driver` duplicates it.
        const TRANSPORT_MAX_QUEUE_LIMIT: i64 = 256;
        if request.arrivals.is_empty() || request.arrivals.len() as i64 > TRANSPORT_MAX_QUEUE_LIMIT
        {
            return Err(host_batch_error(
                HostBatchErrorCode::Other,
                "host input arrivals are unbounded",
            ));
        }

        let mut packets: IndexMap<(String, i64), input_protocol::Packet> = IndexMap::new();
        let mut authority: IndexMap<(i64, i64), input_protocol::AuthorityRow> = IndexMap::new();
        let mut rows: Vec<input_protocol::AuthorityRow> = Vec::new();

        for arrival in request.arrivals {
            let packet = &arrival.packet;
            input_protocol::validate(packet)
                .map_err(|err| host_batch_error(HostBatchErrorCode::Other, err.message))?;
            if packet.kind != input_protocol::PacketKind::Guest {
                return Err(host_batch_error(
                    HostBatchErrorCode::Other,
                    "host collector accepts only local-slot guest bundles",
                ));
            }
            if packet.session_id != request.manifest.session_id
                || packet.manifest_id != manifest_id
                || packet.first_input_tick != request.first_input_tick
            {
                return Err(host_batch_error(
                    HostBatchErrorCode::Other,
                    "guest input identity differs from the frozen match",
                ));
            }
            validate_envelope(packet, &arrival.envelope)?;
            if arrival.arrival_tick != request.transport_tick
                || arrival.arrival_tick < packet.transport_tick
            {
                return Err(host_batch_error(
                    HostBatchErrorCode::Other,
                    "guest input arrival is outside this transport batch",
                ));
            }
            if !valid_ascii_id(&arrival.transport_peer_id, protocol::MAX_PEER_ID_BYTES) {
                return Err(host_batch_error(
                    HostBatchErrorCode::Other,
                    "input transport peer id is invalid",
                ));
            }
            let slot_index = packet.rows[0].slot_index;
            let assignment = &request.assignments[(slot_index - 1) as usize];
            if assignment.producer_id != packet.sender_id {
                return Err(host_batch_error(
                    HostBatchErrorCode::OwnershipMismatch,
                    "input producer does not own the declared slot",
                ));
            }
            let mismatched = match assignment.producer_kind {
                ProducerKind::Peer => arrival.transport_peer_id != assignment.producer_id,
                ProducerKind::Bot => arrival.transport_peer_id != request.host_peer_id,
            };
            if mismatched {
                return Err(host_batch_error(
                    HostBatchErrorCode::OwnershipMismatch,
                    "transport peer cannot carry this input producer",
                ));
            }
            if arrival.transport_peer_id == request.host_peer_id
                && arrival.arrival_tick - packet.transport_tick
                    < input_protocol::FAIRNESS_DELAY_TICKS
            {
                return Err(host_batch_error(
                    HostBatchErrorCode::Other,
                    "host-local input bypassed the fixed fairness delay",
                ));
            }
            let packet_key = (packet.sender_id.clone(), packet.sequence);
            if let Some(previous) = packets.get(&packet_key) {
                input_protocol::classify_duplicate(previous, packet).map_err(|err| {
                    host_batch_error(HostBatchErrorCode::PacketConflict, err.message)
                })?;
            } else {
                packets.insert(packet_key, packet.clone());
                for row in &packet.rows {
                    let key = (row.tick, row.slot_index);
                    if let Some(prior) = authority.get(&key) {
                        if prior.sample != row.sample {
                            return Err(host_batch_error(
                                HostBatchErrorCode::AuthorityConflict,
                                "input authority conflicts within the host transport batch",
                            ));
                        }
                    } else {
                        authority.insert(key, row.clone());
                        rows.push(row.clone());
                    }
                }
            }
        }

        if let Some(repair_rows) = request.repair_rows {
            if repair_rows.len() as i64 > input_protocol::MAX_HOST_ROWS {
                return Err(host_batch_error(
                    HostBatchErrorCode::Other,
                    "host batch repair rows must be a bounded canonical array",
                ));
            }
            for row in repair_rows {
                let key = (row.tick, row.slot_index);
                if let Some(prior) = authority.get(&key) {
                    if prior.sample != row.sample {
                        return Err(host_batch_error(
                            HostBatchErrorCode::AuthorityConflict,
                            "a host repair row conflicts with authority in the same batch",
                        ));
                    }
                } else {
                    authority.insert(key, row.clone());
                    rows.push(row.clone());
                }
            }
        }

        rows.sort_by(row_order);
        if rows.len() as i64 > input_protocol::MAX_HOST_ROWS {
            return Err(host_batch_error(
                HostBatchErrorCode::Other,
                "host batch exceeds the bounded authority row count",
            ));
        }

        input_protocol::new_host(input_protocol::PacketOptions {
            session_id: request.manifest.session_id.clone(),
            manifest_id,
            sender_id: request.host_peer_id.to_string(),
            sequence: request.sequence,
            transport_tick: request.transport_tick,
            first_input_tick: request.first_input_tick,
            confirmed_span: Some(request.confirmed_span),
            rows,
        })
        .map_err(|err| host_batch_error(HostBatchErrorCode::Other, err.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault_transport::TransportPeerState;
    use crate::match_driver;

    #[test]
    fn guest_peer_id_matches_the_lua_naming() {
        assert_eq!(guest_peer_id(1), "guest_1");
        assert_eq!(guest_peer_id(7), "guest_7");
    }

    #[test]
    fn initial_snapshot_is_slot_mode_boundary_zero() {
        let snapshot = initial_snapshot(None, false, None);
        assert_eq!(snapshot.state.input_tick, 0);
        assert!(snapshot.state.slot_mode);
        assert!(!snapshot.state.finished);
    }

    #[test]
    fn initial_snapshot_with_combat_active_carries_the_combat_companion() {
        let with_combat = initial_snapshot(None, true, None);
        assert!(with_combat.combat.is_some());
        let without_combat = initial_snapshot(None, false, None);
        assert!(without_combat.combat.is_none());
    }

    #[test]
    fn a_differing_seed_produces_a_differently_hashed_boundary_zero() {
        let a = initial_snapshot(None, false, Some(1.0));
        let b = initial_snapshot(None, false, Some(2.0));
        assert_ne!(match_snapshot::hash(&a), match_snapshot::hash(&b));
    }

    #[test]
    fn peer_ids_seats_the_host_first_then_guests_in_order() {
        let ids = peer_ids(protocol::MatchMode::OneVOne, None);
        assert_eq!(ids, vec![HOST_PEER_ID.to_string(), "guest_1".to_string()]);
        let ids = peer_ids(protocol::MatchMode::FourVFour, Some(3));
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], HOST_PEER_ID);
    }

    #[test]
    fn freeze_seats_every_human_with_a_contiguous_owned_block() {
        let frozen = freeze(protocol::MatchMode::TwoVTwo, None, None);
        assert_eq!(frozen.owned.len(), 4);
        for slots in frozen.owned.values() {
            assert_eq!(slots.len(), 2);
        }
        assert_eq!(frozen.live.len(), 4);
    }

    #[test]
    fn session_links_a_host_and_every_guest_transport() {
        let mut fixture_session = session(protocol::MatchMode::TwoVTwo, None, None);
        assert_eq!(fixture_session.guest_peer_ids.len(), 3);
        for peer_id in &fixture_session.guest_peer_ids {
            assert_eq!(
                fixture_session.host_transport.peer_state(peer_id),
                Some(TransportPeerState::Connected)
            );
            let guest = fixture_session.guest_transports.get_mut(peer_id).unwrap();
            assert_eq!(
                guest.peer_state(fake_star::HOST_PEER_ID),
                Some(TransportPeerState::Connected)
            );
        }
    }

    fn build_driver(
        fixture_session: &MatchDriverFixtureSession,
        role: match_driver::DriverRole,
        peer_id: &str,
        transport: Box<dyn StarTransportAdapter>,
    ) -> match_driver::MatchDriver {
        match_driver::new(match_driver::MatchDriverOptions {
            role,
            peer_id: peer_id.to_string(),
            freeze: to_driver_freeze(&fixture_session.freeze),
            manifest: to_driver_manifest(&fixture_session.manifest),
            transport,
            initial_snapshot: initial_snapshot(
                None,
                false,
                Some(fixture_session.freeze.seed as f64),
            ),
            max_rollback_ticks: None,
            hash_interval_ticks: Some(5),
            settle_timeout_ticks: None,
            settle_timeout_seconds: None,
            clock: None,
            rules: Box::new(DriverRules::new(
                fixture_session.manifest.clone(),
                fixture_session.freeze.clone(),
            )),
        })
    }

    #[test]
    fn a_host_and_guest_driver_converge_on_the_same_checkpoint_hash() {
        let mut fixture_session = session(protocol::MatchMode::OneVOne, None, None);
        let guest_peer_id = fixture_session.guest_peer_ids[0].clone();
        let guest_transport = fixture_session
            .guest_transports
            .swap_remove(&guest_peer_id)
            .unwrap();
        let host_transport = fixture_session.host_transport.clone();

        let mut host = build_driver(
            &fixture_session,
            match_driver::DriverRole::Host,
            HOST_PEER_ID,
            Box::new(host_transport.clone()),
        );
        let mut guest = build_driver(
            &fixture_session,
            match_driver::DriverRole::Guest,
            &guest_peer_id,
            Box::new(guest_transport.clone()),
        );

        for _ in 0..40 {
            let _ = match_driver::advance(&mut host, None);
            let _ = match_driver::advance(&mut guest, None);
            host_transport.pump();
            guest_transport.pump();
        }
        assert_eq!(
            match_driver::status(&host),
            match_driver::MatchDriverStatus::Active
        );
        assert_eq!(
            match_driver::status(&guest),
            match_driver::MatchDriverStatus::Active
        );
        assert!(
            !host.checkpoints.is_empty(),
            "the host must have published at least one checkpoint"
        );
        assert!(
            !guest.checkpoints.is_empty(),
            "the guest must have published at least one checkpoint"
        );
        let mut compared = 0;
        for host_checkpoint in &host.checkpoints {
            if let Some(guest_checkpoint) = guest
                .checkpoints
                .iter()
                .find(|c| c.tick == host_checkpoint.tick)
            {
                compared += 1;
                assert_eq!(
                    host_checkpoint.hash, guest_checkpoint.hash,
                    "host and guest disagreed on the boundary hash at tick {}",
                    host_checkpoint.tick
                );
            }
        }
        assert!(
            compared > 0,
            "host and guest must share at least one checkpoint boundary"
        );
    }
}
