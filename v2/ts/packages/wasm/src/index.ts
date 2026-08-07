// Typed loader for the `gc-wasm` build artifact.
//
// `gc-wasm` binds the simulation two different ways (see
// `v2/rust/crates/gc-wasm/src/lib.rs`'s doc):
//
// - Session lifecycle, the determinism check, and lobby wire helpers go
//   through `wasm-bindgen` — ergonomic, generated JS classes/functions,
//   reached here via `dist/pkg/gc_wasm.cjs` (built by `scripts/build.mjs`).
// - The per-frame render path is RAW: plain `extern "C"` exports with no
//   wasm-bindgen glue, so a rendered frame's flat `f64` block crosses once,
//   copy-free, as a `Float64Array` view over wasm linear memory.
//
// Both live in the same compiled module and therefore the same
// `WebAssembly.Instance` — `Session`'s registry handle
// (`crates/gc-wasm/src/registry.rs`) is only meaningful against the
// instance that created it. `scripts/build.mjs` patches wasm-bindgen's
// generated output to expose that one shared instance's raw exports
// (`__wbg_raw`) alongside its ergonomic classes, and `loadSimHost` below is
// what stitches the two into one typed surface.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  Coordinator,
  CoordinatorConstructor,
  ControlMessageHeader,
  DeterminismEvidence,
  FixedClockConstructor,
  GcWasmModule,
  InputFrameBridge,
  InputProtocolBridge,
  MatchDriverBridge,
  MatchDriverBridgeConstructor,
  MatchDriverFixtureBridge,
  MatchSnapshotBridge,
  OnlineCombatPhasesBridge,
  PlayerPoseBridge,
  RollbackEventsTimelineConstructor,
  RollbackPlayableLabConstructor,
  SimSessionConstructor,
  TuningRegistryConstructor,
  WasmTuningPreset,
} from "./types.ts";

export type {
  Coordinator,
  CoordinatorConstructor,
  ControlMessageHeader,
  DeterminismEvidence,
  FixedClock,
  FixedClockConstructor,
  InputFrameBridge,
  InputProtocolBridge,
  MatchDriverBridge,
  MatchDriverBridgeConstructor,
  MatchDriverFixtureBridge,
  MatchSnapshotBridge,
  OnlineCombatPhasesBridge,
  OnlineCombatPhaseScenario,
  PlayerPoseBridge,
  RawExports,
  RollbackEventsTimeline,
  RollbackEventsTimelineConstructor,
  RollbackPlayableLab,
  RollbackPlayableLabConstructor,
  RollbackPlayableLabSnapshotLookup,
  SimSession,
  SimSessionConstructor,
  SnapshotLookup,
  TuningRegistry,
  TuningRegistryConstructor,
  WasmFakeStarTransport,
  WasmInputPacket,
  WasmKnob,
  WasmMatchDriverFixtureSession,
  WasmMatchSnapshot,
  WasmTuningPreset,
} from "./types.ts";

const here = dirname(fileURLToPath(import.meta.url));
const artifactPath = join(here, "..", "dist", "pkg", "gc_wasm.cjs");

/** The loaded `gc-wasm` module: session lifecycle, the determinism check,
 * lobby wire helpers, and the raw per-frame render path in one typed
 * surface. */
export interface SimHost
  extends InputFrameBridge,
    InputProtocolBridge,
    MatchDriverFixtureBridge,
    MatchSnapshotBridge,
    OnlineCombatPhasesBridge,
    PlayerPoseBridge {
  /** Constructs a live match session. See `SimSession`'s doc. */
  readonly Session: SimSessionConstructor;
  /** Constructs a session coordinator (host or guest). See `Coordinator`'s
   * doc — `crates/gc-wasm/src/coordinator_bridge.rs`'s wave-2 bridge over
   * `gc_netcode::coordinator`'s reducer. */
  readonly Coordinator: CoordinatorConstructor;
  /** Constructs one peer's online match driver. See `MatchDriverBridge`'s
   * doc — `crates/gc-wasm/src/match_driver_bridge.rs`'s wave-2 bridge over
   * `gc_netcode::match_driver` and its `gc_sim::rollback_events` feed. */
  readonly MatchDriverBridge: MatchDriverBridgeConstructor;
  /** Builds a standalone `RollbackEventsTimeline` — `create`/`apply`/
   * `confirm`/`diagnosticsJson` as separate callables over
   * `gc_sim::rollback_events`, satisfying `@gc/online`'s
   * `RollbackEventsPort`. See `crates/gc-wasm/src/rollback_events_bridge.rs`'s
   * doc. */
  readonly RollbackEventsTimeline: RollbackEventsTimelineConstructor;
  /** Builds a `RollbackPlayableLab` — the deterministic, network-simulated,
   * bot-driven local rollback harness (`gc_sim::rollback_playable_lab`) a
   * real `packages/screens/src/match.ts` `RollbackHostPort` implementation
   * wraps. See `crates/gc-wasm/src/rollback_playable_lab_bridge.rs`'s doc. */
  readonly RollbackPlayableLab: RollbackPlayableLabConstructor;
  /** Constructs a render-driven tick-count planner over
   * `gc_sim::fixed_clock`. See `FixedClock`'s doc — this is the single
   * source of truth for turning a render `dt` into a tick count
   * (v2/README.md §2.1), so a caller like `@gc/app`'s `sim_host.ts` never
   * has to re-derive that policy in TypeScript. */
  readonly FixedClock: FixedClockConstructor;
  /** Constructs a live `gc_sim::tuning` knob registry, at every knob's
   * default value — satisfies `@gc/ui`'s `TuningSource`. See
   * `crates/gc-wasm/src/tuning_bridge.rs`'s doc. */
  readonly TuningRegistry: TuningRegistryConstructor;
  /** `gc_data::tuning_presets::ALL`, as `@gc/ui`'s `TuningPreset[]`, in
   * panel cycle order. */
  tuningPresets(): WasmTuningPreset[];
  /** This build's protocol vocabulary id, for lobby version negotiation. */
  protocolVocabularyId(): string;
  /** Decodes a control message's routing header (kind/session/peer/
   * sequence), without exposing its kind-specific body. */
  decodeControlMessageHeader(wire: string): ControlMessageHeader;
  /** Runs the frozen OMP-1 determinism campaign inside this wasm module and
   * returns its evidence. This is the acceptance surface for "did compiling
   * to wasm change simulation floating-point behavior" — compare
   * `final_hash`/`sequence_digest` against the pinned native-build
   * digests. */
  runDeterminismEvidence(): DeterminismEvidence;
  /** This module's linear memory, for reading the `Float64Array` views
   * `buildRenderFrame` hands back. */
  readonly memory: WebAssembly.Memory;
  /**
   * Builds the current render frame for the session named by `handle`
   * (`SimSession.handle`) and returns a zero-copy `Float64Array` view over
   * it — `gc_render::frame_buffer::encode`'s layout
   * (`v2/rust/crates/gc-render/src/frame_buffer.rs`), decoded field-by-field
   * on the JS side per that module's documented header/field-order
   * contract. Returns `null` if `handle` names no live session.
   *
   * The returned view is only valid until the next `buildRenderFrame`
   * call (the underlying buffer is reused and can move if it grows) —
   * copy out anything that needs to outlive that call.
   */
  buildRenderFrame(handle: number): Float64Array | null;
  /**
   * Builds `bridge`'s CURRENT render frame (the live boundary
   * {@link MatchDriverBridge.advance} most recently produced) and returns a
   * zero-copy `Float64Array` view over it — the `MatchDriverBridge`
   * counterpart of {@link SimHost.buildRenderFrame}, same wire layout
   * (`gc_render::frame_buffer::encode`'s), same decode contract. Unlike
   * `buildRenderFrame` this never returns `null`: a live `MatchDriverBridge`
   * reference can always build its own frame (see
   * {@link MatchDriverBridge.renderFrameBuild}'s doc for why — it is not a
   * `crate::registry` handle lookup that can miss).
   *
   * The returned view is only valid until the next
   * `buildMatchDriverRenderFrame` call (this reused buffer is separate from
   * `buildRenderFrame`'s own — see `RawExports`'s doc — so calling
   * `buildRenderFrame` in between does not itself invalidate this view; any
   * wasm memory growth from either call would, same as `buildRenderFrame`'s
   * own caveat) — copy out anything that needs to outlive that call.
   */
  buildMatchDriverRenderFrame(bridge: MatchDriverBridge): Float64Array;
}

let cached: SimHost | undefined;

/**
 * Loads the compiled `gc-wasm` artifact (see `scripts/build.mjs`) once per
 * process and wraps it as a {@link SimHost}.
 *
 * @throws If the artifact has not been built yet — run
 * `pnpm --filter @gc/wasm build` first.
 */
export function loadSimHost(): SimHost {
  if (cached) {
    return cached;
  }

  const require = createRequire(import.meta.url);
  let native: GcWasmModule;
  try {
    native = require(artifactPath) as GcWasmModule;
  } catch (cause) {
    throw new Error(
      `@gc/wasm: compiled artifact missing at ${artifactPath}. ` +
        'Run "pnpm --filter @gc/wasm build" first (see scripts/build.mjs).',
      { cause },
    );
  }

  const raw = native.__wbg_raw;
  const host: SimHost = {
    Session: native.Session,
    Coordinator: native.Coordinator,
    MatchDriverBridge: native.MatchDriverBridge,
    RollbackEventsTimeline: native.RollbackEventsTimeline,
    RollbackPlayableLab: native.RollbackPlayableLab,
    FixedClock: native.FixedClock,
    TuningRegistry: native.TuningRegistry,
    tuningPresets: native.tuningPresets,
    protocolVocabularyId: native.protocolVocabularyId,
    decodeControlMessageHeader: native.decodeControlMessageHeader,
    runDeterminismEvidence: native.runDeterminismEvidence,
    // `input_frame_bridge.rs` — plain free functions, bound to nothing, so
    // referencing them directly off `native` is exactly as correct as
    // wrapping them in a closure (same as `tuningPresets`/
    // `protocolVocabularyId` above).
    inputFrameEdgeBitsJson: native.inputFrameEdgeBitsJson,
    inputFrameHeldBitsJson: native.inputFrameHeldBitsJson,
    inputFrameConstantsJson: native.inputFrameConstantsJson,
    inputFrameNeutralSample: native.inputFrameNeutralSample,
    inputFrameNewSample: native.inputFrameNewSample,
    inputFrameDecodeSampleJson: native.inputFrameDecodeSampleJson,
    inputFrameQuantizeAxis: native.inputFrameQuantizeAxis,
    // `input_protocol_bridge.rs`.
    inputProtocolConstantsJson: native.inputProtocolConstantsJson,
    inputProtocolNewGuest: native.inputProtocolNewGuest,
    inputProtocolNewHost: native.inputProtocolNewHost,
    inputProtocolEncode: native.inputProtocolEncode,
    inputProtocolDecode: native.inputProtocolDecode,
    inputProtocolPacketId: native.inputProtocolPacketId,
    inputProtocolConfirmedTick: native.inputProtocolConfirmedTick,
    inputProtocolConfirmedSpan: native.inputProtocolConfirmedSpan,
    inputProtocolClassifyDuplicate: native.inputProtocolClassifyDuplicate,
    inputProtocolSupersedeForBackpressure: native.inputProtocolSupersedeForBackpressure,
    // `match_driver_fixture_bridge.rs`.
    matchDriverFixtureConstantsJson: native.matchDriverFixtureConstantsJson,
    matchDriverFixtureGuestPeerId: native.matchDriverFixtureGuestPeerId,
    matchDriverFixturePeerIds: native.matchDriverFixturePeerIds,
    matchDriverFixtureFreezeJson: native.matchDriverFixtureFreezeJson,
    matchDriverFixtureManifestJson: native.matchDriverFixtureManifestJson,
    matchDriverFixtureInitialSnapshot: native.matchDriverFixtureInitialSnapshot,
    matchDriverFixtureSession: native.matchDriverFixtureSession,
    // `online_combat_phases_bridge.rs`.
    onlineCombatPhaseIds: native.onlineCombatPhaseIds,
    onlineCombatPhaseScenarioJson: native.onlineCombatPhaseScenarioJson,
    onlineCombatPhaseBoundaryZero: native.onlineCombatPhaseBoundaryZero,
    onlineCombatPhaseLiveSample: native.onlineCombatPhaseLiveSample,
    onlineCombatPhaseObserved: native.onlineCombatPhaseObserved,
    // `match_snapshot_bridge.rs`.
    matchSnapshotBuild: native.matchSnapshotBuild,
    matchSnapshotStateJson: native.matchSnapshotStateJson,
    // `player_pose_bridge.rs`.
    playerPoseSelect: native.playerPoseSelect,
    memory: raw.memory,
    buildRenderFrame(handle: number): Float64Array | null {
      const ok = raw.render_frame_build(handle);
      if (ok === 0) {
        return null;
      }
      const ptr = raw.render_frame_ptr();
      const len = raw.render_frame_len();
      // `ptr` is already a byte offset (Vec<f64>::as_ptr() cast to u32);
      // `len` is an element count. A fresh view every call, never cached:
      // `raw.memory.buffer` is replaced whenever wasm linear memory grows,
      // and a stale view would point at a detached ArrayBuffer.
      return new Float64Array(raw.memory.buffer, ptr, len);
    },
    buildMatchDriverRenderFrame(bridge: MatchDriverBridge): Float64Array {
      // Always `1` -- see `MatchDriverBridge.renderFrameBuild`'s own doc for
      // why this call can never report "no live bridge" the way
      // `render_frame_build(handle)` can.
      bridge.renderFrameBuild();
      const ptr = raw.driver_render_frame_ptr();
      const len = raw.driver_render_frame_len();
      return new Float64Array(raw.memory.buffer, ptr, len);
    },
  };
  cached = host;
  return host;
}
