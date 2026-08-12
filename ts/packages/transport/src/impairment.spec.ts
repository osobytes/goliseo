// Behaviour of the impairment decorator, asserted through OBSERVABLE
// consequences at the far end of a real adapter: a message that arrives
// later than it was sent, a named seed dropping exactly the packets it is
// supposed to drop, a duplicate arriving twice, a specific out-of-order
// permutation.
//
// "The wrapper returned ok" proves nothing and is deliberately not what any
// test here asserts. `impairment_parity.spec.ts` carries the cross-language
// half; this file carries the properties a reader can check by eye.

import { describe, expect, it } from "vitest";
import * as contract from "./contract.ts";
import { FakeStarTransport } from "./fake_star.ts";
import { FakeTransport } from "./fake.ts";
import { ImpairmentLink, impaired, impairedStar, type ImpairmentOptions } from "./impairment.ts";
import { RNG_MOD, RNG_MULT, rngRoll, rngSeed } from "./impairment_rng.ts";
import {
  NETWORK_PROFILES,
  NETWORK_PROFILE_NAMES,
  networkProfile,
  type NetworkProfile,
} from "./network_profiles.ts";
import type { TransportMessage, TransportResult } from "./contract.ts";

function unwrap<T>(result: TransportResult<T>): T {
  if (!result.ok) {
    throw new Error(`${result.error.code}: ${result.error.message}`);
  }
  return result.value;
}

function packet(seq: number): TransportMessage {
  return unwrap(contract.newMessage({ type: "input", seq, tick: seq, payload: `p${seq}` }));
}

function connectedFake(): FakeTransport {
  const inner = new FakeTransport();
  unwrap(inner.initialize());
  return inner;
}

function drain(inner: FakeTransport): number[] {
  const seqs: number[] = [];
  for (;;) {
    const polled = unwrap(inner.poll());
    if (polled === null) {
      return seqs;
    }
    seqs.push(polled.seq);
  }
}

/**
 * Send one packet per tick for `ticks` ticks, then drain, and report which
 * sequence arrived on which tick. The sequence a packet carries IS its send
 * tick plus one, so `arrivals` reads directly as "sent at, arrived at".
 */
function runTicks(
  options: ImpairmentOptions,
  ticks: number,
): { readonly arrivals: readonly (readonly [number, number])[]; readonly inner: FakeTransport } {
  const inner = connectedFake();
  const link = impaired(inner, options);
  const arrivals: [number, number][] = [];
  for (let tick = 0; tick < ticks; tick += 1) {
    link.setTransportTick(tick);
    unwrap(link.send(packet(tick + 1)));
    unwrap(link.advanceTo(tick));
    for (const seq of drain(inner)) {
      arrivals.push([seq, tick]);
    }
  }
  // Keep advancing one tick at a time after the last send, so a packet
  // still in flight is recorded at the tick it really arrives on rather
  // than at whatever tick the drain happened to jump to.
  for (let tick = ticks; link.pendingCount() > 0; tick += 1) {
    unwrap(link.advanceTo(tick));
    for (const seq of drain(inner)) {
      arrivals.push([seq, tick]);
    }
  }
  return { arrivals, inner };
}

describe("impairment rng", () => {
  it("mirrors the minstd generator the native simulation seeds", () => {
    expect(RNG_MOD).toBe(2147483647);
    expect(RNG_MULT).toBe(16807);
    // The published minstd sequence from state 1.
    let state = rngSeed(1);
    const states: number[] = [];
    for (let index = 0; index < 4; index += 1) {
      const rolled = rngRoll(state);
      state = rolled.state;
      states.push(state);
      expect(rolled.sample).toBeGreaterThanOrEqual(0);
      expect(rolled.sample).toBeLessThan(1);
    }
    expect(states).toEqual([16807, 282475249, 1622650073, 984943658]);
  });

  it("clamps a seed the way gc-core does, and refuses a non-finite one", () => {
    expect(rngSeed(0)).toBe(1);
    expect(rngSeed(-7.9)).toBe(7);
    expect(rngSeed(RNG_MOD)).toBe(1);
    expect(() => rngSeed(Number.NaN)).toThrow(/finite/);
  });
});

describe("authored network profiles", () => {
  it("carries every profile the native matrix names", () => {
    expect([...NETWORK_PROFILE_NAMES]).toEqual(["clean", "omp0_parity", "playable", "stress"]);
    for (const name of NETWORK_PROFILE_NAMES) {
      expect(networkProfile(name)).toBe(NETWORK_PROFILES[name]);
    }
  });

  it("keeps every profile inside the bounds the mechanism asserts", () => {
    for (const name of NETWORK_PROFILE_NAMES) {
      // Construction is where a malformed profile fails loud.
      expect(() => new ImpairmentLink({ profile: networkProfile(name), seed: 1 })).not.toThrow();
    }
  });
});

describe("impaired transport", () => {
  it("delays every message by the profile's fixed delay -- and would fail if it did not", () => {
    const { arrivals } = runTicks({ profile: networkProfile("omp0_parity"), seed: 12345 }, 40);
    expect(arrivals.length).toBeGreaterThan(30);
    for (const [seq, arrivedTick] of arrivals) {
      // seq n was sent on tick n - 1; omp0_parity delays exactly 3 ticks
      // with no jitter, so nothing may arrive on the tick it was sent.
      expect(arrivedTick).toBe(seq - 1 + 3);
    }
  });

  it("delivers a clean profile with no delay at all, so delay is never invented", () => {
    const { arrivals } = runTicks({ profile: networkProfile("clean"), seed: 12345 }, 25);
    expect(arrivals.length).toBe(25);
    for (const [seq, arrivedTick] of arrivals) {
      expect(arrivedTick).toBe(seq - 1);
    }
  });

  it("drops exactly the packets a known seed drops, and the far end never sees them", () => {
    const { arrivals } = runTicks({ profile: networkProfile("stress"), seed: 20260811 }, 40);
    const delivered = arrivals.map(([seq]) => seq);
    // Sequences 13, 16, 25 are independent losses and 38, 39, 40 are a
    // three-tick burst under this seed -- the same drops the native
    // `stress_single_slot` transcript records.
    expect(delivered).not.toContain(13);
    expect(delivered).not.toContain(16);
    expect(delivered).not.toContain(25);
    expect(delivered).not.toContain(38);
    expect(delivered).not.toContain(39);
    expect(delivered).not.toContain(40);
    expect(delivered).toContain(12);
    expect(delivered).toContain(37);
    expect(delivered.length).toBe(34);
  });

  it("produces a specific out-of-order permutation at the far end", () => {
    const { arrivals } = runTicks({ profile: networkProfile("stress"), seed: 20260811 }, 12);
    // Jitter alone reorders: 3 (sent on tick 2, arriving at 5) overtakes 1
    // and 2 (both arriving at 6). Nothing else in the mechanism reorders.
    expect(arrivals.map(([seq]) => seq)).toEqual([3, 1, 2, 6, 4, 5, 7, 8, 9, 11, 10, 12]);
    const arrivalOf = new Map(arrivals);
    expect(arrivalOf.get(3)).toBe(5);
    expect(arrivalOf.get(1)).toBe(6);
  });

  it("delivers a duplicated packet twice, to the same far end", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("playable"), seed: 4713 });
    const received: number[] = [];
    for (let tick = 0; tick < 60; tick += 1) {
      link.setTransportTick(tick);
      unwrap(link.send(packet(tick + 1)));
      unwrap(link.advanceTo(tick));
      received.push(...drain(inner));
    }
    unwrap(link.advanceTo(120));
    received.push(...drain(inner));
    // Sequence 45 is duplicated under this seed: it arrives twice, and the
    // far end sees both copies as complete, valid messages.
    expect(received.filter((seq) => seq === 45).length).toBe(2);
    expect(link.impairmentCounters().duplicated).toBe(1);
    expect(link.impairmentCounters().delivered).toBe(received.length);
  });

  it("is a no-op detector: the same run under `clean` moves no counter", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("clean"), seed: 20260811 });
    for (let tick = 0; tick < 40; tick += 1) {
      link.setTransportTick(tick);
      unwrap(link.send(packet(tick + 1)));
      unwrap(link.advanceTo(tick));
      drain(inner);
    }
    expect(link.impairmentCounters()).toEqual({
      sent: 40,
      delivered: 40,
      independent_lost: 0,
      burst_lost: 0,
      duplicated: 0,
      reordered: 0,
      unclocked_sends: 0,
    });
  });

  // The footgun this counter exists for: `TransportAdapter.send` carries no
  // tick, so a loop that forgets `setTransportTick` attributes every send to
  // the PREVIOUS tick -- silently, forever, with delays that look plausible.
  it("counts sends attributed to a tick the caller never opened", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("omp0_parity"), seed: 12345 });
    const arrivals: [number, number][] = [];
    // The broken loop: advanceTo moves the clock, and nothing else does.
    for (let tick = 0; tick < 20; tick += 1) {
      unwrap(link.send(packet(tick + 1)));
      unwrap(link.advanceTo(tick));
      for (const seq of drain(inner)) {
        arrivals.push([seq, tick]);
      }
    }
    for (let tick = 20; link.pendingCount() > 0; tick += 1) {
      unwrap(link.advanceTo(tick));
      for (const seq of drain(inner)) {
        arrivals.push([seq, tick]);
      }
    }
    expect(link.impairmentCounters().unclocked_sends).toBe(20);
    // And the damage the counter reports is real, in the direction that is
    // hardest to notice: the impairment clock runs a tick BEHIND the loop, so
    // omp0_parity's authored three-tick delay is measured as TWO. Every
    // packet lands one tick earlier than the profile says it should.
    // (Sequence 1 is the exception: the clock starts at tick 0 either way.)
    expect(arrivals.length).toBe(20);
    for (const [seq, arrivedTick] of arrivals) {
      const loopTickThatSentIt = seq - 1;
      expect(arrivedTick).toBe(loopTickThatSentIt + (seq === 1 ? 3 : 2));
    }
  });

  it("counts nothing when the tick loop opens each tick", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("stress"), seed: 99 });
    for (let tick = 0; tick < 20; tick += 1) {
      link.setTransportTick(tick);
      unwrap(link.send(packet(tick + 1)));
      unwrap(link.send(packet(tick + 1)));
      unwrap(link.advanceTo(tick));
      drain(inner);
    }
    expect(link.impairmentCounters().unclocked_sends).toBe(0);
    expect(link.impairmentCounters().sent).toBe(40);
  });

  it("throws on the first unclocked send under strict_clock", () => {
    const inner = connectedFake();
    const link = impaired(inner, {
      profile: networkProfile("clean"),
      seed: 1,
      strict_clock: true,
    });
    link.setTransportTick(0);
    unwrap(link.send(packet(1)));
    unwrap(link.advanceTo(1));
    // Tick 1 was reached by a delivery, not opened by the caller.
    expect(() => link.send(packet(2))).toThrow(/never opened with setTransportTick/);
    link.setTransportTick(1);
    unwrap(link.send(packet(2)));
  });

  it("counts an unclocked send on a star link too", () => {
    const host = new FakeStarTransport();
    unwrap(host.initialize());
    unwrap(host.openPeer("guest_1"));
    const guest = new FakeStarTransport({ role: "guest", peer_id: "guest_1" });
    unwrap(guest.initialize());
    unwrap(host.link(guest));
    const link = impairedStar(host, { profile: networkProfile("clean"), seed: 1 });
    unwrap(link.send("guest_1", "input", packet(1)));
    expect(link.impairmentCounters().unclocked_sends).toBe(1);
    link.setTransportTick(0);
    unwrap(link.send("guest_1", "input", packet(2)));
    expect(link.impairmentCounters().unclocked_sends).toBe(1);
  });

  it("replays exactly from a seed, and diverges from a different one", () => {
    const options = { profile: networkProfile("stress"), seed: 777 };
    const first = runTicks(options, 60).arrivals;
    const second = runTicks(options, 60).arrivals;
    expect(second).toEqual(first);
    const other = runTicks({ ...options, seed: 778 }, 60).arrivals;
    expect(other).not.toEqual(first);
  });

  it("holds a message rather than handing it straight to the wrapped adapter", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("stress"), seed: 5 });
    unwrap(link.send(packet(1)));
    // Not yet released: the wrapped adapter has not been told anything.
    expect(unwrap(inner.poll())).toBeNull();
    expect(inner.diagnostics().sent).toBe(0);
    expect(link.pendingCount()).toBe(1);
    unwrap(link.advanceTo(20));
    expect(unwrap(inner.poll())?.seq).toBe(1);
  });

  it("reports a malformed message immediately, without scheduling it", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("clean"), seed: 1 });
    const bad = { version: 1, type: "input", seq: 0, payload: "" } as unknown as TransportMessage;
    const sent = link.send(bad);
    expect(sent.ok).toBe(false);
    expect(link.pendingCount()).toBe(0);
    expect(link.impairmentCounters().sent).toBe(0);
  });

  it("refuses to send over a link that is not connected", () => {
    const fresh = new FakeTransport();
    const link = impaired(fresh, { profile: networkProfile("clean"), seed: 1 });
    const before = link.send(packet(1));
    expect(before.ok).toBe(false);
    unwrap(link.initialize());
    unwrap(link.send(packet(1)));
    unwrap(link.shutdown());
    expect(link.send(packet(2)).ok).toBe(false);
    // Shutdown discards what was still in flight, as a closed link does.
    expect(link.pendingCount()).toBe(0);
  });

  it("refuses a transport clock that moves backwards", () => {
    const inner = connectedFake();
    const link = impaired(inner, { profile: networkProfile("clean"), seed: 1 });
    link.setTransportTick(10);
    expect(() => {
      link.setTransportTick(9);
    }).toThrow(/monotonic/);
    expect(() => link.advanceTo(9)).toThrow(/monotonic/);
    expect(() => link.advanceTo(-1)).toThrow(/non-negative/);
  });

  it("rejects a profile whose authored bounds are impossible", () => {
    const base: NetworkProfile = networkProfile("playable");
    const bad = (patch: Partial<NetworkProfile>): (() => ImpairmentLink<string>) => {
      return () => new ImpairmentLink<string>({ profile: { ...base, ...patch }, seed: 1 });
    };
    expect(bad({ base_delay_ticks: -1 })).toThrow(/base delay/);
    expect(bad({ jitter_min_ticks: 3, jitter_max_ticks: 1 })).toThrow(/reversed/);
    expect(bad({ independent_loss_rate: 1.5 })).toThrow(/loss rate/);
    expect(bad({ duplication_rate: -0.1 })).toThrow(/duplication rate/);
    expect(bad({ burst_start_rate: 2 })).toThrow(/burst rate/);
    expect(bad({ burst_length_ticks: 0 })).toThrow(/disabled or enabled/);
  });
});

describe("impaired star transport", () => {
  function buildStar(count: number): readonly [FakeStarTransport, FakeStarTransport[]] {
    const host = new FakeStarTransport();
    unwrap(host.initialize());
    const guests: FakeStarTransport[] = [];
    for (let slot = 1; slot <= count; slot += 1) {
      const peerId = `guest_${slot}`;
      unwrap(host.openPeer(peerId));
      const guest = new FakeStarTransport({ role: "guest", peer_id: peerId });
      unwrap(guest.initialize());
      unwrap(host.link(guest));
      guests.push(guest);
    }
    return [host, guests];
  }

  it("impairs each peer's traffic independently while sharing one generator", () => {
    const [host, guests] = buildStar(2);
    const link = impairedStar(host, { profile: networkProfile("stress"), seed: 991 });
    const arrivals: [string, number][] = [];
    for (let tick = 0; tick < 24; tick += 1) {
      const peerId = `guest_${(tick % 2) + 1}`;
      link.setTransportTick(tick);
      unwrap(link.send(peerId, "input", packet(tick + 1)));
      unwrap(link.advanceTo(tick));
      host.pump();
      for (let slot = 1; slot <= 2; slot += 1) {
        const guest = guests[slot - 1] as FakeStarTransport;
        for (const peerMessage of guest.pollBatch()) {
          arrivals.push([`guest_${slot}`, peerMessage.message.seq]);
        }
      }
    }
    // Every packet reached the peer it was addressed to and no other: odd
    // sequences to guest_1, even to guest_2.
    for (const [peerId, seq] of arrivals) {
      expect(peerId).toBe(`guest_${((seq - 1) % 2) + 1}`);
    }
    // The seed's burst hits slot 2 while slot 1 keeps delivering, which a
    // per-link burst window could not produce.
    const counters = link.impairmentCounters();
    expect(counters.burst_lost).toBeGreaterThan(0);
    expect(counters.sent).toBe(24);
    expect(arrivals.length).toBeLessThan(24);
  });

  it("broadcasts as one independently impaired packet per peer", () => {
    const [host] = buildStar(3);
    const link = impairedStar(host, { profile: networkProfile("stress"), seed: 31337 });
    expect(unwrap(link.broadcast("input", packet(1)))).toBe(3);
    // Three sends off one generator, not one send fanned out: the three
    // packets get three different impairment decisions.
    expect(link.impairmentCounters().sent).toBe(3);
    const receipt = link.lastReceipt();
    expect(receipt?.sequence).toBe(3);
  });

  it("drops what is in flight for a peer when that peer closes", () => {
    const [host] = buildStar(2);
    const link = impairedStar(host, { profile: networkProfile("stress"), seed: 4242 });
    unwrap(link.send("guest_1", "input", packet(1)));
    unwrap(link.send("guest_2", "input", packet(2)));
    expect(link.pendingCount()).toBe(2);
    unwrap(link.closePeer("guest_1", "left"));
    expect(link.pendingCount()).toBe(1);
    expect(unwrap(link.advanceTo(40)).map((release) => release.peer_id)).toEqual(["guest_2"]);
  });

  it("refuses a send to a peer the star never opened", () => {
    const [host] = buildStar(1);
    const link = impairedStar(host, { profile: networkProfile("clean"), seed: 1 });
    const sent = link.send("guest_9", "input", packet(1));
    expect(sent.ok).toBe(false);
    expect(link.pendingCount()).toBe(0);
  });

  it("refuses a message on the wrong channel before it is ever scheduled", () => {
    const [host] = buildStar(1);
    const link = impairedStar(host, { profile: networkProfile("clean"), seed: 1 });
    const control = unwrap(contract.newMessage({ type: "event", seq: 1, payload: "" }));
    expect(link.send("guest_1", "input", control).ok).toBe(false);
    expect(link.impairmentCounters().sent).toBe(0);
  });
});
