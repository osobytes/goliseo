// Ported from spec/game/online_match_presentation_spec.lua.
//
// Every assertion in the Lua original is about a claim only a *live*
// rollback session can make: peers converging through a real correction, a
// real combat encounter being revoked and replaced by a resimulation, a
// real driver reaching `completed` at full time. All of that runs through
// `game.online.match_driver` and `sim.rollback_events`, both Rust-owned
// (`crates/gc-sim` / `crates/gc-netcode`; v2/README.md §2.1).
//
// # Re-audited against the current `@gc/wasm`
//
// A prior pass here recorded five numbered blockers and left all 13 cases
// `it.skip`. Re-verified against the code as it exists now (`crates/gc-wasm/
// src/match_driver_fixture_bridge.rs`, `crates/gc-wasm/src/
// match_driver_bridge.rs`, `crates/gc-wasm/src/rollback_events_bridge.rs`):
//
//   * Blockers 1-3 (no `@gc/wasm` dependency, no standalone
//     `RollbackEventsTimeline`, the correction-batch feed skipped) were
//     already fixed by the time that pass was written and are unchanged.
//   * Blocker 4 ("no TS-reachable way to construct a valid
//     `MatchDriverBridge`") is now **stale**. `match_driver_fixture_bridge.rs`
//     landed after that pass: `matchDriverFixtureFreezeJson`/
//     `matchDriverFixtureManifestJson` together are exactly the
//     `freezeJson`/`manifestJson` pair `MatchDriverBridge`'s constructor
//     needs, proven by both a Rust test
//     (`match_driver_fixture_bridge.rs`'s own module tests) and a TS one
//     (`packages/wasm/src/match_driver_fixture.spec.ts`'s "closes the
//     freezeJson/manifestJson gap" block).
//   * Blocker 5 (`RollbackTickOutput` too narrow for
//     `RollbackEventsTimeline.apply`'s `outputsJson`) was real and is fixed
//     here: `RollbackTickOutput` below now carries the full
//     `tick_output_to_json` shape.
//
// A real harness is built and driven below (`describe("online match
// presentation (real wasm bridges...")`), using
// `matchDriverFixtureFreezeJson`/`ManifestJson`/`PeerIds` to build two real
// `MatchDriverBridge` peers, shuttling `drainOutboundJson()` straight into
// the other peer's `enqueueInbound()` (no `StarTransportAdapter` needed for
// a controlled two-peer test -- see `net_diagnostics.spec.ts`'s header for
// why a *real* transport adapter is a separate, harder problem), and a real
// standalone `RollbackEventsTimeline` per peer as `newOnlineMatchPresentation`'s
// `events`. Four of the Lua original's 13 cases run for real this way: the
// four that only need `spec/fixtures/online_match_session.lua`'s plain
// session construction, not `spec/support/online_combat_phases.lua`'s
// pinned combat geometry.
//
// The remaining nine ("keeps feedback honest through a correction during
// <phase>" x7, "never publishes a combat cue a correction took away",
// "publishes the lifecycle exactly once through full time") all call
// `combat_phases.boundary_zero(phase_id)` in the Lua original -- a specific,
// pinned `MatchSnapshot` (exact player positions/combat state) for each of
// seven combat phases, and `combat_phases.live_sample` to script real input
// into that phase. This is the real, current blocker for those nine, and it
// is a *different* gap than the one blockers 1-5 described: `@gc/wasm`'s
// `Session` only ever starts from `sim_match::new`'s default boundary zero
// (`crates/gc-wasm/src/session.rs`'s `Session::new` hard-codes
// `home_formation: None` and there is no snapshot-restore/constructor entry
// point at all -- confirmed by reading `session.rs` end to end). There is no
// way to hand a `Session` or a `MatchDriverBridge` an arbitrary starting
// state from TypeScript; the only snapshots reachable from this side are
// ones a real, freshly-stepped session actually produced. Reaching a
// specific combat phase (windup/guard/contact/...) deterministically would
// need either a `gc-wasm` export of `spec/support/online_combat_phases.lua`'s
// pinned boundary zeroes, or scripting real gameplay input precisely enough
// to reach each phase from scratch inside a headless run -- both are
// significant new engineering (a Rust-side data export, or a TS-side input
// script derived from gameplay mechanics this port does not otherwise need)
// and are left for a follow-up rather than folded into this audit.
//
// What is *not* ported for the remaining nine, and does not need to be
// re-litigated: faking `combat_phases.boundary_zero`/`live_sample` well
// enough to reach a real combat phase would mean reimplementing combat
// geometry here, exactly what v2/README.md §2.1 forbids on this side of the
// determinism line.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import type {
  MatchDriverBridge as WasmMatchDriverBridge,
  RollbackEventsTimeline as WasmRollbackEventsTimeline,
  SimHost,
  SimSession,
  WasmMatchSnapshot,
} from "@gc/wasm";
import {
  consume,
  diagnostics,
  newOnlineMatchPresentation,
  status,
  type MatchDriverBatch,
  type MatchDriverPort,
  type MatchPresentationPorts,
  type OnlineMatchPresentation,
  type RollbackApplyResult,
  type RollbackEventDiff,
  type RollbackEventStep,
  type RollbackEventsDiagnostics,
  type RollbackEventsPort,
  type RollbackEventsStatus,
  type RollbackPlayableLabBatch,
  type RollbackTickOutput,
  type RollbackWrappedEvent,
  type SnapshotLookup,
} from "./match_presentation.ts";

// ---------------------------------------------------------------------------
// Real harness: two `MatchDriverBridge` peers, wired to each other directly
// (no transport adapter -- see the file header) and to real
// `RollbackEventsTimeline`s as `newOnlineMatchPresentation`'s `events`.
// ---------------------------------------------------------------------------

function wasmRollbackEventsPort(
  host: SimHost
): RollbackEventsPort<WasmRollbackEventsTimeline, WasmMatchSnapshot> {
  return {
    create(initialSnapshot, maxUnconfirmedTicks) {
      return host.RollbackEventsTimeline.create(initialSnapshot, maxUnconfirmedTicks);
    },
    apply(timeline, from, through, steps) {
      const outputsJson = JSON.stringify(steps.map((entry) => entry.output));
      const snapshots = steps.map((entry) => entry.snapshot);
      return JSON.parse(timeline.apply(from, through, outputsJson, snapshots)) as RollbackApplyResult;
    },
    confirm(timeline, confirmedOutputTick) {
      return JSON.parse(timeline.confirm(confirmedOutputTick)) as readonly RollbackEventStep[];
    },
    diagnostics(timeline) {
      return JSON.parse(timeline.diagnosticsJson()) as RollbackEventsDiagnostics;
    },
  };
}

function wasmMatchDriverPort(): MatchDriverPort<WasmMatchDriverBridge, WasmMatchSnapshot> {
  return {
    snapshot(driver, boundaryTick) {
      const lookup = driver.snapshotLookup(boundaryTick);
      return {
        status: lookup.status,
        tick: lookup.tick,
        ...(lookup.snapshot !== undefined ? { snapshot: lookup.snapshot } : {}),
      };
    },
    diagnostics(driver) {
      return JSON.parse(driver.diagnosticsJson()) as { readonly confirmed_output_tick: number };
    },
  };
}

function newSession(host: SimHost): SimSession {
  // Mirrors `packages/wasm/src/match_driver_fixture.spec.ts`'s own fixture
  // pairing: any authored team pair works, duration is generous enough that
  // nothing here reaches full time by accident.
  return new host.Session("nebula", "orion", 7, 20, 3);
}

interface RealPeer {
  readonly peerId: string;
  readonly driver: WasmMatchDriverBridge;
  readonly session: SimSession;
  readonly presentation: OnlineMatchPresentation<WasmRollbackEventsTimeline>;
  readonly confirmed: Map<string, number>;
  readonly revoked: Set<string>;
  readonly latest: Map<string, unknown>;
  readonly checkpoints: Map<number, string>;
  stale: number;
  corrections: number;
  confirmedTicks: number[];
}

interface RealHarness {
  readonly peers: readonly RealPeer[];
  readonly ports: MatchPresentationPorts<WasmRollbackEventsTimeline, WasmMatchSnapshot, WasmMatchDriverBridge>;
  readonly firstInputTick: number;
}

function buildHarness(host: SimHost, mode: "1v1" | "2v2" = "1v1", maxUnconfirmedTicks = 30): RealHarness {
  const freezeJson = host.matchDriverFixtureFreezeJson(mode);
  const manifestJson = host.matchDriverFixtureManifestJson(mode);
  const firstInputTick = (JSON.parse(freezeJson) as { readonly first_input_tick: number }).first_input_tick;
  const peerIds = host.matchDriverFixturePeerIds(mode);
  const rollbackEvents = wasmRollbackEventsPort(host);
  const matchDriver = wasmMatchDriverPort();
  const ports: MatchPresentationPorts<WasmRollbackEventsTimeline, WasmMatchSnapshot, WasmMatchDriverBridge> = {
    rollbackEvents,
    matchDriver,
  };

  const peers: RealPeer[] = peerIds.map((peerId, index) => {
    const role = index === 0 ? "host" : "guest";
    const session = newSession(host);
    const driver = new host.MatchDriverBridge(session, role, peerId, freezeJson, manifestJson, undefined);
    driver.initializeTransport();
    // Star topology: the host opens a slot per guest; each guest opens only
    // the host (a guest's transport capacity is fixed at 1 -- see
    // `net_diagnostics.spec.ts`'s harness for the "at peer capacity" this
    // avoids once more than one guest is in play).
    const others = role === "host" ? peerIds.filter((candidate) => candidate !== peerId) : [peerIds[0] as string];
    for (const other of others) {
      driver.openPeer(other);
      driver.setPeerConnected(other);
    }
    const initialSnapshot = driver.initialSnapshotHandle();
    const presentation = newOnlineMatchPresentation(rollbackEvents, initialSnapshot, firstInputTick, maxUnconfirmedTicks);
    return {
      peerId,
      driver,
      session,
      presentation,
      confirmed: new Map<string, number>(),
      revoked: new Set<string>(),
      latest: new Map<string, unknown>(),
      checkpoints: new Map<number, string>(),
      stale: 0,
      corrections: 0,
      confirmedTicks: [],
    };
  });

  return { peers, ports, firstInputTick };
}

interface WasmOutboundMessage {
  readonly kind: "input" | "event" | "state";
  readonly seq: number;
  readonly tick?: number | null;
  readonly payload_bytes: readonly number[];
}

interface WasmOutboundEnvelope {
  readonly peer_id: string;
  readonly channel: "control" | "input";
  readonly message: WasmOutboundMessage;
}

// Shuttles every peer's `drainOutboundJson()` output straight into the
// addressed peer's `enqueueInbound` -- the queue/drain seam
// `match_driver_bridge.rs`'s module doc describes, driven directly rather
// than through a `StarTransportAdapter` (see the file header).
function deliverAll(peers: readonly RealPeer[]): void {
  const drained = peers.map((peer) => JSON.parse(peer.driver.drainOutboundJson()) as WasmOutboundEnvelope[]);
  peers.forEach((sender, senderIndex) => {
    for (const envelope of drained[senderIndex] ?? []) {
      const receiver = peers.find((candidate) => candidate.peerId === envelope.peer_id);
      if (receiver === undefined) {
        continue;
      }
      const payload = new Uint8Array(envelope.message.payload_bytes);
      receiver.driver.enqueueInbound(
        sender.peerId,
        envelope.channel,
        envelope.message.kind,
        envelope.message.seq,
        envelope.message.tick ?? undefined,
        payload
      );
    }
  });
}

interface WasmDriverBatch extends MatchDriverBatch {
  readonly checkpoints: readonly { readonly tick: number; readonly hash: string }[];
}

function payloadsEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

// Mirrors the Lua spec's own `record` helper: tracks what each peer's
// presentation timeline has published, so the assertions below can check
// "exactly once" and "never a revoked id" for real.
function record(peer: RealPeer, batch: RollbackPlayableLabBatch): void {
  peer.corrections += batch.corrections.length;
  for (const diffEntry of batch.event_diffs) {
    for (const event of diffEntry.added) {
      peer.latest.set(event.id, event.payload);
    }
    for (const event of diffEntry.revoked) {
      peer.revoked.add(event.id);
      peer.latest.delete(event.id);
    }
    for (const replacement of diffEntry.replaced) {
      peer.latest.set(replacement.after.id, replacement.after.payload);
    }
  }
  for (const step of batch.confirmed_steps) {
    peer.confirmedTicks.push(step.tick);
    const lists: readonly (readonly RollbackWrappedEvent[])[] = [
      step.match_events,
      step.combat_events ?? [],
      step.lifecycle_events,
    ];
    for (const list of lists) {
      for (const event of list) {
        peer.confirmed.set(event.id, (peer.confirmed.get(event.id) ?? 0) + 1);
        const latest = peer.latest.get(event.id);
        if (latest !== undefined && !payloadsEqual(latest, event.payload)) {
          peer.stale += 1;
        }
      }
    }
  }
}

interface RunOptions {
  /** Deliver every `period` steps; omit to deliver every step. */
  readonly period?: number;
}

function run(host: SimHost, harness: RealHarness, steps: number, options: RunOptions = {}): void {
  const sampleWire = host.inputFrameNeutralSample();
  for (let step = 0; step < steps; step += 1) {
    for (const peer of harness.peers) {
      const batch = JSON.parse(peer.driver.advance(sampleWire)) as WasmDriverBatch;
      for (const checkpoint of batch.checkpoints) {
        peer.checkpoints.set(checkpoint.tick, checkpoint.hash);
      }
      const presentationBatch = consume(peer.presentation, harness.ports, peer.driver, batch);
      record(peer, presentationBatch);
    }
    if (options.period === undefined || (step + 1) % options.period === 0) {
      deliverAll(harness.peers);
    }
  }
}

// Every confirmed event published exactly once, in contiguous tick order,
// and never a payload a correction already replaced.
function assertPublishedOnce(peers: readonly RealPeer[]): void {
  peers.forEach((peer, index) => {
    for (const [id, count] of peer.confirmed) {
      expect(count, `peer ${index} published ${id} more than once`).toBe(1);
    }
    for (const id of peer.revoked) {
      expect(peer.confirmed.has(id), `peer ${index} confirmed a revoked event ${id}`).toBe(false);
    }
    expect(peer.stale, `peer ${index} published a payload a correction had already replaced`).toBe(0);
    for (let index2 = 1; index2 < peer.confirmedTicks.length; index2 += 1) {
      expect(peer.confirmedTicks[index2]).toBe((peer.confirmedTicks[index2 - 1] as number) + 1);
    }
  });
}

// A real substitute for the Lua original's `match_snapshot.hash` comparison:
// `@gc/wasm` never exposes a hash for an arbitrary retained `WasmMatchSnapshot`
// (confirmed by reading `rollback_events_bridge.rs`'s `WasmMatchSnapshot` --
// it is opaque with no hash method), but every `advance()` batch's own
// `checkpoints` field carries the driver's real periodic hash -- exactly the
// artifact the netcode's own hash-consensus mechanism produces for this
// purpose. Comparing those hashes at every boundary both peers checkpointed
// is the same "peers agree" claim, driven by genuinely produced Rust output.
function assertConfirmedAgreement(peers: readonly RealPeer[]): void {
  const [first, ...rest] = peers;
  expect(first, "no peers to compare").toBeDefined();
  if (first === undefined) {
    return;
  }
  let compared = 0;
  for (const [tick, hash] of first.checkpoints) {
    for (const other of rest) {
      const otherHash = other.checkpoints.get(tick);
      if (otherHash !== undefined) {
        compared += 1;
        expect(otherHash, `checkpoint ${tick} disagreed between peers`).toBe(hash);
      }
    }
  }
  expect(compared > 0, "peers shared no checkpointed boundary to compare").toBe(true);
}

describe("online match presentation (real wasm bridges, no combat-phase fixture needed)", () => {
  it("publishes each confirmed event exactly once under clean delivery", () => {
    const host = loadSimHost();
    const harness = buildHarness(host);
    run(host, harness, 40);
    assertPublishedOnce(harness.peers);
  });

  it("tracks the driver's own confirmation ceiling", () => {
    const host = loadSimHost();
    const harness = buildHarness(host);
    run(host, harness, 40);
    for (const peer of harness.peers) {
      const timeline = diagnostics(peer.presentation, harness.ports.rollbackEvents);
      const driverConfirmed = harness.ports.matchDriver.diagnostics(peer.driver).confirmed_output_tick;
      expect(timeline.confirmed_tick + harness.firstInputTick).toBe(driverConfirmed);
      expect(status(peer.presentation)).toBe("active");
    }
  });

  it("replaces the speculative tail on a correction and never re-publishes it", () => {
    const host = loadSimHost();
    const harness = buildHarness(host);
    run(host, harness, 48, { period: 6 });
    const corrected = harness.peers.reduce((sum, peer) => sum + peer.corrections, 0);
    assertPublishedOnce(harness.peers);
    expect(corrected > 0, "bursty delivery must produce at least one correction").toBe(true);
  });

  it("agrees between peers on every confirmed boundary it presented", () => {
    const host = loadSimHost();
    const harness = buildHarness(host);
    run(host, harness, 48, { period: 4 });
    assertConfirmedAgreement(harness.peers);
  });
});

// The seven combat correction phases, plus the two cases built on top of
// them -- still blocked. See the file header for the current, precise
// reason (no way to reach a pinned combat-phase boundary zero from
// TypeScript), which replaces the earlier "no MatchDriverBridge" claim.
describe.skip("online match presentation combat phases (blocked: no wasm-reachable way to build spec/support/online_combat_phases.lua's pinned boundary zeroes -- see the file header comment)", () => {
  it.skip("keeps feedback honest through a correction during windup", () => {});
  it.skip("keeps feedback honest through a correction during guard", () => {});
  it.skip("keeps feedback honest through a correction during contact", () => {});
  it.skip("keeps feedback honest through a correction during projectile_flight", () => {});
  it.skip("keeps feedback honest through a correction during stagger", () => {});
  it.skip("keeps feedback honest through a correction during ball_spill", () => {});
  it.skip("keeps feedback honest through a correction during immunity_expiry", () => {});
  it.skip("never publishes a combat cue a correction took away", () => {});
  it.skip("publishes the lifecycle exactly once through full time", () => {});
});

// ---------------------------------------------------------------------------
// Fake ports: scripted, literal responses only -- no rollback logic.
// ---------------------------------------------------------------------------

interface FakeTimeline {
  readonly maxUnconfirmedTicks: number;
  applied: Array<{ readonly from: number; readonly through: number; readonly count: number }>;
  confirmedTick: number;
  status: RollbackEventsStatus;
  /** When set, the next `apply` call fails with this status instead of succeeding. */
  failNextApply: boolean;
}

function fakeRollbackEvents(): RollbackEventsPort<FakeTimeline, number> {
  return {
    create(_initialSnapshot, maxUnconfirmedTicks): FakeTimeline {
      return { maxUnconfirmedTicks, applied: [], confirmedTick: -1, status: "active", failNextApply: false };
    },
    apply(timeline, from, through, steps): RollbackApplyResult {
      if (timeline.failNextApply) {
        return { ok: false, error: { message: "fake window exceeded", code: "unconfirmed_window_exceeded" } };
      }
      timeline.applied.push({ from, through, count: steps.length });
      const diff: RollbackEventDiff = {
        added: steps.map((step, offset) => ({
          id: `evt_${from + offset}`,
          tick: step.output.tick,
          domain: "match/test",
          ordinal: from + offset,
          payload: null,
        })),
        revoked: [],
        replaced: [],
      };
      return { ok: true, value: diff };
    },
    confirm(timeline, confirmedOutputTick): readonly RollbackEventStep[] {
      const steps: RollbackEventStep[] = [];
      for (let tick = timeline.confirmedTick + 1; tick <= confirmedOutputTick; tick += 1) {
        steps.push({
          tick,
          start_boundary: tick,
          end_boundary: tick + 1,
          state: { score: { home: 0, away: 0 }, time_left: 0, finished: false },
          match_events: [],
          lifecycle_events: [],
        });
      }
      timeline.confirmedTick = Math.max(timeline.confirmedTick, confirmedOutputTick);
      return steps;
    },
    diagnostics(timeline): RollbackEventsDiagnostics {
      return {
        status: timeline.status,
        confirmed_tick: timeline.confirmedTick,
        confirmed_boundary: timeline.confirmedTick + 1,
        max_unconfirmed_ticks: timeline.maxUnconfirmedTicks,
        retained_step_count: timeline.applied.length,
        retained_event_count: timeline.applied.reduce((sum, entry) => sum + entry.count, 0),
      };
    },
  };
}

interface FakeDriver {
  confirmedOutputTick: number;
}

function fakeMatchDriver(): MatchDriverPort<FakeDriver, number> {
  return {
    snapshot(_driver, boundaryTick): SnapshotLookup<number> {
      return { status: "present", tick: boundaryTick, snapshot: boundaryTick };
    },
    diagnostics(driver): { readonly confirmed_output_tick: number } {
      return { confirmed_output_tick: driver.confirmedOutputTick };
    },
  };
}

function output(tick: number): RollbackTickOutput {
  return {
    tick,
    start_boundary: tick,
    end_boundary: tick + 1,
    finished: false,
    score: { home: 0, away: 0 },
    time_left: 0,
    events: [],
    combat_events: [],
  };
}

function batch(outputs: readonly RollbackTickOutput[]): MatchDriverBatch {
  return { outputs };
}

describe("match presentation (pure control flow, fake ports)", () => {
  it("starts inactive-applied and active", () => {
    const rollbackEvents = fakeRollbackEvents();
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 100, 30);
    expect(presentation.first).toBe(100);
    expect(presentation.applied).toBe(-1);
    expect(status(presentation)).toBe("active");
    expect(diagnostics(presentation, rollbackEvents).status).toBe("active");
  });

  it("appends forward outputs in order without producing a correction", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    const driver: FakeDriver = { confirmedOutputTick: -1 };

    const result = consume(presentation, ports, driver, batch([output(0), output(1)]));

    expect(result.status).toBe("active");
    expect(result.corrections.length).toBe(0);
    expect(result.outputs.map((entry) => entry.tick)).toEqual([0, 1]);
    expect(result.event_diffs.length).toBe(2);
    expect(presentation.applied).toBe(1);
  });

  it("treats a tick at or below the applied ceiling as a correction and replaces the tail", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    const driver: FakeDriver = { confirmedOutputTick: -1 };

    consume(presentation, ports, driver, batch([output(0), output(1)]));
    expect(presentation.applied).toBe(1);

    // A corrected replay of tick 1, followed by a fresh tick 2 in the same
    // batch -- the correction must not swallow the fresh append past what
    // it is replacing.
    const result = consume(presentation, ports, driver, batch([output(1), output(2)]));

    expect(result.corrections.length).toBe(1);
    expect(result.corrections[0]).toMatchObject({
      causal_tick: 1,
      replaced_from_tick: 1,
      replaced_through_tick: 1,
      corrected_from_tick: 1,
      corrected_through_tick: 1,
    });
    // The correction's own tick (1) is not re-appended to `outputs` (it was
    // already pushed once, ahead of the branch split); the fresh tick 2 is
    // appended as an ordinary forward output afterwards.
    expect(result.outputs.map((entry) => entry.tick)).toEqual([1, 2]);
    expect(presentation.applied).toBe(2);
  });

  it("stops at unconfirmed_window_exceeded and never resumes", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    const driver: FakeDriver = { confirmedOutputTick: -1 };
    presentation.events.failNextApply = true;

    const first = consume(presentation, ports, driver, batch([output(0)]));
    expect(first.status).toBe("unconfirmed_window_exceeded");
    expect(status(presentation)).toBe("unconfirmed_window_exceeded");
    expect(presentation.applied).toBe(-1);

    // A later call does no further work: the timeline already gave up.
    const second = consume(presentation, ports, driver, batch([output(0), output(1)]));
    expect(second.status).toBe("unconfirmed_window_exceeded");
    expect(second.outputs.length).toBe(0);
  });

  it("clamps confirmation to what has actually been applied this batch", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    // The driver's confirmation ceiling is far ahead of anything presented
    // yet -- confirmation must not claim ticks the timeline never applied.
    const driver: FakeDriver = { confirmedOutputTick: 50 };

    const result = consume(presentation, ports, driver, batch([output(0), output(1)]));

    expect(result.confirmed_steps.map((step) => step.tick)).toEqual([0, 1]);
    expect(presentation.applied).toBe(1);
  });
});
