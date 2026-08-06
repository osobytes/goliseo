//! Port of `spec/game/online_input_protocol_spec.lua`.
//!
//! ## What could not be ported as-is
//!
//! The Lua spec also exercises `game.online.protocol`, `game.online.protocol_fixture`,
//! `game.transport.contract`, `sim.match_snapshot`, and `sim.rollback_input_history`.
//! Per this agent's brief, `protocol.lua` is out of scope ("later agents own
//! those"), `game/transport/contract.lua` is TypeScript-owned with no Rust
//! port planned (`v2/README.md` §2), and `sim/match_snapshot.lua` /
//! `sim/rollback_input_history.lua` are still unported placeholders in
//! `gc-sim` as of this writing (owned by sibling agents). Assertions that
//! genuinely need one of those are marked `#[ignore]` below, naming the
//! missing module. Assertions that only needed *a* valid session/manifest id
//! string (not `protocol.lua`'s specific hashing logic) were ported using the
//! same literal identity `input_protocol_fixture` uses — see that module's
//! doc comment.

use gc_netcode::input_protocol::{self, AuthorityRow, DecodeContext, ErrorCode, PacketOptions};
use gc_netcode::input_protocol_conformance as conformance;
use gc_netcode::input_protocol_fixture as fixture;
use gc_sim::input_frame::{self, InputSampleOptions};

/// Session identity shared with `input_protocol_fixture` — see that module's
/// doc comment for why this is a literal rather than a call into
/// `protocol_fixture` (out of scope; see this file's module doc comment).
const SESSION_ID: &str = "session_alpha";
const MANIFEST_ID: &str = "eb59f113614c35b2";

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
/// `0..=1023` fuzz loop below), so plain `%` agrees with Lua's floored `%` —
/// no `rem_euclid` needed. See `v2/tools/lua_reference/README.md`'s trap note.
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

#[test]
#[ignore = "blocked on gc_sim::match_snapshot (still an unported placeholder; \
this agent does not own gc-sim). The Lua test asserts input_conformance's \
staleness check against match_snapshot.VERSION/COMBAT_VERSION, which \
input_protocol_conformance::verify deliberately does not perform yet — see \
that module's doc comment."]
fn names_the_snapshot_version_its_literals_were_generated_against() {}

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
    // The Lua test also asserts
    // `input_protocol.validate_envelope(decoded, envelope(decoded))` here.
    // `validate_envelope` is not ported (needs `game.transport.contract`,
    // TypeScript-owned with no Rust port planned) — see
    // `omp3_validate_envelope_rejects_a_transport_tick_mismatch` below and
    // this file's module doc comment.

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

#[test]
#[ignore = "blocked on gc_sim::rollback_input_history (still an unported \
placeholder; this agent does not own gc-sim)."]
fn recovers_six_lost_emissions_without_creating_unsent_authority() {}

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

    // Lua builds a sparse array (`rows[2] = nil`), which is a shape violation
    // with no Rust equivalent (`Vec<T>` cannot have a hole — README rule 8 /
    // the precedent in `gc_sim::input_frame`'s doc comment on structurally
    // redundant shape checks). The closest faithful adaptation — actually
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

#[test]
#[ignore = "the Lua case constructs a packet table with an undeclared extra \
field and asserts input_protocol.validate rejects it. gc_netcode::input_protocol::Packet \
is a Rust struct: it cannot have an undeclared field by construction, so this \
check is structurally redundant here — the same class of drop already \
documented in crates/gc-sim/src/input_frame.rs's module doc comment."]
fn rejects_a_packet_with_an_undeclared_extra_field() {}

#[test]
#[ignore = "blocked on game.transport.contract (TypeScript-owned per \
v2/README.md §2; no Rust port exists or is planned) and therefore on \
input_protocol::validate_envelope, which needs its TransportMessage type."]
fn validate_envelope_rejects_a_transport_tick_mismatch() {}

#[test]
fn classifies_packet_and_authority_duplicates_without_first_arrival_wins() {
    let original = fixture::guest();
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

    // The Lua test continues into `input_protocol.canonical_host_batch`,
    // which is not ported (needs `game.online.protocol`'s `SessionManifest` —
    // out of scope for this agent, see this file's module doc comment).
}

#[test]
#[ignore = "blocked on game.online.protocol (out of scope for this agent per \
its brief: \"later agents own those\") and therefore on \
input_protocol::canonical_host_batch, which needs protocol.lua's \
SessionManifest/SessionSlotProducer types and validate_manifest/manifest_id/ \
validate_assignment_manifest."]
fn enforces_frozen_ownership_and_the_hosts_three_tick_local_fairness_path() {}

#[test]
#[ignore = "blocked on game.online.protocol; see \
enforces_frozen_ownership_and_the_hosts_three_tick_local_fairness_path."]
fn emits_one_byte_identical_canonical_host_batch_for_every_peer_polling_order() {}

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
#[ignore = "blocked on game.online.protocol; see \
enforces_frozen_ownership_and_the_hosts_three_tick_local_fairness_path."]
fn merges_host_repair_rows_and_conflict_checks_them_like_any_other() {}

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
