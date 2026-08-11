// The browser half of the impairment differential (#472).
//
// This spec and `rust/crates/gc-sim/tests/browser_impairment_parity.rs` run
// the SAME five scripted scenarios -- the four authored profiles plus a
// two-source star -- and each assert the SAME transcript literal.
// `scripts/check_network_profile_parity.mjs` (gate 0c) additionally requires
// the two literals to be byte-identical, so drift is caught even when only
// one language's tests run.
//
// WHY A TRANSCRIPT RATHER THAN A HANDFUL OF ASSERTIONS. What has to agree
// between the two implementations is the whole impairment DECISION SEQUENCE:
// which packet was dropped and why, which was duplicated, when each arrives
// and in what order they come out. A test that only asserted "some packets
// were dropped" would stay green through a jitter formula that is off by one
// tick, a roll order that drifted, or a burst window that expires a tick
// early -- each of which makes browser evidence disagree with the native
// matrix while both look green.
//
//
// ## WHAT THIS DIFFERENTIAL IS NOT
//
// The two implementations are checked against ONE SHARED GOLDEN LITERAL that
// is duplicated in both files -- not against each other's live output. Both
// sides really do the work to reproduce it and gate 0c keeps the two copies
// byte-identical, so drift after the golden was captured is caught. But a bug
// present in BOTH implementations at the moment the golden was captured would
// be baked into the golden and invisible here forever.
//
// That residual risk is covered by reading the native source directly, not by
// this file. Do not let the word "differential" imply more than it delivers:
// if the two implementations are ever changed together, re-derive the golden
// from the Rust side and re-read `network_conditions.rs` while doing it.
//
// See the Rust file's header for the transcript's grammar.

import { describe, expect, it } from "vitest";
import * as contract from "./contract.ts";
import { FakeStarTransport } from "./fake_star.ts";
import { FakeTransport } from "./fake.ts";
import { impaired, impairedStar, type ImpairmentReceipt } from "./impairment.ts";
import { networkProfile, type NetworkProfileName } from "./network_profiles.ts";
import type { TransportMessage, TransportResult } from "./contract.ts";

const EXPECTED_TRANSCRIPT = `scenario|name=clean_single_slot|profile=clean|seed=20260811|sends=24|slots=1
sends|0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23
deliveries|1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24
counters|sent=24,delivered=24,independent_lost=0,burst_lost=0,duplicated=0,reordered=0
scenario|name=omp0_parity_single_slot|profile=omp0_parity|seed=20260811|sends=96|slots=1
sends|3,4,5,6,7,8,9,10,11,12,13,14,x,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,x,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98
deliveries|1,2,3,4,5,6,7,8,9,10,11,12,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96
counters|sent=96,delivered=94,independent_lost=2,burst_lost=0,duplicated=0,reordered=0
scenario|name=playable_single_slot|profile=playable|seed=4713|sends=160|slots=1
sends|1,6,5,5,5,9,11,11,13,13,12,15,14,17,19,19,17,22,23,24,23,22,25,25,25,27,29,32,32,34,35,35,35,36,36,36,38,39,43,40,41,45,45,46,48+,47,48,52,52,53,51,52,55,58,58,60,58,60,62,64,63,66,65,68,67,66,70,68,70,71,71,73,75,74,75,77,77,79,81,80,85,86,86,85,86,86,91,90,93,90,92,96,97,98,99,96,100,101,103,100,101,x,105,105,109,108,108,110,113,111,113,115,116,117,118,120,121,119,122,124,124,123,123,127,128,126,130,130,133,131,133,136,136,137,137,140,140,138,142,143,143,145,143,147,147,147,151,151,151,153,154,156,153,154,158,159,159,159,161,162
deliveries|1,3,4,5,2,6,7,8,11,9,10,13,12,14,17,15,16,18,22,19,21,20,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,40,41,39,42,43,44,46,45,45d,47,51,48,49,52,50,53,54,55,57,56,58,59,61,60,63,62,66,65,64,68,67,69,70,71,72,74,73,75,76,77,78,80,79,81,84,82,83,85,86,88,90,87,91,89,92,96,93,94,95,97,100,98,101,99,103,104,106,107,105,108,110,109,111,112,113,114,115,118,116,117,119,122,123,120,121,126,124,125,127,128,130,129,131,132,133,134,135,138,136,137,139,140,141,143,142,144,145,146,147,148,149,150,153,151,154,152,155,156,157,158,159,160
counters|sent=160,delivered=160,independent_lost=1,burst_lost=0,duplicated=1,reordered=45
scenario|name=stress_single_slot|profile=stress|seed=20260811|sends=160|slots=1
sends|6,6,5,11,12,10,12,12,14,15,14,18,x,22,22,x,21,22,27,28,29,24,26,30,x,30,31,31,34,36,33,37,40,37,41,38,44,b,b,b,43,45,49,49,49,52,52,52,51,52,53,60,56,56,58,58,64,60,66,b,b,b,69,66,71,71,75,73,x,76,75,80,76,81,81,78,83,82,81,88,89,87,89,88,91,93,93,91,94,92,98,97,99,100,103,100,105,103,102,105,107,108,108,109,112,112,115,116,115,117,116,x,119,118,120,118,123,126,127,124,126,130,126,127,129,130,132,135,136,134,138,136,140,140,139,138,144,144,146,143,149,150,150,151,153,154,153,155,156,156,157,159,159,159,163,164,160,164,162,164
deliveries|3,1,2,6,4,5,7,8,9,11,10,12,17,14,15,18,22,23,19,20,21,24,26,27,28,31,29,30,32,34,36,33,35,41,37,42,43,44,45,49,46,47,48,50,51,53,54,55,56,52,58,57,59,64,63,65,66,68,67,71,70,73,76,72,74,75,79,78,77,82,80,84,81,83,85,88,90,86,87,89,92,91,93,94,96,99,95,98,97,100,101,102,103,104,105,106,107,109,108,111,110,114,116,113,115,117,120,118,121,123,119,124,125,122,126,127,130,128,129,132,131,136,135,133,134,140,137,138,139,141,142,143,144,145,147,146,148,149,150,151,152,153,154,157,159,155,156,158,160
counters|sent=160,delivered=149,independent_lost=5,burst_lost=6,duplicated=0,reordered=58
scenario|name=stress_two_slots|profile=stress|seed=991|sends=160|slots=2
sends|3,10,6,9,7,13,12,11,11,18,18,18,16,b,17,b,24,21,25,25,25,27,28,31,29,31,31,34,35,34,34,38,36+,42,38,43,42,46,47,46,47,44,51+,48,52,50,54,56,57,57,58,58,58,56,60,58,59,b,61,b,66,70,70,68,69,b,69,b,73,78,74,80,79,76,x,84,85,83,87,87,89,84,87,89,92,93,90,94,92,92,98,95,98,101,100,102,103,104,104,103,105,109,111,107,113,109,112,112,x,115,119,115,116,122,120,123,123,123,121,127,126,129,127,131,130,128,129,131,136,133,137,139,141,137,141,138+,139,140,144,146,145,144,150,148,152,153,154,154,156,158,159,155,160,157,162,161,159,163,166,168
deliveries|1,3,5,4,2,8,9,7,6,13,15,10,11,12,18,17,19,20,21,22,23,25,24,26,27,28,30,31,29,33,33d,32,35,34,37,36,42,38,40,39,41,44,46,43,43d,45,47,48,54,49,50,51,52,53,56,57,55,59,61,64,65,67,62,63,69,71,74,70,73,72,78,76,82,77,79,80,83,81,84,87,85,89,90,86,88,92,91,93,95,94,96,97,100,98,99,101,104,102,106,103,107,108,105,110,112,113,111,115,119,114,116,117,118,121,120,123,126,122,127,125,124,128,130,129,131,134,136,136d,132,137,138,133,135,139,142,141,140,144,143,145,146,147,148,152,149,154,150,151,157,153,156,155,158,159,160
counters|sent=160,delivered=155,independent_lost=2,burst_lost=6,duplicated=3,reordered=67`;

interface ScenarioSpec {
  readonly name: string;
  readonly profile: NetworkProfileName;
  readonly seed: number;
  readonly sends: number;
  readonly slots: number;
}

const SCENARIOS: readonly ScenarioSpec[] = [
  { name: "clean_single_slot", profile: "clean", seed: 20260811, sends: 24, slots: 1 },
  { name: "omp0_parity_single_slot", profile: "omp0_parity", seed: 20260811, sends: 96, slots: 1 },
  { name: "playable_single_slot", profile: "playable", seed: 4713, sends: 160, slots: 1 },
  { name: "stress_single_slot", profile: "stress", seed: 20260811, sends: 160, slots: 1 },
  { name: "stress_two_slots", profile: "stress", seed: 991, sends: 160, slots: 2 },
];

function unwrap<T>(result: TransportResult<T>): T {
  if (!result.ok) {
    throw new Error(`${result.error.code}: ${result.error.message}`);
  }
  return result.value;
}

function packet(index: number): TransportMessage {
  return unwrap(
    contract.newMessage({ type: "input", seq: index + 1, tick: index, payload: `p${index}` }),
  );
}

// A `sends` entry: the arrival tick, `+` when a duplicate was scheduled
// alongside it, `x` for an independent loss, `b` for a burst loss.
function sendEntry(receipt: ImpairmentReceipt): string {
  if (receipt.dropped) {
    if (receipt.drop_reason === "burst_loss") {
      return "b";
    }
    if (receipt.drop_reason === "independent_loss") {
      return "x";
    }
    throw new Error("a dropped packet must carry a drop reason");
  }
  if (receipt.arrival_tick === null) {
    throw new Error("a delivered packet must carry an arrival tick");
  }
  return `${receipt.arrival_tick}${receipt.duplicated ? "+" : ""}`;
}

/** The transport tick the last send can still be in flight at. */
function drainTick(scenario: ScenarioSpec): number {
  const profile = networkProfile(scenario.profile);
  return scenario.sends + profile.base_delay_ticks + profile.jitter_max_ticks + 1;
}

function transcriptFor(
  scenario: ScenarioSpec,
  sends: readonly string[],
  deliveries: readonly string[],
  counters: Record<string, number>,
): string {
  return [
    `scenario|name=${scenario.name}|profile=${scenario.profile}|seed=${scenario.seed}` +
      `|sends=${scenario.sends}|slots=${scenario.slots}`,
    `sends|${sends.join(",")}`,
    `deliveries|${deliveries.join(",")}`,
    `counters|sent=${counters.sent},delivered=${counters.delivered},` +
      `independent_lost=${counters.independent_lost},burst_lost=${counters.burst_lost},` +
      `duplicated=${counters.duplicated},reordered=${counters.reordered}`,
  ].join("\n");
}

// One source over a point-to-point link. The wrapped adapter is a real
// loopback transport, and every released envelope is read back out of it, so
// the transcript records what the FAR END actually received -- not merely
// what the schedule intended.
function runSingleSlot(scenario: ScenarioSpec): string {
  const inner = new FakeTransport();
  unwrap(inner.initialize());
  const link = impaired(inner, { profile: networkProfile(scenario.profile), seed: scenario.seed });

  const sends: string[] = [];
  const deliveries: string[] = [];

  const drainInner = (expectedSeqs: readonly number[]): void => {
    const received: number[] = [];
    for (;;) {
      const polled = unwrap(inner.poll());
      if (polled === null) {
        break;
      }
      received.push(polled.seq);
    }
    expect(received).toEqual(expectedSeqs);
  };

  const release = (tick: number): void => {
    const released = unwrap(link.advanceTo(tick));
    for (const envelope of released) {
      deliveries.push(`${envelope.sequence}${envelope.duplicate_ordinal === 0 ? "" : "d"}`);
    }
    drainInner(released.map((envelope) => envelope.sequence));
  };

  for (let index = 0; index < scenario.sends; index += 1) {
    link.setTransportTick(index);
    unwrap(link.send(packet(index)));
    const receipt = link.lastReceipt();
    expect(receipt).not.toBeNull();
    sends.push(sendEntry(receipt as ImpairmentReceipt));
    release(index);
  }
  release(drainTick(scenario));

  expect(link.pendingCount()).toBe(0);
  // The scripted loop opens every tick it sends on, so the transcript's send
  // ticks mean what they say. A nonzero count here would mean the transcript
  // was captured against a clock running behind the loop that drove it.
  expect(link.impairmentCounters().unclocked_sends).toBe(0);
  return transcriptFor(scenario, sends, deliveries, { ...link.impairmentCounters() });
}

// Two sources over one star link. The generator and the send sequence are
// shared across peers; only the loss-burst window is per source.
function runTwoSlots(scenario: ScenarioSpec): string {
  const host = new FakeStarTransport();
  unwrap(host.initialize());
  const guests: FakeStarTransport[] = [];
  for (let slot = 1; slot <= scenario.slots; slot += 1) {
    const peerId = `guest_${slot}`;
    expect(unwrap(host.openPeer(peerId))).toBe(slot);
    const guest = new FakeStarTransport({ role: "guest", peer_id: peerId });
    unwrap(guest.initialize());
    unwrap(host.link(guest));
    guests.push(guest);
  }
  const link = impairedStar(host, {
    profile: networkProfile(scenario.profile),
    seed: scenario.seed,
  });

  const sends: string[] = [];
  const deliveries: string[] = [];

  const release = (tick: number): void => {
    const released = unwrap(link.advanceTo(tick));
    const expectedByPeer = new Map<string, number[]>();
    for (const envelope of released) {
      deliveries.push(`${envelope.sequence}${envelope.duplicate_ordinal === 0 ? "" : "d"}`);
      const forPeer = expectedByPeer.get(envelope.peer_id) ?? [];
      forPeer.push(envelope.sequence);
      expectedByPeer.set(envelope.peer_id, forPeer);
    }
    // The fake star drains its channels only when pumped; a real data
    // channel drains itself.
    host.pump();
    // Each release lands on exactly the peer it was addressed to, and on no
    // other -- a decorator that fanned every send out to every peer would
    // otherwise still produce a plausible transcript.
    for (let slot = 1; slot <= scenario.slots; slot += 1) {
      const guest = guests[slot - 1] as FakeStarTransport;
      const received = guest.pollBatch().map((peerMessage) => peerMessage.message.seq);
      expect(received).toEqual(expectedByPeer.get(`guest_${slot}`) ?? []);
    }
  };

  for (let index = 0; index < scenario.sends; index += 1) {
    const peerId = `guest_${(index % scenario.slots) + 1}`;
    link.setTransportTick(index);
    unwrap(link.send(peerId, "input", packet(index)));
    const receipt = link.lastReceipt();
    expect(receipt).not.toBeNull();
    expect((receipt as ImpairmentReceipt).source_slot).toBe((index % scenario.slots) + 1);
    sends.push(sendEntry(receipt as ImpairmentReceipt));
    release(index);
  }
  release(drainTick(scenario));

  expect(link.pendingCount()).toBe(0);
  expect(link.impairmentCounters().unclocked_sends).toBe(0);
  return transcriptFor(scenario, sends, deliveries, { ...link.impairmentCounters() });
}

function transcript(): string {
  return SCENARIOS.map((scenario) =>
    scenario.slots === 1 ? runSingleSlot(scenario) : runTwoSlots(scenario),
  ).join("\n");
}

describe("browser impairment parity with the native rollback matrix", () => {
  it("reproduces the shared transcript exactly", () => {
    expect(transcript()).toBe(EXPECTED_TRANSCRIPT);
  });

  // The transcript is only evidence if it actually contains impairment. A
  // future edit that quietly turned the decorator into a pass-through would
  // still produce SOME transcript; these floors say what has to be in it.
  it("covers every impairment kind the profiles model", () => {
    const text = transcript();
    const lines = text.split("\n");
    expect(lines.length).toBe(SCENARIOS.length * 4);

    const counters = lines
      .filter((line) => line.startsWith("counters|"))
      .map(
        (line) =>
          Object.fromEntries(
            line
              .slice("counters|".length)
              .split(",")
              .map((pair) => {
                const [key, value] = pair.split("=");
                return [key, Number(value)];
              }),
          ) as Record<string, number>,
      );
    const total = (key: string): number =>
      counters.reduce((sum, entry) => sum + (entry[key] ?? 0), 0);

    expect(total("independent_lost")).toBeGreaterThan(0);
    expect(total("burst_lost")).toBeGreaterThan(0);
    expect(total("duplicated")).toBeGreaterThan(0);
    expect(total("reordered")).toBeGreaterThan(0);
    // The clean profile impairs nothing, and that is a property too: it is
    // the scenario that goes red if delay is ever invented from nowhere.
    expect(counters[0]).toEqual({
      sent: 24,
      delivered: 24,
      independent_lost: 0,
      burst_lost: 0,
      duplicated: 0,
      reordered: 0,
    });
  });
});
