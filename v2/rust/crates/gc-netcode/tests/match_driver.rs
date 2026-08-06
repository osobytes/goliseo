//! Port of `spec/game/online_match_driver_spec.lua`.
//!
//! ## `fixture.session` is unblocked
//!
//! Every one of the spec's 53 assertions (46 static `t.it` cases plus one
//! `t.it` inside a loop over `combat_phases.PHASES`'s 7 named phases) is
//! built through the spec's own `harness(mode, options)` helper
//! (`spec/game/online_match_driver_spec.lua:34-82`), which calls
//! `fixture.session(mode, ...)`. That, in turn, needed `coordinator.lua`
//! (`plan_assignments`, `slot_sources`), `protocol.lua` (`match_mode`,
//! `owned_slots`, `manifest_id`, `assignment_id`), `protocol_fixture.lua`,
//! and `game/transport/fake_star.lua` — all landed now
//! (`crate::coordinator`, `crate::protocol`, `crate::protocol_fixture`,
//! `crate::fake_star`), and `crate::match_driver_fixture::session` builds a
//! real connected host+guest session.
//!
//! `match_driver_differential_matches_the_lua_reference` is the highest-value
//! case this unblocks: it builds the exact `fixture.session("1v1")` scenario
//! the committed reference trace
//! (`tests/fixtures/match_driver_lua_reference.txt`) was captured from —
//! bursty delivery (the star pumps every 5th step only), 90 steps, neutral
//! samples — and asserts `diagnostics()`/`checkpoints()` agree with the real
//! Lua `match_driver.lua` tick for tick, including the boundary hash at every
//! one of the 9 checkpoints (10, 20, ..., 90).
//!
//! `mod online_match_driver` ports 15 more of the spec's static cases for
//! real, using this file's own `harness`/`advance`/`run`/`assert_agreement`/
//! `assert_confirmed_state` (mirroring the spec's own helpers at
//! `spec/game/online_match_driver_spec.lua:34-172`). The remaining ~38 cases
//! in `spec/game/online_match_driver_spec.lua` still need individual porting
//! from the spec file — mostly impaired-delivery/fault-injection scenarios
//! (`wrap_host_transport`/`wrap_guest_transport`), the settle phase, and the
//! combat-phase loop; see this crate's report for what is and is not covered
//! here yet.

use gc_netcode::coordinator;
use gc_netcode::fake_star;
use gc_netcode::fault_transport::{
    StarTransportAdapter, TransportChannel, TransportMessage, TransportMessageType,
};
use gc_netcode::input_protocol;
use gc_netcode::match_driver::{
    self, CoordinatorNetcodeFailure, DriverRole, MatchDriverOptions, MatchDriverStatus,
};
use gc_netcode::match_driver_fixture::{self, DriverRules, MatchDriverFixtureSession};
use gc_netcode::protocol::{self, MatchMode, Value};
use gc_sim::input_frame::{self, InputSample};
use gc_sim::match_snapshot::MatchSnapshot;

// ---------------------------------------------------------------------------
// Port of `spec/game/online_match_driver_spec.lua`'s own `harness`/`advance`/
// `run`/`assert_agreement` helpers (lines 34-172), now that
// `crate::match_driver_fixture::session` builds a real connected session.
// ---------------------------------------------------------------------------

/// Mirrors `DriverHarness`.
struct DriverHarness {
    session: MatchDriverFixtureSession,
    /// Host first, then guests in seating order — matches `session.guest_peer_ids`.
    drivers: Vec<match_driver::MatchDriver>,
    step: i64,
}

/// Mirrors `DriverHarnessOptions` (the subset this file's ported cases use).
#[derive(Default)]
struct DriverHarnessOptions {
    duration: Option<f64>,
    humans: Option<i64>,
    combat: bool,
    hash_interval_ticks: Option<i64>,
    max_rollback_ticks: Option<i64>,
}

fn build_driver(
    session: &MatchDriverFixtureSession,
    role: DriverRole,
    peer_id: &str,
    transport: Box<dyn StarTransportAdapter>,
    snapshot: &MatchSnapshot,
    options: &DriverHarnessOptions,
) -> match_driver::MatchDriver {
    match_driver::new(MatchDriverOptions {
        role,
        peer_id: peer_id.to_string(),
        freeze: match_driver_fixture::to_driver_freeze(&session.freeze),
        manifest: match_driver_fixture::to_driver_manifest(&session.manifest),
        transport,
        initial_snapshot: snapshot.clone(),
        max_rollback_ticks: options.max_rollback_ticks,
        hash_interval_ticks: options.hash_interval_ticks,
        settle_timeout_ticks: None,
        settle_timeout_seconds: None,
        clock: None,
        rules: Box::new(DriverRules::new(
            session.manifest.clone(),
            session.freeze.clone(),
        )),
    })
}

/// Mirrors the spec's `harness(mode, options)`.
fn harness(mode: MatchMode, options: DriverHarnessOptions) -> DriverHarness {
    let session = match_driver_fixture::session(mode, None, options.humans);
    let snapshot = match_driver_fixture::initial_snapshot(options.duration, options.combat, None);

    let mut drivers = Vec::new();
    drivers.push(build_driver(
        &session,
        DriverRole::Host,
        &session.host_peer_id,
        Box::new(session.host_transport.clone()),
        &snapshot,
        &options,
    ));
    for peer_id in &session.guest_peer_ids {
        let transport = session
            .guest_transports
            .get(peer_id)
            .expect("session built a transport for every seated guest")
            .clone();
        drivers.push(build_driver(
            &session,
            DriverRole::Guest,
            peer_id,
            Box::new(transport),
            &snapshot,
            &options,
        ));
    }
    DriverHarness {
        session,
        drivers,
        step: 0,
    }
}

/// Mirrors the spec's `advance(harness_state, samples)`. `samples[i]` is the
/// sample for `drivers[i]`; `None`/short entries default to neutral, exactly
/// like the Lua `samples and samples[index] or input_frame.neutral_sample()`.
fn advance(
    h: &mut DriverHarness,
    samples: &[Option<InputSample>],
) -> Vec<match_driver::MatchDriverBatch> {
    let mut batches = Vec::with_capacity(h.drivers.len());
    for (index, driver) in h.drivers.iter_mut().enumerate() {
        let sample = samples.get(index).copied().flatten();
        batches.push(match_driver::advance(driver, sample));
    }
    h.session.host_transport.pump();
    h.step += 1;
    batches
}

/// Mirrors the spec's `run(harness_state, steps, samples)`.
fn run(
    h: &mut DriverHarness,
    steps: i64,
    samples: &[Option<InputSample>],
) -> Vec<match_driver::MatchDriverBatch> {
    let mut last = Vec::new();
    for _ in 0..steps {
        last = advance(h, samples);
    }
    last
}

/// Mirrors the spec's `assert_agreement(harness_state)`: every checkpoint two
/// drivers both hashed must agree, boundary hash and live-slot map alike.
fn assert_agreement(h: &DriverHarness) -> i64 {
    let reference = match_driver::checkpoints(&h.drivers[0]);
    let mut compared = 0i64;
    for checkpoint in &reference {
        for driver in &h.drivers[1..] {
            let mine = match_driver::checkpoints(driver)
                .into_iter()
                .find(|c| c.tick == checkpoint.tick);
            if let Some(mine) = mine {
                assert_eq!(
                    mine.hash, checkpoint.hash,
                    "boundary hash at {}",
                    checkpoint.tick
                );
                for (producer_id, slot) in &checkpoint.live {
                    assert_eq!(
                        mine.live.get(producer_id),
                        Some(slot),
                        "live slot for {producer_id} at {}",
                        checkpoint.tick
                    );
                }
                compared += 1;
            }
        }
    }
    compared
}

/// Mirrors the spec's `assert_confirmed_state(harness_state)`: the confirmed
/// (not merely present) boundary is where every peer must agree on state,
/// since that is where authority is complete rather than predicted.
fn assert_confirmed_state(h: &DriverHarness) {
    let mut boundary: Option<i64> = None;
    for driver in &h.drivers {
        let confirmed = match_driver::diagnostics(driver).confirmed_output_tick;
        boundary = Some(boundary.map_or(confirmed, |b| b.min(confirmed)));
    }
    let boundary = boundary.expect("at least one driver") + 1;
    assert!(boundary > 0, "no peer confirmed a single boundary");
    let mut reference: Option<String> = None;
    for driver in &h.drivers {
        use gc_sim::rollback_snapshot_history::RollbackSnapshotLookupStatus as Status;
        let lookup = match_driver::snapshot(driver, boundary);
        assert!(matches!(lookup.status, Status::Present | Status::Retained));
        let snapshot = lookup
            .snapshot
            .expect("present/retained lookup carries a snapshot");
        let hash = gc_sim::match_snapshot::hash(&snapshot);
        if let Some(reference) = &reference {
            assert_eq!(&hash, reference, "confirmed boundary {boundary}");
        } else {
            reference = Some(hash);
        }
    }
}

fn wire(slot: input_frame::SlotId) -> &'static str {
    protocol::slot_wire_id(slot)
}

fn switch_sample() -> InputSample {
    input_frame::new_sample(input_frame::InputSampleOptions {
        edges: Some(input_frame::EDGE_SWITCH),
        ..Default::default()
    })
    .expect("a switch-only sample is always valid")
}

mod online_match_driver {
    use super::*;

    #[test]
    fn seats_a_4v4_host_on_one_slot_and_authors_every_bot_fill() {
        let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
        let host = match_driver::diagnostics(&state.drivers[0]);
        assert_eq!(host.role, DriverRole::Host);
        assert_eq!(host.owned.len(), 1);
        assert_eq!(wire(host.owned[0]), "home_1");
        assert_eq!(host.control_slot.map(wire), Some("home_1"));
        // Eight humans in 4v4, so the host authors only its own slot.
        assert_eq!(host.authored.len(), 1);
        let guest = match_driver::diagnostics(&state.drivers[1]);
        assert_eq!(guest.owned.len(), 1);
        assert_eq!(wire(guest.owned[0]), "home_2");
        let _ = &mut state;
    }

    #[test]
    fn gives_a_1v1_human_four_owned_slots_and_one_control_slot() {
        let state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        let host = match_driver::diagnostics(&state.drivers[0]);
        assert_eq!(host.owned.len(), 4);
        assert_eq!(
            host.owned
                .iter()
                .map(|&s| wire(s))
                .collect::<Vec<_>>()
                .join(","),
            "home_1,home_2,home_3,home_4"
        );
        assert_eq!(host.control_slot.map(wire), Some("home_1"));
        // No bot fills at all in 1v1: every slot belongs to one of the two
        // humans, so the host authors exactly its own four.
        assert_eq!(host.authored.len(), 4);
        let live = &state.drivers[0].live[&state.drivers[0].live_tick];
        let drivers = coordinator::slot_drivers(&state.session.freeze, Some(live));
        let index_of =
            |wire_id: &str| protocol::slot_index(wire_id).expect("canonical wire slot id");
        assert_eq!(
            drivers[(index_of("home_1") - 1) as usize],
            coordinator::SlotDriver::Human
        );
        assert_eq!(
            drivers[(index_of("home_2") - 1) as usize],
            coordinator::SlotDriver::Ai
        );
        assert_eq!(
            drivers[(index_of("away_1") - 1) as usize],
            coordinator::SlotDriver::Human
        );
        assert_eq!(
            drivers[(index_of("away_2") - 1) as usize],
            coordinator::SlotDriver::Ai
        );
    }

    #[test]
    fn makes_the_host_author_every_declared_bot_fill_in_a_short_lobby() {
        // A full lobby covers all eight slots in every mode, so a declared bot
        // fill only exists when fewer humans are seated than the mode allows.
        let mut state = harness(
            MatchMode::TwoVTwo,
            DriverHarnessOptions {
                humans: Some(2),
                ..Default::default()
            },
        );
        let mut bots = 0;
        for index in 1..=input_frame::SLOT_COUNT {
            let producer = state
                .session
                .freeze
                .assignments
                .get_index(index)
                .expect("canonical assignment slot");
            if producer
                .get("producer_kind")
                .and_then(protocol::Value::as_str)
                == Some("bot")
            {
                bots += 1;
            }
        }
        assert_eq!(bots, 4);
        let host = match_driver::diagnostics(&state.drivers[0]);
        assert_eq!(host.owned.len(), 2);
        // Its own two slots plus all four bots, every one of them carried by
        // the host's own delayed collector path.
        assert_eq!(host.authored.len(), 6);
        let guest = match_driver::diagnostics(&state.drivers[1]);
        assert_eq!(guest.authored.len(), 2);

        run(&mut state, 24, &[]);
        assert!(assert_agreement(&state) > 0);
        assert_confirmed_state(&state);
        // Declared fills are indistinguishable from a human's non-live owned
        // slots in the stream: every slot is authoritative at a confirmed tick.
        let live = &state.drivers[0].live[&state.drivers[0].live_tick];
        let drivers = coordinator::slot_drivers(&state.session.freeze, Some(live));
        let humans = drivers
            .iter()
            .filter(|&&d| d == coordinator::SlotDriver::Human)
            .count();
        assert_eq!(humans, 2);
    }

    #[test]
    fn converges_every_peer_on_the_same_confirmed_boundary_hashes_in_4v4() {
        let mut state = harness(MatchMode::FourVFour, DriverHarnessOptions::default());
        run(&mut state, 40, &[]);
        assert!(assert_agreement(&state) > 0);
        for driver in &state.drivers {
            assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
            let diagnostics = match_driver::diagnostics(driver);
            assert_eq!(diagnostics.present_input_tick, 40);
            assert!(diagnostics.confirmed_input_tick >= 30);
        }
        assert_confirmed_state(&state);
    }

    #[test]
    fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_1v1() {
        let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        // The host presses switch continuously: liveness must move identically
        // on both peers, and 1v1 is where a divergence can actually appear.
        run(&mut state, 48, &[Some(switch_sample())]);
        assert!(assert_agreement(&state) > 0);
        for driver in &state.drivers {
            assert_eq!(match_driver::status(driver), MatchDriverStatus::Active);
        }
        assert_confirmed_state(&state);
    }

    #[test]
    fn agrees_on_the_live_slot_at_every_confirmed_checkpoint_in_2v2() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(
            &mut state,
            48,
            &[Some(switch_sample()), None, Some(switch_sample())],
        );
        assert!(assert_agreement(&state) > 0);
        assert_confirmed_state(&state);
    }

    #[test]
    fn moves_the_control_slot_only_through_the_canonical_stream() {
        let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        let host_peer_id = state.session.host_peer_id.clone();
        assert_eq!(
            wire(match_driver::control_slot(
                &state.drivers[0],
                &host_peer_id,
                0
            )),
            "home_1"
        );
        run(&mut state, 24, &[Some(switch_sample())]);
        let mut moved = false;
        for tick in 1..=20 {
            if match_driver::control_slot(&state.drivers[0], &host_peer_id, tick)
                != input_frame::SlotId::Home1
            {
                moved = true;
            }
        }
        assert!(moved, "a held switch never moved the control slot");
        // The guest, which never saw the keypress, reaches the same conclusion
        // purely from the canonical rows it received.
        for tick in 0..=20 {
            assert_eq!(
                match_driver::control_slot(&state.drivers[1], &host_peer_id, tick),
                match_driver::control_slot(&state.drivers[0], &host_peer_id, tick)
            );
        }
    }

    #[test]
    fn applies_one_transport_tick_arrival_batch_as_one_reconciliation() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        for _ in 0..24 {
            let batches = advance(&mut state, &[]);
            for batch in &batches {
                assert!(
                    batch.reconciliations <= 1,
                    "more than one reconciliation in one step"
                );
            }
        }
    }

    #[test]
    fn is_insensitive_to_the_order_peers_are_polled_and_stepped() {
        let mut forward = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut forward, 32, &[]);
        let mut reversed = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        for _ in 0..32 {
            for driver in reversed.drivers.iter_mut().rev() {
                let _ = match_driver::advance(driver, None);
            }
            reversed.session.host_transport.pump();
        }
        for index in 0..forward.drivers.len() {
            let boundary = match_driver::diagnostics(&forward.drivers[index]).confirmed_output_tick;
            assert_eq!(
                match_driver::diagnostics(&reversed.drivers[index]).confirmed_output_tick,
                boundary
            );
            let reversed_snapshot = match_driver::snapshot(&reversed.drivers[index], boundary)
                .snapshot
                .expect("boundary is retained");
            let forward_snapshot = match_driver::snapshot(&forward.drivers[index], boundary)
                .snapshot
                .expect("boundary is retained");
            assert_eq!(
                gc_sim::match_snapshot::hash(&reversed_snapshot),
                gc_sim::match_snapshot::hash(&forward_snapshot)
            );
        }
    }

    #[test]
    fn keeps_protected_keepers_ai_only_and_slotless() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 8, &[]);
        let snapshot = match_driver::current_snapshot(&state.drivers[0]);
        for index in 1..=input_frame::SLOT_COUNT {
            let player_index = snapshot.state.slot_players[(index - 1) as usize]
                .expect("canonical slot is mapped");
            assert!(!snapshot.state.players[(player_index - 1) as usize].is_keeper);
        }
    }

    #[test]
    fn accepts_every_slot_inside_a_peers_frozen_owned_set() {
        let mut state = harness(MatchMode::OneVOne, DriverHarnessOptions::default());
        run(&mut state, 16, &[]);
        // A 1v1 guest legitimately authors four different slots every tick.
        let diagnostics = match_driver::diagnostics(&state.drivers[1]);
        assert_eq!(diagnostics.authored.len(), 4);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Active
        );
        assert_eq!(
            match_driver::status(&state.drivers[1]),
            MatchDriverStatus::Active
        );
    }

    #[test]
    fn stops_all_progress_after_a_terminal_status() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 6, &[]);
        let driver = &mut state.drivers[1];
        let before = match_driver::diagnostics(driver);
        for _ in 0..5 {
            assert!(!match_driver::observe_checkpoint(
                driver,
                0,
                "0000000000000000"
            ));
        }
        assert_eq!(
            match_driver::status(driver),
            MatchDriverStatus::HashMismatch
        );
        let batch = match_driver::advance(driver, None);
        assert_eq!(batch.outputs.len(), 0);
        assert_eq!(batch.sent_packets, 0);
        assert_eq!(batch.status, MatchDriverStatus::HashMismatch);
        let after = match_driver::diagnostics(driver);
        assert_eq!(after.present_input_tick, before.present_input_tick);
        assert_eq!(after.confirmed_input_tick, before.confirmed_input_tick);
    }

    #[test]
    fn hands_control_channel_traffic_back_instead_of_eating_it() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 4, &[]);
        let guest_id = state.session.guest_peer_ids[0].clone();
        let mut guest_transport = state
            .session
            .guest_transports
            .get(&guest_id)
            .expect("session built this guest's transport")
            .clone();
        let control = TransportMessage {
            version: 1,
            kind: TransportMessageType::State,
            seq: 3,
            tick: None,
            payload: b"GCOP;control".to_vec(),
        };
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Control, control)
            .expect("a canonical control envelope always sends");
        state.session.host_transport.pump();
        let batches = advance(&mut state, &[]);
        assert_eq!(batches[0].control.len(), 1);
        assert_eq!(batches[0].control[0].channel, TransportChannel::Control);
        assert_eq!(batches[0].control[0].message.payload, b"GCOP;control");
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Active
        );
    }

    #[test]
    fn refuses_authority_from_outside_a_peers_frozen_owned_set() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 4, &[]);
        let guest_id = state.session.guest_peer_ids[0].clone();
        let mut guest_transport = state
            .session
            .guest_transports
            .get(&guest_id)
            .expect("session built this guest's transport")
            .clone();
        // The guest owns home_3/home_4; away_1 (canonical slot index 5)
        // belongs to another human.
        let mut rows = Vec::new();
        for tick in 0..=6 {
            rows.push(input_protocol::AuthorityRow {
                tick,
                slot_index: 5,
                sample: input_frame::neutral_sample(),
            });
        }
        let session_id = state
            .session
            .manifest
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string();
        let forged = input_protocol::new_guest(input_protocol::PacketOptions {
            session_id,
            manifest_id: protocol::manifest_id(&state.session.manifest),
            sender_id: guest_id.clone(),
            sequence: 9000,
            transport_tick: 8,
            first_input_tick: 0,
            confirmed_span: None,
            rows,
        })
        .expect("a forged guest packet is well-formed");
        let wire = input_protocol::encode(&forged).expect("a well-formed packet encodes");
        let envelope = TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq: forged.sequence,
            tick: Some(forged.transport_tick),
            payload: wire,
        };
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Input, envelope)
            .expect("a canonical input envelope always sends");
        state.session.host_transport.pump();
        let _ = advance(&mut state, &[]);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::OwnershipViolation
        );
        let terminal =
            match_driver::terminal(&state.drivers[0]).expect("driver reached a terminal");
        assert_eq!(
            terminal.failure,
            Some(CoordinatorNetcodeFailure::InputChannel)
        );
    }

    #[test]
    fn is_idempotent_for_a_replayed_bundle_and_terminal_for_a_conflicting_one() {
        let mut state = harness(MatchMode::TwoVTwo, DriverHarnessOptions::default());
        run(&mut state, 6, &[]);
        let guest_id = state.session.guest_peer_ids[0].clone();
        let mut guest_transport = state
            .session
            .guest_transports
            .get(&guest_id)
            .expect("session built this guest's transport")
            .clone();
        let owned_slot = match_driver::slot_index_of(state.session.freeze.owned[&guest_id][0]);
        let session_id = state
            .session
            .manifest
            .get("session_id")
            .and_then(Value::as_str)
            .expect("fixture manifest has a session id")
            .to_string();
        let manifest_id = protocol::manifest_id(&state.session.manifest);

        let bundle = |edges: i64| -> TransportMessage {
            let mut rows = Vec::new();
            for tick in 0..=6 {
                let sample = input_frame::new_sample(input_frame::InputSampleOptions {
                    edges: Some(if tick == 6 { edges } else { 0 }),
                    ..Default::default()
                })
                .expect("a neutral-plus-edges sample is always valid");
                rows.push(input_protocol::AuthorityRow {
                    tick,
                    slot_index: owned_slot,
                    sample,
                });
            }
            let packet = input_protocol::new_guest(input_protocol::PacketOptions {
                session_id: session_id.clone(),
                manifest_id: manifest_id.clone(),
                sender_id: guest_id.clone(),
                sequence: 4242,
                transport_tick: 9,
                first_input_tick: 0,
                confirmed_span: None,
                rows,
            })
            .expect("a well-formed guest packet");
            let wire = input_protocol::encode(&packet).expect("a well-formed packet encodes");
            TransportMessage {
                version: 1,
                kind: TransportMessageType::Input,
                seq: packet.sequence,
                tick: Some(packet.transport_tick),
                payload: wire,
            }
        };

        // The same sender sequence with byte-identical authority is a no-op.
        let repeated = bundle(0);
        guest_transport
            .send(
                fake_star::HOST_PEER_ID,
                TransportChannel::Input,
                repeated.clone(),
            )
            .expect("send");
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Input, repeated)
            .expect("send");
        state.session.host_transport.pump();
        let _ = advance(&mut state, &[]);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::Active
        );

        // The same identity with different bytes is not.
        guest_transport
            .send(fake_star::HOST_PEER_ID, TransportChannel::Input, bundle(0))
            .expect("send");
        guest_transport
            .send(
                fake_star::HOST_PEER_ID,
                TransportChannel::Input,
                bundle(input_frame::EDGE_DASH),
            )
            .expect("send");
        state.session.host_transport.pump();
        let _ = advance(&mut state, &[]);
        assert_eq!(
            match_driver::status(&state.drivers[0]),
            MatchDriverStatus::AuthorityConflict
        );
        assert_eq!(
            match_driver::terminal(&state.drivers[0])
                .expect("driver reached a terminal")
                .failure,
            Some(CoordinatorNetcodeFailure::InputChannel)
        );
    }
}

fn status_label(status: MatchDriverStatus) -> &'static str {
    match status {
        MatchDriverStatus::Active => "active",
        MatchDriverStatus::Completed => "completed",
        MatchDriverStatus::SettleTimeout => "settle_timeout",
        MatchDriverStatus::ConfirmationStalled => "confirmation_stalled",
        MatchDriverStatus::LateInput => "late_input",
        MatchDriverStatus::HashMismatch => "hash_mismatch",
        MatchDriverStatus::OwnershipViolation => "ownership_violation",
        MatchDriverStatus::AuthorityConflict => "authority_conflict",
        MatchDriverStatus::InputChannelFailure => "input_channel_failure",
        MatchDriverStatus::TransportLost => "transport_lost",
    }
}

struct ExpectedStep {
    step: i64,
    peer: String,
    status: String,
    present: i64,
    confirmed_in: i64,
    confirmed_out: i64,
    rollback: i64,
    correction: i64,
}

struct ExpectedHash {
    step: i64,
    peer: String,
    tick: i64,
    digest: String,
}

fn parse_fixture(text: &str) -> (Vec<ExpectedStep>, Vec<ExpectedHash>) {
    let mut steps = Vec::new();
    let mut hashes = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields[0] == "hash" {
            hashes.push(ExpectedHash {
                step: fields[1].parse().expect("fixture hash step is an integer"),
                peer: fields[2].to_string(),
                tick: fields[3].parse().expect("fixture hash tick is an integer"),
                digest: fields[4].to_string(),
            });
        } else {
            steps.push(ExpectedStep {
                step: fields[0].parse().expect("fixture step is an integer"),
                peer: fields[1].to_string(),
                status: fields[2].to_string(),
                present: fields[3]
                    .parse()
                    .expect("fixture present tick is an integer"),
                confirmed_in: fields[4]
                    .parse()
                    .expect("fixture confirmed_in tick is an integer"),
                confirmed_out: fields[5]
                    .parse()
                    .expect("fixture confirmed_out tick is an integer"),
                rollback: fields[6]
                    .parse()
                    .expect("fixture rollback count is an integer"),
                correction: fields[7]
                    .parse()
                    .expect("fixture correction count is an integer"),
            });
        }
    }
    (steps, hashes)
}

/// See the module doc: reproduces `fixture.session("1v1")` driven through
/// `run_bursty(state, 90, 5)` with neutral samples, and asserts the Rust
/// driver reaches the identical diagnostics and boundary hashes the real Lua
/// `match_driver.lua` reached, tick for tick, at every one of the 90 steps
/// and every one of the 9 checkpoints.
#[test]
fn match_driver_differential_matches_the_lua_reference() {
    const FIXTURE: &str = include_str!("fixtures/match_driver_lua_reference.txt");
    let (expected_steps, expected_hashes) = parse_fixture(FIXTURE);

    let session = match_driver_fixture::session(MatchMode::OneVOne, None, None);
    assert_eq!(
        session.guest_peer_ids.len(),
        1,
        "1v1 seats exactly one guest"
    );
    let guest_peer_id = session.guest_peer_ids[0].clone();
    let guest_transport = session
        .guest_transports
        .get(&guest_peer_id)
        .expect("the guest transport session built")
        .clone();
    let host_transport = session.host_transport.clone();

    let build = |role: DriverRole, peer_id: &str, transport: Box<dyn StarTransportAdapter>| {
        match_driver::new(MatchDriverOptions {
            role,
            peer_id: peer_id.to_string(),
            freeze: match_driver_fixture::to_driver_freeze(&session.freeze),
            manifest: match_driver_fixture::to_driver_manifest(&session.manifest),
            transport,
            initial_snapshot: match_driver_fixture::initial_snapshot(None, false, None),
            max_rollback_ticks: None,
            hash_interval_ticks: Some(10),
            settle_timeout_ticks: None,
            settle_timeout_seconds: None,
            clock: None,
            rules: Box::new(DriverRules::new(
                session.manifest.clone(),
                session.freeze.clone(),
            )),
        })
    };

    let mut host = build(
        DriverRole::Host,
        &session.host_peer_id,
        Box::new(host_transport.clone()),
    );
    let mut guest = build(
        DriverRole::Guest,
        &guest_peer_id,
        Box::new(guest_transport.clone()),
    );

    for step in 1..=90i64 {
        let _ = match_driver::advance(&mut host, None);
        let _ = match_driver::advance(&mut guest, None);
        if step % 5 == 0 {
            host_transport.pump();
        }

        for (driver, label) in [(&host, "host"), (&guest, "guest")] {
            let expected = expected_steps
                .iter()
                .find(|e| e.step == step && e.peer == label)
                .unwrap_or_else(|| panic!("fixture is missing step {step} for {label}"));
            let diagnostics = match_driver::diagnostics(driver);
            assert_eq!(
                status_label(diagnostics.status),
                expected.status,
                "step {step} {label} status"
            );
            assert_eq!(
                diagnostics.present_input_tick, expected.present,
                "step {step} {label} present"
            );
            assert_eq!(
                diagnostics.confirmed_input_tick, expected.confirmed_in,
                "step {step} {label} confirmed_in"
            );
            assert_eq!(
                diagnostics.confirmed_output_tick, expected.confirmed_out,
                "step {step} {label} confirmed_out"
            );
            assert_eq!(
                diagnostics.rollback_count, expected.rollback,
                "step {step} {label} rollback"
            );
            assert_eq!(
                diagnostics.correction_count, expected.correction,
                "step {step} {label} correction"
            );
        }

        if step % 10 == 0 {
            for (driver, label) in [(&host, "host"), (&guest, "guest")] {
                let expected = expected_hashes
                    .iter()
                    .find(|h| h.step == step && h.peer == label)
                    .unwrap_or_else(|| {
                        panic!("fixture is missing hash at step {step} for {label}")
                    });
                let checkpoints = match_driver::checkpoints(driver);
                let newest = checkpoints
                    .last()
                    .unwrap_or_else(|| panic!("{label} has no checkpoint by step {step}"));
                assert_eq!(
                    newest.tick, expected.tick,
                    "step {step} {label} checkpoint tick"
                );
                assert_eq!(
                    newest.hash, expected.digest,
                    "step {step} {label} checkpoint hash"
                );
            }
        }
    }
}

/// Named for the spec's own `harness(mode, options)` helper
/// (`spec/game/online_match_driver_spec.lua:34-172`, ported above as
/// `mod online_match_driver`'s free functions). Every case below still needs
/// individual porting from the spec file; `fixture.session` itself is no
/// longer the blocker (see this file's module doc and `online_match_driver`'s
/// growing coverage).
const BLOCKED: &str = "not yet ported from spec/game/online_match_driver_spec.lua in this pass; \
    crate::match_driver_fixture::session/DriverRules and this file's own harness()/run()/\
    assert_agreement() helpers are ready to build it on -- see mod online_match_driver for the \
    cases already ported this way.";

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn tolerates_one_boundary_disagreement_and_clears_it_on_the_next_agreement() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn publishes_confirmed_boundary_hashes_on_the_documented_interval() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn ends_with_a_typed_completed_status_at_full_time() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn converges_under_impaired_delivery_after_real_corrections() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn converges_in_1v1_under_impaired_delivery_with_live_slot_switching() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn terminates_explicitly_when_authority_falls_outside_the_retained_window() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn publishes_checkpoints_on_the_simulated_ceiling_not_raw_confirmation() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn never_publishes_a_checkpoint_boundary_that_was_not_simulated() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn terminates_on_confirmation_liveness_before_a_below_floor_row_can_arrive() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn agrees_on_the_live_slot_in_2v2_under_impaired_delivery_with_switching() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn reaches_full_time_under_impaired_delivery() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn carries_the_combat_companion_through_correction_and_resimulation() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn opens_every_combat_phase_scenario_from_a_ready_combat_state() {
    unreachable!("{BLOCKED}");
}

/// The spec's `for _, phase_id in ipairs(combat_phases.PHASES) do t.it(...) end`
/// loop (`spec/game/online_match_driver_spec.lua:867`) produces one case per
/// named combat correction phase; `crate::fault_harness::declare_contingent`
/// names the same 7 phases. Ported as 7 distinctly named cases rather than
/// one generic loop, so each is independently countable and independently
/// re-enableable.
mod converges_a_correction_taken_during_each_combat_phase {
    const REASON: &str = "blocked on fixture.session and spec/support/online_combat_phases.lua's \
        fixtures; see the parent module doc";

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn wind_up() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn guard() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn contact() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn projectile_flight() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn stagger() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn ball_spill() {
        unreachable!("{REASON}");
    }

    #[test]
    #[ignore = "blocked on fixture.session; see REASON"]
    fn immunity_expiry() {
        unreachable!("{REASON}");
    }
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn finds_no_driver_level_geometry_where_the_policy_guards_often_enough() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn still_reconciles_if_a_local_insert_ever_reports_a_divergence() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn costs_no_extra_snapshot_work_when_a_peer_authors_only_its_control_slot() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn settles_the_final_boundary_before_completing_under_clean_delivery() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn completes_with_an_agreed_final_hash_under_a_burst_across_full_time() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn does_not_swallow_a_boundary_disagreement_reported_while_settling() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn settles_a_genuinely_divergent_peer_without_hiding_the_divergence() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn ends_a_settle_nobody_can_finish_with_a_bounded_typed_reason() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn bounds_the_settle_phase_in_wall_clock_as_well_as_in_ticks() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn re_publishes_the_tail_while_settling_and_simulates_nothing() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn reports_a_stalled_confirmation_at_the_step_it_becomes_permanent() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn maps_a_rejected_over_window_batch_onto_late_input_unreachable_by_design() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn fans_out_a_full_redundancy_window_for_a_slot_that_sent_nothing() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn keeps_the_host_relaying_until_its_guests_have_stopped_asking() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn keeps_relaying_for_a_peer_that_reported_behind_and_then_went_silent() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn repairs_a_guest_whose_hole_has_aged_out_of_the_redundancy_window() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn keeps_a_guest_re_publishing_until_the_host_has_confirmed_its_tail() {
    unreachable!("{BLOCKED}");
}

#[test]
#[ignore = "not yet ported from the spec; see BLOCKED"]
fn reports_a_lost_transport_as_a_typed_terminal_status() {
    unreachable!("{BLOCKED}");
}
