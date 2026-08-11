// Every assertion here is about a claim only a *live* rollback session can
// make: peers converging through a real correction, a real combat
// encounter being revoked and replaced by a resimulation, a real driver
// reaching `completed` at full time. All of that runs through
// `game.online.match_driver` and `sim.rollback_events`, both Rust-owned
// (`crates/gc-sim` / `crates/gc-netcode`; ARCHITECTURE.md §1.1).
//
// # Re-audited against the current `@gc/wasm` (12 of 13 cases now real)
//
// A prior pass here recorded five numbered blockers and left all 13 cases
// `it.skip`. A later pass re-verified against `crates/gc-wasm/src/
// match_driver_fixture_bridge.rs`/`match_driver_bridge.rs`/
// `rollback_events_bridge.rs`, found blockers 1-4 already stale or fixed,
// blocker 5 (`RollbackTickOutput` too narrow) real and fixed it, and built a
// real two-peer `MatchDriverBridge` harness (below) that unblocked four of
// the 13 cases -- the four that only need a plain session construction, not
// pinned combat geometry. That pass left the remaining nine
// (`describe("online match presentation combat phases...")` below) skipped:
// reaching a specific combat phase deterministically needed either a
// `gc-wasm` export of pinned per-phase boundary zeroes, or scripting real
// gameplay input precisely enough to reach each phase from scratch, and
// neither existed yet on `@gc/wasm`'s surface.
//
// That gap is now closed: `crates/gc-wasm/src/online_combat_phases_bridge.rs`
// (`onlineCombatPhaseIds`/`ScenarioJson`/`BoundaryZero`/`LiveSample`/
// `Observed`) is exactly the export the previous pass asked for -- a
// wasm-bindgen export of the same pinned per-phase fixture data as
// `crates/gc-netcode/tests/support/online_combat_phases.rs`, cross-checked
// field-for-field against that Rust module rather than re-derived by hand,
// and proven end-to-end by that module's own Rust test seeding a real
// `MatchDriverBridge` from
// `onlineCombatPhaseBoundaryZero`'s output and stepping it forward. Eight of
// the nine combat-phase cases run for real below, using
// `MatchDriverBridge`'s `initialSnapshotOverride` constructor parameter
// (`crates/gc-wasm/src/match_driver_bridge.rs`) to seed each peer from the
// same pinned, combat-active boundary zero -- exactly the "drive a real
// `MatchDriverBridge` from it" workflow that bridge's own module doc
// promises this file.
//
// The ninth ("publishes the lifecycle exactly once through full time") is
// still `it.skip`, but for a new, unrelated reason found *by* driving the
// real bridge to full time: `gc_netcode::match_driver`'s settle phase calls
// `std::time::SystemTime::now()` for its wall-clock bound
// (`crates/gc-netcode/src/match_driver.rs`'s `default_clock`), which traps
// under `wasm32-unknown-unknown` -- a real defect in `crates/gc-netcode`/
// `crates/gc-wasm` (out of this package's ownership), not a gap in this
// package. See that case's own comment.
//
// A real harness is built and driven below (`describe("online match
// presentation (real wasm bridges...")`), using
// `matchDriverFixtureFreezeJson`/`ManifestJson`/`PeerIds` to build two real
// `MatchDriverBridge` peers, shuttling `drainOutboundJson()` straight into
// the other peer's `enqueueInbound()` (no `StarTransportAdapter` needed for
// a controlled two-peer test -- see `net_diagnostics.spec.ts`'s header for
// why a *real* transport adapter is a separate, harder problem), and a real
// standalone `RollbackEventsTimeline` per peer as `newOnlineMatchPresentation`'s
// `events`. The combat-phase `describe` block reuses the same harness
// builder with an `initialSnapshotOverride` factory, and reuses `run` with
// its `sample`/`onBatch` hooks to script phase-specific input and check,
// tick by tick before the driver evicts it, whether a correction actually
// resimulated a tick that ran through the named phase.
//
// What is deliberately not built here, and does not need to be
// re-litigated: faking `combat_phases.boundary_zero`/`live_sample` well
// enough to reach a real combat phase would mean reimplementing combat
// geometry in this package, exactly what ARCHITECTURE.md §1.1 forbids on this
// side of the determinism line. `onlineCombatPhaseObserved` is called with
// opaque snapshot handles and raw JSON, exactly like every other wasm-bridge
// call in this file -- this module never inspects what "windup" or "guard"
// actually mean.

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
  host: SimHost,
): RollbackEventsPort<WasmRollbackEventsTimeline, WasmMatchSnapshot> {
  return {
    create(initialSnapshot, maxUnconfirmedTicks) {
      return host.RollbackEventsTimeline.create(initialSnapshot, maxUnconfirmedTicks);
    },
    apply(timeline, from, through, steps) {
      const outputsJson = JSON.stringify(steps.map((entry) => entry.output));
      const snapshots = steps.map((entry) => entry.snapshot);
      return JSON.parse(
        timeline.apply(from, through, outputsJson, snapshots),
      ) as RollbackApplyResult;
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
  /** Combat-domain (`domain` starting `"combat/"`) counterparts of `revoked`/
   * `event_diffs.replaced`/`event_diffs.added`/`confirmed` above -- see
   * `isCombat`'s doc for why these are tracked separately. */
  revokedCombat: number;
  replacedCombat: number;
  addedCombat: number;
  confirmedCombat: number;
}

interface RealHarness {
  readonly peers: readonly RealPeer[];
  readonly ports: MatchPresentationPorts<
    WasmRollbackEventsTimeline,
    WasmMatchSnapshot,
    WasmMatchDriverBridge
  >;
  readonly firstInputTick: number;
}

function buildHarness(
  host: SimHost,
  mode: "1v1" | "2v2" = "1v1",
  maxUnconfirmedTicks = 30,
  // Builds this peer's boundary-zero override, called once per peer so each
  // gets its own freshly-built (byte-identical) handle -- `WasmMatchSnapshot`
  // is consumed by value, so one handle can never seed two peers (see
  // `MatchDriverBridgeConstructor`'s doc in `packages/wasm/src/types.ts`).
  // Omitted for the plain (non-combat-phase) harness below, which relies on
  // each peer's own `Session.capture_snapshot()` default instead.
  buildInitialSnapshotOverride?: () => WasmMatchSnapshot,
): RealHarness {
  const freezeJson = host.matchDriverFixtureFreezeJson(mode);
  const manifestJson = host.matchDriverFixtureManifestJson(mode);
  const firstInputTick = (JSON.parse(freezeJson) as { readonly first_input_tick: number })
    .first_input_tick;
  const peerIds = host.matchDriverFixturePeerIds(mode);
  const rollbackEvents = wasmRollbackEventsPort(host);
  const matchDriver = wasmMatchDriverPort();
  const ports: MatchPresentationPorts<
    WasmRollbackEventsTimeline,
    WasmMatchSnapshot,
    WasmMatchDriverBridge
  > = {
    rollbackEvents,
    matchDriver,
  };

  const peers: RealPeer[] = peerIds.map((peerId, index) => {
    const role = index === 0 ? "host" : "guest";
    const session = newSession(host);
    const driver = new host.MatchDriverBridge(
      session,
      role,
      peerId,
      freezeJson,
      manifestJson,
      undefined,
      undefined,
      buildInitialSnapshotOverride?.(),
    );
    driver.initializeTransport();
    // Star topology: the host opens a slot per guest; each guest opens only
    // the host (a guest's transport capacity is fixed at 1 -- see
    // `net_diagnostics.spec.ts`'s harness for the "at peer capacity" this
    // avoids once more than one guest is in play).
    const others =
      role === "host"
        ? peerIds.filter((candidate) => candidate !== peerId)
        : [peerIds[0] as string];
    for (const other of others) {
      driver.openPeer(other);
      driver.setPeerConnected(other);
    }
    const initialSnapshot = driver.initialSnapshotHandle();
    const presentation = newOnlineMatchPresentation(
      rollbackEvents,
      initialSnapshot,
      firstInputTick,
      maxUnconfirmedTicks,
    );
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
      revokedCombat: 0,
      replacedCombat: 0,
      addedCombat: 0,
      confirmedCombat: 0,
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
  const drained = peers.map(
    (peer) => JSON.parse(peer.driver.drainOutboundJson()) as WasmOutboundEnvelope[],
  );
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
        payload,
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

// `sim.rollback_events`' wrapped combat events carry a `domain` starting
// `combat/` (`combat/<kind>/<n>`, `crates/gc-sim/src/rollback_events.rs`)
// -- everything else (match, lifecycle) does not.
function isCombat(domain: string): boolean {
  return domain.startsWith("combat/");
}

// Tracks what each peer's presentation timeline has published, so the
// assertions below can check "exactly once" and "never a revoked id" for
// real. The `*Combat` counters
// are the combat-phase cases' own claim -- a correction that merely landed
// near a phase is not enough; it must have rewritten or introduced combat
// feedback (see the combat-phase `describe` block below).
function record(peer: RealPeer, batch: RollbackPlayableLabBatch): void {
  peer.corrections += batch.corrections.length;
  for (const diffEntry of batch.event_diffs) {
    for (const event of diffEntry.added) {
      peer.latest.set(event.id, event.payload);
      if (isCombat(event.domain)) {
        peer.addedCombat += 1;
      }
    }
    for (const event of diffEntry.revoked) {
      peer.revoked.add(event.id);
      peer.latest.delete(event.id);
      if (isCombat(event.domain)) {
        peer.revokedCombat += 1;
      }
    }
    for (const replacement of diffEntry.replaced) {
      peer.latest.set(replacement.after.id, replacement.after.payload);
      if (isCombat(replacement.after.domain)) {
        peer.replacedCombat += 1;
      }
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
        if (isCombat(event.domain)) {
          peer.confirmedCombat += 1;
        }
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
  /** Overrides the neutral sample wire per (0-based step, 0-based peer
   * index) -- e.g. `combat_phases.live_sample`'s TypeScript counterpart,
   * `onlineCombatPhaseLiveSample`. */
  readonly sample?: (step: number, peerIndex: number) => string;
  /** Called once per peer per step, with that step's presentation batch,
   * before the next step evicts the driver's retained boundaries -- used by
   * the combat-phase cases below to check a correction actually
   * resimulated a tick that ran through the named phase. */
  readonly onBatch?: (peer: RealPeer, peerIndex: number, batch: RollbackPlayableLabBatch) => void;
}

function run(host: SimHost, harness: RealHarness, steps: number, options: RunOptions = {}): void {
  const neutralWire = host.inputFrameNeutralSample();
  for (let step = 0; step < steps; step += 1) {
    harness.peers.forEach((peer, peerIndex) => {
      const wire = options.sample ? options.sample(step, peerIndex) : neutralWire;
      const batch = JSON.parse(peer.driver.advance(wire)) as WasmDriverBatch;
      for (const checkpoint of batch.checkpoints) {
        peer.checkpoints.set(checkpoint.tick, checkpoint.hash);
      }
      const presentationBatch = consume(peer.presentation, harness.ports, peer.driver, batch);
      record(peer, presentationBatch);
      options.onBatch?.(peer, peerIndex, presentationBatch);
    });
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
    expect(peer.stale, `peer ${index} published a payload a correction had already replaced`).toBe(
      0,
    );
    for (let index2 = 1; index2 < peer.confirmedTicks.length; index2 += 1) {
      expect(peer.confirmedTicks[index2]).toBe((peer.confirmedTicks[index2 - 1] as number) + 1);
    }
  });
}

// A real substitute for a `match_snapshot.hash` comparison:
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
      const driverConfirmed = harness.ports.matchDriver.diagnostics(
        peer.driver,
      ).confirmed_output_tick;
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
// them. Unblocked by `crates/gc-wasm/src/online_combat_phases_bridge.rs`
// (see the file header): `onlineCombatPhaseBoundaryZero` builds exactly the
// pinned, combat-active `MatchSnapshot`
// `crates/gc-netcode/tests/support/online_combat_phases.rs` pins per phase,
// as `MatchDriverBridge`'s own `initialSnapshotOverride` constructor
// parameter; `onlineCombatPhaseLiveSample` scripts the same per-phase input
// `combat_phases.live_sample` does; `onlineCombatPhaseObserved` is the same
// `combat_phases.observed` predicate. None of the three need this package to
// know a single fact about combat mechanics -- they are called with opaque
// ids/snapshots/JSON, exactly like every other wasm-bridge call in this
// file.
//
// `PHASE_DELIVER_PERIOD` is 12: long enough to force a real correction,
// short enough to stay inside the ~30-tick unconfirmed window.
//
// `PHASE_STEPS` needed more thought: 240 steps was the working assumption
// for every phase, on the theory that it reliably produces at least one
// corrected `ball_spill` tick. Empirically, against this package's real
// `MatchDriverBridge`/`WasmStarTransport` rollback timing, it does not --
// confirmed deterministically (not flaky; the sim is fully seeded) across
// repeated runs, and by scanning every combat event this fixture produces
// over 240 steps directly: exactly one real `CombatEventKind::BallSpill`
// event occurs in the whole run, and it lands outside any tick range a
// correction actually resimulates. This is a legitimate consequence of the
// wasm rollback scheduler's real queue/poll timing: a correction's schedule
// is a function of transport/queue timing infrastructure, not of the
// deterministic sim tick stream alone, so it need not land on a fixed
// schedule -- not a bug in `consume`, `RollbackEventsTimeline`, or the
// combat-phase bridge. 480 (double) was the smallest budget tried that
// reliably (deterministically) produces a corrected `ball_spill` hit; every
// other phase already succeeds at 240 and continues to at 480. This is a
// run-length/test-infrastructure adjustment, not a weakened assertion --
// every case's own claim (`phaseTicks > 0`) is exactly as strict as before.
const PHASE_IDS = [
  "windup",
  "guard",
  "contact",
  "projectile_flight",
  "stagger",
  "ball_spill",
  "immunity_expiry",
] as const;
type PhaseId = (typeof PHASE_IDS)[number];

const PHASE_DELIVER_PERIOD = 12;
const PHASE_STEPS = 480;

// Bursts delivery while scripting the phase's live input, and -- inside
// each step, before the driver evicts its retained boundaries -- checks
// whether a correction actually resimulated a tick that ran through the
// named phase. Returns, per peer, how many corrected ticks did.
function runPhase(host: SimHost, harness: RealHarness, phaseId: PhaseId): number[] {
  const observed = harness.peers.map(() => 0);
  const first = harness.firstInputTick;
  run(host, harness, PHASE_STEPS, {
    period: PHASE_DELIVER_PERIOD,
    sample: (step, peerIndex) => host.onlineCombatPhaseLiveSample(phaseId, step, peerIndex + 1),
    onBatch: (peer, peerIndex, batch) => {
      // `batch.outputs` mixes a correction's re-derived ticks with the
      // ordinary forward tick appended on nearly every call; this map lets
      // a correction's tick range be paired back up with its own combat
      // events. A repeated tick would silently overwrite the earlier entry,
      // so this asserts uniqueness rather than let that happen quietly.
      const byTick = new Map<number, RollbackTickOutput>();
      for (const output of batch.outputs) {
        if (byTick.has(output.tick)) {
          throw new Error("one presentation batch reported a tick twice");
        }
        byTick.set(output.tick, output);
      }
      for (const correction of batch.corrections) {
        for (
          let tick = correction.corrected_from_tick;
          tick <= correction.corrected_through_tick;
          tick += 1
        ) {
          const before = harness.ports.matchDriver.snapshot(peer.driver, tick + first);
          const after = harness.ports.matchDriver.snapshot(peer.driver, tick + 1 + first);
          const beforeRetained = before.status === "present" || before.status === "retained";
          const afterRetained = after.status === "present" || after.status === "retained";
          if (
            beforeRetained &&
            before.snapshot !== undefined &&
            afterRetained &&
            after.snapshot !== undefined
          ) {
            const combatEventsJson = JSON.stringify(byTick.get(tick)?.combat_events ?? []);
            if (
              host.onlineCombatPhaseObserved(
                phaseId,
                before.snapshot,
                after.snapshot,
                combatEventsJson,
              )
            ) {
              observed[peerIndex] = (observed[peerIndex] ?? 0) + 1;
            }
          }
        }
      }
    },
  });
  return observed;
}

describe("online match presentation combat phases (real wasm bridges + online_combat_phases_bridge)", () => {
  for (const phaseId of PHASE_IDS) {
    it(`keeps feedback honest through a correction during ${phaseId}`, () => {
      const host = loadSimHost();
      const harness = buildHarness(host, "1v1", 30, () =>
        host.onlineCombatPhaseBoundaryZero(phaseId),
      );
      const observed = runPhase(host, harness, phaseId);

      let phaseTicks = 0;
      let corrections = 0;
      let replaced = 0;
      let added = 0;
      harness.peers.forEach((peer, index) => {
        expect(
          status(peer.presentation),
          `peer ${index}'s timeline gave up during ${phaseId}`,
        ).toBe("active");
        phaseTicks += observed[index] ?? 0;
        corrections += peer.corrections;
        replaced += peer.replacedCombat;
        added += peer.addedCombat;
      });
      expect(corrections > 0, `the ${phaseId} burst never corrected anyone`).toBe(true);
      expect(phaseTicks > 0, `no correction ever resimulated a ${phaseId} tick`).toBe(true);
      // The correction did not merely happen near the phase: it rewrote or
      // introduced combat cues that presentation then had to reconcile. A
      // run where the corrected tail produced byte-identical feedback would
      // satisfy every assertion above for free.
      expect(
        replaced + added > 0,
        `no combat cue was rewritten or introduced during ${phaseId}`,
      ).toBe(true);
      assertPublishedOnce(harness.peers);
      assertConfirmedAgreement(harness.peers);
    });
  }

  // Revoking a *combat* cue is the rare half of the contract, and it has to
  // be sought out rather than waited for: the unarmed scrum (`contact`'s
  // fixture) is where it happens, because eight bodies are inside one 30px
  // reach of each other, so a corrected pixel is the difference between a
  // contact and a miss.
  it("never publishes a combat cue a correction took away", () => {
    const host = loadSimHost();
    const harness = buildHarness(host, "1v1", 30, () =>
      host.onlineCombatPhaseBoundaryZero("contact"),
    );
    runPhase(host, harness, "contact");
    const revoked = harness.peers.reduce((sum, peer) => sum + peer.revokedCombat, 0);
    expect(revoked > 0, "the scrum never revoked a speculative combat cue").toBe(true);
    assertPublishedOnce(harness.peers);
    assertConfirmedAgreement(harness.peers);
  });

  // The exactly-once contract has to survive the end of the match, not only
  // hold mid-run -- this takes a combat-active boundary zero (a 4-second
  // match, so the run below reaches full time) all the way through under
  // bursty delivery, using a `4 * 60 + 90` step budget with `period = 6`.
  //
  // # Unblocked
  //
  // A prior pass here recorded this as blocked by a real defect: driving a
  // `MatchDriverBridge` to full time trapped the wasm instance
  // ("RuntimeError: unreachable"), because `gc_netcode::match_driver`'s
  // settle phase calls `default_clock` (`crates/gc-netcode/src/
  // match_driver.rs`), which uses `std::time::SystemTime::now()` --
  // unimplemented on `wasm32-unknown-unknown`. That is now fixed, entirely
  // on the Rust side (out of this package's ownership; nothing here
  // changed): `crates/gc-wasm/src/match_driver_bridge.rs`'s
  // `driver_settle_clock` injects `js_sys::Date::now() / 1000.0` as
  // `MatchDriverOptions.clock` on `wasm32`, reproducing `default_clock`'s
  // own documented wall-clock-seconds-since-epoch semantics instead of
  // leaving the field `None`. `packages/wasm/src/
  // match_driver_fixture.spec.ts`'s "does not trap the wasm instance when a
  // match runs all the way to full time" is the wasm-target regression test
  // proving it end to end.
  //
  // That fix is seat-agnostic -- the trap fired for any driver reaching full
  // time, host or guest, alone or paired -- so this case is ported for real
  // using the same two-peer, both-real `buildHarness`/`run` machinery every
  // other case in this describe block already uses, rather than the lone
  // `humans: 1` host that fixture's regression test uses to sidestep an
  // unrelated concern (a second, *silent* peer -- opened in the transport
  // but with no driver of its own ever calling `advance`/delivering --
  // stalls on `confirmation_stalled` around tick 30, `ROLLBACK_WINDOW_TICKS`,
  // long before full time). Here both peers are real `MatchDriverBridge`
  // instances that call `advance` and deliver every `run` step, exactly
  // like the seven combat-phase cases above that already complete hundreds
  // of steps without stalling -- so there is no silent peer to seat around.
  it("publishes the lifecycle exactly once through full time", () => {
    const host = loadSimHost();
    const harness = buildHarness(host, "1v1", 30, () =>
      host.onlineCombatPhaseBoundaryZero("contact", 4),
    );
    run(host, harness, 4 * 60 + 90, { period: 6 });

    let restarts = 0;
    let combatRows = 0;
    harness.peers.forEach((peer, index) => {
      expect(JSON.parse(peer.driver.statusJson()), `peer ${index}'s driver`).toBe("completed");
      expect(status(peer.presentation), `peer ${index}`).toBe("active");
      combatRows += peer.confirmedCombat;
      let fullTime = 0;
      for (const id of peer.confirmed.keys()) {
        if (id.includes("lifecycle/full_time")) {
          fullTime += 1;
        } else if (id.includes("lifecycle/goal") || id.includes("lifecycle/kickoff")) {
          restarts += 1;
        }
      }
      // The unarmed scrum this boundary zero seeds (eight bodies stacked
      // around the pitch's midline, fighting rather than advancing, under
      // the fixture's own no-goal-limit duration) confirms
      // `lifecycle/full_time` exactly once per peer, and nothing else -- a
      // record, not a requirement: `assertPublishedOnce` below already
      // holds kickoff/goal rows to exactly once each if they ever appear,
      // so pinning `restarts` at zero means the day this fixture starts
      // scoring, this assertion fails and someone decides what the case now
      // covers, instead of a stale comment quietly claiming coverage it
      // never had.
      expect(fullTime, `peer ${index} published full time ${fullTime} times`).toBe(1);
    });
    expect(restarts, "the scrum fixture scored, so this case now covers more than full time").toBe(
      0,
    );
    expect(combatRows > 0, "a combat-active run confirmed no combat feedback at all").toBe(true);
    assertPublishedOnce(harness.peers);
    assertConfirmedAgreement(harness.peers);
  });
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
      return {
        maxUnconfirmedTicks,
        applied: [],
        confirmedTick: -1,
        status: "active",
        failNextApply: false,
      };
    },
    apply(timeline, from, through, steps): RollbackApplyResult {
      if (timeline.failNextApply) {
        return {
          ok: false,
          error: { message: "fake window exceeded", code: "unconfirmed_window_exceeded" },
        };
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
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = {
      rollbackEvents,
      matchDriver,
    };
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
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = {
      rollbackEvents,
      matchDriver,
    };
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
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = {
      rollbackEvents,
      matchDriver,
    };
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
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = {
      rollbackEvents,
      matchDriver,
    };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    // The driver's confirmation ceiling is far ahead of anything presented
    // yet -- confirmation must not claim ticks the timeline never applied.
    const driver: FakeDriver = { confirmedOutputTick: 50 };

    const result = consume(presentation, ports, driver, batch([output(0), output(1)]));

    expect(result.confirmed_steps.map((step) => step.tick)).toEqual([0, 1]);
    expect(presentation.applied).toBe(1);
  });
});
