import { describe, expect, it } from "vitest";
import * as contract from "./contract.ts";
import * as transport from "./index.ts";
import { FakeStarTransport, type FakeStarTransportOptions } from "./fake_star.ts";
import { type StarEvalFn } from "./browser_star.ts";
import type {
  StarTransportAdapter,
  TransportChannel,
  TransportFailure,
  TransportMessage,
  TransportPeerDiagnostics,
  TransportPeerEvent,
  TransportResult,
} from "./contract.ts";

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

interface BuildStarOptions {
  readonly queue_limit?: number;
  readonly buffered_amount_limit?: number;
}

function buildStar(
  count: number,
  options: BuildStarOptions = {},
): readonly [FakeStarTransport, FakeStarTransport[]] {
  const starOptions: FakeStarTransportOptions = {
    ...(options.queue_limit !== undefined ? { queue_limit: options.queue_limit } : {}),
    ...(options.buffered_amount_limit !== undefined
      ? { buffered_amount_limit: options.buffered_amount_limit }
      : {}),
  };
  const host = transport.fakeStar(starOptions);
  unwrap(host.initialize());
  const guests: FakeStarTransport[] = [];
  for (let index = 1; index <= count; index += 1) {
    const peerId = `guest_${index}`;
    unwrap(host.openPeer(peerId));
    const guest = transport.fakeStar({ role: "guest", peer_id: peerId, ...starOptions });
    unwrap(guest.initialize());
    unwrap(host.link(guest));
    guests.push(guest);
  }
  return [host, guests];
}

function peerDiagnostics(star: StarTransportAdapter, peerId: string): TransportPeerDiagnostics {
  for (const peer of star.diagnostics().peers) {
    if (peer.peer_id === peerId) {
      return peer;
    }
  }
  throw new Error(`no diagnostics for peer ${peerId}`);
}

describe("star transport contract", () => {
  it("round-trips a peer-addressed envelope", () => {
    const msg = input(3, 90, "left|right\n%\xff");
    const line = unwrap(contract.encodeAddressed("guest_4", "input", msg));
    const addressed = unwrap(contract.decodeAddressed(line));
    expect(addressed.peer_id).toBe("guest_4");
    expect(addressed.channel).toBe("input");
    expect(addressed.message.tick).toBe(90);
    expect(addressed.message.payload).toBe(msg.payload);
  });

  it("pins the control and input channel configuration", () => {
    expect(contract.CHANNEL_CONFIG.control.ordered).toBe(true);
    expect(contract.CHANNEL_CONFIG.control.reliable).toBe(true);
    expect(contract.CHANNEL_CONFIG.input.ordered).toBe(false);
    expect(contract.CHANNEL_CONFIG.input.maxRetransmits).toBe(0);
    expect(contract.MAX_GUESTS).toBe(7);
  });

  it("rejects invalid peer ids, channels, and channel/type pairings", () => {
    expect(contract.validatePeerId("guest_1").ok).toBe(true);
    expect(expectErr(contract.validatePeerId("guest 1")).code).toBe("malformed");
    expect(expectErr(contract.validatePeerId("guest|1")).code).toBe("malformed");
    expect(expectErr(contract.validatePeerId("")).code).toBe("malformed");

    expect(expectErr(contract.validateChannel("gossip")).code).toBe("channel_mismatch");

    expect(expectErr(contract.validateChannelMessage("input", control(1))).code).toBe(
      "channel_mismatch",
    );
    expect(expectErr(contract.validateChannelMessage("control", input(1, 2))).code).toBe(
      "channel_mismatch",
    );
  });

  it("round-trips the star diagnostics record", () => {
    const [host] = buildStar(2);
    unwrap(host.send("guest_1", "control", control(0, "hello")));
    const original = host.diagnostics();
    const decoded = unwrap(
      contract.decodeStarDiagnostics(contract.encodeStarDiagnostics(original)),
    );
    expect(decoded.role).toBe(original.role);
    expect(decoded.capacity).toBe(original.capacity);
    expect(decoded.peer_count).toBe(2);
    expect(decoded.peers.length).toBe(2);
    expect(decoded.peers[0]?.peer_id).toBe("guest_1");
    expect(decoded.peers[0]?.slot).toBe(1);
    expect(decoded.peers[0]?.control.outbound_depth).toBe(
      original.peers[0]?.control.outbound_depth,
    );
    expect(decoded.peers[1]?.state).toBe("connected");
  });

  it("reports a malformed bridge diagnostics payload instead of throwing", () => {
    const broken = contract.decodeStarDiagnostics("star|host|connected");
    expect(expectErr(broken).code).toBe("bridge_error");
  });
});

describe("fake host-star transport", () => {
  it("holds one host and seven independently addressed guests", () => {
    const [host, guests] = buildStar(7);
    expect(host.capacity()).toBe(7);
    expect(host.peerIds().length).toBe(7);
    for (let index = 1; index <= 7; index += 1) {
      expect(host.peerState(`guest_${index}`)).toBe("connected");
      expect(guests[index - 1]?.peerState(contract.HOST_PEER_ID)).toBe("connected");
    }

    unwrap(host.send("guest_3", "control", control(0, "only-three")));
    host.pump();
    const addressed = unwrap((guests[2] as FakeStarTransport).poll());
    expect(addressed?.peer_id).toBe(contract.HOST_PEER_ID);
    expect(addressed?.channel).toBe("control");
    expect(addressed?.message.payload).toBe("only-three");
    for (let index = 1; index <= 7; index += 1) {
      if (index !== 3) {
        expect(unwrap((guests[index - 1] as FakeStarTransport).poll())).toBeNull();
      }
    }
  });

  it("rejects an eighth guest, a duplicate id, and the reserved host id", () => {
    const [host] = buildStar(7);
    const overflow = host.openPeer("guest_8");
    expect(expectErr(overflow).code).toBe("capacity");
    const duplicate = host.openPeer("guest_1");
    expect(expectErr(duplicate).code).toBe("duplicate_peer");
    const reserved = host.openPeer(contract.HOST_PEER_ID);
    expect(expectErr(reserved).code).toBe("duplicate_peer");
    expect(host.peerIds().length).toBe(7);
  });

  it("fans a canonical batch to every connected guest exactly once", () => {
    const [host, guests] = buildStar(7);
    expect(unwrap(host.broadcast("input", input(1, 120, "batch")))).toBe(7);
    host.pump();
    for (let index = 1; index <= 7; index += 1) {
      const guest = guests[index - 1] as FakeStarTransport;
      const addressed = unwrap(guest.poll());
      expect(addressed?.message.tick).toBe(120);
      expect(unwrap(guest.poll())).toBeNull();
    }
  });

  it("attributes guest traffic by link identity, never by payload", () => {
    const [host, guests] = buildStar(3);
    for (let index = 1; index <= 3; index += 1) {
      // Every guest claims to be guest_1 in its payload; the star must
      // report the link it actually arrived on.
      const guest = guests[index - 1] as FakeStarTransport;
      unwrap(guest.send(contract.HOST_PEER_ID, "input", input(0, 10, "guest_1")));
      guest.pump();
    }
    const seen = new Set<string>();
    for (let i = 0; i < 3; i += 1) {
      const addressed = unwrap(host.poll());
      expect(addressed).not.toBeNull();
      const peerId = addressed?.peer_id as string;
      expect(seen.has(peerId)).toBe(false);
      seen.add(peerId);
    }
    expect(seen.has("guest_1") && seen.has("guest_2") && seen.has("guest_3")).toBe(true);
    expect(unwrap(host.poll())).toBeNull();
  });

  it("lets only the host fan out and only guests address the host", () => {
    const [host, guests] = buildStar(2);
    const guest = guests[0] as FakeStarTransport;

    const fanned = guest.broadcast("input", input(0, 1, "x"));
    expect(expectErr(fanned).code).toBe("role_forbidden");

    const crossed = guest.send("guest_2", "control", control(0));
    expect(expectErr(crossed).code).toBe("role_forbidden");

    const opened = guest.openPeer("guest_9");
    expect(expectErr(opened).code).toBe("role_forbidden");

    expect(unwrap(host.send("guest_1", "control", control(0)))).toBe(true);
  });

  it("keeps input off the reliable channel and control off the lossy channel", () => {
    const [host] = buildStar(1);
    const reliable = host.send("guest_1", "control", input(0, 5));
    expect(expectErr(reliable).code).toBe("channel_mismatch");

    const lossy = host.send("guest_1", "input", control(0));
    expect(expectErr(lossy).code).toBe("channel_mismatch");

    expect(host.diagnostics().malformed).toBe(2);
    expect(peerDiagnostics(host, "guest_1").malformed).toBe(2);
  });

  it("reports an unknown peer without throwing", () => {
    const [host] = buildStar(1);
    const sent = host.send("nobody", "control", control(0));
    expect(expectErr(sent).code).toBe("unknown_peer");
  });

  it("bounds the outbound queue and drops the overflow", () => {
    const [host] = buildStar(1, { queue_limit: 4 });
    for (let seq = 0; seq <= 3; seq += 1) {
      expect(unwrap(host.send("guest_1", "control", control(seq)))).toBe(true);
    }
    const overflowed = host.send("guest_1", "control", control(4));
    expect(expectErr(overflowed).code).toBe("overflow");

    const diagnostics = host.diagnostics();
    expect(diagnostics.peers[0]?.control.outbound_depth).toBe(4);
    expect(diagnostics.peers[0]?.control.dropped_outbound).toBe(1);
    expect(diagnostics.dropped_outbound).toBe(1);
    expect(diagnostics.overflow).toBe(1);
  });

  it("bounds the inbound queue when the peer never polls", () => {
    const [host, guests] = buildStar(1, { queue_limit: 3 });
    for (let seq = 0; seq <= 2; seq += 1) {
      unwrap(host.send("guest_1", "input", input(seq, 100 + seq)));
    }
    host.pump();
    unwrap(host.send("guest_1", "input", input(3, 103)));
    host.pump();

    const diagnostics = (guests[0] as FakeStarTransport).diagnostics();
    expect(diagnostics.peers[0]?.input.inbound_depth).toBe(3);
    expect(diagnostics.peers[0]?.input.dropped_inbound).toBe(1);
    expect(diagnostics.overflow).toBeGreaterThanOrEqual(1);
  });

  it("queues instead of blocking when the send buffer is saturated", () => {
    const [host] = buildStar(1, { queue_limit: 8, buffered_amount_limit: 1 });
    expect(unwrap(host.send("guest_1", "control", control(0, "payload")))).toBe(true);
    expect(peerDiagnostics(host, "guest_1").control.buffered_amount).toBeGreaterThan(0);

    host.pump();
    const diagnostics = host.diagnostics();
    expect(diagnostics.peers[0]?.control.outbound_depth).toBe(1);
    expect(diagnostics.backpressure).toBe(1);
    expect(diagnostics.peers[0]?.backpressure).toBe(1);

    // The saturated buffer costs one event, not one per drain pass.
    host.pump();
    host.pump();
    expect(host.diagnostics().backpressure).toBe(1);

    let backpressure: TransportPeerEvent | null = null;
    let event = host.pollEvent();
    while (event) {
      if (event.code === "backpressure") {
        backpressure = event;
      }
      event = host.pollEvent();
    }
    expect(backpressure?.kind).toBe("peer_error");
    expect(backpressure?.peer_id).toBe("guest_1");
    expect(backpressure?.channel).toBe("control");
  });

  it("re-arms the backpressure latch after the channel drains", () => {
    // A budget that fits some but not all of a burst, so congestion can be
    // relieved and then genuinely recur. A latch that never re-armed would
    // hide the second congestion entirely.
    const [host, guests] = buildStar(1, { queue_limit: 16, buffered_amount_limit: 40 });
    function drainEvents(): number {
      const codes: (string | undefined)[] = [];
      let event = host.pollEvent();
      while (event) {
        codes.push(event.code);
        event = host.pollEvent();
      }
      return codes.filter((code) => code === "backpressure").length;
    }
    drainEvents();

    for (let seq = 0; seq <= 2; seq += 1) {
      unwrap(host.send("guest_1", "control", control(seq, "payload")));
    }
    host.pump();
    const congested = host.diagnostics();
    expect(congested.peers[0]?.control.outbound_depth).toBeGreaterThan(0);
    expect(congested.backpressure).toBe(1);
    expect(drainEvents()).toBe(1);

    // Relieve: the next pass has a fresh budget and empties the queue.
    host.pump();
    const relieved = host.diagnostics();
    expect(relieved.peers[0]?.control.outbound_depth).toBe(0);
    expect(relieved.backpressure).toBe(1);
    expect(drainEvents()).toBe(0);
    // Everything the host queued actually arrived.
    expect((guests[0] as FakeStarTransport).pollBatch(8).length).toBe(3);

    // Re-saturate: a genuine second congestion is reported again.
    for (let seq = 3; seq <= 5; seq += 1) {
      unwrap(host.send("guest_1", "control", control(seq, "payload")));
    }
    host.pump();
    expect(host.diagnostics().backpressure).toBe(2);
    expect(drainEvents()).toBe(1);
  });

  it("drains peers and channels in a fixed order without starving a peer", () => {
    const [host, guests] = buildStar(3);
    for (let index = 1; index <= 3; index += 1) {
      const guest = guests[index - 1] as FakeStarTransport;
      unwrap(guest.send(contract.HOST_PEER_ID, "control", control(0, "c")));
      unwrap(guest.send(contract.HOST_PEER_ID, "input", input(0, 7, "i")));
      unwrap(guest.send(contract.HOST_PEER_ID, "input", input(1, 8, "i")));
      guest.pump();
    }

    const batch = host.pollBatch(6);
    expect(batch.length).toBe(6);
    const expected: readonly (readonly [string, TransportChannel])[] = [
      ["guest_1", "control"],
      ["guest_1", "input"],
      ["guest_2", "control"],
      ["guest_2", "input"],
      ["guest_3", "control"],
      ["guest_3", "input"],
    ];
    batch.forEach((entry, index) => {
      const [peerId, channel] = expected[index] as readonly [string, TransportChannel];
      expect(entry.peer_id).toBe(peerId);
      expect(entry.channel).toBe(channel);
    });
    expect(batch[0]?.arrival_seq).toBe(1);

    // The cursor persists, so the second pass resumes the same rotation.
    const rest = host.pollBatch(8);
    expect(rest.length).toBe(3);
    for (const entry of rest) {
      expect(entry.channel).toBe("input");
    }
  });

  it("reports per-peer sequence gaps", () => {
    const [host, guests] = buildStar(1);
    unwrap(host.send("guest_1", "input", input(0, 10)));
    unwrap(host.send("guest_1", "input", input(5, 15)));
    host.pump();
    expect(
      peerDiagnostics(guests[0] as FakeStarTransport, contract.HOST_PEER_ID).sequence_gaps,
    ).toBe(4);
  });

  it("closes one link without disturbing the others", () => {
    const [host, guests] = buildStar(3);
    unwrap(host.send("guest_2", "control", control(0)));
    unwrap(host.closePeer("guest_2", "peer left"));

    expect(host.peerState("guest_2")).toBe("closed");
    expect(host.peerState("guest_1")).toBe("connected");
    expect(host.peerState("guest_3")).toBe("connected");
    expect((guests[1] as FakeStarTransport).peerState(contract.HOST_PEER_ID)).toBe("disconnected");

    const blocked = host.send("guest_2", "control", control(1));
    expect(expectErr(blocked).code).toBe("not_connected");
    expect(unwrap(host.send("guest_1", "control", control(1)))).toBe(true);
    expect(unwrap(host.broadcast("input", input(2, 30)))).toBe(2);

    expect(peerDiagnostics(host, "guest_2").control.outbound_depth).toBe(0);
    expect(unwrap(host.closePeer("guest_2"))).toBe(true);
  });

  it("tears down every link and survives a repeated shutdown", () => {
    const [host, guests] = buildStar(7);
    unwrap(host.broadcast("input", input(0, 5)));
    unwrap(host.shutdown());
    unwrap(host.shutdown());

    const diagnostics = host.diagnostics();
    expect(diagnostics.state).toBe("closed");
    expect(diagnostics.peer_count).toBe(0);
    expect(diagnostics.peers.length).toBe(0);

    const sent = host.send("guest_1", "control", control(0));
    expect(expectErr(sent).code).toBe("closed");
    expect(host.pollBatch(8).length).toBe(0);
    for (let index = 1; index <= 7; index += 1) {
      expect((guests[index - 1] as FakeStarTransport).peerState(contract.HOST_PEER_ID)).toBe(
        "disconnected",
      );
    }
  });

  it("refuses traffic before initialize", () => {
    const host = transport.fakeStar();
    const sent = host.send("guest_1", "control", control(0));
    expect(expectErr(sent).code).toBe("not_initialized");
  });

  it("completes a manual offer/answer handshake in process", () => {
    const rendezvous = transport.fakeStarRendezvous();
    const host = transport.fakeStar({ rendezvous });
    unwrap(host.initialize());
    unwrap(host.openPeer("guest_1"));
    const guest = transport.fakeStar({
      role: "guest",
      peer_id: "guest_1",
      rendezvous,
    });
    unwrap(guest.initialize());

    expect(unwrap(host.takeSignal("guest_1"))).toBeNull();
    expect(unwrap(host.requestOffer("guest_1"))).toBe(true);
    const offer = unwrap(host.takeSignal("guest_1"));
    expect(unwrap(host.takeSignal("guest_1"))).toBeNull(); // a signal is handed out once

    expect(unwrap(guest.acceptOffer(offer as string))).toBe(true);
    const answer = unwrap(guest.takeSignal(contract.HOST_PEER_ID));
    expect(unwrap(host.acceptAnswer("guest_1", answer as string))).toBe(true);

    expect(host.peerState("guest_1")).toBe("connected");
    expect(guest.peerState(contract.HOST_PEER_ID)).toBe("connected");
    unwrap(host.send("guest_1", "control", control(0, "after-handshake")));
    host.pump();
    expect(unwrap(guest.poll())?.message.payload).toBe("after-handshake");
  });

  it("keeps two stars in one process from sharing signaling state", () => {
    // A harness that runs a host plus seven guests in a single process is
    // proving those clients converge WITHOUT shared mutable state. A token
    // minted by one star must be meaningless to another star's guest, or
    // such a harness can pass for the wrong reason.
    const first = transport.fakeStarRendezvous();
    const second = transport.fakeStarRendezvous();

    const hostA = transport.fakeStar({ rendezvous: first });
    unwrap(hostA.initialize());
    unwrap(hostA.openPeer("guest_1"));
    const hostB = transport.fakeStar({ rendezvous: second });
    unwrap(hostB.initialize());
    unwrap(hostB.openPeer("guest_1"));

    const guestB = transport.fakeStar({
      role: "guest",
      peer_id: "guest_1",
      rendezvous: second,
    });
    unwrap(guestB.initialize());

    unwrap(hostA.requestOffer("guest_1"));
    const foreign = unwrap(hostA.takeSignal("guest_1"));

    // The other star's guest must not accept star A's token.
    const crossed = guestB.acceptOffer(foreign as string);
    expect(expectErr(crossed).code).toBe("signal_error");
    expect(guestB.peerState(contract.HOST_PEER_ID)).toBe("connecting");
    expect(hostA.peerState("guest_1")).toBe("connecting");

    // And star B's own handshake still completes independently.
    unwrap(hostB.requestOffer("guest_1"));
    const own = unwrap(hostB.takeSignal("guest_1"));
    unwrap(guestB.acceptOffer(own as string));
    const answer = unwrap(guestB.takeSignal(contract.HOST_PEER_ID));
    unwrap(hostB.acceptAnswer("guest_1", answer as string));
    expect(hostB.peerState("guest_1")).toBe("connected");
    // Star A is untouched by star B completing.
    expect(hostA.peerState("guest_1")).toBe("connecting");
  });

  it("gives an endpoint a private rendezvous when none is supplied", () => {
    // The default must not be a shared registry: two endpoints built
    // without an explicit rendezvous cannot see each other's signals.
    const host = transport.fakeStar();
    unwrap(host.initialize());
    unwrap(host.openPeer("guest_1"));
    const guest = transport.fakeStar({ role: "guest", peer_id: "guest_1" });
    unwrap(guest.initialize());

    unwrap(host.requestOffer("guest_1"));
    const token = unwrap(host.takeSignal("guest_1"));
    const accepted = guest.acceptOffer(token as string);
    expect(expectErr(accepted).code).toBe("signal_error");
  });

  it("rejects signaling misuse with typed errors", () => {
    const rendezvous = transport.fakeStarRendezvous();
    const host = transport.fakeStar({ rendezvous });
    unwrap(host.initialize());
    unwrap(host.openPeer("guest_1"));
    const guest = transport.fakeStar({
      role: "guest",
      peer_id: "guest_1",
      rendezvous,
    });
    unwrap(guest.initialize());

    const guestOffer = guest.requestOffer("guest_1");
    expect(expectErr(guestOffer).code).toBe("role_forbidden");

    const hostAccept = host.acceptOffer("anything");
    expect(expectErr(hostAccept).code).toBe("role_forbidden");

    const unknown = host.requestOffer("nobody");
    expect(expectErr(unknown).code).toBe("unknown_peer");

    const bogus = guest.acceptOffer("not-a-real-offer");
    expect(expectErr(bogus).code).toBe("signal_error");

    unwrap(host.requestOffer("guest_1"));
    const stale = host.acceptAnswer("guest_1", "answer:offer:nonexistent");
    expect(expectErr(stale).code).toBe("signal_error");
  });
});

function starBridge(host: FakeStarTransport): StarEvalFn {
  function unescapeHex(value: string): string {
    return value.replace(/%([0-9A-Fa-f]{2})/g, (_all, hex: string) =>
      String.fromCharCode(parseInt(hex, 16)),
    );
  }

  // The real bridge percent-decodes quoted arguments, except for the
  // addressed and broadcast wires, which it forwards verbatim. Mirror both.
  function parseArguments(argument: string): readonly [string[], string[]] {
    const values: string[] = [];
    const raw: string[] = [];
    if (argument === "") {
      return [values, raw];
    }
    for (const field of (argument + ",").matchAll(/([^,]*),/g)) {
      const chunk = field[1] as string;
      const quoted = /^'([\s\S]*)'$/.exec(chunk);
      raw.push(quoted ? (quoted[1] as string) : chunk);
      values.push(quoted ? unescapeHex(quoted[1] as string) : chunk);
    }
    return [values, raw];
  }

  function escapeHex(value: string): string {
    let out = "";
    for (let index = 0; index < value.length; index += 1) {
      const char = value[index] as string;
      if (/^[A-Za-z0-9\-._~]$/.test(char)) {
        out += char;
      } else {
        out += "%" + (value.charCodeAt(index) & 0xff).toString(16).toUpperCase().padStart(2, "0");
      }
    }
    return out;
  }

  function bridgeError(code: string | undefined, error: string | undefined): string {
    return `error|${code}|${escapeHex(error ?? "error")}`;
  }

  return (command: string) => {
    const match = /^window\.GoliseoStarTransport\.([A-Za-z0-9_]+)\((.*)\)$/.exec(command);
    if (!match) {
      throw new Error(`unexpected browser star command: ${command}`);
    }
    const name = match[1] as string;
    const argument = match[2] as string;
    const [args, raw] = parseArguments(argument);
    if (name === "initialize") {
      unwrap(host.initialize());
      return ["star|connected", null] as const;
    } else if (name === "shutdown") {
      unwrap(host.shutdown());
      return ["star|closed", null] as const;
    } else if (name === "open_peer") {
      const result = host.openPeer(args[0] as string);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      return [`slot|${result.value}`, null] as const;
    } else if (name === "close_peer") {
      const result = host.closePeer(args[0] as string, args[1]);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      return ["ok", null] as const;
    } else if (name === "request_offer") {
      const result = host.requestOffer(args[0] as string);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      return ["ok", null] as const;
    } else if (name === "accept_offer") {
      const result = host.acceptOffer(args[0] as string);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      return ["ok", null] as const;
    } else if (name === "accept_answer") {
      const result = host.acceptAnswer(args[0] as string, args[1] as string);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      return ["ok", null] as const;
    } else if (name === "take_signal") {
      const result = host.takeSignal(args[0] as string);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      if (result.value === null) {
        return ["", null] as const;
      }
      return [`signal|${escapeHex(result.value)}`, null] as const;
    } else if (name === "send") {
      const addressedResult = contract.decodeAddressed(raw[0]);
      if (!addressedResult.ok) {
        return [
          bridgeError(addressedResult.error.code, addressedResult.error.message),
          null,
        ] as const;
      }
      const addressed = addressedResult.value;
      const result = host.send(addressed.peer_id, addressed.channel, addressed.message);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      // The real bridge drains the data channel inside the same call.
      host.pump();
      return ["ok", null] as const;
    } else if (name === "broadcast") {
      const broadcastMatch = /^([^|]*)\|([\s\S]*)$/.exec(raw[0] as string);
      const channel = broadcastMatch?.[1] as TransportChannel;
      const wire = broadcastMatch?.[2] as string;
      const message = unwrap(contract.decode(wire));
      const result = host.broadcast(channel, message);
      if (!result.ok) {
        return [bridgeError(result.error.code, result.error.message), null] as const;
      }
      host.pump();
      return [`delivered|${result.value}`, null] as const;
    } else if (name === "poll") {
      host.pump();
      const entry = unwrap(host.poll());
      if (!entry) {
        return ["", null] as const;
      }
      return [
        unwrap(contract.encodeAddressed(entry.peer_id, entry.channel, entry.message)),
        null,
      ] as const;
    } else if (name === "poll_event") {
      const event = host.pollEvent();
      if (!event) {
        return ["", null] as const;
      }
      if (event.kind === "star_state") {
        return [`star_state|${event.state}`, null] as const;
      } else if (event.kind === "peer_state") {
        return [`peer_state|${event.peer_id}|${event.state}`, null] as const;
      } else if (event.kind === "star_error") {
        return [`star_error|${event.code}|`, null] as const;
      }
      return [
        ["peer_error", event.peer_id, event.channel ?? "", event.code, ""].join("|"),
        null,
      ] as const;
    } else if (name === "diagnostics") {
      host.pump();
      return [contract.encodeStarDiagnostics(host.diagnostics()), null] as const;
    }
    throw new Error(`unexpected browser star method: ${name}`);
  };
}

describe("browser host-star transport", () => {
  it("mirrors the fake adapter through the JavaScript host seam", () => {
    const [reference, guests] = buildStar(3);
    const browser = transport.browserStar({ eval: starBridge(reference) });
    expect(unwrap(browser.initialize())).toBe(true);
    expect(browser.role()).toBe("host");
    expect(browser.capacity()).toBe(7);

    expect(unwrap(browser.send("guest_2", "control", control(0, "addressed")))).toBe(true);
    const addressed = unwrap((guests[1] as FakeStarTransport).poll());
    expect(addressed?.message.payload).toBe("addressed");

    expect(unwrap(browser.broadcast("input", input(1, 44, "batch")))).toBe(3);
    for (let index = 1; index <= 3; index += 1) {
      const guest = guests[index - 1] as FakeStarTransport;
      expect(unwrap(guest.poll())?.message.tick).toBe(44);
      unwrap(guest.send(contract.HOST_PEER_ID, "input", input(0, 45, "reply")));
      guest.pump();
    }

    const batch = browser.pollBatch(3);
    expect(batch.length).toBe(3);
    expect(batch[0]?.peer_id).toBe("guest_1");
    expect(batch[0]?.channel).toBe("input");
    expect(batch[0]?.arrival_seq).toBe(1);
    expect(batch[2]?.peer_id).toBe("guest_3");

    const diagnostics = browser.diagnostics();
    expect(diagnostics.role).toBe("host");
    expect(diagnostics.peer_count).toBe(3);
    expect(diagnostics.capacity).toBe(7);
    expect(diagnostics.peers.length).toBe(3);
    expect(diagnostics.peers[0]?.state).toBe("connected");
    expect(browser.peerState("guest_1")).toBe("connected");
  });

  it("surfaces bridge-reported failures as typed transport errors", () => {
    const [reference] = buildStar(7);
    const browser = transport.browserStar({ eval: starBridge(reference) });
    unwrap(browser.initialize());

    const slot = browser.openPeer("guest_8");
    expect(expectErr(slot).code).toBe("capacity");

    const mismatch = browser.send("guest_1", "control", input(0, 3));
    expect(expectErr(mismatch).code).toBe("channel_mismatch");

    const unknown = browser.send("nobody", "control", control(0));
    expect(expectErr(unknown).code).toBe("unknown_peer");
  });

  it("enforces role permissions before reaching the bridge", () => {
    const [reference] = buildStar(2);
    const guest = transport.browserStar({ role: "guest", eval: starBridge(reference) });
    unwrap(guest.initialize());
    expect(guest.capacity()).toBe(1);

    const fanned = guest.broadcast("input", input(0, 1));
    expect(expectErr(fanned).code).toBe("role_forbidden");

    const crossed = guest.send("guest_2", "control", control(0));
    expect(expectErr(crossed).code).toBe("role_forbidden");

    const opened = guest.openPeer("guest_9");
    expect(expectErr(opened).code).toBe("role_forbidden");
  });

  it("reports a missing bridge instead of throwing", () => {
    const browser = transport.browserStar({
      eval: () => [null, "love.js.eval is not available"] as const,
    });
    const initResult = browser.initialize();
    expect(expectErr(initResult).code).toBe("bridge_error");

    const fallback = browser.diagnostics();
    expect(fallback.state).toBe("new");
    expect(fallback.peer_count).toBe(0);
    expect(fallback.last_error).not.toBeNull();
  });

  it("drives the manual handshake through the bridge", () => {
    const rendezvous = transport.fakeStarRendezvous();
    const reference = transport.fakeStar({ rendezvous });
    const browser = transport.browserStar({ eval: starBridge(reference) });
    unwrap(browser.initialize());
    unwrap(browser.openPeer("guest_1"));

    expect(unwrap(browser.takeSignal("guest_1"))).toBeNull();
    expect(unwrap(browser.requestOffer("guest_1"))).toBe(true);
    const offer = unwrap(browser.takeSignal("guest_1"));
    expect(unwrap(browser.takeSignal("guest_1"))).toBeNull();

    // The guest side runs against its own endpoint and bridge.
    const guestReference = transport.fakeStar({
      role: "guest",
      peer_id: "guest_1",
      rendezvous,
    });
    const guest = transport.browserStar({
      role: "guest",
      eval: starBridge(guestReference),
    });
    unwrap(guest.initialize());
    expect(unwrap(guest.acceptOffer(offer as string))).toBe(true);
    const answer = unwrap(guest.takeSignal(contract.HOST_PEER_ID));
    expect(unwrap(browser.acceptAnswer("guest_1", answer as string))).toBe(true);
    expect(reference.peerState("guest_1")).toBe("connected");
  });

  it("round-trips a signal blob with SDP characters through the eval seam", () => {
    // A real SDP blob carries newlines, quotes, equals signs, and commas.
    // Everything crossing the eval seam has to survive them intact.
    const blob = '{"type":"offer","sdp":"v=0\r\na=candidate:1 1 udp 2 10.0.0.1 5,6\'\\"}';
    let captured: string | null = null;
    const browser = transport.browserStar({
      eval: (command: string) => {
        const match = /^window\.GoliseoStarTransport\.([A-Za-z0-9_]+)\((.*)\)$/.exec(command);
        const name = match?.[1];
        const argument = match?.[2] ?? "";
        if (name === "initialize") {
          return ["star|connected", null] as const;
        } else if (name === "open_peer") {
          return ["slot|1", null] as const;
        } else if (name === "accept_answer") {
          captured = argument;
          return ["ok", null] as const;
        } else if (name === "take_signal") {
          let escaped = "";
          for (let index = 0; index < blob.length; index += 1) {
            const char = blob[index] as string;
            if (/^[A-Za-z0-9\-._~]$/.test(char)) {
              escaped += char;
            } else {
              escaped +=
                "%" + (blob.charCodeAt(index) & 0xff).toString(16).toUpperCase().padStart(2, "0");
            }
          }
          return [`signal|${escaped}`, null] as const;
        }
        throw new Error(`unexpected command: ${command}`);
      },
    });
    unwrap(browser.initialize());
    unwrap(browser.openPeer("guest_1"));
    expect(unwrap(browser.takeSignal("guest_1"))).toBe(blob);

    unwrap(browser.acceptAnswer("guest_1", blob));
    // The command the bridge received must contain no raw quote, comma, or
    // newline that could terminate or extend the command string.
    const quotedMatch = /^'guest_1','([\s\S]*)'$/.exec(captured as unknown as string);
    expect(quotedMatch).not.toBeNull();
    expect((quotedMatch?.[1] as string).search(/[',\r\n\\]/)).toBe(-1);
  });

  it("keeps a wire payload injection-safe across the eval seam", () => {
    const hostile = "');window.owned=1;('";
    let captured: string | null = null;
    const browser = transport.browserStar({
      eval: (command: string) => {
        const match = /^window\.GoliseoStarTransport\.([A-Za-z0-9_]+)\((.*)\)$/.exec(command);
        const name = match?.[1];
        const argument = match?.[2] ?? "";
        if (name === "initialize") {
          return ["star|connected", null] as const;
        } else if (name === "open_peer") {
          return ["slot|1", null] as const;
        } else if (name === "send" || name === "broadcast") {
          captured = argument;
          return ["ok", null] as const;
        }
        throw new Error(`unexpected command: ${command}`);
      },
    });
    unwrap(browser.initialize());
    unwrap(browser.openPeer("guest_1"));
    unwrap(browser.send("guest_1", "control", control(0, hostile)));
    const wireMatch = /^'([\s\S]*)'$/.exec(captured as unknown as string);
    expect(wireMatch).not.toBeNull();
    expect((wireMatch?.[1] as string).includes("'")).toBe(false);
    // The payload still round-trips through the addressed decoder.
    const addressed = unwrap(contract.decodeAddressed(wireMatch?.[1] as string));
    expect(addressed.message.payload).toBe(hostile);
  });

  it("makes a locally rejected call as observable as a bridge rejection", () => {
    // The fake records role violations through its error path; the browser
    // adapter rejects them before the bridge is reached, so it has to queue
    // an equivalent event and own last_error rather than leaving whatever
    // the bridge last said.
    const [reference] = buildStar(2);
    const guestReference = transport.fakeStar({ role: "guest", peer_id: "guest_1" });
    const guest = transport.browserStar({
      role: "guest",
      eval: starBridge(guestReference),
    });
    unwrap(guest.initialize());

    const fanned = guest.broadcast("input", input(0, 1));
    expect(expectErr(fanned).code).toBe("role_forbidden");

    const event = guest.pollEvent();
    expect(event?.kind).toBe("star_error");
    expect(event?.code).toBe("role_forbidden");
    expect(guest.diagnostics().last_error?.includes("fans out")).toBe(true);

    // The fake reports the same fault the same way.
    const fakeGuest = transport.fakeStar({ role: "guest", peer_id: "guest_2" });
    unwrap(fakeGuest.initialize());
    const fakeFanned = fakeGuest.broadcast("input", input(0, 1));
    expect(expectErr(fakeFanned).code).toBe("role_forbidden");
    let fakeEvent: TransportPeerEvent | null = null;
    let nextEvent = fakeGuest.pollEvent();
    while (nextEvent) {
      if (nextEvent.code === "role_forbidden") {
        fakeEvent = nextEvent;
      }
      nextEvent = fakeGuest.pollEvent();
    }
    expect(fakeEvent?.kind).toBe("star_error");
    expect(fakeGuest.diagnostics().last_error?.includes("fans out")).toBe(true);
    expect(reference).not.toBeNull();
  });

  it("judges message shape before peer resolution on both adapters", () => {
    // A call that is BOTH addressed to an unknown peer AND carries a
    // message the channel cannot take must report the same code either way.
    // Shape wins: only the bridge can authoritatively resolve a peer, so an
    // adapter must never answer `unknown_peer` from a cached peer table.
    const [fake] = buildStar(1);
    const fakeCode = expectErr(fake.send("nobody", "control", input(0, 3))).code;
    expect(fakeCode).toBe("channel_mismatch");

    const [reference] = buildStar(1);
    const browser = transport.browserStar({ eval: starBridge(reference) });
    unwrap(browser.initialize());
    const browserCode = expectErr(browser.send("nobody", "control", input(0, 3))).code;
    expect(browserCode).toBe("channel_mismatch");
    expect(browserCode).toBe(fakeCode);

    // A well-shaped message to an unknown peer still reports unknown_peer.
    const fakeUnknown = expectErr(fake.send("nobody", "control", control(0))).code;
    expect(fakeUnknown).toBe("unknown_peer");
    const browserUnknown = expectErr(browser.send("nobody", "control", control(0))).code;
    expect(browserUnknown).toBe("unknown_peer");
  });

  it("attributes a combined-fault event the same way on both adapters", () => {
    function lastErrorEvent(star: StarTransportAdapter): TransportPeerEvent | null {
      let found: TransportPeerEvent | null = null;
      let event = star.pollEvent();
      while (event) {
        if (event.kind === "peer_error" || event.kind === "star_error") {
          found = event;
        }
        event = star.pollEvent();
      }
      return found;
    }

    const [fake] = buildStar(1);
    const [reference] = buildStar(1);
    const browser = transport.browserStar({ eval: starBridge(reference) });
    unwrap(browser.initialize());
    unwrap(browser.openPeer("guest_2"));

    // Shape fault aimed at an OPEN link: attributed to that link.
    lastErrorEvent(fake);
    expect(fake.send("guest_1", "control", input(0, 3)).ok).toBe(false);
    const fakeKnown = lastErrorEvent(fake);
    expect(fakeKnown?.kind).toBe("peer_error");
    expect(fakeKnown?.peer_id).toBe("guest_1");

    lastErrorEvent(browser);
    expect(browser.send("guest_2", "control", input(0, 3)).ok).toBe(false);
    const browserKnown = lastErrorEvent(browser);
    expect(browserKnown?.kind).toBe("peer_error");
    expect(browserKnown?.peer_id).toBe("guest_2");

    // Shape fault aimed at a peer that was NEVER opened: the link is not
    // known, so neither adapter may tag the event with a peer id.
    lastErrorEvent(fake);
    expect(fake.send("nobody", "control", input(0, 3)).ok).toBe(false);
    const fakeUnknown = lastErrorEvent(fake);
    expect(fakeUnknown?.kind).toBe("star_error");
    expect(fakeUnknown?.peer_id).toBeUndefined();

    lastErrorEvent(browser);
    expect(browser.send("nobody", "control", input(0, 3)).ok).toBe(false);
    const browserUnknown = lastErrorEvent(browser);
    expect(browserUnknown?.kind).toBe("star_error");
    expect(browserUnknown?.peer_id).toBeUndefined();

    expect(browserUnknown?.kind).toBe(fakeUnknown?.kind);
    expect(browserKnown?.kind).toBe(fakeKnown?.kind);
  });

  it("numbers arrival_seq per peer at poll time on both adapters", () => {
    const [fake, fakeGuests] = buildStar(2);
    for (let index = 1; index <= 2; index += 1) {
      const guest = fakeGuests[index - 1] as FakeStarTransport;
      unwrap(guest.send(contract.HOST_PEER_ID, "control", control(0, "a")));
      unwrap(guest.send(contract.HOST_PEER_ID, "input", input(0, 1, "b")));
      guest.pump();
    }
    const fakeBatch = fake.pollBatch(4);

    const [reference, guests] = buildStar(2);
    const browser = transport.browserStar({ eval: starBridge(reference) });
    unwrap(browser.initialize());
    for (let index = 1; index <= 2; index += 1) {
      const guest = guests[index - 1] as FakeStarTransport;
      unwrap(guest.send(contract.HOST_PEER_ID, "control", control(0, "a")));
      unwrap(guest.send(contract.HOST_PEER_ID, "input", input(0, 1, "b")));
      guest.pump();
    }
    const browserBatch = browser.pollBatch(4);

    expect(fakeBatch.length).toBe(4);
    expect(browserBatch.length).toBe(4);
    for (let index = 0; index < 4; index += 1) {
      expect(fakeBatch[index]?.peer_id).toBe(browserBatch[index]?.peer_id);
      expect(fakeBatch[index]?.channel).toBe(browserBatch[index]?.channel);
      expect(fakeBatch[index]?.arrival_seq).toBe(browserBatch[index]?.arrival_seq);
    }
    // Two peers, two messages each, numbered 1 and 2 within each peer.
    expect(fakeBatch[0]?.arrival_seq).toBe(1);
    expect(fakeBatch[1]?.arrival_seq).toBe(2);
    expect(fakeBatch[2]?.arrival_seq).toBe(1);
    expect(fakeBatch[3]?.arrival_seq).toBe(2);
  });

  it("closes peers and tears the star down through the bridge", () => {
    const [reference] = buildStar(3);
    const browser = transport.browserStar({ eval: starBridge(reference) });
    unwrap(browser.initialize());
    unwrap(browser.closePeer("guest_2", "peer left"));
    expect(browser.peerState("guest_2")).toBe("closed");

    unwrap(browser.shutdown());
    expect(browser.state()).toBe("closed");
    unwrap(browser.shutdown());
    expect(browser.peerIds().length).toBe(0);

    const sent = browser.send("guest_1", "control", control(0));
    expect(expectErr(sent).code).toBe("closed");
  });
});

describe("transport layering", () => {
  it.skip('keeps browser and WebRTC APIs out of core, data, and sim -- no TS equivalent: this scans the Lua source tree\'s core/data/sim directories via love.filesystem for forbidden identifiers (love.js.eval, RTCPeerConnection, Goliseo*, RTCDataChannel) and for `require("game.")`. There is no analogous Lua directory tree, love.filesystem API, or dynamic `require` in this TS package to scan; the layering it protects (sim never depending on browser/DOM/WebRTC) is enforced here instead by the TypeScript project boundaries (gc-sim is a separate Rust crate that never imports this package, and this package has no dependency back onto it).', () => {
    // Intentionally empty -- see the skip reason above.
  });
});
