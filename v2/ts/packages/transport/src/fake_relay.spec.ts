// Ported from spec/game/transport_relay_spec.lua.
//
// That Lua file has two halves that answer different questions. The first
// -- ported below -- is the adapter: does `fake_relay` satisfy
// `StarTransportAdapter`, and does it differ from `fake_star` in exactly the
// way the OMP-4 topology decision says it does (one upload per `broadcast`,
// no privileged member, no manual signaling, a room that forwards opaque
// bytes without parsing them).
//
// The second half ("relay topology probe: no peer is the sequencer") drives
// `game.online.coordinator`, `game.online.fault_harness`, `game.online.
// match_driver`, `game.online.input_protocol`, and friends -- the
// rollback/session layer, not the transport layer. Per v2/README.md section
// 2.1, that layer is `crates/gc-netcode` in Rust, not
// this TS package, and those Lua modules do not exist in `@gc/transport`'s
// dependency graph. That half is left for whichever agent ports
// `crates/gc-netcode` -- it belongs beside `coordinator.rs` /
// `match_driver.rs` / `fault_harness.rs`, not here.

import { describe, expect, it } from "vitest";
import * as contract from "./contract.ts";
import * as transport from "./index.ts";
import { FakeRelayTransport, type FakeRelayRoom } from "./fake_relay.ts";
import type { TransportFailure, TransportMessage, TransportResult } from "./contract.ts";

function unwrap<T>(result: TransportResult<T>): T {
  if (!result.ok) {
    throw new Error(`${result.error.code}: ${result.error.message}`);
  }
  return result.value;
}

function expectErr<T>(result: TransportResult<T>): TransportFailure {
  expect(result.ok).toBe(false);
  if (result.ok) {
    throw new Error("expected a transport failure");
  }
  return result.error;
}

function control(seq: number, payload = ""): TransportMessage {
  return unwrap(contract.newMessage({ type: "event", seq, payload }));
}

function input(seq: number, tick: number, payload = ""): TransportMessage {
  return unwrap(contract.newMessage({ type: "input", seq, tick, payload }));
}

function memberId(index: number): string {
  return index === 1 ? "host" : `guest_${index - 1}`;
}

function buildRoom(count: number): readonly [FakeRelayRoom, FakeRelayTransport[]] {
  const room = transport.fakeRelayRoom();
  const members: FakeRelayTransport[] = [];
  for (let index = 1; index <= count; index += 1) {
    const endpoint = transport.fakeRelay({
      role: index === 1 ? "host" : "guest",
      peer_id: memberId(index),
      room,
    });
    unwrap(endpoint.initialize());
    members[index - 1] = endpoint;
  }
  return [room, members];
}

describe("relay transport adapter", () => {
  it("joins every member to every other without a privileged opener", () => {
    const [, members] = buildRoom(8);
    members.forEach((endpoint, index) => {
      expect(endpoint.peerIds().length).toBe(7);
      expect(endpoint.state()).toBe("connected");
      for (let other = 1; other <= 8; other += 1) {
        if (other !== index + 1) {
          expect(endpoint.peerState(memberId(other))).toBe("connected");
        }
      }
    });
  });

  it("delivers an addressed envelope and never echoes a member's own line", () => {
    const [, members] = buildRoom(3);
    const host = members[0] as FakeRelayTransport;
    unwrap(host.broadcast("input", input(1, 40, "authority")));
    host.pump();
    expect(host.pollBatch(16).length).toBe(0); // a relay never returns a member's own rows
    for (let index = 2; index <= 3; index += 1) {
      const batch = (members[index - 1] as FakeRelayTransport).pollBatch(16);
      expect(batch.length).toBe(1);
      expect(batch[0]?.peer_id).toBe("host");
      expect(batch[0]?.channel).toBe("input");
      expect(batch[0]?.message.payload).toBe("authority");
    }
  });

  it("charges one upload per broadcast where the star charges one per link", () => {
    const [, members] = buildRoom(8);
    const message = input(1, 40, "x".repeat(200));
    const host = members[0] as FakeRelayTransport;
    unwrap(host.broadcast("input", message));
    host.pump();
    const relay = host.wireCounters();

    const star = transport.fakeStar({ role: "host" });
    unwrap(star.initialize());
    for (let index = 2; index <= 8; index += 1) {
      const guest = transport.fakeStar({ role: "guest", peer_id: memberId(index) });
      unwrap(guest.initialize());
      unwrap(star.openPeer(memberId(index)));
      unwrap(star.link(guest));
    }
    unwrap(star.broadcast("input", message));
    const hub = star.wireCounters();

    expect(relay.uplink_units).toBe(1); // one relay upload
    expect(hub.uplink_units).toBe(7); // one star upload per guest link
    expect(hub.uplink_bytes).toBe(relay.uplink_bytes * 7); // the star pays the fan-out seven times
    // The room forwards the same bytes to seven members, so total traffic is
    // unchanged; only *where* it is paid moves.
    expect(hub.input_uplink_bytes).toBe(relay.input_uplink_bytes * 7);
  });

  it("lets any member address any other, which the star forbids", () => {
    const [, members] = buildRoom(3);
    expect((members[1] as FakeRelayTransport).send("guest_2", "control", control(0, "hello")).ok).toBe(
      true
    );

    const star = transport.fakeStar({ role: "guest", peer_id: "guest_1" });
    unwrap(star.initialize());
    const result = star.send("guest_2", "control", control(0, "hello"));
    expect(expectErr(result).code).toBe("role_forbidden"); // a star guest may only address the host
  });

  it("forwards a payload the room cannot possibly parse", () => {
    const [room, members] = buildRoom(2);
    // Deliberately not an input packet, and carrying the framing separators
    // the room concatenates with. If the room parsed anything, this breaks.
    const payload = "not|an|input|packet\nwith a newline and % escapes";
    unwrap((members[0] as FakeRelayTransport).send("guest_1", "input", input(7, 12, payload)));
    (members[0] as FakeRelayTransport).pump();
    const batch = (members[1] as FakeRelayTransport).pollBatch(16);
    expect(batch.length).toBe(1);
    expect(batch[0]?.message.payload).toBe(payload); // the relay returns the bytes it was handed
    expect(room.counters.lines).toBe(1);
    expect(room.counters.dropped).toBe(0);
  });

  it("refuses manual signaling instead of faking it", () => {
    const [, members] = buildRoom(2);
    const host = members[0] as FakeRelayTransport;
    const offer = expectErr(host.requestOffer("guest_1")).code;
    expect(offer).toBe("signal_error");
    const accepted = expectErr((members[1] as FakeRelayTransport).acceptOffer("offer:1")).code;
    expect(accepted).toBe("signal_error");
    const answered = expectErr(host.acceptAnswer("guest_1", "answer:offer:1")).code;
    expect(answered).toBe("signal_error");
    expect(unwrap(host.takeSignal("guest_1"))).toBeNull();
  });

  it("reports diagnostics the contract encoder round-trips", () => {
    const [, members] = buildRoom(3);
    const host = members[0] as FakeRelayTransport;
    unwrap(host.send("guest_1", "control", control(0, "hello")));
    const original = host.diagnostics();
    const decoded = unwrap(contract.decodeStarDiagnostics(contract.encodeStarDiagnostics(original)));
    expect(decoded.peer_count).toBe(2);
    expect(decoded.peers.length).toBe(2);
    expect(decoded.peers[0]?.peer_id).toBe("guest_1");
    expect(decoded.peers[0]?.slot).toBe(1);
    expect(decoded.peers[0]?.state).toBe("connected");
    expect(decoded.peers[0]?.control.outbound_depth).toBe(1); // the addressed unit shows on that link
    expect(decoded.peers[1]?.control.outbound_depth).toBe(0); // and only on that link
  });

  it("closes one member link without disturbing the room", () => {
    const [, members] = buildRoom(3);
    const host = members[0] as FakeRelayTransport;
    unwrap(host.closePeer("guest_1", "peer_left"));
    expect(host.peerState("guest_1")).toBe("closed");
    expect((members[1] as FakeRelayTransport).peerState("host")).toBe("disconnected");
    expect((members[2] as FakeRelayTransport).peerState("host")).toBe("connected");
    unwrap(host.broadcast("input", input(1, 40, "authority")));
    host.pump();
    expect((members[1] as FakeRelayTransport).pollBatch(16).length).toBe(0); // a closed link receives nothing
    expect((members[2] as FakeRelayTransport).pollBatch(16).length).toBe(1); // an untouched link is unaffected
  });

  it("drains and leaves the room on shutdown", () => {
    const [room, members] = buildRoom(3);
    const host = members[0] as FakeRelayTransport;
    unwrap(host.send("guest_1", "control", control(0, "hello")));
    unwrap(host.shutdown());
    expect(host.state()).toBe("closed");
    expect(host.diagnostics().peers.length).toBe(0); // no orphaned links survive teardown
    expect(room.members.length).toBe(2); // the member left the room
    expect((members[1] as FakeRelayTransport).peerState("host")).toBe("disconnected");
  });
});
