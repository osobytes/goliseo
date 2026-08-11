// The "diagnostics schema" describe block below is pure and needs no live
// driver. Everything else drives `net_diagnostics_fixture.ts`'s
// `fixture.harness`, which needs a real `game.online.match_driver`
// (Rust-owned, `crates/gc-netcode`).
//
// Two different kinds of case follow, kept apart deliberately:
//
//   * Where the *only* reason a case reaches for `fixture.harness` is
//     to obtain a valid, opted-in recorder (manifest/freeze data plus a
//     handful of direct `record_*` calls), this file constructs that
//     recorder directly with `newTestRecorder` instead. The assertion under
//     test is unchanged; only how the fixture is obtained is different.
//   * Where the case's actual claim is about the *driver's own* behaviour
//     under a live rollback session (a real correction, a real hash
//     mismatch, a real ownership violation, a real transport loss) it is
//     written as `it.skip`, because faking that behaviour in TypeScript
//     would mean re-implementing rollback scheduling here -- exactly what
//     ARCHITECTURE.md §1.1 says must never happen on this side of the
//     determinism line.
//
// # Re-audited against the current `@gc/wasm`
//
// The claim this header used to make -- "no wasm bridge exists for
// match_driver_fixture/input_protocol/input_frame" -- is now **stale** for
// all three: `match_driver_fixture_bridge.rs`, `input_protocol_bridge.rs`,
// and `input_frame_bridge.rs` all landed and are exercised by real specs
// (`packages/wasm/src/match_driver_fixture.spec.ts`,
// `input_protocol.spec.ts`, `input_frame.spec.ts`). A real, two-peer
// `MatchDriverBridge` harness is built directly below (not through
// `net_diagnostics_fixture.ts`'s `NetDiagnosticsFixtureEnv` -- see why in
// the next paragraph), and now unblocks all eleven of this file's
// live-driver cases.
//
// `net_diagnostics_fixture.ts`'s own `MatchDriverPort` (`create`/`advance`/
// `diagnostics`, taking an *injected* `transport: StarTransportAdapter` the
// driver calls into) is the same dependency-injection shape as
// `match_driver.new({transport = tap})`, where the driver calls
// `tap:send`/`tap:poll` itself, and `DiagnosticTransport`'s wrapped methods
// record star/channel/packet diagnostics as a side effect of being called
// *by the driver*. `MatchDriverBridge` does not support that shape at all:
// it owns its own internal transport (`crate::wasm_transport::WasmStarTransport`,
// which carries no `#[wasm_bindgen]` attribute of its own and is never
// injectable) and expects the *caller* to relay bytes via
// `drainOutboundJson`/`enqueueInbound` -- the "queue/drain seam"
// `match_driver_bridge.rs`'s module doc describes. So a `DiagnosticTransport`
// tap can never wrap a `MatchDriverBridge` directly, and that is why the
// real harness below relays bytes itself (exactly like
// `match_presentation.spec.ts`'s real harness) instead of going through
// `net_diagnostics_fixture.ts`.
//
// `TransportStarDiagnostics`/`TransportChannelDiagnostics`-shaped data for
// that internal transport *is* reachable now, just not via a tap:
// `MatchDriverBridge.transportDiagnosticsJson()` (`match_driver_bridge.rs`)
// wraps `WasmStarTransport::diagnostics()` directly, in the same JSON shape
// `match_driver_fixture_bridge::star_diagnostics_to_json` already produces
// for the fixture's in-process star. This closed what used to be the real
// gap here: `wasm_transport.rs`'s `diagnostics()` handed back `input:
// TransportChannelDiagnostics::default()` verbatim regardless of traffic --
// `PeerLink` now carries separate `control`/`input` `ChannelCounters`,
// credited by `send`/`broadcast`/`enqueue_inbound`, and both channel blocks
// carry `state: Some(peer.state)` -- confirmed against `wasm_transport.rs`'s
// own `control_and_input_channel_diagnostics_are_tracked_independently` and
// `ice_state_is_never_empty_once_a_peer_exists_and_tracks_its_lifecycle`
// tests. `outbound_depth`/`inbound_depth`/`buffered_amount` stay `0` on both
// channels deliberately -- that queue/buffer state lives in the browser's
// `RTCDataChannel`, never surfaced to Rust -- and `dropped_outbound` is `0`
// because this transport enforces no outbound bound; those are honest
// zeros, not gaps. `runReal` below already calls
// `transportDiagnosticsJson()` every step behind the opt-in
// `recordTransportDiagnostics` flag -- see that option's own doc.
//
// All eleven of this file's live-driver cases are ported for real below:
// three "live-driver runs, transport-shaped claims" cases needing
// `runtime.star`/`runtime.peers`/packet-lifecycle fields; three
// "live-driver runs, canonical/simulation claims" cases needing only
// `MatchDriverBridge.advance()`'s batch and `diagnosticsJson()`; and five
// "live-driver fault detection" cases reachable via `observeCheckpoint`,
// `enqueueInbound` (forged bundles), or `setPeerDisconnected` directly.

import { describe, expect, it } from "vitest";
import { newMessage, fakeStar, type TransportChannelDiagnostics, type TransportPeerMessage, type TransportStarDiagnostics } from "@gc/transport";
import { loadSimHost } from "@gc/wasm";
import type { MatchDriverBridge as WasmMatchDriverBridge, SimHost, SimSession } from "@gc/wasm";
import * as schema from "./diagnostics_schema.ts";
import {
  newNetDiagnostics,
  recordAnchor,
  recordCheckpoint,
  recordControl,
  recordEvent,
  recordMismatch,
  recordPacket,
  recordRuntimeSample,
  recordSignal,
  recordStep,
  recordTeardown,
  recordTransport,
  optInExport,
  exportArtifact,
  canonicalBytes,
  canonicalDigest,
  digest as recorderDigest,
  summary,
  EXPORT,
  SCHEMA_VERSION,
  type CoordinatorFreeze,
  type MatchDriverBatch,
  type MatchDriverDiagnostics,
  type MatchMode,
  type NetDiagnostics,
  type NetDiagnosticsLimits,
  type NetDiagnosticsOptions,
  type ProtocolControlMessage,
  type ProtocolDecoder,
  type SessionManifest,
  type SessionMessageKind,
  type SessionRole,
} from "./net_diagnostics.ts";
import { DiagnosticTransport } from "./diagnostic_transport.ts";
import { build as desyncPackageBuild, encode as desyncPackageEncode } from "./desync_package.ts";

// ---------------------------------------------------------------------------
// Test fixtures (direct construction -- see the module doc comment)
// ---------------------------------------------------------------------------

function testManifest(overrides: Partial<SessionManifest> = {}): SessionManifest {
  return {
    session_id: "session_1",
    match_mode: "2v2",
    combat_status: "accepted_proceed",
    build_id: "build_1",
    source_id: "source_1",
    content_id: "content_1",
    tuning_id: "tuning_1",
    match_config_id: "match_config_1",
    fixture_id: "fixture_1",
    arena_id: "arena_1",
    combat_rules_id: "combat_rules_1",
    gameplay_ai_policy_id: "policy_1",
    protocol_version: 1,
    input_version: 1,
    snapshot_version: 1,
    tape_version: 1,
    combat_schema_version: 1,
    seed: 1,
    tick_rate: 60,
    duration_ticks: 18000,
    max_goals: 5,
    ...overrides,
  };
}

function testFreeze(overrides: Partial<CoordinatorFreeze> = {}): CoordinatorFreeze {
  return {
    manifest_id: "0123456789abcdef",
    assignment_id: "fedcba9876543210",
    countdown_id: "countdown_1",
    first_input_tick: 0,
    ...overrides,
  };
}

function neverDecodes(): ProtocolDecoder {
  return () => null;
}

function newTestRecorder(overrides: Partial<NetDiagnosticsOptions> = {}): NetDiagnostics {
  return newNetDiagnostics({
    role: "host",
    peer_id: "host_1",
    manifest: testManifest(),
    freeze: testFreeze(),
    export_opt_in: true,
    decodeControlMessage: neverDecodes(),
    ...overrides,
  });
}

function testDriverDiagnostics(overrides: Partial<MatchDriverDiagnostics> = {}): MatchDriverDiagnostics {
  return {
    status: "active",
    present_input_tick: 0,
    confirmed_input_tick: -1,
    confirmed_output_tick: -1,
    retained_floor_tick: 0,
    rollback_count: 0,
    correction_count: 0,
    predicted_slot_samples: 0,
    max_rollback_depth: 0,
    hash_mismatches: 0,
    checkpoint_count: 0,
    owned: [],
    authored: [],
    ...overrides,
  };
}

function testDriverBatch(overrides: Partial<MatchDriverBatch> = {}): MatchDriverBatch {
  return {
    input_tick: 0,
    applied_rows: 0,
    reconciliations: 0,
    sent_packets: 0,
    outputs: [],
    checkpoints: [],
    control: [],
    ...overrides,
  };
}

function channelDiagnostics(): TransportChannelDiagnostics {
  return {
    state: "connected",
    outbound_depth: 0,
    inbound_depth: 0,
    buffered_amount: 0,
    sent: 1,
    received: 1,
    dropped_outbound: 0,
    dropped_inbound: 0,
  };
}

function starDiagnostics(overrides: Partial<TransportStarDiagnostics> = {}): TransportStarDiagnostics {
  return {
    role: "host",
    state: "connected",
    capacity: 7,
    peer_count: 1,
    queue_limit: 64,
    buffered_amount_limit: 65536,
    event_depth: 0,
    sent: 1,
    received: 1,
    dropped_outbound: 0,
    dropped_inbound: 0,
    malformed: 0,
    unsupported_version: 0,
    overflow: 0,
    backpressure: 0,
    last_error: null,
    peers: [
      {
        peer_id: "guest_1",
        slot: 1,
        state: "connected",
        ice_state: "connected",
        control: channelDiagnostics(),
        input: channelDiagnostics(),
        sequence_gaps: 0,
        backpressure: 0,
        malformed: 0,
        last_error: null,
      },
    ],
    ...overrides,
  };
}

// The exported artifact's shape is dynamically constructed (see
// `net_diagnostics.ts`'s `exportArtifact`), so this narrow view names only
// the fields these tests actually read back.
interface TestArtifact {
  readonly schema_version: number;
  readonly session: Record<string, unknown>;
  readonly collection: {
    readonly rejected_values: number;
    readonly retention: Record<string, string>;
    readonly dropped: Record<string, number>;
  };
  readonly canonical: {
    readonly simulation: {
      readonly status: string;
      readonly step_count: number;
      readonly confirmed_output_tick: number;
      readonly input_delay_ticks: number;
      readonly checkpoint_count: number;
      readonly rollback_count: number;
      readonly resimulated_ticks: number;
      readonly max_rollback_depth: number;
      readonly terminal_status?: string;
      readonly terminal_failure?: string;
      readonly terminal_detail?: string;
      readonly terminal_tick?: number;
      readonly retained_floor_tick: number;
      readonly late_input_tick?: number;
    };
    readonly checkpoints: readonly { readonly tick: number; readonly hash: string; readonly live: Readonly<Record<string, string>> }[];
    readonly mismatches: readonly {
      readonly tick: number;
      readonly peer_id: string;
      readonly local_hash: string;
      readonly remote_hash: string;
      readonly first_difference_path?: string;
    }[];
    readonly control: readonly { readonly ordinal: number; readonly kind: string }[];
    readonly delivery: {
      readonly sent: number;
      readonly arrived: number;
      readonly packets: readonly {
        readonly sender_id: string;
        readonly disposition: string;
        readonly sequence: number;
        readonly payload_bytes: number;
        readonly sample_step: number;
        readonly send_transport_tick: number;
        readonly authority_input_tick: number;
        readonly arrival_transport_tick?: number;
        readonly apply_input_tick?: number;
      }[];
    };
    readonly worst_correction?: {
      readonly causal_tick: number;
      readonly through_tick: number;
      readonly depth: number;
      readonly resimulated_ticks: number;
    };
    readonly events: {
      readonly added: number;
      readonly resimulated_tick_count: number;
      readonly unchanged: number;
      readonly replaced: number;
      readonly revoked: number;
    };
  };
  readonly runtime: {
    readonly star:
      | {
          readonly role: string;
          readonly state: string;
          readonly capacity: number;
          readonly peer_count: number;
          readonly sent: number;
          readonly received: number;
          readonly dropped_outbound: number;
          readonly dropped_inbound: number;
          readonly malformed: number;
          readonly overflow: number;
          readonly last_error?: string;
        }
      | undefined;
    readonly peers: readonly {
      readonly peer_id: string;
      readonly state: string;
      readonly sequence_gaps: number;
      readonly backpressure: number;
      readonly malformed: number;
      readonly control: {
        readonly buffered_amount: number;
        readonly outbound_depth: number;
        readonly inbound_depth: number;
        readonly dropped_outbound: number;
        readonly dropped_inbound: number;
        readonly sent: number;
      };
      readonly input: {
        readonly buffered_amount: number;
        readonly outbound_depth: number;
        readonly inbound_depth: number;
        readonly dropped_outbound: number;
        readonly dropped_inbound: number;
        readonly sent: number;
      };
      readonly last_error?: string;
    }[];
    readonly events: readonly { readonly ordinal: number; readonly detail?: string; readonly code?: string }[];
    readonly signals: readonly { readonly content: string; readonly byte_length: number; readonly direction: string }[];
    readonly teardown: { readonly requested: boolean; readonly complete: boolean; readonly orphaned_peers: readonly string[] } | undefined;
    readonly latency: readonly {
      readonly peer_id: string;
      readonly sample_count: number;
      readonly rtt_ms_min?: number;
      readonly rtt_ms_max?: number;
      readonly monotonic_ms_first?: number;
      readonly monotonic_ms_last?: number;
    }[];
  };
  readonly anchors: readonly { readonly mapping_error_ms: number }[];
}

function exportOf(recorder: NetDiagnostics): TestArtifact {
  const result = exportArtifact(recorder);
  if (!result.ok) {
    throw new Error(result.error);
  }
  return result.value as unknown as TestArtifact;
}

// Every string anywhere in an artifact, so a redaction assertion can be
// about the whole thing rather than about the handful of fields a test
// remembered.
function collectStrings(value: unknown, out: string[]): void {
  if (typeof value === "string") {
    out.push(value);
  } else if (Array.isArray(value)) {
    for (const child of value) {
      collectStrings(child, out);
    }
  } else if (typeof value === "object" && value !== null) {
    for (const [key, child] of Object.entries(value)) {
      collectStrings(key, out);
      collectStrings(child, out);
    }
  }
}

function allText(artifact: unknown): string {
  const parts: string[] = [];
  collectStrings(artifact, parts);
  parts.sort();
  return parts.join("\n");
}

function assertAbsent(artifact: unknown, poison: string): void {
  const text = allText(artifact);
  expect(text.includes(poison), `${poison} survived into the export`).toBe(false);
  for (const token of poison.match(/[\w.\-:@]+/g) ?? []) {
    if (token.length >= 6 && /[.:@]/.test(token)) {
      expect(text.includes(token), `fragment ${token} survived into the export`).toBe(false);
    }
  }
}

// ---------------------------------------------------------------------------
// Real harness: two `MatchDriverBridge` peers relaying `drainOutboundJson`/
// `enqueueInbound` directly (see the file header for why not through
// `net_diagnostics_fixture.ts`), driving real `NetDiagnostics` recorders.
// ---------------------------------------------------------------------------

function newSession(host: SimHost): SimSession {
  return new host.Session("nebula", "orion", 7, 20, 3);
}

function decodeControlMessageReal(host: SimHost): ProtocolDecoder {
  return (payload) => {
    try {
      const header = host.decodeControlMessageHeader(payload);
      return { kind: header.kind as SessionMessageKind, sequence: header.sequence, message_id: header.message_id };
    } catch {
      return null;
    }
  };
}

interface PendingArrival {
  readonly senderId: string;
  readonly message: WasmOutboundEnvelope["message"];
}

interface RealNetPeer {
  readonly peerId: string;
  readonly role: SessionRole;
  readonly driver: WasmMatchDriverBridge;
  readonly session: SimSession;
  readonly recorder: NetDiagnostics;
  /** Envelopes delivered (`enqueueInbound`ed) since this peer's own last
   * `advance()` call -- drained and recorded as "arrived" right before this
   * peer's *next* `advance()`, mirroring `DiagnosticTransport.pollInbound`'s
   * real timing (recorded synchronously inside the receiving peer's own
   * poll, not at relay/pump time) -- see `runReal`'s doc for why this
   * matters for `apply_input_tick`. */
  pendingArrivals: PendingArrival[];
}

interface RealNetHarness {
  readonly peers: readonly RealNetPeer[];
  readonly freeze: CoordinatorFreeze;
  readonly manifest: SessionManifest;
  step: number;
  clock_ms: number;
}

function buildRealHarness(host: SimHost, mode: MatchMode, hashIntervalTicks: number): RealNetHarness {
  const freezeJson = host.matchDriverFixtureFreezeJson(mode);
  const manifestJson = host.matchDriverFixtureManifestJson(mode);
  const freeze = JSON.parse(freezeJson) as CoordinatorFreeze;
  const manifest = JSON.parse(manifestJson) as SessionManifest;
  const peerIds = host.matchDriverFixturePeerIds(mode);
  const decode = decodeControlMessageReal(host);

  const peers: RealNetPeer[] = peerIds.map((peerId, index) => {
    const role: SessionRole = index === 0 ? "host" : "guest";
    const session = newSession(host);
    const driver = new host.MatchDriverBridge(session, role, peerId, freezeJson, manifestJson, undefined);
    driver.initializeTransport();
    const recorder = newNetDiagnostics({
      role,
      peer_id: peerId,
      manifest,
      freeze,
      // Mirrors `net_diagnostics_fixture.ts`'s own `env.inputProtocol.
      // FAIRNESS_DELAY_TICKS` default: purely descriptive metadata the
      // recorder carries alongside the driver's own data, not something
      // read back off `MatchDriverBridge`.
      input_delay_ticks: (
        JSON.parse(host.inputProtocolConstantsJson()) as { readonly fairness_delay_ticks: number }
      ).fairness_delay_ticks,
      hash_interval_ticks: hashIntervalTicks,
      export_opt_in: true,
      decodeControlMessage: decode,
    });
    // Star topology: the host driver opens a slot for every guest; each
    // guest driver opens only the host (a guest's own transport capacity is
    // 1 -- `wasm_transport.rs`'s `WasmStarTransport::new` fixes it there for
    // any non-host role, so opening a sibling guest by mistake throws "wasm
    // transport is at peer capacity").
    //
    // `MatchDriverBridge` owns its transport internally with no `pollEvent`
    // surface at all (confirmed by reading `match_driver_bridge.rs` end to
    // end -- `openPeer`/`setPeerConnected` are fire-and-forget, unlike
    // `StarTransportAdapter`'s `pollEvent`, which is what
    // `DiagnosticTransport.pollEvent` -- `diagnostic_transport.ts` -- taps to
    // produce a `peer_state` `recordEvent` row), so there is no wrapped tap
    // this harness could install the way `DiagnosticTransport` installs one
    // over an injected `StarTransportAdapter`. What this harness *does* know,
    // with certainty rather than by polling, is that each of these two calls
    // just caused exactly the state transition `DiagnosticTransport.openPeer`
    // (`state: "opened"`) and a real `TransportPeerState` "connected"
    // transition (`@gc/transport`'s `contract.ts`; `fake_star.ts`'s own
    // `_setPeerState` pushes the identical `{ kind: "peer_state", peer_id,
    // state }` shape for this exact transition) would have recorded, because
    // this harness is the caller that made it happen -- the same reasoning
    // `recordEnvelopeReal` below already relies on to record "sent"/"arrived"
    // packet rows at this harness's own queue/drain seam instead of through a
    // `DiagnosticTransport` wrapper. `monotonic_ms: 0` matches `clock_ms`'s
    // own starting value (this runs before `runReal` advances it).
    const others = role === "host" ? peerIds.filter((candidate) => candidate !== peerId) : [peerIds[0] as string];
    for (const other of others) {
      driver.openPeer(other);
      recordEvent(recorder, { kind: "peer_state", peer_id: other, monotonic_ms: 0, state: "opened" });
      driver.setPeerConnected(other);
      recordEvent(recorder, { kind: "peer_state", peer_id: other, monotonic_ms: 0, state: "connected" });
    }
    return { peerId, role, driver, session, recorder, pendingArrivals: [] };
  });

  return { peers, freeze, manifest, step: 0, clock_ms: 0 };
}

interface WasmOutboundEnvelope {
  readonly peer_id: string;
  readonly channel: "control" | "input";
  readonly message: {
    readonly kind: "input" | "event" | "state";
    readonly seq: number;
    readonly tick?: number | null;
    readonly payload_bytes: readonly number[];
  };
}

// `diagnostic_transport.ts`'s own `recordEnvelope`, applied at this
// harness's queue/drain seam instead of a `StarTransportAdapter` -- see the
// file header for why a `DiagnosticTransport` tap can never wrap a
// `MatchDriverBridge` directly. This is the exact same policy-math-only
// projection that module's doc describes (`authority_input_tick =
// first_input_tick + transport_tick`, `sample_step = transport_tick -
// FAIRNESS_DELAY_TICKS`) -- every field comes from the envelope itself or a
// public constant, never a re-decode of the payload, so this duplicates
// arithmetic, not rollback/netcode logic.
function recordEnvelopeReal(
  recorder: NetDiagnostics,
  senderId: string,
  message: WasmOutboundEnvelope["message"],
  disposition: "sent" | "arrived",
  firstInputTick: number,
  fairnessDelayTicks: number,
  arrivalTransportTick?: number
): void {
  const tick = message.tick;
  if (typeof tick !== "number") {
    // Mirrors `recordEnvelope`'s own guard: control messages carry no tick
    // and are never routed through the packet-lifecycle record.
    return;
  }
  recordPacket(recorder, {
    sender_id: senderId,
    disposition,
    sequence: message.seq,
    payload_bytes: message.payload_bytes.length,
    sample_step: tick - fairnessDelayTicks,
    send_transport_tick: tick,
    authority_input_tick: firstInputTick + tick,
    ...(arrivalTransportTick !== undefined ? { arrival_transport_tick: arrivalTransportTick } : {}),
  });
}

// Shuttles every peer's `drainOutboundJson()` output straight into the
// addressed peer's `enqueueInbound` -- see the file header. Also records
// each envelope's "sent" packet-lifecycle event onto the sender's own
// recorder immediately (see `recordEnvelopeReal`'s doc), but only *queues*
// the "arrived" half onto the receiver's own `pendingArrivals` rather than
// recording it here.
//
// This matters for timing, not just bookkeeping: `DiagnosticTransport`
// records "arrived" from inside `pollInbound`, which the real match driver
// calls *from inside the receiving peer's own `advance()`* -- so in
// production, "arrived" and the `recordStep` that immediately drains
// `pending_apply` and stamps `apply_input_tick` happen in the very same
// step, for the very same peer. Recording "arrived" here instead (at
// delivery/relay time, from the *sender's* step) would attribute it to the
// wrong step and -- for whatever was delivered on the run's very last
// step -- leave it permanently unstamped, since no later `advance()` would
// ever come along to drain it. `runReal`'s own loop drains
// `pendingArrivals` (and records "arrived") immediately before that peer's
// own next `advance()` call, which reproduces the real timing.
function deliverAllReal(host: SimHost, harness: RealNetHarness): void {
  const fairnessDelayTicks = (
    JSON.parse(host.inputProtocolConstantsJson()) as { readonly fairness_delay_ticks: number }
  ).fairness_delay_ticks;
  const peers = harness.peers;
  const drained = peers.map((peer) => JSON.parse(peer.driver.drainOutboundJson()) as WasmOutboundEnvelope[]);
  peers.forEach((sender, senderIndex) => {
    for (const envelope of drained[senderIndex] ?? []) {
      recordEnvelopeReal(
        sender.recorder,
        sender.peerId,
        envelope.message,
        "sent",
        harness.freeze.first_input_tick,
        fairnessDelayTicks
      );
      const receiver = peers.find((candidate) => candidate.peerId === envelope.peer_id);
      if (receiver === undefined) {
        continue;
      }
      receiver.pendingArrivals.push({ senderId: sender.peerId, message: envelope.message });
      receiver.driver.enqueueInbound(
        sender.peerId,
        envelope.channel,
        envelope.message.kind,
        envelope.message.seq,
        envelope.message.tick ?? undefined,
        new Uint8Array(envelope.message.payload_bytes)
      );
    }
  });
}

// Mirrors `net_diagnostics_fixture.ts`'s own `quality`: synthetic and a
// function of the step, not a real transport measurement -- see that
// module's doc for why that is still a legitimate runtime sample.
function realQuality(step: number, rttBiasMs: number): { readonly rtt_ms: number; readonly jitter_ms: number } {
  const phase = step % 5;
  return { rtt_ms: 18 + phase * 2 + rttBiasMs, jitter_ms: phase * 0.5 };
}

const REAL_CLOCK_STEP_MS = 1000 / 60;
const REAL_ANCHOR_INTERVAL = 15;
const REAL_MAPPING_ERROR_MS = REAL_CLOCK_STEP_MS / 2;

interface RunRealOptions {
  /** Deliver every `period` steps; omit to deliver every step. */
  readonly period?: number;
  readonly rttBiasMs?: number;
  readonly clockStepMs?: number;
  readonly anchorInterval?: number;
  /** Per-peer-index sample wire override (0 = host); neutral otherwise. */
  readonly samples?: Readonly<Record<number, string>>;
  /**
   * Feeds `MatchDriverBridge.transportDiagnosticsJson()` into
   * `recordTransport` every step -- opt-in, default off, because doing so
   * populates `recorder.peers` with a real `WasmStarTransport` peer
   * diagnostic whose `ice_state` is unconditionally the empty string
   * (`crates/gc-wasm/src/wasm_transport.rs`'s only site that sets it,
   * confirmed by reading the whole crate: this is the *one* place any
   * `ice_state` value is ever produced). `net_diagnostics`'s own export
   * schema requires `ice_state` to be at least 1 byte -- a real invariant,
   * not an arbitrary one: it matches `crates/gc-netcode/src/fake_star.rs`'s
   * own `"new"`/`"checking"`/`"connected"`/`"closed"` state machine, which
   * is what a fixture star (and every real `@gc/transport` WebRTC star)
   * actually produces. `WasmStarTransport` is a from-scratch,
   * ICE-free queue/drain relay for netcode testing, not a WebRTC stand-in,
   * so it structurally cannot satisfy that invariant -- any harness that
   * turns this on and then calls `exportArtifact`/`exportOf` fails with
   * "ice_state must be at least 1 bytes" the moment a peer is present.
   * Left off by default so every other `runReal` caller (most of this
   * file) is unaffected; the two cases that actually need `runtime.star`/
   * `runtime.peers` are still blocked by this, and stay `it.skip` with this
   * exact reason -- see the file header.
   */
  readonly recordTransportDiagnostics?: boolean;
  /**
   * Once `step` reaches this, every peer's outbound queue is still drained
   * every step (so it cannot grow unbounded on its own) but never
   * delivered to anyone -- a permanent transport stall, rather than
   * `period`'s periodic burst. `MatchDriverBridge`'s internal transport has
   * a fixed 64-message inbound queue with no way to raise it from this
   * bridge's public surface (`WasmStarTransport::new`'s `queue_limit` is
   * always `None` from `MatchDriverBridge::new`) -- a `period` large enough
   * to exceed the ~30-tick rollback window reliably overflows that queue
   * first ("wasm transport is at peer capacity" / "wasm transport inbound
   * queue is full", confirmed empirically), so a genuine window-exceeded
   * case needs a permanent stall instead of a burst.
   */
  readonly stopDeliveryAfterStep?: number;
}

function runReal(host: SimHost, harness: RealNetHarness, steps: number, options: RunRealOptions = {}): void {
  const neutral = host.inputFrameNeutralSample();
  const clockStepMs = options.clockStepMs ?? REAL_CLOCK_STEP_MS;
  const anchorInterval = options.anchorInterval ?? REAL_ANCHOR_INTERVAL;
  const fairnessDelayTicks = (
    JSON.parse(host.inputProtocolConstantsJson()) as { readonly fairness_delay_ticks: number }
  ).fairness_delay_ticks;
  for (let step = 0; step < steps; step += 1) {
    harness.peers.forEach((peer, index) => {
      // Record "arrived" for whatever `deliverAllReal` queued onto this
      // peer since its own last `advance()`, immediately before this call
      // -- see `deliverAllReal`'s doc for why this timing (not delivery
      // time) is what makes `apply_input_tick` stamp correctly.
      if (peer.pendingArrivals.length > 0) {
        const arrivalTransportTick = (
          JSON.parse(peer.driver.diagnosticsJson()) as { readonly transport_tick: number }
        ).transport_tick;
        for (const { senderId, message } of peer.pendingArrivals) {
          recordEnvelopeReal(
            peer.recorder,
            senderId,
            message,
            "arrived",
            harness.freeze.first_input_tick,
            fairnessDelayTicks,
            arrivalTransportTick
          );
        }
        peer.pendingArrivals = [];
      }
      const sampleWire = options.samples?.[index] ?? neutral;
      const batch = JSON.parse(peer.driver.advance(sampleWire)) as MatchDriverBatch;
      // `diagnosticsJson()` now omits an absent `terminal`/`late_input_tick`/
      // `control_slot` entirely rather than emitting a JSON `null`
      // (`match_driver_bridge.rs`'s `diagnostics_to_json` uses
      // `Json::obj_omit_null`, proven by that module's own
      // `diagnostics_json_omits_absent_optional_fields_rather_than_nulling_them`
      // test) -- a raw parse now matches `MatchDriverDiagnostics`'s
      // `field?: T` (absent-or-present) shape directly, so the
      // null-vs-absent normalization a previous pass needed here is gone.
      const diagnostics = JSON.parse(peer.driver.diagnosticsJson()) as MatchDriverDiagnostics;
      recordStep(peer.recorder, diagnostics, batch);
      if (options.recordTransportDiagnostics === true) {
        recordTransport(
          peer.recorder,
          JSON.parse(peer.driver.transportDiagnosticsJson()) as TransportStarDiagnostics
        );
      }
      const { rtt_ms, jitter_ms } = realQuality(harness.step, options.rttBiasMs ?? 0);
      for (const other of harness.peers) {
        if (other.peerId !== peer.peerId) {
          recordRuntimeSample(peer.recorder, { peer_id: other.peerId, monotonic_ms: harness.clock_ms, rtt_ms, jitter_ms });
        }
      }
      if (anchorInterval > 0 && harness.step % anchorInterval === 0) {
        recordAnchor(peer.recorder, {
          input_tick: batch.input_tick,
          monotonic_ms: harness.clock_ms,
          mapping_error_ms: REAL_MAPPING_ERROR_MS,
        });
      }
    });
    if (options.stopDeliveryAfterStep !== undefined && step >= options.stopDeliveryAfterStep) {
      // Drain-only: prevents each peer's own outbound buffer from growing
      // unbounded, but never reaches another peer -- see this option's doc.
      for (const peer of harness.peers) {
        peer.driver.drainOutboundJson();
      }
    } else if (options.period === undefined || (step + 1) % options.period === 0) {
      deliverAllReal(host, harness);
    }
    harness.step += 1;
    harness.clock_ms += clockStepMs;
  }
}

function realPeer(harness: RealNetHarness, peerId: string): RealNetPeer {
  const found = harness.peers.find((candidate) => candidate.peerId === peerId);
  if (found === undefined) {
    throw new Error(`no real peer named ${peerId}`);
  }
  return found;
}

// A real forged input bundle -- `net_diagnostics_fixture.ts`'s
// `forgedBundle`, driven by real `@gc/wasm` calls instead of injected ports.
// Ownership is a frozen partition, so a slot outside the sender's owned set
// is an `ownership_violation` and a second bundle on the same sequence with
// different bytes is an `authority_conflict`.
function forgeBundleReal(
  host: SimHost,
  harness: RealNetHarness,
  senderId: string,
  slotIndex: number,
  sequence: number,
  transportTick: number,
  edges: number
): { readonly seq: number; readonly tick: number; readonly bytes: Uint8Array } {
  const constants = JSON.parse(host.inputProtocolConstantsJson()) as { readonly history_rows: number };
  const rows: { readonly tick: number; readonly slot_index: number; readonly sample: string }[] = [];
  for (let tick = 0; tick <= constants.history_rows; tick += 1) {
    const sample = host.inputFrameNewSample(0, 0, 0, tick === constants.history_rows ? edges : 0);
    rows.push({ tick, slot_index: slotIndex, sample });
  }
  const packet = host.inputProtocolNewGuest(
    harness.manifest.session_id,
    harness.freeze.manifest_id,
    senderId,
    sequence,
    transportTick,
    harness.freeze.first_input_tick,
    undefined,
    JSON.stringify(rows)
  );
  const bytes = host.inputProtocolEncode(packet);
  return { seq: packet.sequence, tick: packet.transport_tick, bytes };
}

describe("diagnostics schema", () => {
  it("refuses a canonical field that borrows wall-clock vocabulary", () => {
    expect(() =>
      schema.record("bad", "canonical", [
        { name: "confirmed_tick", kind: "integer" },
        { name: "rtt_ms", kind: "number" },
      ])
    ).toThrow();
  });

  it("refuses a runtime field that borrows simulation vocabulary", () => {
    expect(() =>
      schema.record("bad", "runtime", [
        { name: "monotonic_ms", kind: "number" },
        { name: "confirmed_tick", kind: "integer" },
      ])
    ).toThrow();
  });

  it("requires an anchor to bind both clocks and declare its error", () => {
    expect(() =>
      schema.record("bad_anchor", "anchor", [
        { name: "input_tick", kind: "integer" },
        { name: "monotonic_ms", kind: "number" },
      ])
    ).toThrow();

    const good = schema.record("good_anchor", "anchor", [
      { name: "input_tick", kind: "integer" },
      { name: "monotonic_ms", kind: "number" },
      { name: "mapping_error_ms", kind: "number" },
    ]);
    expect(good.kind).toBe("record");
  });

  // Regression: the completeness check used to be gated on being the
  // outermost call, so it never ran for the shape that actually ships --
  // `EXPORT` is a domainless container and its `anchors` field is nested
  // one level down. These build the anchor exactly the way `EXPORT` does.
  it("enforces anchor completeness on a nested anchor, as EXPORT nests it", () => {
    expect(EXPORT.domain, "EXPORT is no longer a domainless container").toBeUndefined();
    let anchorPaths = 0;
    for (const domain of Object.values(schema.domains(EXPORT))) {
      if (domain === "anchor") {
        anchorPaths += 1;
      }
    }
    expect(anchorPaths > 0, "EXPORT declares no anchor field to enforce").toBe(true);

    function builds(fields: schema.DiagnosticsField[]): boolean {
      try {
        schema.record("nested_probe", undefined, [
          { name: "session", kind: "record", domain: "identity", fields: [{ name: "peer_id", kind: "id" }] },
          { name: "anchors", kind: "array", domain: "anchor", element: { kind: "record", fields } },
        ]);
        return true;
      } catch {
        return false;
      }
    }

    expect(
      builds([
        { name: "input_tick", kind: "integer" },
        { name: "monotonic_ms", kind: "number" },
      ]),
      "a nested anchor without a mapping error was accepted"
    ).toBe(false);
    expect(
      builds([
        { name: "monotonic_ms", kind: "number" },
        { name: "mapping_error_ms", kind: "number" },
      ]),
      "a nested anchor naming no simulation tick was accepted"
    ).toBe(false);
    expect(
      builds([
        { name: "input_tick", kind: "integer" },
        { name: "mapping_error_ms", kind: "number" },
      ]),
      "a nested anchor whose only wall-clock word is its own error term is still a binding"
    ).toBe(true);
    expect(
      builds([
        { name: "input_tick", kind: "integer" },
        { name: "monotonic_ms", kind: "number" },
        { name: "mapping_error_ms", kind: "number" },
      ]),
      "a well-formed nested anchor was rejected"
    ).toBe(true);
  });

  // The scope must be per-anchor-subtree. A sibling section naming a tick
  // or a millisecond must not be able to satisfy the anchor's own
  // requirements.
  it("does not let sibling sections satisfy an anchor's completeness", () => {
    expect(() =>
      schema.record("sibling_probe", undefined, [
        { name: "canonical", kind: "record", domain: "canonical", fields: [{ name: "confirmed_tick", kind: "integer" }] },
        { name: "runtime", kind: "record", domain: "runtime", fields: [{ name: "mapping_error_ms", kind: "number" }] },
        {
          name: "anchors",
          kind: "array",
          domain: "anchor",
          element: { kind: "record", fields: [{ name: "monotonic_ms", kind: "number" }] },
        },
      ])
    ).toThrow();
  });

  it("keeps the two vocabularies disjoint across the whole export shape", () => {
    const domains = schema.domains(EXPORT);
    let canonical = 0;
    let runtime = 0;
    let anchor = 0;
    for (const [path, domain] of Object.entries(domains)) {
      const match = /([^.[]+)\]?$/.exec(path);
      const name = match?.[1] ?? path;
      if (domain === "canonical" || domain === "identity") {
        canonical += 1;
        expect(
          schema.isWallClockName(name),
          `${path} is deterministic evidence but reads as a wall-clock measurement`
        ).toBe(false);
      } else if (domain === "runtime") {
        runtime += 1;
        expect(
          schema.isSimulationName(name),
          `${path} is a runtime observation but reads as simulation truth`
        ).toBe(false);
      } else if (domain === "anchor") {
        anchor += 1;
      }
    }
    expect(canonical > 0 && runtime > 0 && anchor > 0).toBe(true);
  });

  it("rejects direct identifiers, paths, and raw addresses as ids", () => {
    const shape = schema.record("id_probe", "identity", [{ name: "peer_id", kind: "id" }]);
    for (const bad of [
      "player@example.com",
      "home/oscar/save.json",
      "c:/users/oscar",
      "../secrets",
      "192.168.1.14",
      "[fe80::1]",
    ]) {
      expect(schema.validate(shape, { peer_id: bad }).ok, `${bad} was accepted as a pseudonymous id`).toBe(false);
    }
    expect(schema.validate(shape, { peer_id: "guest_1" }).ok).toBe(true);
  });

  it("rejects non-finite and out-of-range values", () => {
    const shape = schema.record("numbers", "canonical", [
      { name: "count", kind: "integer", min: 0 },
      { name: "ratio", kind: "number" },
    ]);
    expect(schema.validate(shape, { count: 1, ratio: 0 / 0 }).ok).toBe(false);
    expect(schema.validate(shape, { count: 1, ratio: Infinity }).ok).toBe(false);
    expect(schema.validate(shape, { count: -1, ratio: 0 }).ok).toBe(false);
    expect(schema.validate(shape, { count: 1.5, ratio: 0 }).ok).toBe(false);
    expect(schema.validate(shape, { count: 1, ratio: 0.5 }).ok).toBe(true);
  });

  it("encodes maps in sorted key order, not table order", () => {
    const shape = schema.record("m", "canonical", [{ name: "live", kind: "map", element: { kind: "id" } }]);
    const first = schema.encode(shape, { live: { host: "home_1", guest_1: "home_2" } });
    const second = schema.encode(shape, { live: { guest_1: "home_2", host: "home_1" } });
    expect(first.ok && second.ok && first.value).toBe(second.ok && second.value);
  });
});

describe("net diagnostics collection", () => {
  // Six cases below ("summarises a clean 2v2 run...", "folds star and
  // per-channel transport counters...", "records the sample, send,
  // arrival, and apply lifecycle of a packet", "counts rollback,
  // resimulation, and event reconciliation under impairment", "keeps
  // canonical evidence byte-stable while runtime observation moves",
  // "reproduces the canonical projection across two identical runs") all
  // assert on values a *live* `match_driver` produces across a real
  // multi-tick run -- rollback counts, checkpoint hashes, packet lifecycle
  // timing. See "Re-audited against the current `@gc/wasm`" below.
  //
  // # Re-audited against the current `@gc/wasm` -- both gaps are now fixed
  //
  // A prior pass here recorded both cases blocked by `WasmStarTransport`'s
  // `TransportPeerDiagnostics.ice_state` being unconditionally the empty
  // string, failing `net_diagnostics`'s own export schema (a real
  // invariant: it mirrors `crates/gc-netcode/src/fake_star.rs`'s own
  // `"new"`/`"checking"`/`"connected"`/`"closed"` vocabulary, what every
  // real `@gc/transport` WebRTC star actually produces), and then by a
  // second, narrower defect underneath it: `WasmStarTransport::diagnostics()`
  // built each peer's `control` channel from real per-peer counters but
  // handed back `input: TransportChannelDiagnostics::default()` verbatim --
  // `state: None`, every counter `0` -- regardless of how much real traffic
  // moved on the input channel (which, for a `MatchDriverBridge`, is every
  // sample wire every peer sends every step; the primary payload, not an
  // idle channel).
  //
  // Both are fixed on the Rust side (`crates/gc-wasm/src/wasm_transport.rs`,
  // out of this package's ownership; nothing here changed): `PeerLink` now
  // carries a real `ice_state`, set at `open_peer` (`"new"`),
  // `set_peer_connected` (`"connected"`), and `close_peer`/
  // `set_peer_disconnected` (`"closed"`); and it also carries separate
  // `control`/`input` `ChannelCounters`, selected by `channel_mut(channel)`
  // -- `send`/`broadcast` credit the named channel's `sent`, `enqueue_inbound`
  // credits `received`/`dropped_inbound` -- so `diagnostics()` now builds
  // both channel blocks from real per-channel counters, with `state:
  // Some(peer.state)` on each, mirroring `fake_star::set_peer_state`.
  // Confirmed directly against `wasm_transport.rs`'s own
  // `ice_state_is_never_empty_once_a_peer_exists_and_tracks_its_lifecycle`
  // and `control_and_input_channel_diagnostics_are_tracked_independently`
  // tests, and empirically: driving either case below with
  // `recordTransportDiagnostics: true` now passes `exportArtifact` cleanly.
  // `outbound_depth`/`inbound_depth`/`buffered_amount` are `0` on **both**
  // channels deliberately -- that state lives in the browser's
  // `RTCDataChannel`, never surfaced to Rust -- and `dropped_outbound` is
  // `0` because this transport enforces no outbound bound; those are honest
  // zeros, not gaps left to work around.
  //
  // Reaching the fix also needed `MatchDriverBridge.transportDiagnosticsJson()`
  // (`match_driver_bridge.rs`), which did not exist at all in the prior
  // pass -- nothing on the bridge's public surface could report its
  // internal transport's diagnostics. It now wraps
  // `WasmStarTransport::diagnostics()` in the same JSON shape
  // `match_driver_fixture_bridge::star_diagnostics_to_json` already produces
  // for the fixture's in-process star, and `runReal` below already calls it
  // every step behind the opt-in `recordTransportDiagnostics` flag.
  //
  // Separately: the second case ("folds star and per-channel transport
  // counters...") also needs `artifact.runtime.events` non-empty with
  // monotonic ordinals -- i.e. that the tap records transport lifecycle
  // events, in order. `DiagnosticTransport`'s own
  // forwarded-surface wrapping (`sendMessage`/`pollInbound`/`shutdown`
  // calling `this.note(...)`) still cannot wrap a `MatchDriverBridge` (it
  // owns its transport internally, with no `pollEvent`-shaped surface --
  // confirmed by reading `match_driver_bridge.rs` end to end), but
  // `buildRealHarness` below does not need that wrapper: it is the caller
  // of `openPeer`/`setPeerConnected` on that bridge, so it already knows --
  // with certainty, not by polling -- exactly the state transitions a
  // `DiagnosticTransport` tap would have recorded for those same two calls,
  // and calls `recordEvent` there directly (see that call site's own
  // comment), the same pattern `recordEnvelopeReal` below already
  // established for packet events at this harness's own queue/drain seam.
  describe("live-driver runs, transport-shaped claims (real wasm bridges)", () => {
    it("summarises a clean 2v2 run with both clocks kept apart", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      const fairnessDelayTicks = (
        JSON.parse(host.inputProtocolConstantsJson()) as { readonly fairness_delay_ticks: number }
      ).fairness_delay_ticks;
      runReal(host, harness, 40, {
        samples: { 0: host.inputFrameNewSample(60, 0) },
        recordTransportDiagnostics: true,
      });
      const artifact = exportOf(realPeer(harness, "host").recorder);

      expect(artifact.schema_version).toBe(SCHEMA_VERSION);
      expect(artifact.canonical.simulation.status).toBe("active");
      expect(artifact.canonical.simulation.step_count).toBe(40);
      expect(artifact.canonical.simulation.input_delay_ticks).toBe(fairnessDelayTicks);
      expect(artifact.canonical.simulation.confirmed_output_tick >= 0).toBe(true);
      expect(artifact.canonical.simulation.checkpoint_count > 0).toBe(true);
      expect(artifact.canonical.checkpoints.length > 0).toBe(true);
      expect(artifact.canonical.delivery.sent > 0).toBe(true);
      expect(artifact.canonical.delivery.arrived > 0).toBe(true);

      // Runtime is present and separate.
      expect(artifact.runtime.star).toBeDefined();
      expect(artifact.runtime.peers.length > 0).toBe(true);
      expect(artifact.runtime.latency.length > 0).toBe(true);
      expect(artifact.anchors.length > 0).toBe(true);
      for (const anchor of artifact.anchors) {
        expect(anchor.mapping_error_ms > 0, "an anchor claimed a perfect clock mapping").toBe(true);
      }
    });

    // The runtime star/peers section was previously only asserted to be
    // non-empty. It is the half of the artifact carrying transport counters
    // and free text, so it gets its own coverage.
    it("folds star and per-channel transport counters into the runtime section", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      runReal(host, harness, 25, { recordTransportDiagnostics: true });
      const artifact = exportOf(realPeer(harness, "host").recorder);
      const star = artifact.runtime.star;
      expect(star).toBeDefined();
      if (star === undefined) {
        throw new Error("unreachable: asserted defined above");
      }
      expect(star.role).toBe("host");
      expect(star.state).toBe("connected");
      expect(star.sent > 0 && star.received > 0).toBe(true);
      expect(star.dropped_outbound).toBe(0);
      expect(star.dropped_inbound).toBe(0);
      expect(star.malformed).toBe(0);
      expect(star.overflow).toBe(0);
      expect(star.peer_count).toBe(harness.peers.length - 1);
      expect(star.last_error, "a clean run reported a transport error").toBeUndefined();

      expect(artifact.runtime.peers.length).toBe(harness.peers.length - 1);
      for (const peer of artifact.runtime.peers) {
        expect(peer.state).toBe("connected");
        expect(peer.sequence_gaps).toBe(0);
        expect(peer.backpressure).toBe(0);
        expect(peer.malformed).toBe(0);
        for (const channel of [peer.control, peer.input]) {
          expect(channel.buffered_amount >= 0).toBe(true);
          expect(channel.outbound_depth >= 0 && channel.inbound_depth >= 0).toBe(true);
          expect(channel.dropped_outbound === 0 && channel.dropped_inbound === 0).toBe(true);
        }
        expect(peer.input.sent > 0, "no input traffic was attributed to a peer channel").toBe(true);
      }

      // The tap records transport lifecycle events, in order.
      expect(artifact.runtime.events.length > 0, "no runtime event was ever recorded").toBe(true);
      for (let index = 1; index < artifact.runtime.events.length; index += 1) {
        expect(
          (artifact.runtime.events[index] as { readonly ordinal: number }).ordinal >
            (artifact.runtime.events[index - 1] as { readonly ordinal: number }).ordinal,
          "runtime event ordinals are not monotonic"
        ).toBe(true);
      }
    });

    it("records the sample, send, arrival, and apply lifecycle of a packet", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      const fairnessDelayTicks = (
        JSON.parse(host.inputProtocolConstantsJson()) as { readonly fairness_delay_ticks: number }
      ).fairness_delay_ticks;
      runReal(host, harness, 20);
      const artifact = exportOf(realPeer(harness, "host").recorder);
      let arrived = 0;
      for (const packet of artifact.canonical.delivery.packets) {
        expect(
          packet.authority_input_tick,
          "authority tick did not follow the stated delay policy"
        ).toBe(harness.freeze.first_input_tick + packet.send_transport_tick);
        expect(packet.sample_step).toBe(packet.send_transport_tick - fairnessDelayTicks);
        if (packet.disposition === "arrived") {
          arrived += 1;
          expect(packet.arrival_transport_tick).toBeDefined();
          expect(packet.apply_input_tick, "an arrival was never attributed a step").toBeDefined();
          expect(
            packet.authority_input_tick > (packet.apply_input_tick as number),
            "authority landed at or behind the tick already simulated"
          ).toBe(true);
        }
      }
      expect(arrived > 0, "a clean run recorded no arrivals at all").toBe(true);
    });
  });

  describe("live-driver runs, canonical/simulation claims (real wasm bridges)", () => {
    it("counts rollback, resimulation, and event reconciliation under impairment", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      const samples = {
        0: host.inputFrameNewSample(90, 0),
        1: host.inputFrameNewSample(0, -70),
      };
      // Bursty, under-delivered: every 6th step.
      runReal(host, harness, 60, { period: 6, samples });

      const artifact = exportOf(realPeer(harness, "host").recorder);
      const simulation = artifact.canonical.simulation;
      expect(simulation.status).toBe("active");
      expect(simulation.rollback_count > 0, "bursty delivery produced no rollback").toBe(true);
      expect(simulation.resimulated_ticks > 0, "a rollback resimulated nothing").toBe(true);
      expect(simulation.max_rollback_depth > 0).toBe(true);

      const worst = artifact.canonical.worst_correction;
      expect(worst, "no worst correction recorded").toBeDefined();
      expect(worst !== undefined && worst.depth >= 1).toBe(true);
      expect(worst !== undefined && worst.through_tick >= worst.causal_tick).toBe(true);
      expect(worst !== undefined && worst.resimulated_ticks > 0).toBe(true);

      const events = artifact.canonical.events;
      expect(events.added > 0, "no tick was ever emitted for the first time").toBe(true);
      expect(events.resimulated_tick_count).toBe(events.unchanged + events.replaced + events.revoked);

      // Two peers' exports must agree on every confirmed boundary both
      // hashed, and on the live slot each human held there.
      const other = exportOf(realPeer(harness, "guest_1").recorder);
      const byTick = new Map(other.canonical.checkpoints.map((checkpoint) => [checkpoint.tick, checkpoint] as const));
      let compared = 0;
      for (const checkpoint of artifact.canonical.checkpoints) {
        const mine = byTick.get(checkpoint.tick);
        if (mine !== undefined) {
          compared += 1;
          expect(mine.hash, `boundary ${checkpoint.tick}`).toBe(checkpoint.hash);
          for (const [producerId, slot] of Object.entries(checkpoint.live)) {
            expect(mine.live[producerId], `live slot for ${producerId}`).toBe(slot);
          }
        }
      }
      expect(compared > 0, "two peers shared no confirmed boundary to compare").toBe(true);
    });

    it("keeps canonical evidence byte-stable while runtime observation moves", () => {
      const host = loadSimHost();
      function build(rttBiasMs: number, clockStepMs: number): RealNetHarness {
        const harness = buildRealHarness(host, "2v2", 6);
        runReal(host, harness, 45, { period: 5, rttBiasMs, clockStepMs, samples: { 0: host.inputFrameNewSample(45, 0) } });
        return harness;
      }

      const reference = build(0, REAL_CLOCK_STEP_MS);
      const sameScheduleOtherClock = build(37, REAL_CLOCK_STEP_MS * 2);

      const referenceRecorder = realPeer(reference, "host").recorder;
      const otherRecorder = realPeer(sameScheduleOtherClock, "host").recorder;

      const referenceBytes = canonicalBytes(referenceRecorder);
      const otherBytes = canonicalBytes(otherRecorder);
      expect(referenceBytes.ok && otherBytes.ok).toBe(true);
      expect(
        referenceBytes.ok && otherBytes.ok && otherBytes.value,
        "canonical evidence changed when only the clock changed"
      ).toBe(referenceBytes.ok && referenceBytes.value);

      const referenceDigest = recorderDigest(referenceRecorder);
      const otherDigest = recorderDigest(otherRecorder);
      expect(referenceDigest.ok && otherDigest.ok).toBe(true);
      expect(
        referenceDigest.ok && otherDigest.ok && referenceDigest.value !== otherDigest.value,
        "the full export claimed byte-identity across two different clocks"
      ).toBe(true);

      const a = exportOf(referenceRecorder);
      const b = exportOf(otherRecorder);
      expect(a.runtime.latency.length).toBe(b.runtime.latency.length);
      a.runtime.latency.forEach((mine, index) => {
        const other = b.runtime.latency[index];
        expect(other?.peer_id).toBe(mine.peer_id);
        expect(other?.sample_count, "sample counts are a schedule, not a clock").toBe(mine.sample_count);
        expect(
          other?.rtt_ms_min !== undefined && mine.rtt_ms_min !== undefined && other.rtt_ms_min > mine.rtt_ms_min,
          "the bias did not move rtt"
        ).toBe(true);
      });
    });

    it("reproduces the canonical projection across two identical runs", () => {
      const host = loadSimHost();
      function runDigest(): string {
        const harness = buildRealHarness(host, "1v1", 5);
        runReal(host, harness, 40, { period: 4, samples: { 0: host.inputFrameNewSample(30, 0) } });
        const result = canonicalDigest(realPeer(harness, "host").recorder);
        if (!result.ok) {
          throw new Error(result.error);
        }
        return result.value;
      }
      expect(runDigest()).toBe(runDigest());
    });
  });

  it("holds the export behind an explicit opt-in", () => {
    const recorder = newTestRecorder({ export_opt_in: false });
    const failed = exportArtifact(recorder);
    expect(failed.ok).toBe(false);
    expect(!failed.ok && failed.error.includes("opted into")).toBe(true);
    // The summary is local-only and never gated: it does not leave the machine.
    expect(summary(recorder).length > 0).toBe(true);
    optInExport(recorder);
    expect(exportArtifact(recorder).ok).toBe(true);
  });
});

describe("net diagnostics privacy", () => {
  it("keeps signalling blobs out of the export entirely", () => {
    const recorder = newTestRecorder();
    // A blob shaped like the real thing: SDP with ICE credentials and a
    // candidate line carrying a host address.
    const secret = [
      "v=0",
      "a=ice-ufrag:F7gI",
      "a=ice-pwd:x9Yb3kQwPl2sVn8tRc4mHd",
      "a=candidate:1 1 udp 2130706431 192.168.1.14 54321 typ host",
      "a=fingerprint:sha-256 AB:CD:EF",
    ].join("\r\n");
    recordSignal(recorder, "guest_1", "inbound", secret);

    const artifact = exportOf(recorder);
    expect(artifact.runtime.signals.length).toBe(1);
    const signal = artifact.runtime.signals[0];
    expect(signal?.content).toBe(schema.REDACTED);
    expect(signal?.byte_length).toBe(secret.length);
    expect(signal?.direction).toBe("inbound");

    const text = allText(artifact);
    for (const forbidden of [
      "ice-ufrag",
      "ice-pwd",
      "x9Yb3kQwPl2sVn8tRc4mHd",
      "candidate",
      "192.168.1.14",
      "fingerprint",
      "v=0",
    ]) {
      expect(text.includes(forbidden), `${forbidden} survived into the diagnostic export`).toBe(false);
    }
  });

  const POISON = [
    "ICE failed for candidate 192.168.1.14:54321 typ host",
    "connection reset by [fe80::1a2b:3c4d:5e6f:7890]",
    "bridge error: a=ice-pwd:x9Yb3kQwPl2sVn8tRc4mHd",
    "failed to apply sdp offer",
    "signalling failed for turn:turn.example.com:3478",
    "peer someone@example.com is unreachable",
  ];

  it("redacts sensitive free text arriving as a transport error", () => {
    const recorder = newTestRecorder();
    for (const poison of POISON) {
      expect(
        recordTransport(
          recorder,
          starDiagnostics({
            last_error: poison,
            peers: [
              {
                peer_id: "guest_1",
                slot: 1,
                state: "connected",
                ice_state: "connected",
                control: channelDiagnostics(),
                input: channelDiagnostics(),
                sequence_gaps: 0,
                backpressure: 0,
                malformed: 0,
                last_error: poison,
              },
            ],
          })
        ).ok
      ).toBe(true);
      const artifact = exportOf(recorder);
      expect(artifact.runtime.star?.last_error).toBe(schema.REDACTED);
      expect(artifact.runtime.peers[0]?.last_error).toBe(schema.REDACTED);
      assertAbsent(artifact, poison);
    }
  });

  it("redacts sensitive free text arriving as a runtime event detail", () => {
    const recorder = newTestRecorder();
    for (const [index, poison] of POISON.entries()) {
      expect(
        recordEvent(recorder, {
          kind: "peer_error",
          monotonic_ms: index * 10,
          peer_id: "guest_1",
          code: "signal_error",
          detail: poison,
        }).ok
      ).toBe(true);
    }
    const artifact = exportOf(recorder);
    const redacted = artifact.runtime.events.filter((event) => event.detail === schema.REDACTED).length;
    expect(redacted, "a poisoned event detail survived unredacted").toBe(POISON.length);
    for (const poison of POISON) {
      assertAbsent(artifact, poison);
    }
  });

  it("keeps benign transport error text readable", () => {
    const recorder = newTestRecorder();
    expect(
      recordEvent(recorder, {
        kind: "star_error",
        monotonic_ms: 5,
        code: "overflow",
        detail: "outbound queue reached its limit of 64 messages",
      }).ok
    ).toBe(true);
    const artifact = exportOf(recorder);
    const last = artifact.runtime.events[artifact.runtime.events.length - 1];
    expect(last?.detail).toBe("outbound queue reached its limit of 64 messages");
    expect(last?.code).toBe("overflow");
  });

  // `desync_package.ts` is a narrow port -- see that file's header for what
  // it covers and what it deliberately leaves for the Rust-owned
  // `crates/gc-netcode/src/desync_package.rs` (wire identity, schema-checked
  // round trip, cross-language digest). What it does have is enough for
  // this case: build a package from a recorder that already redacted
  // poisoned free text at `recordEvent` time, and confirm the package (and
  // its encoded form) never carries it. The package embeds runtime events
  // verbatim, so redaction has to have happened on the way in rather than
  // on the way out.
  it("stops poisoned free text reaching a desync package", () => {
    const recorder = newTestRecorder();
    for (const [index, poison] of POISON.entries()) {
      expect(
        recordEvent(recorder, {
          kind: "peer_error",
          monotonic_ms: index * 10,
          peer_id: "guest_1",
          detail: poison,
        }).ok
      ).toBe(true);
    }
    const built = desyncPackageBuild({
      recorder,
      peer_id: "host_1",
      remote_peer_id: "guest_1",
      agreed_boundary_tick: 0,
      agreed_boundary_hash: "0123456789abcdef",
      divergence_tick: 30,
      local_hash: "fedcba9876543210",
      remote_hash: "deadbeefdeadbeef",
      input_wires: [],
    });
    expect(built.ok).toBe(true);
    if (!built.ok) return;
    for (const poison of POISON) {
      assertAbsent(built.value, poison);
    }
    const encoded = desyncPackageEncode(built.value);
    expect(encoded.ok).toBe(true);
    if (encoded.ok) {
      expect(encoded.value.includes("192.168.1.14")).toBe(false);
    }
  });

  it("refuses to store a direct identifier as a peer id", () => {
    const recorder = newTestRecorder();
    recordRuntimeSample(recorder, { peer_id: "someone@example.com", monotonic_ms: 10, rtt_ms: 20 });
    const result = exportArtifact(recorder);
    expect(result.ok, "an email address reached a validated export").toBe(false);
    expect(!result.ok && result.error.includes("pseudonymous")).toBe(true);
  });

  it("carries no participant or playtest payload fields at all", () => {
    const paths = schema.domains(EXPORT);
    for (const path of Object.keys(paths)) {
      const lowered = path.toLowerCase();
      for (const forbidden of ["participant", "consent", "research", "clipboard", "address", "sdp", "candidate", "email"]) {
        expect(lowered.includes(forbidden), `${path} names a field this schema must never carry`).toBe(false);
      }
    }
  });
});

describe("net diagnostics bounds", () => {
  it("marks truncation explicitly instead of silently shrinking", () => {
    const limits: NetDiagnosticsLimits = {
      checkpoints: 2,
      packets: 4,
      control: 2,
      events: 2,
      signals: 1,
      mismatches: 2,
      anchors: 2,
      latency_peers: 2,
    };
    const recorder = newTestRecorder({ limits });
    // The published count and the retained count are different claims (see
    // `net_diagnostics.ts`'s `recordStep`); fold one step declaring how
    // many checkpoints the driver published before pushing the checkpoints
    // themselves, so the "published >= retained" assertion below is
    // actually exercised rather than vacuously true at zero.
    recordStep(recorder, testDriverDiagnostics({ checkpoint_count: 6 }), testDriverBatch());
    for (let index = 0; index < 6; index += 1) {
      recordCheckpoint(recorder, { tick: index, hash: (16 + index).toString(16).padStart(16, "0"), live: {} });
    }
    for (let index = 0; index < 10; index += 1) {
      recordPacket(recorder, {
        sender_id: "guest_1",
        disposition: "sent",
        sequence: index,
        payload_bytes: 10,
        sample_step: index,
        send_transport_tick: index,
        authority_input_tick: index,
      });
    }
    for (let index = 0; index < 6; index += 1) {
      recordAnchor(recorder, { input_tick: index, monotonic_ms: index, mapping_error_ms: 1 });
    }
    const artifact = exportOf(recorder);

    expect(artifact.collection.retention.packets).toBe(schema.TRUNCATED);
    expect((artifact.collection.dropped.packets ?? 0) > 0).toBe(true);
    expect(artifact.canonical.delivery.packets.length <= limits.packets).toBe(true);
    expect(artifact.canonical.checkpoints.length <= limits.checkpoints).toBe(true);
    expect(artifact.anchors.length <= limits.anchors).toBe(true);
    // The count of published checkpoints is not the count retained, and the
    // artifact says both rather than quietly conflating them.
    expect(artifact.canonical.simulation.checkpoint_count >= artifact.canonical.checkpoints.length).toBe(true);

    const lines = summary(recorder);
    expect(lines.some((line) => line.includes("truncated")), "a truncated ring did not show up in the human summary").toBe(true);
  });

  it("keeps the oldest mismatches and the newest packets", () => {
    const recorder = newTestRecorder({
      limits: {
        checkpoints: 8,
        packets: 8,
        control: 8,
        events: 8,
        signals: 4,
        mismatches: 2,
        anchors: 4,
        latency_peers: 4,
      },
    });
    for (let index = 1; index <= 5; index += 1) {
      recordMismatch(recorder, {
        tick: index * 10,
        peer_id: "guest_1",
        local_hash: index.toString(16).padStart(16, "0"),
        remote_hash: (index + 100).toString(16).padStart(16, "0"),
      });
    }
    const artifact = exportOf(recorder);
    expect(artifact.canonical.mismatches.length).toBe(2);
    // The first divergence is the causal one, so it is the one kept.
    expect(artifact.canonical.mismatches[0]?.tick).toBe(10);
    expect(artifact.canonical.mismatches[1]?.tick).toBe(20);
    expect(artifact.collection.retention.mismatches).toBe(schema.TRUNCATED);
  });

  it("counts rejected values instead of storing them", () => {
    const recorder = newTestRecorder();
    expect(recordAnchor(recorder, { input_tick: 0, monotonic_ms: 0 / 0, mapping_error_ms: 1 }).ok).toBe(false);
    expect(recordAnchor(recorder, { input_tick: 0, monotonic_ms: 10, mapping_error_ms: -1 }).ok).toBe(false);
    expect(recordRuntimeSample(recorder, { peer_id: "guest_1", monotonic_ms: 10, rtt_ms: Infinity }).ok).toBe(false);
    const artifact = exportOf(recorder);
    expect(artifact.collection.rejected_values).toBe(3);
  });
});

describe("net diagnostics failure fixtures", () => {
  // These cases -- an ownership violation, an authority conflict,
  // over-window input, hash divergence, and a guest disconnect / host loss
  // -- all require a real `match_driver` to actually detect the fault:
  // forging input bundles and handing them to a live driver's transport, or
  // calling `match_driver.observe_checkpoint` directly.
  // Unblocked: `MatchDriverBridge.observeCheckpoint(tick, hash)` is exactly
  // `gc_netcode::match_driver::observe_checkpoint`, confirmed present on
  // both `match_driver_bridge.rs` and `types.ts` -- absent when the earlier
  // pass wrote this comment, landed since.
  describe("live-driver fault detection, hash divergence (real wasm bridges)", () => {
    it("types hash divergence and keeps the first divergent boundary", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 5);
      runReal(host, harness, 30);
      const hostPeer = realPeer(harness, "host");
      const guestId = (harness.peers[1] as RealNetPeer).peerId;
      const checkpoints = exportOf(hostPeer.recorder).canonical.checkpoints;
      expect(checkpoints.length > 0, "no checkpoint was published to disagree about").toBe(true);
      const target = checkpoints[0] as { readonly tick: number; readonly hash: string };
      const remoteHash = "deadbeefdeadbeef";

      // `observeCheckpoint` itself terminates the driver after
      // `gc_netcode::match_driver::MAX_HASH_MISMATCHES` (3) consecutive
      // disagreements -- looped with a safety cap rather than hardcoding
      // that constant, since nothing on `@gc/wasm`'s surface exposes it.
      let status = "active";
      for (let attempt = 0; attempt < 10 && status !== "hash_mismatch"; attempt += 1) {
        const matched = hostPeer.driver.observeCheckpoint(target.tick, remoteHash);
        expect(matched, "a deliberately wrong remote hash must never agree").toBe(false);
        expect(
          recordMismatch(hostPeer.recorder, {
            tick: target.tick,
            peer_id: guestId,
            local_hash: target.hash,
            remote_hash: remoteHash,
            first_difference_path: "state.ball.pos.x",
          }).ok
        ).toBe(true);
        status = JSON.parse(hostPeer.driver.statusJson()) as string;
      }
      expect(status, "observeCheckpoint never terminated the driver").toBe("hash_mismatch");

      // One more step lets `recordStep` observe the now-terminated
      // diagnostics and stamp the artifact's own terminal fields.
      runReal(host, harness, 1);

      const artifact = exportOf(hostPeer.recorder);
      expect(artifact.canonical.simulation.status).toBe("hash_mismatch");
      expect(artifact.canonical.simulation.terminal_failure).toBe("desync");
      const mismatch = artifact.canonical.mismatches[0];
      expect(mismatch?.tick).toBe(target.tick);
      expect(mismatch?.local_hash).toBe(target.hash);
      expect(mismatch?.remote_hash).toBe(remoteHash);
      expect(mismatch?.first_difference_path).toBe("state.ball.pos.x");
    });
  });

  describe("live-driver fault detection (real wasm bridges)", () => {
    it("types an ownership violation", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      runReal(host, harness, 4);
      // guest_1 owns home_3/home_4 in this fixture's "2v2" ownership
      // (confirmed by reading `matchDriverFixtureFreezeJson("2v2")`'s
      // `owned` map); away_1 belongs to another human entirely. `slot_index`
      // is 1-based on the wire (`match_driver.rs`'s `fill_relay_window`
      // indexes `covered[slot_index - 1]`) -- confirmed empirically after
      // an 0-based first attempt landed on guest_1's own `home_4` instead
      // and produced no violation at all.
      const guestId = (harness.peers[1] as RealNetPeer).peerId;
      const hostDriver = realPeer(harness, "host").driver;
      const transportTick = (JSON.parse(hostDriver.diagnosticsJson()) as { readonly transport_tick: number }).transport_tick;
      const forged = forgeBundleReal(host, harness, guestId, 5 /* away_1 */, 9000, transportTick, 0);
      hostDriver.enqueueInbound(guestId, "input", "input", forged.seq, forged.tick, forged.bytes);
      runReal(host, harness, 1);

      const simulation = exportOf(realPeer(harness, "host").recorder).canonical.simulation;
      expect(simulation.status).toBe("ownership_violation");
      expect(simulation.terminal_status).toBe("ownership_violation");
      expect(simulation.terminal_failure).toBe("input_channel");
      expect(simulation.terminal_detail).toBeDefined();
    });

    it("types an authority conflict", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      runReal(host, harness, 6);
      const guestId = (harness.peers[1] as RealNetPeer).peerId;
      const hostDriver = realPeer(harness, "host").driver;
      const slotIndex = 3; // home_3 (1-based wire slot index), one of guest_1's own owned slots.
      const dashBits = (JSON.parse(host.inputFrameEdgeBitsJson()) as Record<string, number>).dash as number;
      const send = (edges: number): void => {
        const transportTick = (JSON.parse(hostDriver.diagnosticsJson()) as { readonly transport_tick: number }).transport_tick;
        const forged = forgeBundleReal(host, harness, guestId, slotIndex, 4242, transportTick, edges);
        hostDriver.enqueueInbound(guestId, "input", "input", forged.seq, forged.tick, forged.bytes);
      };

      // One sender sequence with byte-identical authority is idempotent, so
      // this establishes the rows rather than failing on them.
      send(0);
      send(0);
      runReal(host, harness, 1);
      expect(exportOf(realPeer(harness, "host").recorder).canonical.simulation.status).toBe("active");

      // The same identity with different bytes conflicts with what history
      // already holds.
      send(0);
      send(dashBits);
      runReal(host, harness, 1);
      const simulation = exportOf(realPeer(harness, "host").recorder).canonical.simulation;
      expect(simulation.status).toBe("authority_conflict");
      expect(simulation.terminal_failure).toBe("input_channel");
    });

    it("types over-window input and names the tick it was attributed to", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 10);
      // A permanent stall past step 5, not a `period` burst -- see
      // `stopDeliveryAfterStep`'s doc for why a burst wide enough to exceed
      // the rollback window overflows the transport's fixed inbound queue
      // first.
      runReal(host, harness, 45, { stopDeliveryAfterStep: 5 });
      let terminal = 0;
      for (const peer of harness.peers) {
        const simulation = exportOf(peer.recorder).canonical.simulation;
        // Since #241 the driver catches this on confirmation liveness at the
        // step the tick becomes unconfirmable, rather than on the arrival
        // that used to reveal it.
        if (simulation.status === "confirmation_stalled") {
          terminal += 1;
          expect(simulation.terminal_failure).toBe("late_input");
          expect(simulation.terminal_tick).toBeDefined();
          expect(simulation.late_input_tick).toBeDefined();
          expect((simulation.late_input_tick as number) < simulation.retained_floor_tick + 1, "a late input was attributed above the retained floor").toBe(
            true
          );
        }
      }
      expect(terminal > 0, "an over-window burst terminated nobody").toBe(true);
    });

    it("types a guest disconnect and a host loss", () => {
      const host = loadSimHost();
      const harness = buildRealHarness(host, "2v2", 6);
      runReal(host, harness, 8);
      const guestId = (harness.peers[1] as RealNetPeer).peerId;
      realPeer(harness, "host").driver.setPeerDisconnected(guestId, "peer_left");
      runReal(host, harness, 1);
      expect(exportOf(realPeer(harness, "host").recorder).canonical.simulation.status).toBe("transport_lost");

      const other = buildRealHarness(host, "2v2", 6);
      runReal(host, other, 8);
      const hostId = (other.peers[0] as RealNetPeer).peerId;
      for (const peer of other.peers) {
        if (peer.role !== "host") {
          peer.driver.setPeerDisconnected(hostId, "host_left");
        }
      }
      runReal(host, other, 1);
      for (const peer of other.peers) {
        if (peer.role !== "host") {
          expect(exportOf(peer.recorder).canonical.simulation.status).toBe("transport_lost");
        }
      }
    });
  });

  it("reports orphaned links and residual queues at teardown", () => {
    const recorder = newTestRecorder();
    const star = fakeStar({ role: "host" });
    expect(star.initialize().ok).toBe(true);
    const tap = new DiagnosticTransport({
      transport: star,
      recorder,
      peerId: "host_1",
      firstInputTick: 0,
      inputDelayTicks: 0,
    });
    expect(tap.shutdown().ok).toBe(true);
    const artifact = exportOf(recorder);
    const teardown = artifact.runtime.teardown;
    expect(teardown, "shutdown recorded no teardown").toBeDefined();
    expect(teardown?.requested).toBe(true);
    expect(teardown?.orphaned_peers.length).toBe(0);
    expect(teardown?.complete, "a clean shutdown was not reported complete").toBe(true);
  });

  it("reports an incomplete teardown as incomplete", () => {
    const recorder = newTestRecorder();
    recordTeardown(recorder, {
      requested: true,
      closed_peers: 1,
      orphaned_peers: ["guest_1"],
      residual_outbound: 3,
      residual_inbound: 0,
    });
    const artifact = exportOf(recorder);
    const teardown = artifact.runtime.teardown;
    expect(teardown, "teardown was not recorded").toBeDefined();
    expect(teardown?.complete).toBe(false);
    expect(teardown?.orphaned_peers[0]).toBe("guest_1");
  });

  it("records accepted control traffic in order", () => {
    // A fake, injected decoder standing in for `game.online.protocol`
    // (Rust-owned) -- see the module doc comment. `recordControl` itself
    // does not care what the decoder's grammar is, only that it recovers
    // `kind`/`sequence`/`message_id`, so a simple pipe-delimited fake wire
    // exercises exactly the same code path a real one would.
    const decode: ProtocolDecoder = (payload) => {
      const [kind, sequence, messageId] = payload.split("|");
      if (kind === undefined || sequence === undefined || messageId === undefined) {
        return null;
      }
      const message: ProtocolControlMessage = { kind: kind as ProtocolControlMessage["kind"], sequence: Number(sequence), message_id: messageId };
      return message;
    };
    const recorder = newTestRecorder({ decodeControlMessage: decode });
    for (let index = 1; index <= 3; index += 1) {
      const envelope = newMessage({ type: "event", seq: index, payload: `hash_report|${index}|msg_${index}` });
      if (!envelope.ok) {
        throw new Error(envelope.error.message);
      }
      const addressed: TransportPeerMessage = {
        peer_id: "guest_1",
        channel: "control",
        message: envelope.value,
        arrival_seq: index,
      };
      expect(recordControl(recorder, addressed).ok).toBe(true);
    }
    const artifact = exportOf(recorder);
    expect(artifact.canonical.control.length).toBe(3);
    for (let index = 1; index < 3; index += 1) {
      const current = artifact.canonical.control[index];
      const previous = artifact.canonical.control[index - 1];
      expect(current !== undefined && previous !== undefined && current.ordinal > previous.ordinal, "control ordinals are not monotonic").toBe(true);
      expect(current?.kind).toBe("hash_report");
    }
  });
});
