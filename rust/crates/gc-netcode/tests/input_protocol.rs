//! Input-protocol tests.
//!
//! ## What this file cannot exercise directly
//!
//! Full coverage also touches `game.online.protocol`,
//! `game.online.protocol_fixture`, and `game.transport.contract`.
//! `protocol`/`coordinator`/`match_driver` have since landed in
//! `gc-netcode` (they were out of scope when this file was first written,
//! but are not any more — see `host_fixture`/`packet_arrival` below, which
//! drive the real `match_driver_fixture::DriverRules::canonical_host_batch`
//! for the three cases that need `input_protocol.canonical_host_batch`).
//! `game/transport/contract` is still TypeScript-owned with no Rust
//! implementation planned (`ARCHITECTURE.md` §1), so
//! `input_protocol::validate_envelope` (which needs its `TransportMessage`
//! type) stays unreachable — one `#[ignore]` below still names that.
//! Assertions that only needed *a* valid session/manifest id string (not
//! `protocol`'s specific hashing logic) use the same literal identity
//! `input_protocol_fixture` uses — see that module's doc comment.

use gc_netcode::coordinator;
use gc_netcode::fault_transport::{TransportMessage, TransportMessageType};
use gc_netcode::input_protocol::{self, AuthorityRow, DecodeContext, ErrorCode, PacketOptions};
use gc_netcode::input_protocol_conformance as conformance;
use gc_netcode::input_protocol_fixture as fixture;
use gc_netcode::match_driver::{
    HostBatchErrorCode, HostBatchRequest, InputPacketArrival, MatchDriverRules, ProducerKind,
    SessionManifest as DriverSessionManifest, SlotAssignment,
};
use gc_netcode::match_driver_fixture::DriverRules;
use gc_netcode::protocol::{self, Value};
use gc_netcode::protocol_fixture;
use gc_sim::input_frame::{self, InputSampleOptions};

/// Session identity shared with `input_protocol_fixture` — see that module's
/// doc comment for why this is a literal rather than a call into
/// `protocol_fixture` (out of scope; see this file's module doc comment).
const SESSION_ID: &str = "session_alpha";
// #489: moved with `input_protocol_fixture::MANIFEST_ID` -- see that
// constant's doc comment for why this tracks `match_snapshot::COMBAT_VERSION`
// despite `input_protocol` not depending on `protocol`.
const MANIFEST_ID: &str = "572bbff19cdfc603";

const HELD_BITS: [i64; 8] = [
    input_frame::HELD_SHOOT,
    input_frame::HELD_PASS,
    input_frame::HELD_SPRINT,
    input_frame::HELD_JOCKEY,
    input_frame::HELD_LOB,
    input_frame::HELD_AERIAL_STRIKE,
    input_frame::HELD_AERIAL_ACROBATIC,
    input_frame::HELD_EQUIPMENT,
];

const EDGE_BITS: [i64; 7] = [
    input_frame::EDGE_SHOOT,
    input_frame::EDGE_PASS,
    input_frame::EDGE_SWITCH,
    input_frame::EDGE_DASH,
    input_frame::EDGE_DODGE,
    input_frame::EDGE_EQUIPMENT_PRESSED,
    input_frame::EDGE_EQUIPMENT_RELEASED,
];

fn row(tick: i64, slot_index: i64, sample: input_frame::InputSample) -> AuthorityRow {
    AuthorityRow {
        tick,
        slot_index,
        sample,
    }
}

/// `value * 73 % 256` and friends are always non-negative here (every call
/// site feeds a non-negative tick/offset sum, see `guest_packet` and the
/// `0..=1023` fuzz loop below), so plain `%` agrees with the reference
/// implementation's floored `%` — no `rem_euclid` needed. See
/// `tools/lua_reference/README.md`'s trap note.
fn fuzz_sample(value: i64) -> input_frame::InputSample {
    let mut held = (value * 73) % 256;
    let edges = (value * 41) % 128;
    let equipment = input_frame::HELD_EQUIPMENT;
    let pressed = input_frame::EDGE_EQUIPMENT_PRESSED;
    let released = input_frame::EDGE_EQUIPMENT_RELEASED;
    if (edges & released) != 0 && (held & equipment) != 0 {
        held -= equipment;
    }
    if (edges & pressed) != 0 && (edges & released) == 0 && (held & equipment) == 0 {
        held += equipment;
    }
    input_frame::new_sample(InputSampleOptions {
        move_x: Some((value * 37) % 255 - 127),
        move_y: Some((value * 91) % 255 - 127),
        held: Some(held),
        edges: Some(edges),
    })
    .expect("fuzz sample is always canonical")
}

#[allow(clippy::too_many_arguments)]
fn guest_packet(
    slot_index: i64,
    sender_id: &str,
    sequence: i64,
    transport_tick: i64,
    current_tick: i64,
    first_input_tick: Option<i64>,
    sample_offset: Option<i64>,
) -> input_protocol::Packet {
    let first = first_input_tick.unwrap_or(0);
    let mut rows = Vec::new();
    let start = first.max(current_tick - input_protocol::HISTORY_ROWS);
    for tick in start..=current_tick {
        rows.push(row(
            tick,
            slot_index,
            fuzz_sample(tick + sample_offset.unwrap_or(slot_index)),
        ));
    }
    input_protocol::new_guest(PacketOptions {
        session_id: SESSION_ID.to_string(),
        manifest_id: MANIFEST_ID.to_string(),
        sender_id: sender_id.to_string(),
        sequence,
        transport_tick,
        first_input_tick: first,
        confirmed_span: None,
        rows,
    })
    .expect("guest_packet fixture is always valid")
}

fn decode_context(packet: &input_protocol::Packet) -> DecodeContext {
    DecodeContext {
        session_id: packet.session_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        sender_id: packet.sender_id.clone(),
    }
}

#[test]
fn omp3_pins_literal_native_and_lovejs_conformance_vectors() {
    let report = conformance::verify();
    assert_eq!(report.guest_digest, "a099c86d6520d6bc");
    assert_eq!(report.host_digest, "fb46b26858818ead");
    assert_eq!(report.maximal_wire_bytes, 958);
    assert_eq!(report.maximal_wire_margin, 66);
    assert_eq!(
        conformance::marker(&report),
        "GC_INPUT_PROTOCOL|golden|schema=1|input=2|history=6|delay=3|vectors=2\
|guest=a099c86d6520d6bc|host=fb46b26858818ead|host_rows=72\
|max_bytes=958|margin=66"
    );
}

// `gc_sim::match_snapshot` has since landed (it was the blocker this test
// used to name), so the golden-version identity check is asserted for real
// below. The other half of this scenario — bump `GOLDEN.snapshot_version`
// by one, call `verify`, and expect it to panic naming "input packet
// goldens are stale for snapshot" — is still not exercised here:
// `input_protocol_conformance::GOLDEN` is a Rust `const`, and `verify` reads
// it directly rather than taking one as a parameter (see that module's own
// doc comment: the live staleness cross-check was never implemented in
// `verify`, deliberately, because `match_snapshot` had no Rust
// implementation when that file was written). There is still no seam to
// force a mutated golden through `verify` without changing its signature —
// a `src/` change, out of this pass's scope; worth reopening now that the
// equalities below hold and the module doc's premise for skipping the
// check no longer does.
#[test]
fn names_the_snapshot_version_its_literals_were_generated_against() {
    assert_eq!(
        conformance::GOLDEN.snapshot_version,
        gc_sim::match_snapshot::VERSION
    );
    assert_eq!(
        conformance::GOLDEN.combat_version,
        gc_sim::match_snapshot::COMBAT_VERSION
    );
}

#[test]
fn round_trips_current_plus_exactly_six_prior_guest_rows_with_distinct_clocks() {
    let packet = fixture::guest();
    let wire = input_protocol::encode(&packet).unwrap();
    assert!(wire.len() <= input_protocol::MAX_WIRE_BYTES);
    let decoded = input_protocol::decode(&wire, &decode_context(&packet)).unwrap();
    assert_eq!(input_protocol::encode(&decoded).unwrap(), wire);
    assert_eq!(decoded.transport_tick, 12);
    assert_eq!(decoded.rows[0].tick, 0);
    assert_eq!(decoded.rows[6].tick, 6);
    assert_eq!(decoded.rows[0].slot_index, 2);
    assert_eq!(decoded.input_delay_ticks, 3);
    // Full coverage also asserts
    // `input_protocol.validate_envelope(decoded, envelope(decoded))` here.
    // `input_protocol::validate_envelope` has no `pub` free-function
    // equivalent exposed here — see
    // `validate_envelope_rejects_a_transport_tick_mismatch` below, which
    // exercises the same check indirectly through
    // `DriverRules::canonical_host_batch`, and that test's own comment for
    // the current (non-stale) reason there is no direct standalone call.

    let early = guest_packet(2, "guest_2", 8, 13, 2, Some(2), None);
    assert_eq!(
        early.rows.len(),
        1,
        "the packet does not invent pre-start authority"
    );
    let later = guest_packet(2, "guest_2", 9, 14, 8, Some(2), None);
    assert_eq!(later.rows.len(), 7);
    assert_eq!(later.rows[0].tick, 2);
    assert_eq!(later.rows[6].tick, 8);
}

#[test]
fn covers_every_slot_soccer_combat_bit_axis_boundary_and_generated_valid_sample() {
    let mut sequence = 20;
    for slot_index in 1..=input_frame::SLOT_COUNT {
        for &bit in &HELD_BITS {
            let edges = if bit == input_frame::HELD_EQUIPMENT {
                input_frame::EDGE_EQUIPMENT_PRESSED
            } else {
                0
            };
            let sample = input_frame::new_sample(InputSampleOptions {
                move_x: Some(if slot_index % 2 == 0 { -127 } else { 127 }),
                move_y: Some(if slot_index % 2 == 0 { 127 } else { -127 }),
                held: Some(bit),
                edges: Some(edges),
            })
            .unwrap();
            let packet = input_protocol::new_guest(PacketOptions {
                session_id: SESSION_ID.to_string(),
                manifest_id: MANIFEST_ID.to_string(),
                sender_id: format!("producer_{slot_index}"),
                sequence,
                transport_tick: sequence,
                first_input_tick: 0,
                confirmed_span: None,
                rows: vec![row(0, slot_index, sample)],
            })
            .unwrap();
            let decoded = input_protocol::decode(
                &input_protocol::encode(&packet).unwrap(),
                &decode_context(&packet),
            )
            .unwrap();
            assert_eq!(decoded.rows[0].sample.held, bit);
            sequence += 1;
        }
        for &bit in &EDGE_BITS {
            let held = if bit == input_frame::EDGE_EQUIPMENT_PRESSED {
                input_frame::HELD_EQUIPMENT
            } else {
                0
            };
            let sample = input_frame::new_sample(InputSampleOptions {
                held: Some(held),
                edges: Some(bit),
                ..Default::default()
            })
            .unwrap();
            let packet = input_protocol::new_guest(PacketOptions {
                session_id: SESSION_ID.to_string(),
                manifest_id: MANIFEST_ID.to_string(),
                sender_id: format!("producer_{slot_index}"),
                sequence,
                transport_tick: sequence,
                first_input_tick: 0,
                confirmed_span: None,
                rows: vec![row(0, slot_index, sample)],
            })
            .unwrap();
            let decoded = input_protocol::decode(
                &input_protocol::encode(&packet).unwrap(),
                &decode_context(&packet),
            )
            .unwrap();
            assert_eq!(decoded.rows[0].sample.edges, bit);
            sequence += 1;
        }
    }

    for value in 0..=1023i64 {
        let sample = fuzz_sample(value);
        let mut packet = guest_packet(
            value % input_frame::SLOT_COUNT + 1,
            "fuzzer",
            value,
            value,
            0,
            Some(0),
            Some(value),
        );
        packet.rows[0].sample = sample;
        let wire = input_protocol::encode(&packet).unwrap();
        let decoded = input_protocol::decode(&wire, &decode_context(&packet)).unwrap();
        assert_eq!(
            input_frame::encode_sample(&decoded.rows[0].sample).unwrap(),
            input_frame::encode_sample(&sample).unwrap()
        );
    }
}

/// Every canonical slot owned remotely.
fn remote_sources() -> [gc_sim::rollback_input_history::RollbackInputSource; 8] {
    [gc_sim::rollback_input_history::RollbackInputSource::Remote; 8]
}

#[test]
fn recovers_six_lost_emissions_without_creating_unsent_authority() {
    let recovered = guest_packet(2, "guest_2", 6, 6, 6, None, None);
    let mut history = gc_sim::rollback_input_history::new(remote_sources());
    let arrivals: Vec<gc_sim::rollback_input_history::RollbackAuthoritativeInput> =
        input_protocol::rows(&recovered)
            .into_iter()
            .map(
                |row| gc_sim::rollback_input_history::RollbackAuthoritativeInput {
                    tick: row.tick,
                    slot_index: row.slot_index,
                    sample: row.sample,
                },
            )
            .collect();
    let accepted = gc_sim::rollback_input_history::add_authoritative_batch(&mut history, &arrivals)
        .expect("a well-formed authoritative batch is accepted");
    assert_eq!(accepted.inserted, 7);
    assert!(gc_sim::rollback_input_history::authoritative_record(&history, 0, 2).is_some());
    assert!(gc_sim::rollback_input_history::authoritative_record(&history, 6, 2).is_some());
    assert_eq!(
        gc_sim::rollback_input_history::authoritative_record(&history, 7, 2),
        None
    );

    let after_seven_losses = guest_packet(2, "guest_2", 7, 7, 7, None, None);
    assert_eq!(after_seven_losses.rows[0].tick, 1);
    let fresh_history = gc_sim::rollback_input_history::new(remote_sources());
    assert_eq!(
        gc_sim::rollback_input_history::authoritative_record(&fresh_history, 0, 2),
        None,
        "a later packet cannot invent the fallen-out tick"
    );
}

#[test]
fn rejects_malformed_noncanonical_unsupported_mismatched_and_oversized_data() {
    let packet = fixture::guest();
    let wire = input_protocol::encode(&packet).unwrap();

    let mut context = decode_context(&packet);
    context.session_id = "other_session".to_string();
    let (decoded, code) = match input_protocol::decode(&wire, &context) {
        Ok(_) => panic!("expected identity_mismatch"),
        Err(err) => (None::<()>, err.code),
    };
    assert!(decoded.is_none());
    assert_eq!(code, ErrorCode::IdentityMismatch);

    let mut context = decode_context(&packet);
    context.manifest_id = "fedcba9876543210".to_string();
    let err = input_protocol::decode(&wire, &context).unwrap_err();
    assert_eq!(err.code, ErrorCode::IdentityMismatch);

    // GCIP;1; -> GCIP;2; (packet version)
    let mut version_bumped = wire.clone();
    version_bumped[5] = b'2';
    assert_eq!(
        input_protocol::decode(&version_bumped, &decode_context(&packet))
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );

    // GCIP;1;G;2; -> GCIP;1;G;3; (sample version)
    let mut sample_version_bumped = wire.clone();
    sample_version_bumped[9] = b'3';
    assert_eq!(
        input_protocol::decode(&sample_version_bumped, &decode_context(&packet))
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );

    // First ";7;" (the sequence field) -> ";07;": non-canonical leading zero.
    let leading_zero = replace_first(&wire, b";7;", b";07;");
    assert_eq!(
        input_protocol::decode(&leading_zero, &decode_context(&packet))
            .unwrap_err()
            .code,
        ErrorCode::Malformed
    );

    // Corrupt the final base64 character.
    let mut corrupted_tail = wire[..wire.len() - 1].to_vec();
    corrupted_tail.push(b'*');
    assert_eq!(
        input_protocol::decode(&corrupted_tail, &decode_context(&packet))
            .unwrap_err()
            .code,
        ErrorCode::Malformed
    );

    let oversized = vec![b'x'; input_protocol::MAX_WIRE_BYTES + 1];
    assert_eq!(
        input_protocol::decode(&oversized, &decode_context(&packet))
            .unwrap_err()
            .code,
        ErrorCode::WireTooLarge
    );

    let mut invalid = input_protocol::copy(&packet).unwrap();
    invalid.rows[0].sample.edges = 128;
    assert_eq!(
        input_protocol::encode(&invalid).unwrap_err().code,
        ErrorCode::Malformed
    );

    let mut invalid = input_protocol::copy(&packet).unwrap();
    invalid.rows.swap(0, 1);
    assert_eq!(
        input_protocol::encode(&invalid).unwrap_err().code,
        ErrorCode::Malformed
    );

    // The original scenario for this case builds a sparse array
    // (`rows[2] = nil`), which is a shape violation with no Rust equivalent
    // (`Vec<T>` cannot have a hole, so the shape check the case targets is
    // structurally redundant here — the same reasoning `gc_sim::input_frame`'s
    // doc comment records). The closest faithful adaptation — actually
    // removing a row, which breaks tick contiguity instead of array shape —
    // still reaches the same observable outcome: `Malformed`.
    let mut invalid = input_protocol::copy(&packet).unwrap();
    invalid.rows.remove(1);
    let constructed = input_protocol::new_guest(PacketOptions {
        session_id: packet.session_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        sender_id: packet.sender_id.clone(),
        sequence: packet.sequence + 1,
        transport_tick: packet.transport_tick + 1,
        first_input_tick: packet.first_input_tick,
        confirmed_span: None,
        rows: invalid.rows,
    });
    assert_eq!(constructed.unwrap_err().code, ErrorCode::Malformed);
}

// The original scenario builds a packet table carrying a field nobody
// declared and asserts `input_protocol.validate_envelope` rejects it. A
// Rust `Packet` is a typed struct, so that exact injection is
// unconstructible — but the assertion protects the envelope check itself,
// and that IS reachable. This drives every rejection branch
// `validate_envelope` has, each with the error code the reference
// implementation distinguishes, because a peer branches on the code and
// not the message.
#[test]
fn rejects_a_packet_with_an_undeclared_extra_field() {
    let packet = gc_netcode::input_protocol_fixture::guest();
    let payload = input_protocol::encode(&packet).expect("fixture encodes");

    let sound = TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq: packet.sequence,
        tick: Some(packet.transport_tick),
        payload: payload.clone(),
    };
    assert!(
        input_protocol::validate_envelope(&packet, &sound).is_ok(),
        "a faithful envelope must validate"
    );

    let wrong_kind = TransportMessage {
        kind: TransportMessageType::Event,
        ..sound.clone()
    };
    assert_eq!(
        input_protocol::validate_envelope(&packet, &wrong_kind)
            .expect_err("a non-input envelope must be refused")
            .code,
        ErrorCode::Malformed
    );

    let wrong_tick = TransportMessage {
        tick: Some(packet.transport_tick + 1),
        ..sound.clone()
    };
    assert_eq!(
        input_protocol::validate_envelope(&packet, &wrong_tick)
            .expect_err("a mismatched transport tick must be refused")
            .code,
        ErrorCode::TickMismatch
    );

    // The undeclared-field case's real teeth: bytes that are not this packet's
    // encoding. Whatever produced them — an extra field, a truncation, a
    // different packet — the envelope must not vouch for them.
    let mut tampered = payload;
    tampered.push(b'!');
    let wrong_bytes = TransportMessage {
        payload: tampered,
        ..sound
    };
    assert_eq!(
        input_protocol::validate_envelope(&packet, &wrong_bytes)
            .expect_err("payload that is not this packet's encoding must be refused")
            .code,
        ErrorCode::IdentityMismatch
    );
}

// `game/transport/contract`'s `TransportMessage` shape now exists in
// Rust as `fault_transport::TransportMessage` (the old blocker this test
// used to name), and the envelope check itself is even implemented in this
// crate — but as a private `fn validate_envelope` inside
// `match_driver_fixture.rs`, not as the `pub` free function
// `input_protocol::validate_envelope` a direct call would need. There
// is still no way to call it standalone from a test; the current blocker is
// that gap, not the absent-type one this used to name. The check is real and
// reachable indirectly, though: every `DriverRules::canonical_host_batch`
// arrival runs through it first, before ownership is even considered, so the
// same rejection is exercised below through that entry point instead.
#[test]
fn validate_envelope_rejects_a_transport_tick_mismatch() {
    let host = host_fixture();
    let packet = guest_packet(2, "guest_2", 9, 14, 8, None, None);
    let mut mismatched = packet_arrival(&packet, 20, "guest_2");
    mismatched.envelope.tick = Some(packet.transport_tick + 1);
    let err = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 70,
            transport_tick: 20,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[mismatched],
        })
        .unwrap_err();
    assert_eq!(err.code, HostBatchErrorCode::Other);
    assert!(
        err.message
            .contains("sequence or transport tick mismatches envelope")
    );
}

// ---------------------------------------------------------------------------
// `input_protocol.canonical_host_batch` was out of scope when this file was
// first written (needed `protocol`'s `SessionManifest`/
// `SessionSlotProducer`, "out of scope for this agent" per its brief). It
// was never implemented as a free function even after `protocol` landed —
// see `input_protocol.rs`'s own module doc — but a real implementation
// exists on `match_driver_fixture::DriverRules`, driven directly below to
// exercise `input_protocol.canonical_host_batch`'s behaviour without going
// through a live driver.
// ---------------------------------------------------------------------------

/// `coordinator::plan_assignments`'s wire-shaped `Value`, converted into
/// `match_driver::SlotAssignment`, one per canonical slot. Reads the
/// assignment `Value` through its public field accessors rather than
/// `coordinator`'s own (`pub(crate)`) assignment helpers, which this
/// integration-test crate cannot reach.
fn slot_assignments(assignments_value: &Value) -> [SlotAssignment; 8] {
    std::array::from_fn(|i| {
        let producer = assignments_value
            .get_index(i as i64 + 1)
            .expect("eight canonical slots are always assigned");
        let kind = producer
            .get("producer_kind")
            .and_then(Value::as_str)
            .expect("every assignment names a producer kind");
        SlotAssignment {
            producer_kind: if kind == "peer" {
                ProducerKind::Peer
            } else {
                ProducerKind::Bot
            },
            producer_id: producer
                .get("producer_id")
                .and_then(Value::as_str)
                .expect("every assignment names a producer id")
                .to_string(),
            bot_seed: producer.get("bot_seed").and_then(Value::as_int),
        }
    })
}

/// The fixed session this file's `canonical_host_batch` cases run under: the
/// `protocol_fixture` manifest/assignments (host, five more peers, two
/// declared bot fills), and the [`DriverRules`] built from them. The
/// per-call `sequence`/`transport_tick` a full options bundle also carries
/// stay per-test instead of living here.
struct HostFixture {
    driver_manifest: DriverSessionManifest,
    driver_assignments: [SlotAssignment; 8],
    rules: DriverRules,
}

fn host_fixture() -> HostFixture {
    let manifest_value = protocol_fixture::manifest(None);
    let assignments_value = protocol_fixture::assignments();
    let driver_assignments = slot_assignments(&assignments_value);
    let session_id = manifest_value
        .get("session_id")
        .and_then(Value::as_str)
        .expect("fixture manifest has a session id")
        .to_string();
    let driver_manifest = DriverSessionManifest {
        session_id: session_id.clone(),
    };
    let get_str = |field: &str| {
        manifest_value
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("fixture manifest has {field}"))
            .to_string()
    };
    let get_int = |field: &str| {
        manifest_value
            .get(field)
            .and_then(Value::as_int)
            .unwrap_or_else(|| panic!("fixture manifest has {field}"))
    };
    let freeze = coordinator::Freeze {
        manifest_id: protocol::manifest_id(&manifest_value),
        assignment_id: protocol::assignment_id(&assignments_value, 1),
        countdown_id: "countdown.1".to_string(),
        first_input_tick: 0,
        seed: get_int("seed"),
        tick_rate: get_int("tick_rate"),
        duration_ticks: get_int("duration_ticks"),
        max_goals: get_int("max_goals"),
        content_id: get_str("content_id"),
        tuning_id: get_str("tuning_id"),
        combat_rules_id: get_str("combat_rules_id"),
        gameplay_ai_policy_id: get_str("gameplay_ai_policy_id"),
        combat_status: get_str("combat_status"),
        match_mode: protocol::MatchMode::from_wire_str(&get_str("match_mode"))
            .expect("fixture manifest names a known match mode"),
        assignments: assignments_value,
        owned: indexmap::IndexMap::new(),
        live: indexmap::IndexMap::new(),
    };
    let rules = DriverRules::new(manifest_value, freeze);
    HostFixture {
        driver_manifest,
        driver_assignments,
        rules,
    }
}

/// One arrived packet, wrapped in its wire envelope.
fn packet_arrival(
    packet: &input_protocol::Packet,
    arrival_tick: i64,
    transport_peer_id: &str,
) -> InputPacketArrival {
    InputPacketArrival {
        packet: packet.clone(),
        envelope: TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq: packet.sequence,
            tick: Some(packet.transport_tick),
            payload: input_protocol::encode(packet).expect("a well-formed packet encodes"),
        },
        arrival_tick,
        transport_peer_id: transport_peer_id.to_string(),
    }
}

/// The `sequence: 60, transport_tick: 20` host batch request used by the
/// polling-order test, for `arrivals` alone — a named function rather than a
/// closure because a closure that both captures `host` and is generic over
/// its `arrivals` parameter's lifetime cannot express the single shared
/// lifetime `HostBatchRequest<'a>` needs.
fn polling_order_request<'a>(
    host: &'a HostFixture,
    arrivals: &'a [InputPacketArrival],
) -> HostBatchRequest<'a> {
    HostBatchRequest {
        manifest: &host.driver_manifest,
        assignments: &host.driver_assignments,
        host_peer_id: "host",
        sequence: 60,
        transport_tick: 20,
        first_input_tick: 0,
        confirmed_span: 0,
        repair_rows: None,
        arrivals,
    }
}

#[test]
fn classifies_packet_and_authority_duplicates_without_first_arrival_wins() {
    let mut original = fixture::guest();
    // #489: `fixture::guest()` carries `input_protocol_fixture::MANIFEST_ID`,
    // frozen alongside `input_protocol_conformance::GOLDEN`'s wire bytes (see
    // that constant's doc comment) rather than tracking
    // `match_snapshot::COMBAT_VERSION`. This test's second half runs the
    // packet through a real `canonical_host_batch`, which DOES require the
    // live manifest id (`host_fixture`'s manifest), so it is overwritten
    // here to this file's own, COMBAT_VERSION-tracking `MANIFEST_ID` --
    // `classify_duplicate` below never reads or validates it, only
    // `canonical_host_batch` does.
    original.manifest_id = MANIFEST_ID.to_string();
    let duplicate = input_protocol::copy(&original).unwrap();
    assert_eq!(
        input_protocol::classify_duplicate(&original, &duplicate).unwrap(),
        input_protocol::DuplicateDisposition::Idempotent
    );

    let mut conflict = input_protocol::copy(&original).unwrap();
    conflict.rows[6].sample.move_x -= 1;
    let err = input_protocol::classify_duplicate(&original, &conflict).unwrap_err();
    assert_eq!(err.code, ErrorCode::PacketConflict);

    let other = input_protocol::new_guest(PacketOptions {
        session_id: original.session_id.clone(),
        manifest_id: original.manifest_id.clone(),
        sender_id: original.sender_id.clone(),
        sequence: original.sequence + 1,
        transport_tick: 13,
        first_input_tick: original.first_input_tick,
        confirmed_span: None,
        rows: input_protocol::rows(&original),
    })
    .unwrap();
    let err = input_protocol::classify_duplicate(&original, &other).unwrap_err();
    assert_eq!(err.code, ErrorCode::Duplicate);

    // `original`/`duplicate`/`other` are all idempotent-or-duplicate restates
    // of the same authority, so a host batch built from all three arrivals
    // (first-arrival-wins, never doubled) is exactly `original`'s own rows.
    let host = host_fixture();
    let repeated = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 40,
            transport_tick: 20,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[
                packet_arrival(&original, 20, "guest_2"),
                packet_arrival(&duplicate, 20, "guest_2"),
                packet_arrival(&other, 20, "guest_2"),
            ],
        })
        .expect("idempotent-or-duplicate restates of the same authority canonicalise");
    assert_eq!(repeated.rows.len(), 7);

    let changed = guest_packet(
        2,
        "guest_2",
        original.sequence + 2,
        14,
        6,
        Some(0),
        Some(100),
    );
    let err = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 41,
            transport_tick: 20,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[
                packet_arrival(&original, 20, "guest_2"),
                packet_arrival(&changed, 20, "guest_2"),
            ],
        })
        .unwrap_err();
    assert_eq!(err.code, HostBatchErrorCode::AuthorityConflict);
}

/// Finding: ownership is enforced against the *frozen* slot assignment, not
/// against whichever sender named itself, and the host's own local input is
/// held to the same fixed fairness delay a network arrival gets for free —
/// `input_protocol.canonical_host_batch`'s "fairness_delay" outcome
/// coarsens to [`HostBatchErrorCode::Other`] in this crate (the enum has no
/// dedicated variant; see [`DriverRules::canonical_host_batch`]'s body).
#[test]
fn enforces_frozen_ownership_and_the_hosts_three_tick_local_fairness_path() {
    let host = host_fixture();
    let host_local = guest_packet(1, "host", 1, 7, 6, None, None);

    let too_soon = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 50,
            transport_tick: 9,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[packet_arrival(&host_local, 9, "host")],
        })
        .unwrap_err();
    assert_eq!(too_soon.code, HostBatchErrorCode::Other);
    assert!(too_soon.message.contains("fixed fairness delay"));

    let batch = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 50,
            transport_tick: 10,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[packet_arrival(&host_local, 10, "host")],
        })
        .expect("three ticks of local delay clears the fairness path");
    assert_eq!(
        batch.rows[0].sample.move_x,
        host_local.rows[0].sample.move_x
    );
    assert_eq!(
        batch.rows[batch.rows.len() - 1].sample.edges,
        host_local.rows[host_local.rows.len() - 1].sample.edges
    );

    let false_claim = guest_packet(1, "guest_2", 2, 8, 6, None, None);
    let ownership_err = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 51,
            transport_tick: 10,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[packet_arrival(&false_claim, 10, "guest_2")],
        })
        .unwrap_err();
    assert_eq!(ownership_err.code, HostBatchErrorCode::OwnershipMismatch);

    let bot = guest_packet(7, "bot_away_3", 3, 7, 6, None, None);
    let bot_batch = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 52,
            transport_tick: 10,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: None,
            arrivals: &[packet_arrival(&bot, 10, "host")],
        })
        .expect("the host authors its own declared bot fills");
    assert_eq!(bot_batch.rows[0].slot_index, 7);
}

#[test]
fn emits_one_byte_identical_canonical_host_batch_for_every_peer_polling_order() {
    let host = host_fixture();
    let mut arrivals = Vec::with_capacity(8);
    for slot_index in 1..=8i64 {
        let assignment = &host.driver_assignments[(slot_index - 1) as usize];
        let local_producer =
            assignment.producer_id == "host" || assignment.producer_kind == ProducerKind::Bot;
        let packet = guest_packet(
            slot_index,
            &assignment.producer_id,
            slot_index,
            if local_producer { 17 } else { 19 },
            6,
            None,
            None,
        );
        let transport_peer_id = if assignment.producer_kind == ProducerKind::Bot {
            "host".to_string()
        } else {
            assignment.producer_id.clone()
        };
        arrivals.push(packet_arrival(&packet, 20, &transport_peer_id));
    }
    let mut reversed = arrivals.clone();
    reversed.reverse();

    let first = host
        .rules
        .canonical_host_batch(polling_order_request(&host, &arrivals))
        .expect("a full 8-slot batch canonicalises");
    let second = host
        .rules
        .canonical_host_batch(polling_order_request(&host, &reversed))
        .expect("polling order cannot change the canonical batch");

    // Eight single-slot bundles at a full window each: the steady-state
    // batch, which is 56 rows and no longer the row bound itself.
    assert_eq!(
        first.rows.len() as i64,
        input_frame::SLOT_COUNT * input_protocol::RETAINED_ROWS
    );
    assert!(first.rows.len() as i64 <= input_protocol::MAX_HOST_ROWS);
    assert_eq!(
        input_protocol::encode(&first).unwrap(),
        input_protocol::encode(&second).unwrap()
    );
    for index in 1..first.rows.len() {
        let previous = &first.rows[index - 1];
        let current = &first.rows[index];
        assert!(
            previous.tick < current.tick
                || (previous.tick == current.tick && previous.slot_index < current.slot_index)
        );
    }
}

#[test]
fn fits_the_measured_72_row_maximum_with_its_declared_margin() {
    let maximal = fixture::maximal();
    let wire = input_protocol::encode(&maximal).unwrap();
    assert_eq!(maximal.rows.len(), 72);
    assert_eq!(maximal.rows.len() as i64, input_protocol::MAX_HOST_ROWS);
    assert_eq!(wire.len(), 958);
    assert!(wire.len() <= input_protocol::MAX_WIRE_BYTES);
    assert!(input_protocol::MAX_WIRE_BYTES - wire.len() >= input_protocol::MIN_WIRE_MARGIN_BYTES);
    let decoded = input_protocol::decode(&wire, &decode_context(&maximal)).unwrap();
    assert_eq!(decoded.rows.len(), 72);
    assert_eq!(
        decoded.rows[0].tick,
        input_frame::MAX_TICK - input_protocol::HOST_WINDOW_ROWS + 1
    );
    assert_eq!(decoded.rows[71].tick, input_frame::MAX_TICK);

    let mut over = input_protocol::copy(&maximal).unwrap();
    over.rows
        .push(row(input_frame::MAX_TICK, 8, input_frame::neutral_sample()));
    assert_eq!(
        input_protocol::encode(&over).unwrap_err().code,
        ErrorCode::Malformed
    );
}

#[test]
fn carries_a_senders_confirmation_as_a_span_and_round_trips_it() {
    let packet = guest_packet(2, "guest_2", 9, 14, 8, Some(2), None);
    assert_eq!(packet.confirmed_span, 0);
    assert_eq!(input_protocol::confirmed_tick(&packet), 1);

    let reporting = input_protocol::new_guest(PacketOptions {
        session_id: packet.session_id.clone(),
        manifest_id: packet.manifest_id.clone(),
        sender_id: packet.sender_id.clone(),
        sequence: packet.sequence,
        transport_tick: packet.transport_tick,
        first_input_tick: packet.first_input_tick,
        confirmed_span: Some(5),
        rows: input_protocol::rows(&packet),
    })
    .unwrap();
    assert_eq!(input_protocol::confirmed_tick(&reporting), 6);
    let wire = input_protocol::encode(&reporting).unwrap();
    let decoded = input_protocol::decode(&wire, &decode_context(&reporting)).unwrap();
    assert_eq!(decoded.confirmed_span, 5);
    assert_eq!(input_protocol::encode(&decoded).unwrap(), wire);

    // The span is bounded by the ticks that can exist above `first_input_tick`,
    // so it can never describe a confirmation past the end of the session.
    let mut over = input_protocol::copy(&reporting).unwrap();
    over.confirmed_span = input_frame::MAX_TICK - over.first_input_tick + 2;
    assert_eq!(
        input_protocol::encode(&over).unwrap_err().code,
        ErrorCode::Malformed
    );

    assert_eq!(input_protocol::confirmed_span(10, 9), 0);
    assert_eq!(input_protocol::confirmed_span(10, 4), 0);
    assert_eq!(input_protocol::confirmed_span(10, 14), 5);
}

#[test]
fn merges_host_repair_rows_and_conflict_checks_them_like_any_other() {
    let host = host_fixture();
    let packet = guest_packet(2, "guest_2", 9, 14, 8, Some(0), None);
    let arrival = packet_arrival(&packet, 20, "guest_2");

    // Sorted into the one canonical (tick, slot) order, with the repaired
    // ticks ahead of the arrival's window rather than appended after it.
    let repair_rows = vec![
        AuthorityRow {
            tick: 0,
            slot_index: 5,
            sample: fuzz_sample(41),
        },
        AuthorityRow {
            tick: 1,
            slot_index: 5,
            sample: fuzz_sample(42),
        },
    ];
    let batch = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 60,
            transport_tick: 20,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: Some(&repair_rows),
            arrivals: std::slice::from_ref(&arrival),
        })
        .expect("repair rows outside the arrival window merge cleanly");
    assert_eq!(batch.rows.len(), packet.rows.len() + 2);
    assert_eq!(batch.rows[0].tick, 0);
    assert_eq!(batch.rows[0].slot_index, 5);
    assert_eq!(batch.rows[1].tick, 1);
    assert_eq!(batch.rows[1].slot_index, 5);
    assert_eq!(batch.rows[2].tick, 2);
    assert_eq!(batch.rows[2].slot_index, 2);

    // A repair that repeats a row the arrivals already carry is idempotent,
    // which is what makes re-sending one every tick safe.
    let repeated_repair = vec![input_protocol::rows(&packet)[0].clone()];
    let same = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 61,
            transport_tick: 20,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: Some(&repeated_repair),
            arrivals: std::slice::from_ref(&arrival),
        })
        .expect("a repair that repeats an arrival row is idempotent");
    assert_eq!(same.rows.len(), packet.rows.len());

    let conflicting_repair = vec![AuthorityRow {
        tick: 8,
        slot_index: 2,
        sample: fuzz_sample(999),
    }];
    let err = host
        .rules
        .canonical_host_batch(HostBatchRequest {
            manifest: &host.driver_manifest,
            assignments: &host.driver_assignments,
            host_peer_id: "host",
            sequence: 62,
            transport_tick: 20,
            first_input_tick: 0,
            confirmed_span: 0,
            repair_rows: Some(&conflicting_repair),
            arrivals: &[arrival],
        })
        .unwrap_err();
    assert_eq!(err.code, HostBatchErrorCode::AuthorityConflict);
}

#[test]
fn leaves_five_rows_of_headroom_under_the_hard_byte_ceiling() {
    let wire = input_protocol::encode(&fixture::maximal()).unwrap();
    let row_bytes = input_protocol::RECORD_BYTES * 4 / 3;
    assert_eq!(row_bytes, 12);
    assert_eq!(input_protocol::MAX_HOST_ROWS, 72);
    assert!(wire.len() as i64 + 5 * row_bytes <= input_protocol::MAX_WIRE_BYTES as i64);
    assert!(wire.len() as i64 + 6 * row_bytes > input_protocol::MAX_WIRE_BYTES as i64);
}

#[test]
fn coalesces_only_when_no_unsent_authority_can_fall_through_backpressure() {
    let older = guest_packet(2, "guest_2", 70, 30, 6, None, None);
    let mut conflicting_reuse = input_protocol::copy(&older).unwrap();
    conflicting_reuse.transport_tick += 1;
    let err = input_protocol::supersede_for_backpressure(&older, &conflicting_reuse).unwrap_err();
    assert_eq!(err.code, ErrorCode::PacketConflict);

    let repeated = guest_packet(2, "guest_2", 71, 31, 6, None, None);
    assert!(input_protocol::supersede_for_backpressure(&older, &repeated).is_ok());

    let next_tick = guest_packet(2, "guest_2", 72, 32, 7, None, None);
    let err = input_protocol::supersede_for_backpressure(&older, &next_tick).unwrap_err();
    assert_eq!(err.code, ErrorCode::BackpressureGap);
}

#[test]
fn does_not_coalesce_across_a_different_frozen_first_input_tick() {
    let older = guest_packet(2, "guest_2", 70, 30, 7, Some(0), None);
    let newer = guest_packet(2, "guest_2", 71, 31, 7, Some(1), None);
    let err = input_protocol::supersede_for_backpressure(&older, &newer).unwrap_err();
    assert_eq!(err.code, ErrorCode::BackpressureGap);
}

#[test]
fn does_not_coalesce_when_transport_time_regresses() {
    let older = guest_packet(2, "guest_2", 70, 30, 6, None, None);
    let newer = guest_packet(2, "guest_2", 71, 29, 6, None, None);
    let err = input_protocol::supersede_for_backpressure(&older, &newer).unwrap_err();
    assert_eq!(err.code, ErrorCode::BackpressureGap);
}

fn replace_first(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    if let Some(position) = haystack
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let mut result = Vec::with_capacity(haystack.len() - needle.len() + replacement.len());
        result.extend_from_slice(&haystack[..position]);
        result.extend_from_slice(replacement);
        result.extend_from_slice(&haystack[position + needle.len()..]);
        result
    } else {
        haystack.to_vec()
    }
}
