//! The slice of `@gc/wasm`'s compiled surface `online_ports.ts` needs from
//! the session coordinator (`crates/gc-wasm/src/coordinator_bridge.rs`) and
//! the online match driver (`crates/gc-wasm/src/match_driver_bridge.rs`).
//!
//! Factored out into its own type-only contract so `online_ports.ts` never
//! imports a concrete wasm build target -- exactly the split
//! `browser_sim_host.ts`/`@gc/wasm`'s own `sim_host.ts` already establish
//! for the offline path (that file's header: "wasm loading in a browser
//! differs from node"). `browser_online_wasm_host.ts` wraps `@gc/wasm/web`
//! (production; loaded once per page, sharing `browser_sim_host.ts`'s own
//! `init()` promise -- see that file's header for why a second `init()`
//! call would be wasted, not wrong). `online_ports.spec.ts` wraps `@gc/wasm`'s
//! Node-target `loadSimHost()` (a `devDependency` of this package, spec-only)
//! against this exact same interface, so the production wiring this file's
//! sibling builds is what the spec drives -- not a parallel reimplementation
//! of it.
//!
//! Every method mirrors `@gc/wasm/src/types.ts`'s hand-written `Coordinator`/
//! `MatchDriverBridge`/`SimSessionConstructor` interfaces field-for-field
//! (both wasm-bindgen targets are generated from the identical
//! `#[wasm_bindgen(js_name = ...)]`-annotated Rust source, so the same
//! method names bind under `--target nodejs` and `--target web` alike) --
//! restated here, not imported, because `types.ts` lives in `@gc/wasm`'s own
//! `src/`, which this package may depend on (`package.json`) but whose
//! *types* this module intentionally keeps decoupled from, the same
//! reasoning `match_presentation.ts` (`@gc/online`) gives for its own
//! narrow ports.

/** The three canonical online match modes, `gc_netcode::protocol::MatchMode`'s wire strings. */
export type OnlineMatchMode = "1v1" | "2v2" | "4v4";

/** Mirrors `@gc/wasm`'s `Coordinator` -- see that interface's own doc
 * (`types.ts`) for the JSON-string convention every method here follows. */
export interface OnlineCoordinatorHandle {
  enqueueControlWire(linkId: string, wire: string): void;
  enqueueLinkLost(linkId: string, code?: string): void;
  tick(): string;
  connect(): string;
  proposeManifest(manifestJson: string): string;
  assignSlots(assignmentsJson: string, preserveClaims: boolean): string;
  preferPair(slots: string[]): string;
  setReady(ready: boolean): string;
  beginCountdown(countdownId: string, remainingTicks: number, firstInputTick: number): string;
  matchPhase(phase: string, tick: number, homeScore: number, awayScore: number): string;
  hashReport(tick: number, boundaryHash: string): string;
  matchFinish(finalTick: number, homeScore: number, awayScore: number, finalHash: string): string;
  netcodeFailure(failure: string, peerId?: string, detail?: string): string;
  leave(): string;
  abort(code?: string, detail?: string): string;
  stateJson(): string;
}

/** Mirrors `@gc/wasm`'s `CoordinatorConstructor`. */
export interface OnlineCoordinatorConstructor {
  new (
    role: "host" | "guest",
    sessionId: string,
    peerId: string,
    hostPeerId: string | undefined,
    hostLinkId: string | undefined,
    runtimeJson: string,
    buildId: string | undefined,
    expectationJson: string | undefined,
  ): OnlineCoordinatorHandle;
}

/** The slice of `@gc/wasm`'s `SimSession` a fresh `MatchDriverBridge`
 * needs behind it -- only `free()` is ever called directly; the rest of
 * this handle's identity lives inside the constructed bridge. */
export interface OnlineMatchSessionHandle {
  free(): void;
}

/** Mirrors `@gc/wasm`'s `SimSessionConstructor`, narrowed to the
 * positional arguments `online_ports.ts` actually supplies (team ids,
 * seed, duration, max goals) -- the rest are optional and left unset. */
export interface OnlineMatchSessionConstructor {
  new (
    homeTeamId: string,
    awayTeamId: string,
    seed: number,
    durationSeconds: number,
    maxGoals: number,
  ): OnlineMatchSessionHandle;
}

/** Mirrors `@gc/wasm`'s `MatchDriverBridge` -- see that interface's own doc
 * (`types.ts`) for the wire/JSON shapes each method crosses. */
export interface OnlineMatchDriverHandle {
  initializeTransport(): void;
  openPeer(peerId: string): number;
  setPeerConnected(peerId: string): void;
  setPeerDisconnected(peerId: string, detail: string): void;
  enqueueInbound(
    peerId: string,
    channel: "control" | "input",
    kind: "input" | "event" | "state",
    seq: number,
    tick: number | undefined,
    payload: Uint8Array,
  ): void;
  drainOutboundJson(): string;
  advance(sampleWire?: string): string;
  observeCheckpoint(tick: number, hash: string): boolean;
  statusJson(): string;
  terminalJson(): string;
  diagnosticsJson(): string;
  snapshotLookup(boundaryTick: number): unknown;
  renderFrameBuild(kickFollowSlots: number, dispossessedSlots: number): number;
  rosterNumeric(): Float64Array;
  rosterIdsAndNames(): string;
  free(): void;
}

/** Mirrors `@gc/wasm`'s `MatchDriverBridgeConstructor`. */
export interface OnlineMatchDriverConstructor {
  new (
    session: OnlineMatchSessionHandle,
    role: "host" | "guest",
    peerId: string,
    freezeJson: string,
    manifestJson: string,
    maxGuests: number | undefined,
    queueLimit?: number,
  ): OnlineMatchDriverHandle;
}

/** The `@gc/wasm` surface `online_ports.ts` needs, independent of build
 * target -- see this file's header. */
export interface OnlineWasmHost {
  readonly Coordinator: OnlineCoordinatorConstructor;
  readonly Session: OnlineMatchSessionConstructor;
  readonly MatchDriverBridge: OnlineMatchDriverConstructor;
  /** `gc_netcode::protocol_fixture::manifest`, as JSON. See
   * `online_ports.ts`'s header for why this fixture content, rather than a
   * real content-pipeline manifest, is what a lobby proposes today --
   * a disclosed, pre-existing gap (`browser_content.ts`'s own header
   * documents the same one for `matchContract`), not one this issue's
   * scope can close (out of scope: "any change to the session protocol").
   */
  matchDriverFixtureManifestJson(mode: OnlineMatchMode): string;
  /** `gc_sim::input_frame::encode_sample`, as a canonical wire string --
   * `MatchDriverPort.advance`'s local sample, encoded the same way
   * `browser_sim_host.ts`'s own input path already does. */
  inputFrameNewSample(moveX: number, moveY: number, held: number, edges: number): string;
  /** {@link OnlineMatchDriverHandle.renderFrameBuild} plus the raw ptr/len
   * read into one `Float64Array` view -- the `MatchDriverBridge`
   * counterpart of the offline path's `SimHost.buildRenderFrame`, mirrored
   * exactly (`browser_sim_host.ts`'s own `frame()`). */
  buildMatchDriverRenderFrame(
    bridge: OnlineMatchDriverHandle,
    kickFollowSlots: number,
    dispossessedSlots: number,
  ): Float64Array;
}
