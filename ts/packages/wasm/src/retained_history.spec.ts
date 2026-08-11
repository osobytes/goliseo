// Exercises `MatchDriverBridge.retainedHistoryAccountingJson` — the seam a
// long-duration soak samples to watch a browser peer's retained rollback
// history — against the real compiled artifact, under node.
//
// A number that crosses the wasm boundary wrongly (truncated, mis-scaled,
// wired to the neighbouring field, or quietly reporting only one half of
// what it names) still looks like a perfectly good number from JS. So this
// file does not check that the fields exist and are numeric. It checks:
//
// 1. **The value observed here is the value Rust computed.**
//    `session.snapshot_bytes` — the dominant term — is re-derived
//    independently *in JavaScript*, by summing every retained boundary's own
//    `SnapshotLookup.canonicalBytes`. The engine reports that total from a
//    counter the snapshot ring maintains incrementally; the sum here is
//    assembled one boundary at a time through a different export. They must
//    agree exactly.
// 2. **`history_bytes` is the combined figure, not the event half.** The
//    session and event components must add to it, and `events.total_bytes`
//    must equal what the pre-existing `rollbackAccountingJson` reports on
//    its own — the surface that used to be the only retained-byte reading a
//    browser peer could take, and which omits the snapshot ring, the input
//    history and the retained outputs (measured at 97.3% of the number under
//    this file's own deliberately deep unconfirmed window, and 98.7-98.9%
//    with a lightly-lagged one -- the dominant term either way).
// 3. **The sample is real, not scaffolding.** Retained step wrappers holding
//    no events still report a plausible nonzero byte total that no per-event
//    encoder contributed to, so the occupancy counts are asserted alongside
//    the bytes.
//
// Requires `pnpm --filter @gc/wasm build` to have run first.

import { describe, expect, it } from "vitest";

import { loadSimHost, type SimHost } from "./index.ts";
import type { MatchDriverBridge, RetainedHistoryAccounting, SimSession } from "./types.ts";

interface Peer {
  readonly peerId: string;
  readonly driver: MatchDriverBridge;
  readonly session: SimSession;
}

interface OutboundEnvelope {
  readonly peer_id: string;
  readonly channel: "control" | "input";
  readonly message: {
    readonly kind: "input" | "event" | "state";
    readonly seq: number;
    readonly tick?: number | null;
    readonly payload_bytes: readonly number[];
  };
}

// A full 4v4 lobby: every canonical slot is authored by a real peer sending
// real input, rather than predicted neutral samples standing in for the
// slots nobody drives. That is what makes the retained window hold real
// content.
const MODE = "4v4" as const;

// Enough steps to fill the 31-boundary snapshot ring, so every reading below
// is steady-state rather than warming.
const STEPS = 45;

// Envelopes are delivered in bursts during the warm phase, then held for the
// last `HOLD` steps. Both parts matter:
//
// - With instant delivery every tick confirms immediately, the speculative
//   event timeline drains to nothing, and `events.total_bytes` reads zero. A
//   peer on a perfect connection genuinely retains no unconfirmed window; no
//   real connection is that.
// - Holding the tail open leaves the timeline at a deep-but-legal
//   unconfirmed window (27 steps against the 30-tick limit), which is the
//   expensive case a retention budget has to bound.
const DELIVER_EVERY = 5;
const HOLD = 28;

// The default 64-envelope inbound queue is sized for per-step delivery; a
// burst of `DELIVER_EVERY` steps across eight peers overflows it (a
// different, unrelated failure that would mask what this file measures).
const QUEUE_LIMIT = 1024;

function buildPeers(host: SimHost): Peer[] {
  const freezeJson = host.matchDriverFixtureFreezeJson(MODE);
  const manifestJson = host.matchDriverFixtureManifestJson(MODE);
  const peerIds = host.matchDriverFixturePeerIds(MODE);
  return peerIds.map((peerId, index) => {
    const session = new host.Session("nebula", "orion", 7, 20, 3);
    const driver = new host.MatchDriverBridge(
      session,
      index === 0 ? "host" : "guest",
      peerId,
      freezeJson,
      manifestJson,
      undefined,
      QUEUE_LIMIT,
      // A COMBAT-ACTIVE boundary zero, built once per peer (the handle is
      // consumed by value, and every peer must start from a byte-identical
      // one -- see `MatchDriverBridgeConstructor`'s doc). Combat is what
      // makes this fixture produce real event traffic: the same run seated
      // on the plain boundary zero produces a single match event in 45
      // ticks, which would leave the retained window holding wrappers.
      host.matchDriverFixtureInitialSnapshot(undefined, true, undefined),
    );
    driver.initializeTransport();
    const others =
      index === 0 ? peerIds.filter((candidate) => candidate !== peerId) : [peerIds[0] as string];
    for (const other of others) {
      driver.openPeer(other);
      driver.setPeerConnected(other);
    }
    return { peerId, driver, session };
  });
}

// Shuttles every peer's `drainOutboundJson()` output straight into the
// addressed peer's `enqueueInbound` -- the queue/drain seam
// `match_driver_bridge.rs`'s module doc describes, driven directly, the same
// way `packages/online/src/match_presentation.spec.ts`'s harness does.
function deliverAll(peers: readonly Peer[]): void {
  const drained = peers.map(
    (peer) => JSON.parse(peer.driver.drainOutboundJson()) as OutboundEnvelope[],
  );
  peers.forEach((sender, senderIndex) => {
    for (const envelope of drained[senderIndex] ?? []) {
      const receiver = peers.find((candidate) => candidate.peerId === envelope.peer_id);
      if (receiver === undefined) {
        continue;
      }
      receiver.driver.enqueueInbound(
        sender.peerId,
        envelope.channel,
        envelope.message.kind,
        envelope.message.seq,
        envelope.message.tick ?? undefined,
        new Uint8Array(envelope.message.payload_bytes),
      );
    }
  });
}

// Contested play, not eight players standing still: both sides sprint at the
// ball holding the equipment action, pressing a combat request every sixth
// tick and dashing on the third -- the same shape `gc-sim`'s
// `tests/retained_history.rs` scripts, and for the same reason (neutral
// input measurably produces no events at all).
function scriptedSample(host: SimHost, step: number, peerIndex: number): string {
  const held = JSON.parse(host.inputFrameHeldBitsJson()) as Record<string, number>;
  const edgeBits = JSON.parse(host.inputFrameEdgeBitsJson()) as Record<string, number>;
  const heldMask = (held.equipment ?? 0) | (held.sprint ?? 0);
  let edges = 0;
  if (step % 6 === 0) {
    edges = edgeBits.equipment_pressed ?? 0;
  } else if (step % 6 === 3) {
    edges = edgeBits.dash ?? 0;
  }
  return host.inputFrameNewSample(
    peerIndex < 4 ? 100 : -100,
    peerIndex % 2 === 0 ? 40 : -40,
    heldMask,
    edges,
  );
}

/** Steps every peer `STEPS` times, delivering in bursts until the last
 * `HOLD` steps -- see `DELIVER_EVERY`/`HOLD`. */
function run(host: SimHost, peers: readonly Peer[]): void {
  for (let step = 0; step < STEPS; step += 1) {
    peers.forEach((peer, peerIndex) => {
      peer.driver.advance(scriptedSample(host, step, peerIndex));
    });
    if (step < STEPS - HOLD && step % DELIVER_EVERY === DELIVER_EVERY - 1) {
      deliverAll(peers);
    }
  }
}

function sample(peer: Peer): RetainedHistoryAccounting {
  return JSON.parse(peer.driver.retainedHistoryAccountingJson()) as RetainedHistoryAccounting;
}

function freeAll(peers: readonly Peer[]): void {
  for (const peer of peers) {
    peer.driver.free();
    peer.session.free();
  }
}

describe("MatchDriverBridge.retainedHistoryAccountingJson", () => {
  it("reports the combined session-plus-event total, not the event half", () => {
    const host = loadSimHost();
    const peers = buildPeers(host);
    try {
      run(host, peers);
      const peer = peers[0] as Peer;
      const retained = sample(peer);

      for (const value of [
        retained.history_bytes,
        retained.session.input_bytes,
        retained.session.output_bytes,
        retained.session.snapshot_bytes,
        retained.session.total_bytes,
        retained.events.total_bytes,
      ]) {
        expect(Number.isInteger(value)).toBe(true);
        expect(value).toBeLessThan(Number.MAX_SAFE_INTEGER);
      }

      expect(retained.history_bytes).toBe(
        retained.session.total_bytes + retained.events.total_bytes,
      );
      expect(retained.session.total_bytes).toBe(
        retained.session.input_bytes +
          retained.session.output_bytes +
          retained.session.snapshot_bytes,
      );
      expect(retained.events.total_bytes).toBe(retained.events.retained_step_bytes);

      // The event half is exactly what the pre-existing surface reports on
      // its own -- and it is a small fraction of the whole, which is the
      // point: a peer sampling only that one was watching 2.7% of its own
      // retained memory here, and nearer 1% on a lighter connection whose
      // unconfirmed window is shallower.
      const eventOnly = JSON.parse(peer.driver.rollbackAccountingJson()) as {
        readonly total_bytes: number;
      };
      expect(retained.events.total_bytes).toBe(eventOnly.total_bytes);
      expect(retained.events.total_bytes).toBeGreaterThan(0);
      expect(retained.history_bytes).toBeGreaterThan(retained.events.total_bytes * 10);
      expect(retained.history_bytes).toBeGreaterThan(retained.session.total_bytes);
    } finally {
      freeAll(peers);
    }
  });

  it("reports snapshot bytes JavaScript can re-derive boundary by boundary", () => {
    const host = loadSimHost();
    const peers = buildPeers(host);
    try {
      run(host, peers);
      const peer = peers[0] as Peer;
      const retained = sample(peer);

      const oldest = retained.oldest_boundary_tick;
      const latest = retained.latest_boundary_tick;
      expect(oldest).toBeDefined();
      expect(latest).toBeDefined();
      expect((latest as number) - (oldest as number) + 1).toBe(retained.retained_boundary_count);

      let summed = 0;
      let counted = 0;
      for (let tick = oldest as number; tick <= (latest as number); tick += 1) {
        const lookup = peer.driver.snapshotLookup(tick);
        try {
          expect(lookup.canonicalBytes).toBeDefined();
          expect(Number.isInteger(lookup.canonicalBytes)).toBe(true);
          summed += lookup.canonicalBytes as number;
          counted += 1;
        } finally {
          lookup.free();
        }
      }

      // The engine's figure is an incrementally maintained sum; this one was
      // assembled here, one boundary at a time, through a different export.
      // Exact agreement is what makes the sampled total trustworthy rather
      // than merely well-formed.
      expect(counted).toBe(retained.retained_boundary_count);
      expect(summed).toBe(retained.session.snapshot_bytes);
    } finally {
      freeAll(peers);
    }
  });

  it("carries the occupancy that says whether the bytes measured anything", () => {
    const host = loadSimHost();
    const peers = buildPeers(host);
    try {
      run(host, peers);
      const retained = sample(peers[0] as Peer);

      // A full ring: a partially warmed reading understates retention while
      // looking comfortable.
      expect(retained.retained_boundary_count).toBe(31);
      expect(retained.peak_retained_boundary_count).toBe(31);
      expect(retained.peak_snapshot_bytes).toBeGreaterThanOrEqual(retained.session.snapshot_bytes);

      // And real content inside the speculative window. Retained step
      // wrappers holding zero events would still report a plausible byte
      // total that no per-event encoder contributed to -- `gc-sim`'s
      // `tests/retained_history.rs` builds exactly that window and shows the
      // bytes cannot tell the difference, while this count can.
      expect(retained.retained_step_count).toBeGreaterThan(20);
      expect(retained.retained_event_count).toBeGreaterThan(0);
    } finally {
      freeAll(peers);
    }
  });

  it("is a read a soak can repeat cheaply", () => {
    const host = loadSimHost();
    const peers = buildPeers(host);
    try {
      run(host, peers);
      const peer = peers[0] as Peer;

      // Sampling must not perturb the driver it measures.
      const before = peer.driver.diagnosticsJson();
      const first = peer.driver.retainedHistoryAccountingJson();
      expect(peer.driver.retainedHistoryAccountingJson()).toBe(first);
      expect(peer.driver.diagnosticsJson()).toBe(before);

      // Cost, measured rather than assumed -- reported for the soak that
      // will sample this repeatedly (run vitest with
      // `--disableConsoleIntercept` to see it). Deliberately not asserted as
      // a wall-clock bound: on a shared CI runner that would measure the
      // runner (AGENTS.md §9).
      const iterations = 50;
      const started = performance.now();
      for (let index = 0; index < iterations; index += 1) {
        peer.driver.retainedHistoryAccountingJson();
      }
      const perSampleMs = (performance.now() - started) / iterations;
      console.log(
        `GC_RETAINED_HISTORY|wasm_per_sample_ms=${perSampleMs.toFixed(3)}` +
          `|history_bytes=${sample(peer).history_bytes}`,
      );
    } finally {
      freeAll(peers);
    }
  });
});
