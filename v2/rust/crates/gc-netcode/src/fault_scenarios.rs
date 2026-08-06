//! Port of `game/online/fault_scenarios.lua`.
//!
//! `SCENARIOS` is the data: one row per `(mode x profile x fault)`
//! combination the campaign claims to cover. The Lua `run` is the
//! interpreter that turns a row into a driven `FaultHarness` and a report.
//! [`crate::fault_harness::FaultHarness`] is now real (construction, the
//! pre-match lifecycle over a real star, the match itself, and teardown —
//! see that module's doc comment), so [`run`]/[`inject`]/[`first_guest`] are
//! ported for real below, following the Lua interpreter's own shape.
//! `SessionMatchMode` — this file's former local placeholder for
//! `protocol.lua`'s match-mode alias — is now [`crate::protocol::MatchMode`]
//! directly: that module's own doc comment named this file's placeholder as
//! the one to retire "once `protocol.rs` is stable", and building a live
//! harness needs `protocol::MatchMode` anyway (every
//! [`crate::fault_harness::FaultHarness`] entry point takes one).

use crate::fault_harness::{self, FaultHarness, FaultHarnessOptions};
use crate::fault_transport::{
    FaultTransportPollOrder, StarTransportAdapter, TransportChannel, TransportMessage,
    TransportMessageType,
};
use crate::input_protocol;
use crate::match_driver::{self, CoordinatorFreeze, slot_id_of};
use crate::protocol;
use gc_data::network_profiles::NetworkProfileName;
use gc_sim::input_frame;

/// Which shape of fault one [`FaultScenario`] injects. Mirrors
/// `FaultInjectionKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultInjectionKind {
    /// No injected fault; a pure convergence row.
    None,
    /// Input arrivals released last-first.
    PollReversed,
    /// Every Nth control envelope re-delivered.
    ControlDuplication,
    /// A clamped send buffer that must actually latch.
    Backpressure,
    /// A declared hold window straddling full time.
    BurstAcrossFullTime,
    /// A guest link closed mid-match.
    PeerDisconnect,
    /// The host shuts down mid-match.
    HostDeparture,
    /// A forged bundle claims a slot its sender does not own.
    OwnershipViolation,
    /// Two forged bundles disagree on one tick's authority.
    AuthorityConflict,
    /// A malformed (undecodable) input envelope.
    MalformedInput,
    /// A forged bundle claims the wrong manifest identity.
    ManifestMismatch,
    /// Input withheld well past the retained rollback window.
    OverWindowInput,
    /// A peer repeatedly reports a foreign hash for its own checkpoint.
    HashDivergence,
}

/// One declared row of the OMP-3 fault matrix. Mirrors `FaultScenario`.
#[derive(Clone, Copy, Debug)]
pub struct FaultScenario {
    /// Stable row identity.
    pub id: &'static str,
    /// Which match mode this row seats.
    pub mode: protocol::MatchMode,
    /// Which network profile this row runs under.
    pub profile: NetworkProfileName,
    /// Which fault (if any) this row injects.
    pub injection: FaultInjectionKind,
    /// Seated humans; `None` means the mode's own full seating.
    pub humans: Option<i64>,
    /// Match duration override, in ticks.
    pub duration_ticks: Option<i64>,
    /// Boundary-hash interval override, in ticks.
    pub hash_interval_ticks: Option<i64>,
    /// Driver step the injection fires on.
    pub at_step: Option<i64>,
    /// Declared terminal for a deliberately fatal row.
    pub expect_status: Option<crate::match_driver::MatchDriverStatus>,
    /// Part of the bounded CI subset.
    pub smoke: bool,
    /// A defect this row currently exposes, named rather than excused.
    pub known_gap: Option<&'static str>,
    /// Free-form context.
    pub note: Option<&'static str>,
}

/// The forged sequence sits far above anything a real peer emits in a
/// scenario of this length, so a forged bundle can never collide with
/// honest traffic.
pub const FORGED_SEQUENCE: i64 = 9000;
/// Comfortably past the 30-tick retained floor, so the released backlog is
/// provably outside the window rather than marginally so.
pub const OVER_WINDOW_HOLD_TICKS: i64 = 45;
/// Twice the largest legal payload (mirrors
/// `transport_contract.MAX_PAYLOAD_BYTES`, duplicated the same way
/// `crate::match_driver` duplicates its transport-contract bounds), so a
/// single message always fits and several queued in one tick do not.
pub const BACKPRESSURE_LIMIT_BYTES: i64 = 2 * 1024;
/// A liveness bound on one scenario run's driver steps.
pub const MAX_STEPS: i64 = 900;
/// The bounded CI subset's match duration, in ticks.
pub const SMOKE_DURATION_TICKS: i64 = 120;

use FaultInjectionKind::{
    AuthorityConflict, Backpressure, BurstAcrossFullTime, ControlDuplication, HashDivergence,
    HostDeparture, MalformedInput, ManifestMismatch, None as NoFault, OverWindowInput,
    OwnershipViolation, PeerDisconnect, PollReversed,
};
use protocol::MatchMode::{FourVFour, OneVOne, TwoVTwo};

/// The declared OMP-3 fault matrix. Mirrors `fault_scenarios.SCENARIOS`.
pub static SCENARIOS: &[FaultScenario] = &[
    // Convergence across every mode and every documented profile. 1v1 and
    // 2v2 are the rows that can exhibit a live-slot divergence at all.
    FaultScenario {
        id: "1v1.clean",
        mode: OneVOne,
        profile: NetworkProfileName::Clean,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "1v1.omp0_parity",
        mode: OneVOne,
        profile: NetworkProfileName::Omp0Parity,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "1v1.playable",
        mode: OneVOne,
        profile: NetworkProfileName::Playable,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "1v1.stress",
        mode: OneVOne,
        profile: NetworkProfileName::Stress,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.clean",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.omp0_parity",
        mode: TwoVTwo,
        profile: NetworkProfileName::Omp0Parity,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.playable",
        mode: TwoVTwo,
        profile: NetworkProfileName::Playable,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.stress",
        mode: TwoVTwo,
        profile: NetworkProfileName::Stress,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "4v4.clean",
        mode: FourVFour,
        profile: NetworkProfileName::Clean,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: Some("the eight-client row: one host plus seven guests"),
    },
    FaultScenario {
        id: "4v4.omp0_parity",
        mode: FourVFour,
        profile: NetworkProfileName::Omp0Parity,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: Some("the eight-client row under a lossy but jitter-free profile"),
    },
    FaultScenario {
        id: "4v4.playable",
        mode: FourVFour,
        profile: NetworkProfileName::Playable,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: Some(
            "the eight-client row under the documented playable profile; this is the row that \
             found #241, and it is the row that proves the fix -- eight clients under a \
             reordering profile is the only shape in this matrix where the guest-to-host and \
             host-to-guest legs compound",
        ),
    },
    FaultScenario {
        id: "4v4.stress",
        mode: FourVFour,
        profile: NetworkProfileName::Stress,
        injection: NoFault,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: Some(
            "the same eight-client shape at a higher loss and burst rate; this is the row that \
             found #243, and the row that proves it closed -- it stranded a guest on 18 of 48 \
             impairment seeds until the host's canonical batch was sized to the byte budget and \
             aimed by the confirmation each guest now reports, and it converges on all 48 since",
        ),
    },
    // Short lobbies: fewer humans than the mode seats, so declared bot
    // fills exist at all.
    FaultScenario {
        id: "4v4.bot_fill",
        mode: FourVFour,
        profile: NetworkProfileName::Playable,
        injection: NoFault,
        humans: Some(3),
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: Some("five declared bot fills authored by the host"),
    },
    // Recovery paths that must not depend on transport polling order.
    FaultScenario {
        id: "2v2.poll_reversed",
        mode: TwoVTwo,
        profile: NetworkProfileName::Stress,
        injection: PollReversed,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "4v4.control_duplication",
        mode: FourVFour,
        // Deliberately the jitter-free profile: this row is about a
        // duplicated control envelope, and pairing it with `playable` would
        // only re-run what `4v4.playable` already covers.
        profile: NetworkProfileName::Omp0Parity,
        injection: ControlDuplication,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.backpressure",
        mode: TwoVTwo,
        profile: NetworkProfileName::Playable,
        injection: Backpressure,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: false,
        known_gap: None,
        note: Some("a clamped send buffer, so the star latches backpressure and drains it again"),
    },
    FaultScenario {
        id: "2v2.burst_across_full_time",
        mode: TwoVTwo,
        profile: NetworkProfileName::Playable,
        injection: BurstAcrossFullTime,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: None,
        expect_status: None,
        smoke: true,
        known_gap: None,
        note: Some(
            "a declared window straddling full time, which a profile roll cannot be made to hit",
        ),
    },
    // The explicit terminal taxonomy. Every row declares the status it
    // expects.
    FaultScenario {
        id: "2v2.peer_disconnect",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: PeerDisconnect,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(40),
        expect_status: Some(crate::match_driver::MatchDriverStatus::TransportLost),
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.host_departure",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: HostDeparture,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(40),
        expect_status: Some(crate::match_driver::MatchDriverStatus::TransportLost),
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.ownership_violation",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: OwnershipViolation,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(30),
        expect_status: Some(crate::match_driver::MatchDriverStatus::OwnershipViolation),
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.authority_conflict",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: AuthorityConflict,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(30),
        expect_status: Some(crate::match_driver::MatchDriverStatus::AuthorityConflict),
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.malformed_input",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: MalformedInput,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(30),
        expect_status: Some(crate::match_driver::MatchDriverStatus::InputChannelFailure),
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.manifest_mismatch",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: ManifestMismatch,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(30),
        expect_status: Some(crate::match_driver::MatchDriverStatus::InputChannelFailure),
        smoke: false,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.over_window_input",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: OverWindowInput,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: None,
        at_step: Some(30),
        expect_status: Some(crate::match_driver::MatchDriverStatus::ConfirmationStalled),
        smoke: true,
        known_gap: None,
        note: None,
    },
    FaultScenario {
        id: "2v2.hash_divergence",
        mode: TwoVTwo,
        profile: NetworkProfileName::Clean,
        injection: HashDivergence,
        humans: None,
        duration_ticks: None,
        hash_interval_ticks: Some(6),
        at_step: Some(45),
        expect_status: Some(crate::match_driver::MatchDriverStatus::HashMismatch),
        smoke: true,
        known_gap: None,
        note: None,
    },
];

/// Looks up a declared row by id. Mirrors `fault_scenarios.find`.
#[must_use]
pub fn find(id: &str) -> Option<&'static FaultScenario> {
    SCENARIOS.iter().find(|scenario| scenario.id == id)
}

/// The full matrix, or just the bounded CI subset. Mirrors
/// `fault_scenarios.select`.
#[must_use]
pub fn select(smoke_only: bool) -> Vec<&'static FaultScenario> {
    if !smoke_only {
        return SCENARIOS.iter().collect();
    }
    SCENARIOS.iter().filter(|scenario| scenario.smoke).collect()
}

// ---------------------------------------------------------------------------
// Injections
// ---------------------------------------------------------------------------

/// Inputs to [`forged_bundle`]. Bundled into one struct (rather than eight
/// positional parameters) purely for call-site clarity and to keep the
/// function under clippy's `too_many_arguments` — the Lua original is a
/// positional function of the same eight values.
pub struct ForgedBundleRequest<'a> {
    /// See [`input_protocol::Packet::session_id`].
    pub session_id: &'a str,
    /// See [`input_protocol::Packet::manifest_id`].
    pub manifest_id: &'a str,
    /// See [`input_protocol::Packet::sender_id`].
    pub sender_id: &'a str,
    /// Which canonical slot the forged rows claim authority over.
    pub slot_index: i64,
    /// See [`input_protocol::Packet::first_input_tick`].
    pub first_input_tick: i64,
    /// See [`input_protocol::Packet::sequence`].
    pub sequence: i64,
    /// See [`input_protocol::Packet::transport_tick`].
    pub transport_tick: i64,
    /// Edge bitmask carried by the newest (highest-tick) forged row only;
    /// every earlier row is neutral.
    pub edges: i64,
}

/// A real, encodable guest bundle that the sender's own driver would never
/// author. Mirrors `forged_bundle`. Every fault this crate can express is
/// reached this way rather than by reaching into a driver's internals: a
/// harness that pokes private state proves the poke works.
///
/// # Panics
///
/// Panics if the forged rows or packet somehow fail to validate (a
/// programmer invariant: every field here is well-formed by construction).
#[must_use]
pub fn forged_bundle(request: ForgedBundleRequest<'_>) -> TransportMessage {
    let mut rows = Vec::new();
    for tick in 0..=input_protocol::HISTORY_ROWS {
        let sample = input_frame::new_sample(input_frame::InputSampleOptions {
            edges: Some(if tick == input_protocol::HISTORY_ROWS {
                request.edges
            } else {
                0
            }),
            ..Default::default()
        })
        .expect("a forged neutral-plus-edges sample is always valid");
        rows.push(input_protocol::AuthorityRow {
            tick,
            slot_index: request.slot_index,
            sample,
        });
    }
    let packet = input_protocol::new_guest(input_protocol::PacketOptions {
        session_id: request.session_id.to_string(),
        manifest_id: request.manifest_id.to_string(),
        sender_id: request.sender_id.to_string(),
        sequence: request.sequence,
        transport_tick: request.transport_tick,
        first_input_tick: request.first_input_tick,
        confirmed_span: None,
        rows,
    })
    .expect("a forged guest packet is always well-formed");
    let wire = input_protocol::encode(&packet).expect("a forged guest packet always encodes");
    TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq: packet.sequence,
        tick: Some(packet.transport_tick),
        payload: wire,
    }
}

/// A slot `guest_peer_id` provably does not own: the frozen partition covers
/// all eight, so the host's own first owned slot is outside every guest's
/// set. Mirrors `foreign_slot_index`.
///
/// # Panics
///
/// Panics if every slot belongs to `guest_peer_id`, which the frozen
/// partition forbids for any lobby with more than one seated producer.
#[must_use]
pub fn foreign_slot_index(freeze: &CoordinatorFreeze, guest_peer_id: &str) -> i64 {
    let owned: Vec<_> = freeze.owned.get(guest_peer_id).cloned().unwrap_or_default();
    for index in 1..=input_frame::SLOT_COUNT {
        if !owned.contains(&slot_id_of(index)) {
            return index;
        }
    }
    panic!("every slot belongs to one guest, which the frozen partition forbids");
}

// ---------------------------------------------------------------------------
// The interpreter: turns one declared row into a driven `FaultHarness` and a
// report.
// ---------------------------------------------------------------------------

/// A guest-side fault needs at least one guest. Mirrors `first_guest`.
///
/// # Panics
///
/// Panics if `harness` seats no guests.
fn first_guest(harness: &FaultHarness) -> &fault_harness::FaultHarnessClient {
    assert!(
        harness.clients.len() > 1,
        "a guest-side fault needs at least one guest"
    );
    &harness.clients[1]
}

/// Fires one scenario's declared fault. Mirrors `inject`. Every fault below
/// is reached the same way the Lua does: over the real wire (a forged
/// bundle, a declared hold, a transport-level close/shutdown), never by
/// reaching into a driver's internals.
///
/// # Panics
///
/// Panics if the scenario names a guest-side fault and `harness` seats no
/// guest, or (for `hash_divergence`) if the injection fires before enough
/// boundaries were hashed — both scenario-authoring errors, never expected
/// from a declared [`SCENARIOS`] row.
fn inject(harness: &mut FaultHarness, scenario: &FaultScenario) {
    match scenario.injection {
        FaultInjectionKind::PeerDisconnect => {
            let guest_peer_id = first_guest(harness).peer_id.clone();
            let mut star = harness.host_star();
            let _ = star.close_peer(&guest_peer_id, Some("peer_left"));
            return;
        }
        FaultInjectionKind::HostDeparture => {
            let mut star = harness.host_star();
            let _ = star.shutdown();
            return;
        }
        FaultInjectionKind::HashDivergence => {
            // The driver's own detection path: report a foreign hash for a
            // boundary this peer did hash, repeatedly, until the documented
            // streak is reached.
            let checkpoints = first_guest(harness).checkpoints.clone();
            assert!(
                checkpoints.len() as i64 >= match_driver::MAX_HASH_MISMATCHES,
                "the divergence row fires before enough boundaries were hashed"
            );
            let start = (checkpoints.len() as i64 - match_driver::MAX_HASH_MISMATCHES) as usize;
            let driver = harness.clients[1]
                .driver
                .as_mut()
                .expect("the first guest has a driver once the match has started");
            for checkpoint in &checkpoints[start..] {
                match_driver::observe_checkpoint(driver, checkpoint.tick, "deadbeefdeadbeef");
            }
            return;
        }
        FaultInjectionKind::OverWindowInput => {
            // Nothing is delivered for well over the 30-tick retained floor.
            // The backlog is still released (a `hold`, not a `withhold`),
            // because the row must prove the peer says so *before* it
            // arrives rather than only that something eventually
            // terminates.
            let from = harness.transport_tick + 1;
            for client in &harness.clients {
                client.hold(from, from + OVER_WINDOW_HOLD_TICKS);
            }
            return;
        }
        _ => {}
    }

    let guest = &harness.clients[1];
    let freeze = guest
        .coordinator
        .freeze
        .clone()
        .expect("the first guest froze a session");
    let session_id = guest.coordinator.session_id.clone();
    let sender_id = guest.peer_id.clone();
    let transport_tick = harness.step + match_driver::DELAY_TICKS;

    match scenario.injection {
        FaultInjectionKind::OwnershipViolation => {
            let driver_freeze = crate::match_driver_fixture::to_driver_freeze(&freeze);
            let slot_index = foreign_slot_index(&driver_freeze, &sender_id);
            let message = forged_bundle(ForgedBundleRequest {
                session_id: &session_id,
                manifest_id: &freeze.manifest_id,
                sender_id: &sender_id,
                slot_index,
                first_input_tick: freeze.first_input_tick,
                sequence: FORGED_SEQUENCE,
                transport_tick,
                edges: 0,
            });
            let _ = harness.clients[1].send_raw(
                fault_harness::HOST_PEER_ID,
                TransportChannel::Input,
                message,
            );
        }
        FaultInjectionKind::ManifestMismatch => {
            let forged_manifest_id = "0".repeat(freeze.manifest_id.len());
            let message = forged_bundle(ForgedBundleRequest {
                session_id: &session_id,
                manifest_id: &forged_manifest_id,
                sender_id: &sender_id,
                slot_index: 1,
                first_input_tick: freeze.first_input_tick,
                sequence: FORGED_SEQUENCE,
                transport_tick,
                edges: 0,
            });
            let _ = harness.clients[1].send_raw(
                fault_harness::HOST_PEER_ID,
                TransportChannel::Input,
                message,
            );
        }
        FaultInjectionKind::AuthorityConflict => {
            let owned_first = freeze
                .owned
                .get(&sender_id)
                .expect("the first guest owns at least one slot")[0];
            let slot_index = match_driver::slot_index_of(owned_first);
            for edges in [0, input_frame::EDGE_DASH] {
                let message = forged_bundle(ForgedBundleRequest {
                    session_id: &session_id,
                    manifest_id: &freeze.manifest_id,
                    sender_id: &sender_id,
                    slot_index,
                    first_input_tick: freeze.first_input_tick,
                    sequence: FORGED_SEQUENCE,
                    transport_tick,
                    edges,
                });
                let _ = harness.clients[1].send_raw(
                    fault_harness::HOST_PEER_ID,
                    TransportChannel::Input,
                    message,
                );
            }
        }
        FaultInjectionKind::MalformedInput => {
            let _ = harness.clients[1].send_raw(
                fault_harness::HOST_PEER_ID,
                TransportChannel::Input,
                TransportMessage {
                    version: 1,
                    kind: TransportMessageType::Input,
                    seq: FORGED_SEQUENCE,
                    tick: Some(transport_tick),
                    payload: b"not-an-input-packet".to_vec(),
                },
            );
        }
        _ => {}
    }
}

/// Inputs to [`run`], beyond what the scenario itself declares. Mirrors the
/// Lua `run`'s second parameter, minus `topology`: only the star topology is
/// offered (see [`fault_harness::FaultHarnessOptions`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct RunOptions {
    /// Overrides the scenario's own duration; itself overridden by a
    /// duration the scenario declares.
    pub duration_ticks: Option<i64>,
    /// Impairment seed override; never the match seed.
    pub network_seed: Option<f64>,
}

/// Run one declared row end to end and report on it. Mirrors
/// `fault_scenarios.run`.
///
/// # Panics
///
/// Panics if the session never reaches its frozen start boundary — a
/// harness-construction defect, never expected for a declared [`SCENARIOS`]
/// row.
pub fn run(scenario: &FaultScenario, options: RunOptions) -> fault_harness::FaultHarnessReport {
    let duration = scenario
        .duration_ticks
        .or(options.duration_ticks)
        .unwrap_or(SMOKE_DURATION_TICKS);
    let mut harness = FaultHarness::new(FaultHarnessOptions {
        mode: Some(scenario.mode),
        humans: scenario.humans,
        profile: Some(scenario.profile),
        seed: None,
        network_seed: options.network_seed,
        duration_ticks: Some(duration),
        hash_interval_ticks: scenario.hash_interval_ticks,
        poll_order: matches!(scenario.injection, FaultInjectionKind::PollReversed)
            .then_some(FaultTransportPollOrder::Reverse),
        duplicate_control_every: matches!(
            scenario.injection,
            FaultInjectionKind::ControlDuplication
        )
        .then_some(3),
        buffered_amount_limit: matches!(scenario.injection, FaultInjectionKind::Backpressure)
            .then_some(BACKPRESSURE_LIMIT_BYTES),
    });
    assert!(
        harness.reach_start(None, None),
        "the session never reached its start boundary"
    );
    harness.start_match();

    if matches!(scenario.injection, FaultInjectionKind::BurstAcrossFullTime) {
        // Full time lands at driver step `duration`; the transport clock
        // already advanced through the pre-match control rounds, so the
        // window is expressed against the harness clock. The hold is the
        // profile's own burst length (three ticks), placed on purpose
        // across the boundary -- a probabilistic roll cannot be aimed at a
        // named tick.
        let full_time = harness.transport_tick + duration;
        for client in &harness.clients {
            client.hold(full_time - 1, full_time + 1);
        }
    }

    let at = scenario.at_step;
    for _ in 0..MAX_STEPS {
        if at == Some(harness.step) {
            inject(&mut harness, scenario);
        }
        harness.advance();
        if harness.finished() {
            break;
        }
    }
    harness.teardown();
    let mut report = harness.report(scenario.expect_status.is_none());
    report.notes.push(format!(
        "scenario {} injection={:?} duration_ticks={duration}",
        scenario.id, scenario.injection
    ));
    if let Some(expected) = scenario.expect_status {
        let reached = harness
            .clients
            .iter()
            .any(|client| client.driver.as_ref().map(match_driver::status) == Some(expected));
        report.findings.push(fault_harness::FaultHarnessFinding {
            id: format!("terminal.{}", fault_harness::status_label(Some(expected))),
            ok: reached,
            skipped: false,
            detail: format!(
                "at least one peer reached the declared terminal {}",
                fault_harness::status_label(Some(expected))
            ),
        });
        report.ok = report.ok && reached;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_short(id: &str) -> fault_harness::FaultHarnessReport {
        let scenario = find(id).unwrap_or_else(|| panic!("unknown scenario {id}"));
        run(
            scenario,
            RunOptions {
                duration_ticks: Some(60),
                network_seed: None,
            },
        )
    }

    /// The whole declared matrix, at a short duration, run for real: every
    /// row reaches its declared outcome (a clean convergence, or its
    /// declared terminal — [`run`] appends the `terminal.*` finding itself).
    /// This is deliberately broader than the bounded CI subset
    /// `crates/gc-netcode/tests/fault_harness.rs` exercises — a regression
    /// anywhere in the matrix, not just the `smoke` rows, fails here.
    #[test]
    fn every_declared_row_reaches_its_declared_outcome() {
        for scenario in SCENARIOS {
            let report = run_short(scenario.id);
            let failed: Vec<&str> = report
                .findings
                .iter()
                .filter(|finding| !finding.ok)
                .map(|finding| finding.id.as_str())
                .collect();
            assert!(
                report.ok,
                "{} did not converge on its declared outcome; failing findings: {failed:?}",
                scenario.id
            );
        }
    }

    #[test]
    fn every_scenario_id_is_unique() {
        let mut ids: Vec<&str> = SCENARIOS.iter().map(|s| s.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate scenario id");
    }

    #[test]
    fn select_smoke_only_returns_a_strict_subset() {
        let all = select(false);
        let smoke = select(true);
        assert_eq!(all.len(), SCENARIOS.len());
        assert!(smoke.len() < all.len());
        assert!(smoke.iter().all(|s| s.smoke));
    }

    #[test]
    fn find_looks_up_by_id() {
        assert!(find("1v1.clean").is_some());
        assert!(find("nonexistent.row").is_none());
    }

    #[test]
    fn every_terminal_row_declares_its_expected_status() {
        for scenario in SCENARIOS {
            let terminal_injection = !matches!(
                scenario.injection,
                FaultInjectionKind::None
                    | FaultInjectionKind::PollReversed
                    | FaultInjectionKind::ControlDuplication
                    | FaultInjectionKind::Backpressure
                    | FaultInjectionKind::BurstAcrossFullTime
            );
            assert_eq!(
                scenario.expect_status.is_some(),
                terminal_injection,
                "row {} disagrees on whether it declares a terminal",
                scenario.id
            );
        }
    }

    #[test]
    fn forged_bundle_encodes_and_decodes_back_to_the_same_rows() {
        let message = forged_bundle(ForgedBundleRequest {
            session_id: "session-1",
            manifest_id: "0123456789abcdef",
            sender_id: "guest_1",
            slot_index: 5,
            first_input_tick: 0,
            sequence: 9000,
            transport_tick: 3,
            edges: 0,
        });
        let context = input_protocol::DecodeContext {
            session_id: "session-1".to_string(),
            manifest_id: "0123456789abcdef".to_string(),
            sender_id: "guest_1".to_string(),
        };
        let packet =
            input_protocol::decode(&message.payload, &context).expect("forged bundle decodes");
        assert_eq!(
            packet.rows.len(),
            (input_protocol::HISTORY_ROWS + 1) as usize
        );
        assert_eq!(packet.rows.last().unwrap().slot_index, 5);
    }

    #[test]
    fn foreign_slot_index_is_never_in_the_guests_owned_set() {
        use crate::match_driver::{ProducerKind, SlotAssignment};
        use gc_sim::input_frame::SlotId;
        use indexmap::IndexMap;

        let assignments: [SlotAssignment; 8] = std::array::from_fn(|i| SlotAssignment {
            producer_kind: if i == 0 {
                ProducerKind::Peer
            } else {
                ProducerKind::Bot
            },
            producer_id: if i == 0 {
                "guest_1".to_string()
            } else {
                "bot".to_string()
            },
            bot_seed: None,
        });
        let mut owned = IndexMap::new();
        owned.insert("guest_1".to_string(), vec![SlotId::Home1]);
        let freeze = CoordinatorFreeze {
            manifest_id: "m".to_string(),
            first_input_tick: 0,
            seed: 1,
            assignments,
            owned,
            live: IndexMap::new(),
        };
        let foreign = foreign_slot_index(&freeze, "guest_1");
        assert_ne!(slot_id_of(foreign), SlotId::Home1);
    }
}
