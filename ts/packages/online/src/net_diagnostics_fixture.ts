// Fixtures for OMP-3 network diagnostics: a full peer set, each with its own
// recorder and its own diagnostic transport tap, driven step by step over
// the in-process star.
//
// The clock is injected and advances by a fixed amount per step. That is
// what makes the *runtime* half of an export reproducible in a test at all
// -- and it is also why the specs never generalise from that
// reproducibility. A frozen clock proves the recorder is deterministic
// given its inputs; it proves nothing about a real machine, where the same
// match yields different milliseconds every time. The specs make
// byte-identity claims only about the canonical projection, which does not
// contain a millisecond at all.
//
// ## Boundary note
//
// Every dependency this fixture drives -- `game.online.match_driver`,
// `game.online.match_driver_fixture`, `game.online.input_protocol`,
// `game.online.protocol`, and `sim.input_frame` -- is Rust-owned
// (ARCHITECTURE.md §1.1). Building a real match session means running the
// real rollback session, which is precisely the logic that must never be
// duplicated on this side of the determinism line. So, following the same
// pattern as `match_presentation.ts`, every one of those dependencies is
// threaded through as an injected `NetDiagnosticsFixtureEnv` rather than
// required. The driver, snapshot, sample, and packet values this module
// carries are opaque (`unknown`) end to end -- it never inspects them,
// only passes them to the env and to `MatchDriverBatch`'s already-typed
// fields.
//
// No TS-only fake for `NetDiagnosticsFixtureEnv` ships here: a
// fake real enough to exercise rollback, ownership violations, hash
// mismatches, or confirmation stalls would itself be a rollback
// implementation, which is exactly what the boundary rule forbids here.
//
// # Re-audited against the current `@gc/wasm`
//
// This comment used to say four of this env's five ports (`matchDriverFixture`,
// `protocol`, `inputProtocol`, `inputFrame`) had no wasm bridge at all. That
// is stale: `match_driver_fixture_bridge.rs`, `input_protocol_bridge.rs`, and
// `input_frame_bridge.rs` all landed since, each with its own passing
// `packages/wasm/src/*.spec.ts`.
//
// What is real, and turned out to matter more than the missing bridges did:
// this env's `MatchDriverPort` shape -- `create(options)` taking an
// *injected* `options.transport: StarTransportAdapter` the driver calls
// into, mirroring `game.online.match_driver.new({transport = tap})` -- does
// not fit `MatchDriverBridge`'s architecture at all. `MatchDriverBridge`
// owns its own internal transport (`crate::wasm_transport::WasmStarTransport`,
// no `#[wasm_bindgen]` surface, never injectable) and expects the *caller*
// to relay bytes via `drainOutboundJson`/`enqueueInbound` (the "queue/drain
// seam" `match_driver_bridge.rs`'s module doc describes) -- the inverse of
// dependency injection. `DiagnosticTransport`'s wrapped `send`/`poll`
// methods (which record star/channel/packet diagnostics as a side effect
// of being called *by the driver*) can therefore never observe a
// `MatchDriverBridge`'s traffic, and `TransportStarDiagnostics`-shaped data
// for its internal transport is not exposed by `@gc/wasm` at all (confirmed
// by reading `crates/gc-wasm/src/wasm_transport.rs` end to end).
//
// `net_diagnostics.spec.ts` does not route through this module's
// `NetDiagnosticsFixtureEnv`/`MatchDriverPort` for that reason: it builds a
// real two-peer `MatchDriverBridge` harness directly (relaying
// `drainOutboundJson`/`enqueueInbound` itself, and calling `recordStep`/
// `recordAnchor`/`recordRuntimeSample` straight from the harness rather than
// through a `DiagnosticTransport` tap), unblocking seven of that file's
// eleven live-driver cases -- the ones whose claims are canonical/simulation
// data or reachable via `enqueueInbound`/`setPeerDisconnected` directly, not
// transport-level counters. See that file's header for the full breakdown
// of what is and is not reachable this way, and for why that leaves this
// module's own port shapes unused rather than rewritten: a caller who does
// want this dependency-injection shape (e.g. wiring a *real*
// `@gc/transport` adapter, not a test fixture) still needs the thin
// `StarTransportAdapter` wrapper over `MatchDriverBridge`'s queue/drain seam
// that `match_driver_bridge.rs`'s own doc names as `@gc/online`'s job --
// that is a production concern, not a test-harness one, and is out of this
// audit's scope.

import type { Result } from "@gc/core";
import { newMessage, type StarTransportAdapter, type TransportMessage, type TransportPeerMessage } from "@gc/transport";
import {
  newNetDiagnostics,
  recordAnchor,
  recordControl,
  recordRuntimeSample,
  recordStep,
  type CoordinatorFreeze,
  type MatchDriverBatch,
  type MatchDriverDiagnostics,
  type MatchMode,
  type NetDiagnostics,
  type NetDiagnosticsLimits,
  type ProtocolDecoder,
  type SessionManifest,
  type SessionRole,
} from "./net_diagnostics.ts";
import { DiagnosticTransport } from "./diagnostic_transport.ts";

// ---------------------------------------------------------------------------
// Injected environment (all Rust-owned; see the module doc comment)
// ---------------------------------------------------------------------------

export interface MatchDriverCreateOptions {
  readonly role: SessionRole;
  readonly peer_id: string;
  readonly freeze: CoordinatorFreeze;
  readonly manifest: SessionManifest;
  readonly transport: StarTransportAdapter;
  readonly initial_snapshot: unknown;
  readonly hash_interval_ticks?: number;
  readonly max_rollback_ticks?: number;
}

/** `game.online.match_driver`, the slice this fixture drives. */
export interface MatchDriverPort {
  readonly DEFAULT_HASH_INTERVAL_TICKS: number;
  readonly DELAY_TICKS: number;
  create(options: MatchDriverCreateOptions): unknown;
  advance(driver: unknown, sample: unknown): MatchDriverBatch;
  diagnostics(driver: unknown): MatchDriverDiagnostics;
}

export interface MatchDriverFixtureSession {
  readonly manifest: SessionManifest;
  readonly freeze: CoordinatorFreeze;
  readonly host_peer_id: string;
  readonly host_transport: StarTransportAdapter;
  readonly guest_peer_ids: readonly string[];
  readonly guest_transports: Readonly<Record<string, StarTransportAdapter>>;
}

/** `game.online.match_driver_fixture`. */
export interface MatchDriverFixturePort {
  session(mode: MatchMode, template: unknown, humans: number | undefined): MatchDriverFixtureSession;
  initialSnapshot(duration: number | undefined, combat: boolean | undefined): unknown;
}

export interface InputProtocolRow {
  readonly tick: number;
  readonly slot_index: number;
  readonly sample: unknown;
}

export interface InputProtocolNewGuestOptions {
  readonly session_id: string;
  readonly manifest_id: string;
  readonly sender_id: string;
  readonly sequence: number;
  readonly transport_tick: number;
  readonly first_input_tick: number;
  readonly rows: readonly InputProtocolRow[];
}

/** An `input_protocol` packet handle: opaque except for the two fields this fixture reads back. */
export interface InputProtocolPacket {
  readonly sequence: number;
  readonly transport_tick: number;
}

/** `game.online.input_protocol`, the slice this fixture drives. */
export interface InputProtocolPort {
  readonly FAIRNESS_DELAY_TICKS: number;
  readonly HISTORY_ROWS: number;
  newGuest(options: InputProtocolNewGuestOptions): Result<InputProtocolPacket, string>;
  encode(packet: InputProtocolPacket): Result<string, string>;
}

/** `game.online.protocol`, the slice this fixture drives. */
export interface ProtocolPort extends ProtocolDecoderPort {
  readonly new_: (
    kind: string,
    sessionId: string,
    peerId: string,
    sequence: number,
    body: unknown
  ) => Result<unknown, string>;
  encode(message: unknown): Result<string, string>;
}

/** The decoder half of `ProtocolPort`, reused as `NetDiagnostics`'s injected `decodeControlMessage`. */
export interface ProtocolDecoderPort {
  decode: ProtocolDecoder;
}

export interface InputFramePort {
  neutralSample(): unknown;
  newSample(options: { readonly edges?: number }): Result<unknown, string>;
  readonly EDGE_BITS: Readonly<Record<string, number>>;
}

export interface NetDiagnosticsFixtureEnv {
  readonly matchDriver: MatchDriverPort;
  readonly matchDriverFixture: MatchDriverFixturePort;
  readonly protocol: ProtocolPort;
  readonly inputProtocol: InputProtocolPort;
  readonly inputFrame: InputFramePort;
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

export interface NetDiagnosticsFixturePeer {
  readonly peer_id: string;
  readonly role: SessionRole;
  readonly recorder: NetDiagnostics;
  readonly tap: DiagnosticTransport;
  readonly driver: unknown;
}

export interface NetDiagnosticsFixtureHarness {
  readonly session: MatchDriverFixtureSession;
  readonly peers: NetDiagnosticsFixturePeer[];
  step: number;
  clock_ms: number;
  readonly clock_step_ms: number;
  readonly rtt_bias_ms: number;
  readonly anchor_interval: number;
}

export interface NetDiagnosticsFixtureOptions {
  readonly humans?: number;
  readonly duration?: number;
  readonly combat?: boolean;
  readonly hash_interval_ticks?: number;
  readonly max_rollback_ticks?: number;
  readonly clock_step_ms?: number;
  readonly rtt_bias_ms?: number;
  readonly anchor_interval?: number;
  readonly retain_wires?: number;
  readonly export_opt_in?: boolean;
  readonly limits?: NetDiagnosticsLimits;
}

// One 60 Hz frame, to the microsecond the fixed clock actually implies.
export const DEFAULT_CLOCK_STEP_MS = 1000 / 60;
export const DEFAULT_ANCHOR_INTERVAL = 15;
export const DEFAULT_RETAIN_WIRES = 192;

// Half a frame: an anchor taken once per step can only place an input tick
// within the frame it was sampled in, and saying so is the entire point of
// carrying a mapping error next to the anchor.
export const MAPPING_ERROR_MS = DEFAULT_CLOCK_STEP_MS / 2;

function unwrap<T, E>(result: Result<T, E>, message: string, describe: (error: E) => string = String): T {
  if (!result.ok) {
    throw new Error(`${message}: ${describe(result.error)}`);
  }
  return result.value;
}

export function harness(
  env: NetDiagnosticsFixtureEnv,
  mode: MatchMode,
  options: NetDiagnosticsFixtureOptions = {}
): NetDiagnosticsFixtureHarness {
  const session = env.matchDriverFixture.session(mode, undefined, options.humans);
  const snapshot = env.matchDriverFixture.initialSnapshot(options.duration, options.combat);
  const state: NetDiagnosticsFixtureHarness = {
    session,
    peers: [],
    step: 0,
    clock_ms: 0,
    clock_step_ms: options.clock_step_ms ?? DEFAULT_CLOCK_STEP_MS,
    rtt_bias_ms: options.rtt_bias_ms ?? 0,
    anchor_interval: options.anchor_interval ?? DEFAULT_ANCHOR_INTERVAL,
  };

  const add = (peerId: string, role: SessionRole, transport: StarTransportAdapter): void => {
    const recorder = newNetDiagnostics({
      role,
      peer_id: peerId,
      manifest: session.manifest,
      freeze: session.freeze,
      input_delay_ticks: env.inputProtocol.FAIRNESS_DELAY_TICKS,
      hash_interval_ticks: options.hash_interval_ticks ?? env.matchDriver.DEFAULT_HASH_INTERVAL_TICKS,
      max_rollback_ticks: options.max_rollback_ticks ?? 0,
      ...(options.limits !== undefined ? { limits: options.limits } : {}),
      export_opt_in: options.export_opt_in !== false,
      decodeControlMessage: env.protocol.decode,
    });
    const tap = new DiagnosticTransport({
      transport,
      recorder,
      peerId,
      firstInputTick: session.freeze.first_input_tick,
      inputDelayTicks: env.matchDriver.DELAY_TICKS,
      clock: () => state.clock_ms,
      transportTick: () => state.step + env.matchDriver.DELAY_TICKS,
      retainWires: options.retain_wires ?? DEFAULT_RETAIN_WIRES,
    });
    const driver = env.matchDriver.create({
      role,
      peer_id: peerId,
      freeze: session.freeze,
      manifest: session.manifest,
      transport: tap,
      initial_snapshot: snapshot,
      ...(options.hash_interval_ticks !== undefined ? { hash_interval_ticks: options.hash_interval_ticks } : {}),
      ...(options.max_rollback_ticks !== undefined ? { max_rollback_ticks: options.max_rollback_ticks } : {}),
    });
    state.peers.push({ peer_id: peerId, role, recorder, tap, driver });
  };

  add(session.host_peer_id, "host", session.host_transport);
  for (const peerId of session.guest_peer_ids) {
    const transport = session.guest_transports[peerId];
    if (transport === undefined) {
      throw new Error(`net diagnostics fixture: no guest transport for ${peerId}`);
    }
    add(peerId, "guest", transport);
  }
  return state;
}

// The synthetic quality sample. It varies with the step so the recorder's
// min/max/last folding is actually exercised, and it is a *function of the
// step* so a repeated run with the same bias reproduces it -- which is the
// property the runtime invariant checks lean on, not byte-identity of a
// real measurement.
export function quality(state: NetDiagnosticsFixtureHarness): { readonly rtt_ms: number; readonly jitter_ms: number } {
  const phase = state.step % 5;
  return { rtt_ms: 18 + phase * 2 + state.rtt_bias_ms, jitter_ms: phase * 0.5 };
}

function fold(state: NetDiagnosticsFixtureHarness, env: NetDiagnosticsFixtureEnv, peer: NetDiagnosticsFixturePeer, batch: MatchDriverBatch): void {
  // Reading driver diagnostics also reads the tap's transport diagnostics,
  // which is what folds the star's counters into the recorder.
  const diagnostics = env.matchDriver.diagnostics(peer.driver);
  recordStep(peer.recorder, diagnostics, batch);
  for (const addressed of batch.control) {
    recordControl(peer.recorder, addressed);
  }
  const { rtt_ms, jitter_ms } = quality(state);
  for (const other of state.peers) {
    if (other.peer_id !== peer.peer_id) {
      recordRuntimeSample(peer.recorder, {
        peer_id: other.peer_id,
        monotonic_ms: state.clock_ms,
        rtt_ms,
        jitter_ms,
      });
    }
  }
  if (state.anchor_interval > 0 && state.step % state.anchor_interval === 0) {
    recordAnchor(peer.recorder, {
      input_tick: batch.input_tick,
      monotonic_ms: state.clock_ms,
      mapping_error_ms: MAPPING_ERROR_MS,
    });
  }
}

// One step for every peer, then one delivery. `deliver` false holds the
// star, so every peer predicts and later corrects -- the impaired path.
export function advance(
  env: NetDiagnosticsFixtureEnv,
  state: NetDiagnosticsFixtureHarness,
  samples?: Readonly<Record<number, unknown>>,
  deliver = true
): MatchDriverBatch[] {
  const batches: MatchDriverBatch[] = [];
  state.peers.forEach((peer, index) => {
    const sample = samples?.[index + 1] ?? env.inputFrame.neutralSample();
    const batch = env.matchDriver.advance(peer.driver, sample);
    batches.push(batch);
    fold(state, env, peer, batch);
  });
  if (deliver) {
    pump(state.session.host_transport);
  }
  state.step += 1;
  state.clock_ms += state.clock_step_ms;
  return batches;
}

// The in-process fake star exposes a `pump()` test affordance outside the
// `StarTransportAdapter` contract (see `@gc/transport`'s `FakeStarTransport`).
// It is called through a narrow structural type here rather than importing
// the fake directly, since a real (browser) star has no such method and
// this fixture is transport-agnostic otherwise.
function pump(transport: StarTransportAdapter): void {
  const pumpable = transport as StarTransportAdapter & { pump?: () => void };
  if (typeof pumpable.pump === "function") {
    pumpable.pump();
  }
}

export function run(
  env: NetDiagnosticsFixtureEnv,
  state: NetDiagnosticsFixtureHarness,
  steps: number,
  samples?: Readonly<Record<number, unknown>>
): MatchDriverBatch[] {
  let last: MatchDriverBatch[] = [];
  for (let index = 0; index < steps; index += 1) {
    last = advance(env, state, samples);
  }
  return last;
}

export function runBursty(
  env: NetDiagnosticsFixtureEnv,
  state: NetDiagnosticsFixtureHarness,
  steps: number,
  period: number,
  samples?: Readonly<Record<number, unknown>>
): void {
  for (let step = 1; step <= steps; step += 1) {
    advance(env, state, samples, step % period === 0);
  }
}

export function peer(state: NetDiagnosticsFixtureHarness, peerId: string): NetDiagnosticsFixturePeer {
  const found = state.peers.find((candidate) => candidate.peer_id === peerId);
  if (found === undefined) {
    throw new Error(`no fixture peer named ${peerId}`);
  }
  return found;
}

// A real, decodable session control message, so `recordControl` is
// exercised against the protocol rather than against a stub payload.
export function hashReport(
  env: NetDiagnosticsFixtureEnv,
  state: NetDiagnosticsFixtureHarness,
  peerId: string,
  sequence: number,
  tick: number,
  boundaryHash: string
): TransportPeerMessage {
  const message = unwrap(
    env.protocol.new_("hash_report", state.session.manifest.session_id, peerId, sequence, {
      tick,
      boundary_hash: boundaryHash,
    }),
    "hash report message"
  );
  const payload = unwrap(env.protocol.encode(message), "hash report encode");
  const envelope = unwrap(
    newMessage({ type: "event", seq: sequence, payload }),
    "hash report envelope",
    (failure) => failure.message
  );
  return { peer_id: peerId, channel: "control", message: envelope, arrival_seq: sequence };
}

// Forge an input bundle from `senderId` naming `slotIndex`. Ownership is a
// frozen partition, so a slot outside the sender's owned set is an
// `ownership_violation` and a second bundle on the same sequence with
// different bytes is an `authority_conflict`. Both are reached this way
// rather than by reaching into the driver.
export function forgedBundle(
  env: NetDiagnosticsFixtureEnv,
  state: NetDiagnosticsFixtureHarness,
  senderId: string,
  slotIndex: number,
  sequence: number,
  transportTick: number,
  edges: number
): TransportMessage {
  const rows: InputProtocolRow[] = [];
  for (let tick = 0; tick <= env.inputProtocol.HISTORY_ROWS; tick += 1) {
    const sample = unwrap(
      env.inputFrame.newSample({ edges: tick === env.inputProtocol.HISTORY_ROWS ? edges : 0 }),
      "forged bundle sample"
    );
    rows.push({ tick, slot_index: slotIndex, sample });
  }
  const packet = unwrap(
    env.inputProtocol.newGuest({
      session_id: state.session.manifest.session_id,
      manifest_id: state.session.freeze.manifest_id,
      sender_id: senderId,
      sequence,
      transport_tick: transportTick,
      first_input_tick: state.session.freeze.first_input_tick,
      rows,
    }),
    "forged bundle packet"
  );
  const payload = unwrap(env.inputProtocol.encode(packet), "forged bundle encode");
  return unwrap(
    newMessage({ type: "input", seq: packet.sequence, tick: packet.transport_tick, payload }),
    "forged bundle envelope",
    (failure) => failure.message
  );
}
