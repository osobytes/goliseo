//! Differential evidence for `gc_netcode::fake_relay` (the Rust port of
//! `game/transport/fake_relay.lua`) against `tests/fixtures/fake_relay_lua_reference.txt`,
//! captured from the real Lua per `v2/tools/lua_reference/README.md`. See
//! that fixture's own header comment for exactly what each scenario proves
//! and how to regenerate it.
//!
//! `crates/gc-netcode/src/fake_relay.rs`'s own `#[cfg(test)]` module already
//! ports every assertion from `spec/game/transport_relay_spec.lua`'s first
//! `describe` block one for one — that proves the port satisfies the
//! assertions someone wrote down. This file is the second, independent
//! half README rule 9 asks for: real byte/count values computed by the real
//! Lua, compared against this port's own output for the identical scenario,
//! so a subtly-wrong port that still happens to pass every ported assertion
//! cannot hide.

use gc_netcode::fake_relay::{FakeRelayTransport, FakeRelayTransportOptions};
use gc_netcode::fake_star::{FakeStarTransport, FakeStarTransportOptions};
use gc_netcode::fault_transport::{
    StarTransportAdapter, TransportChannel, TransportMessage, TransportMessageType, TransportRole,
};
use gc_netcode::input_protocol;
use gc_sim::input_frame;

const LUA_REFERENCE: &str = include_str!("fixtures/fake_relay_lua_reference.txt");

/// Looks up `KEY=value` from the differential fixture file.
fn lua_ref(key: &str) -> &'static str {
    for line in LUA_REFERENCE.lines() {
        if let Some(rest) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            return rest;
        }
    }
    panic!("missing lua reference key: {key}");
}

fn lua_ref_i64(key: &str) -> i64 {
    lua_ref(key).parse().expect("reference value is an i64")
}

fn hex_decode(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "hex string has an odd length");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("valid hex digit pair"))
        .collect()
}

fn member_id(index: i64) -> String {
    if index == 1 {
        "host".to_string()
    } else {
        format!("guest_{}", index - 1)
    }
}

fn build_room(
    count: i64,
) -> (
    gc_netcode::fake_relay::FakeRelayRoom,
    Vec<FakeRelayTransport>,
) {
    let room = FakeRelayTransport::new_room();
    let mut members = Vec::new();
    for index in 1..=count {
        let mut endpoint = FakeRelayTransport::new(FakeRelayTransportOptions {
            role: if index == 1 {
                TransportRole::Host
            } else {
                TransportRole::Guest
            },
            peer_id: Some(member_id(index)),
            room: Some(room.clone()),
            ..Default::default()
        });
        endpoint.initialize().expect("member initializes");
        members.push(endpoint);
    }
    (room, members)
}

fn control(seq: i64, payload: &[u8]) -> TransportMessage {
    TransportMessage {
        version: 1,
        kind: TransportMessageType::Event,
        seq,
        tick: None,
        payload: payload.to_vec(),
    }
}

fn input(seq: i64, tick: i64, payload: &[u8]) -> TransportMessage {
    TransportMessage {
        version: 1,
        kind: TransportMessageType::Input,
        seq,
        tick: Some(tick),
        payload: payload.to_vec(),
    }
}

/// Splits `wire` (`peer_id|channel|<envelope>`) on its first two `|` bytes
/// and returns the envelope tail — the same split `contract.decode_addressed`
/// does, used here only to locate byte offsets in a captured Lua wire, never
/// to decode it (this crate's own `decode_addressed` is exercised for real
/// by every other test in this file, through `send`/`pump`/`poll`).
fn envelope_tail(wire: &[u8]) -> &[u8] {
    let first = wire.iter().position(|&b| b == b'|').expect("first pipe");
    let rest = &wire[first + 1..];
    let second = rest.iter().position(|&b| b == b'|').expect("second pipe");
    &rest[second + 1..]
}

/// Scenario 1: exact wire bytes for `contract.encode_addressed`, including
/// percent-escaping a payload that contains every reserved character the
/// scheme has to handle (space, `|`, `%`, `\n`), cross-checked two ways:
/// structurally against the captured Lua bytes directly, and numerically
/// against this port's own `wire_counters()` for the identical message
/// (`uplink_bytes`/`frame_overhead_bytes` are a direct, public function of
/// `encode_addressed`'s escaping and prefix construction, even though the
/// function itself is private). `fake_relay` is the one module in this
/// crate that ever decodes a wire it produced (see its module doc), so the
/// round trip through `send`/`pump`/`poll` below is this port's only
/// real-decode differential coverage.
#[test]
fn differential_encode_addressed_matches_the_real_lua_byte_for_byte() {
    let expected_control_wire = hex_decode(lua_ref("ENCODE_ADDRESSED_CONTROL_HEX"));
    // `host|control|1|event|7|` is fixed structure: peer id `host`, channel
    // literal `control`, message version 1, type `event` (needs no
    // escaping), seq 7 — six fields, five pipes. The next byte starts the
    // tick field, empty for a control message, so exactly one more `|`
    // remains before the (percent-escaped) payload begins.
    let known_prefix: &[u8] = b"host|control|1|event|7|";
    assert!(expected_control_wire.starts_with(known_prefix));
    let after_known_prefix = &expected_control_wire[known_prefix.len()..];
    assert!(
        after_known_prefix.starts_with(b"|"),
        "the tick field is empty for a control message"
    );
    let payload_wire = &after_known_prefix[1..];
    let mut decoded_payload = Vec::new();
    let mut i = 0;
    while i < payload_wire.len() {
        if payload_wire[i] == b'%' {
            let hex = std::str::from_utf8(&payload_wire[i + 1..i + 3]).unwrap();
            decoded_payload.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            decoded_payload.push(payload_wire[i]);
            i += 1;
        }
    }
    let original_payload = b"hello world|needs% escaping\nhere".to_vec();
    assert_eq!(decoded_payload, original_payload);

    // Numeric cross-check: send the identical message from a real `host`
    // endpoint and compare the resulting uplink/frame-overhead byte counts
    // against the captured Lua wire's own lengths.
    let (_, mut pair) = build_room(2);
    pair[0]
        .send(
            "guest_1",
            TransportChannel::Control,
            control(7, &original_payload),
        )
        .expect("send is accepted");
    let counters = pair[0].wire_counters();
    let expected_envelope_len = envelope_tail(&expected_control_wire).len() as i64;
    assert_eq!(counters.uplink_bytes, expected_envelope_len);
    assert_eq!(
        counters.frame_overhead_bytes,
        expected_control_wire.len() as i64 - expected_envelope_len
    );

    // The round trip: pump and poll the other side, confirming
    // `decode_addressed` recovers the exact original payload this port's
    // own `encode_addressed` produced.
    pair[0].pump();
    let received = pair[1].poll_batch(Some(1));
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].message.payload, original_payload);

    // The input-channel wire: `guest_3|input|1|input|3|41|input%20payload`
    // — same six-field prefix shape, but the tick field is `41`, not empty.
    let expected_input_wire = hex_decode(lua_ref("ENCODE_ADDRESSED_INPUT_HEX"));
    let known_input_prefix: &[u8] = b"guest_3|input|1|input|3|41|";
    assert!(expected_input_wire.starts_with(known_input_prefix));
    assert_eq!(
        &expected_input_wire[known_input_prefix.len()..],
        b"input%20payload"
    );
}

/// Scenario 2: the exact persistent round-robin drain order `take` computes
/// across two senders on two channels, with deliberately uneven queue
/// depths and one empty (slot, channel) pair per pass.
#[test]
fn differential_drain_order_matches_the_real_lua() {
    let (_, mut members) = build_room(3);
    let (host, guest1, guest2) = {
        let [host, rest @ ..] = members.as_mut_slice() else {
            unreachable!()
        };
        let [guest1, guest2] = rest else {
            unreachable!()
        };
        (host, guest1, guest2)
    };
    guest1
        .send("host", TransportChannel::Control, control(0, b"g1c0"))
        .unwrap();
    guest2
        .send("host", TransportChannel::Input, input(0, 10, b"g2i0"))
        .unwrap();
    guest1
        .send("host", TransportChannel::Control, control(1, b"g1c1"))
        .unwrap();
    guest2
        .send("host", TransportChannel::Input, input(1, 11, b"g2i1"))
        .unwrap();
    guest1
        .send("host", TransportChannel::Control, control(2, b"g1c2"))
        .unwrap();
    guest1.pump();

    let mut order = Vec::new();
    for _ in 0..8 {
        let Some(entry) = host.poll() else { break };
        order.push(format!(
            "{}:{}:{}:{}",
            entry.peer_id,
            match entry.channel {
                TransportChannel::Control => "control",
                TransportChannel::Input => "input",
            },
            String::from_utf8(entry.message.payload).unwrap(),
            entry.arrival_seq
        ));
    }
    assert_eq!(order.join("|"), lua_ref("DRAIN_ORDER"));
}

/// Scenario 3: a shared uplink budget is charged once per broadcast unit,
/// not once per target — the property that is the whole point of a relay.
#[test]
fn differential_backpressure_matches_the_real_lua() {
    let room = FakeRelayTransport::new_room();
    let mut host = FakeRelayTransport::new(FakeRelayTransportOptions {
        role: TransportRole::Host,
        peer_id: Some("host".to_string()),
        room: Some(room.clone()),
        buffered_amount_limit: Some(25),
        ..Default::default()
    });
    host.initialize().unwrap();
    let mut guests = Vec::new();
    for index in 2..=3 {
        let mut guest = FakeRelayTransport::new(FakeRelayTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some(member_id(index)),
            room: Some(room.clone()),
            ..Default::default()
        });
        guest.initialize().unwrap();
        guests.push(guest);
    }
    for seq in 0..4 {
        host.broadcast(TransportChannel::Input, input(seq, seq, b"abcdefghij"))
            .unwrap();
    }
    host.pump();
    let diagnostics = host.diagnostics();
    assert_eq!(
        diagnostics.backpressure,
        lua_ref_i64("BACKPRESSURE_AFTER_ONE_PUMP_BACKPRESSURE_COUNT")
    );
    assert_eq!(
        diagnostics.peers[0].input.outbound_depth,
        lua_ref_i64("BACKPRESSURE_AFTER_ONE_PUMP_OUTBOUND_DEPTH_GUEST1")
    );

    let mut delivered = 0i64;
    for guest in &mut guests {
        delivered += guest.poll_batch(Some(16)).len() as i64;
    }
    for _ in 0..4 {
        host.pump();
        for guest in &mut guests {
            delivered += guest.poll_batch(Some(16)).len() as i64;
        }
    }
    assert_eq!(delivered, lua_ref_i64("BACKPRESSURE_TOTAL_DELIVERED"));
    let counters = host.wire_counters();
    assert_eq!(
        counters.uplink_units,
        lua_ref_i64("BACKPRESSURE_FINAL_UPLINK_UNITS")
    );
    assert_eq!(
        counters.uplink_bytes,
        lua_ref_i64("BACKPRESSURE_FINAL_UPLINK_BYTES")
    );
}

/// Scenario 4: the exact (and, at first glance, surprising) compounding
/// overflow counter — see the fixture's own header for why three failed
/// sends produce a count of 7, not 3. A port that only tracked the obvious
/// half of this (the per-send overflow branch) would diverge here even
/// though every unit test in `fake_relay.rs` would still pass.
#[test]
fn differential_overflow_counter_matches_the_real_lua_compounding() {
    let room = FakeRelayTransport::new_room();
    let mut host = FakeRelayTransport::new(FakeRelayTransportOptions {
        role: TransportRole::Host,
        peer_id: Some("host".to_string()),
        room: Some(room.clone()),
        queue_limit: Some(2),
        ..Default::default()
    });
    host.initialize().unwrap();
    let mut guest = FakeRelayTransport::new(FakeRelayTransportOptions {
        role: TransportRole::Guest,
        peer_id: Some("guest_1".to_string()),
        room: Some(room),
        ..Default::default()
    });
    guest.initialize().unwrap();

    let mut accepted = 0i64;
    let mut rejected = 0i64;
    for seq in 0..5 {
        match host.send("guest_1", TransportChannel::Control, control(seq, b"y")) {
            Ok(_) => accepted += 1,
            Err(_) => rejected += 1,
        }
    }
    assert_eq!(accepted, lua_ref_i64("OVERFLOW_ACCEPTED"));
    assert_eq!(rejected, lua_ref_i64("OVERFLOW_REJECTED"));
    assert_eq!(
        host.diagnostics().overflow,
        lua_ref_i64("OVERFLOW_DIAGNOSTICS_OVERFLOW_COUNT")
    );
}

/// Scenario 5: exact per-member wire cost over the same 8-member, 60-tick
/// scenario `transport_relay_topology_probe_measures_sequencer_less_per_node_wire_cost`
/// (`tests/protocol.rs`) only brackets. Also confirms the asymmetric
/// framing overhead between the host's 7-byte-id origins and a guest's
/// mixed 4-byte/7-byte-id origins — a real per-line effect, not noise.
#[test]
fn differential_per_node_wire_cost_matches_the_real_lua_exactly() {
    fn guest_bundle(peer_id: &str, slot_index: i64, transport_tick: i64) -> TransportMessage {
        let mut rows = Vec::new();
        for tick in 0..=input_protocol::HISTORY_ROWS {
            rows.push(input_protocol::AuthorityRow {
                tick: transport_tick + tick,
                slot_index,
                sample: input_frame::neutral_sample(),
            });
        }
        let packet = input_protocol::new_guest(input_protocol::PacketOptions {
            session_id: "session_relay_probe01".to_string(),
            manifest_id: "a".repeat(16),
            sender_id: peer_id.to_string(),
            sequence: transport_tick,
            transport_tick,
            first_input_tick: 0,
            confirmed_span: None,
            rows,
        })
        .expect("a well-formed guest packet");
        TransportMessage {
            version: 1,
            kind: TransportMessageType::Input,
            seq: packet.sequence,
            tick: Some(packet.transport_tick),
            payload: input_protocol::encode(&packet).expect("a well-formed packet encodes"),
        }
    }

    let (_, mut members) = build_room(8);
    let ticks = 60i64;
    for tick in 1..=ticks {
        for (offset, endpoint) in members.iter_mut().enumerate() {
            let index = offset as i64 + 1;
            endpoint
                .broadcast(
                    TransportChannel::Input,
                    guest_bundle(&member_id(index), index, tick),
                )
                .unwrap();
        }
        members[0].pump();
        for endpoint in members.iter_mut() {
            endpoint.poll_batch(Some(256));
        }
    }

    for (offset, endpoint) in members.iter().enumerate() {
        let index = offset as i64 + 1;
        let counters = endpoint.wire_counters();
        let prefix = format!("WIRE_COST_{index}_");
        assert_eq!(
            counters.uplink_units,
            lua_ref_i64(&format!("{prefix}UPLINK_UNITS")),
            "member {index} uplink_units"
        );
        assert_eq!(
            counters.uplink_bytes,
            lua_ref_i64(&format!("{prefix}UPLINK_BYTES")),
            "member {index} uplink_bytes"
        );
        assert_eq!(
            counters.input_uplink_bytes,
            lua_ref_i64(&format!("{prefix}INPUT_UPLINK_BYTES")),
            "member {index} input_uplink_bytes"
        );
        assert_eq!(
            counters.downlink_bytes,
            lua_ref_i64(&format!("{prefix}DOWNLINK_BYTES")),
            "member {index} downlink_bytes"
        );
        assert_eq!(
            counters.input_downlink_bytes,
            lua_ref_i64(&format!("{prefix}INPUT_DOWNLINK_BYTES")),
            "member {index} input_downlink_bytes"
        );
        assert_eq!(
            counters.downlink_framed_bytes,
            lua_ref_i64(&format!("{prefix}DOWNLINK_FRAMED_BYTES")),
            "member {index} downlink_framed_bytes"
        );
        assert_eq!(
            counters.downlink_frames,
            lua_ref_i64(&format!("{prefix}DOWNLINK_FRAMES")),
            "member {index} downlink_frames"
        );
        assert_eq!(
            counters.frame_overhead_bytes,
            lua_ref_i64(&format!("{prefix}FRAME_OVERHEAD_BYTES")),
            "member {index} frame_overhead_bytes"
        );
    }
}

/// Scenario 6: one broadcast's exact byte cost, relay versus star — the
/// headline OMP-4 claim (`hub.uplink_bytes == relay.uplink_bytes * 7`),
/// confirmed against real Lua-computed integers rather than only against
/// this port's own star.
#[test]
fn differential_relay_vs_star_broadcast_cost_matches_the_real_lua() {
    let (_, mut members) = build_room(8);
    let payload = vec![b'x'; 200];
    members[0]
        .broadcast(TransportChannel::Input, input(1, 40, &payload))
        .unwrap();
    members[0].pump();
    let relay = members[0].wire_counters();
    assert_eq!(
        relay.uplink_units,
        lua_ref_i64("COMPARE_RELAY_UPLINK_UNITS")
    );
    assert_eq!(
        relay.uplink_bytes,
        lua_ref_i64("COMPARE_RELAY_UPLINK_BYTES")
    );
    assert_eq!(
        relay.input_uplink_bytes,
        lua_ref_i64("COMPARE_RELAY_INPUT_UPLINK_BYTES")
    );

    let mut star = FakeStarTransport::new(FakeStarTransportOptions {
        role: TransportRole::Host,
        ..Default::default()
    });
    star.initialize().unwrap();
    for index in 2..=8 {
        let mut guest = FakeStarTransport::new(FakeStarTransportOptions {
            role: TransportRole::Guest,
            peer_id: Some(member_id(index)),
            ..Default::default()
        });
        guest.initialize().unwrap();
        star.open_peer(&member_id(index)).unwrap();
        star.link(&guest).unwrap();
    }
    star.broadcast(TransportChannel::Input, input(1, 40, &payload))
        .unwrap();
    let (hub_uplink_bytes, _) = star.wire_bytes();
    assert_eq!(hub_uplink_bytes, lua_ref_i64("COMPARE_STAR_UPLINK_BYTES"));
    assert_eq!(hub_uplink_bytes, relay.uplink_bytes * 7);
}
