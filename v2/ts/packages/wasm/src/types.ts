// Hand-written interfaces mirroring `gc-wasm`'s generated bindings
// (`dist/pkg/gc_wasm.d.cts`, produced from `v2/rust/crates/gc-wasm/src/**`
// by `scripts/build.mjs`). Kept separate from a generated import so this
// package's public API is explicit and reviewable without reading Rust —
// but it must be kept in sync by hand: if a `gc-wasm` binding's shape
// changes, update the matching interface here.

/** Mirrors `gc_wasm::session::Session` (`crates/gc-wasm/src/session.rs`). */
export interface SimSession {
  readonly handle: number;
  readonly scoreHome: number;
  readonly scoreAway: number;
  readonly timeLeft: number;
  readonly finished: boolean;
  readonly inputTick: number;
  /** Advances the match by one fixed tick. `wire` is a canonical
   * `gc_sim::input_frame` wire string. Throws (a string) on a malformed
   * wire. */
  step(wire: string): void;
  /** The current canonical snapshot hash. */
  snapshotHash(): string;
  /** Match-constant per-player roster fields, as a flat array. */
  rosterNumeric(): Float64Array;
  /** Match-constant roster ids and display names, newline-joined. */
  rosterIdsAndNames(): string;
  /** This session's current boundary snapshot, as an opaque handle — see
   * {@link WasmMatchSnapshot}'s doc. For building a
   * {@link RollbackEventsTimeline} via {@link RollbackEventsTimelineConstructor.create}
   * without a {@link MatchDriverBridge}. */
  snapshotHandle(): WasmMatchSnapshot;
  /** Releases the underlying wasm-side registry slot. Call when done with
   * a session — the wasm module does not garbage-collect on its own. */
  free(): void;
}

/** Constructs a {@link SimSession}. */
export interface SimSessionConstructor {
  new (
    homeTeamId: string,
    awayTeamId: string,
    seed: number,
    durationSeconds: number,
    maxGoals: number,
  ): SimSession;
}

/**
 * Mirrors `gc_wasm::determinism::DeterminismEvidence`
 * (`crates/gc-wasm/src/determinism.rs`). Field names are verbatim Rust
 * field names — plain `#[wasm_bindgen]` struct fields are not
 * camelCase-renamed the way methods are.
 */
export interface DeterminismEvidence {
  readonly fixture_id: string;
  readonly ticks: number;
  readonly boundaries: number;
  readonly final_hash: string;
  readonly sequence_digest: string;
  readonly score_home: number;
  readonly score_away: number;
  readonly outcome: "home" | "away" | "draw";
  readonly snapshot_bytes: number;
}

/** Mirrors `gc_wasm::protocol_bridge::ControlMessageHeader`. */
export interface ControlMessageHeader {
  readonly kind: string;
  readonly session_id: string;
  readonly peer_id: string;
  readonly sequence: number;
  readonly message_id: string;
}

/**
 * The raw, non-wasm-bindgen ABI `gc_wasm::render_export` exports — plain
 * numeric `extern "C"` functions plus the module's linear memory. See
 * `crates/gc-wasm/src/render_export.rs`'s doc for the three-call contract
 * (`render_frame_build` then read `render_frame_ptr`/`render_frame_len`).
 */
export interface RawExports {
  readonly memory: WebAssembly.Memory;
  render_frame_build(handle: number): number;
  render_frame_ptr(): number;
  render_frame_len(): number;
}

/**
 * Mirrors `gc_wasm::coordinator_bridge::Coordinator`
 * (`crates/gc-wasm/src/coordinator_bridge.rs`) — the bridge over
 * `gc_netcode::coordinator`'s reducer.
 *
 * Every method that takes or returns structured coordinator data (a
 * manifest, an outcome, the full state) crosses as a JSON *string* rather
 * than a typed object: the Rust side's own `Json` encoder
 * (`crates/gc-wasm/src/json.rs`) is a small, hand-rolled, dependency-free
 * codec built specifically to avoid pulling `serde` onto
 * `gc_netcode::coordinator`'s determinism-adjacent types (see that module's
 * doc comment), and mirroring its exact output shape as a second,
 * hand-maintained set of TypeScript interfaces here would only invite the
 * two to drift. A caller does `JSON.parse(coordinator.tick())` and reads
 * the fields documented on the Rust side
 * (`coordinator_bridge.rs`'s `*_to_json` functions) — `outcomes`/`summary`
 * for every event-applying method, `state` for {@link Coordinator.stateJson}.
 *
 * ## The queue/drain seam
 *
 * `enqueueControlWire`/`enqueueLinkLost` only ever append to an internal
 * queue — see `crates/gc-wasm/src/net_inbox.rs`'s doc. Nothing queued is
 * applied to coordinator state until the next {@link Coordinator.tick} call,
 * which drains the queue once, in arrival order. Call `enqueueControlWire`
 * directly from a WebRTC/WebSocket `onmessage` handler at any time — that is
 * exactly the discipline this shape exists to make easy to get right; call
 * `tick()` once per fixed tick from the render/simulation loop.
 */
export interface Coordinator {
  readonly queuedCount: number;
  enqueueControlWire(linkId: string, wire: string): void;
  enqueueLinkLost(linkId: string, code?: string): void;
  /** Drains the queue, applies every queued event then one `Tick` event,
   * and returns `{"outcomes": [...], "summary": {...}}` as JSON. */
  tick(): string;
  connect(): string;
  /** `manifestJson` is a JSON encoding of a `gc_netcode::protocol::Value`
   * record — see {@link Coordinator}'s doc for the array/object rule.
   * Throws (a string) if `manifestJson` fails to parse. */
  proposeManifest(manifestJson: string): string;
  assignSlots(assignmentsJson: string, preserveClaims: boolean): string;
  /** `slots` are canonical slot wire ids (e.g. `"home_1"`). Throws (a
   * string) if any entry is not one. */
  preferPair(slots: string[]): string;
  setReady(ready: boolean): string;
  beginCountdown(countdownId: string, remainingTicks: number, firstInputTick: number): string;
  matchPhase(phase: string, tick: number, homeScore: number, awayScore: number): string;
  hashReport(tick: number, boundaryHash: string): string;
  matchFinish(finalTick: number, homeScore: number, awayScore: number, finalHash: string): string;
  netcodeFailure(failure: string, peerId?: string, detail?: string): string;
  leave(): string;
  abort(code?: string, detail?: string): string;
  /** A compact summary (role, phase, peer/ready counts, manifest id, frozen
   * flag, terminal), as JSON. Cheap to call every frame. */
  summaryJson(): string;
  /** The complete coordinator state, as JSON. */
  stateJson(): string;
}

/** Constructs a {@link Coordinator}. `role` is `"host"`/`"guest"`.
 * `runtimeJson`/`expectationJson` are JSON encodings of
 * `gc_netcode::protocol::Value`/`ManifestExpectation` — see
 * {@link Coordinator}'s doc. Throws (a string) if `role` is unrecognized or
 * either JSON argument fails to parse/validate. */
export interface CoordinatorConstructor {
  new (
    role: "host" | "guest",
    sessionId: string,
    peerId: string,
    hostPeerId: string | undefined,
    hostLinkId: string | undefined,
    runtimeJson: string,
    buildId: string | undefined,
    expectationJson: string | undefined,
  ): Coordinator;
}

/**
 * Mirrors `gc_wasm::rollback_events_bridge::WasmMatchSnapshot`
 * (`crates/gc-wasm/src/rollback_events_bridge.rs`) — an opaque
 * `gc_sim::match_snapshot::MatchSnapshot` handle. Never serialized to JSON
 * and never inspected on the JS side (see that Rust module's doc); a JS
 * caller only ever holds a reference obtained from {@link SimSession.snapshotHandle}
 * or {@link MatchDriverBridge.snapshotLookup}, and passes it back into
 * {@link RollbackEventsTimelineConstructor.create}/{@link RollbackEventsTimeline.apply}.
 * This is the `TSnapshot` type parameter
 * `packages/online/src/match_presentation.ts`'s `RollbackEventsPort`/
 * `MatchDriverPort` declare.
 */
export interface WasmMatchSnapshot {
  free(): void;
}

/**
 * Mirrors `gc_wasm::match_driver_bridge::SnapshotLookup`
 * (`crates/gc-wasm/src/match_driver_bridge.rs`) — one
 * `gc_netcode::match_driver::snapshot` lookup result:
 * `packages/online/src/match_presentation.ts`'s `SnapshotLookup<TSnapshot>`
 * (`TSnapshot` = {@link WasmMatchSnapshot}).
 */
export interface SnapshotLookup {
  /** This boundary's retention status: `"present"`, `"retained"`,
   * `"missing"`, or `"outside_window"` — only the first two carry a
   * {@link SnapshotLookup.snapshot}. */
  readonly status: string;
  /** The queried boundary tick. */
  readonly tick: number;
  /** The retained snapshot, present exactly when {@link SnapshotLookup.status}
   * is `"present"` or `"retained"`. */
  readonly snapshot?: WasmMatchSnapshot;
  free(): void;
}

/**
 * Mirrors `gc_wasm::rollback_events_bridge::RollbackEventsTimeline`
 * (`crates/gc-wasm/src/rollback_events_bridge.rs`) — `create`/`apply`/
 * `confirm`/`diagnosticsJson` as separate callables over
 * `gc_sim::rollback_events`, satisfying
 * `packages/online/src/match_presentation.ts`'s
 * `RollbackEventsPort<TTimeline, TSnapshot>` (`TTimeline` =
 * {@link RollbackEventsTimeline}, `TSnapshot` = {@link WasmMatchSnapshot}).
 * There is no public constructor — build one via
 * {@link RollbackEventsTimelineConstructor.create}, not `new`.
 */
export interface RollbackEventsTimeline {
  /**
   * Mirrors `gc_sim::rollback_events::apply`. `outputsJson` is a JSON array
   * of `gc_wasm::rollback_events_bridge::tick_output_to_json`'s shape —
   * exactly what {@link MatchDriverBridge.advance}'s batch embeds per
   * output under `"outputs"` — and `snapshots` is the parallel array of
   * boundary snapshots (same length and order). Returns
   * `RollbackApplyResult` as JSON: `{"ok": true, "value": <RollbackEventDiff>}`
   * or `{"ok": false, "error": {"message", "code"}}` — the
   * `unconfirmed_window_exceeded` failure is reported this way, never
   * thrown. Throws (a string) if `outputsJson`/`snapshots` are malformed or
   * mismatched in length.
   */
  apply(replacedFromTick: number, replacedThroughTick: number, outputsJson: string, snapshots: WasmMatchSnapshot[]): string;
  /** Mirrors `gc_sim::rollback_events::confirm`. Returns the confirmed
   * steps as a JSON array, in causal order. */
  confirm(confirmedOutputTick: number): string;
  /** Mirrors `gc_sim::rollback_events::diagnostics`, as JSON. */
  diagnosticsJson(): string;
  free(): void;
}

/** Builds a {@link RollbackEventsTimeline}. Named `create`, not `new`, to
 * mirror `RollbackEventsPort.create`'s own doc comment (`new` is a reserved
 * word). `maxUnconfirmedTicks` defaults the same way
 * `gc_sim::rollback_events::new` does when omitted. */
export interface RollbackEventsTimelineConstructor {
  create(initialSnapshot: WasmMatchSnapshot, maxUnconfirmedTicks?: number): RollbackEventsTimeline;
}

/**
 * Mirrors `gc_wasm::match_driver_bridge::MatchDriverBridge`
 * (`crates/gc-wasm/src/match_driver_bridge.rs`) — the bridge over
 * `gc_netcode::match_driver` (the OMP-3 online match driver) and its
 * `gc_sim::rollback_events` feed. See that Rust module's doc for the full
 * picture: it reuses `gc_netcode::match_driver_fixture::DriverRules` as its
 * rules environment, is built from an already-constructed {@link SimSession}
 * (reused for its boundary-zero snapshot), and feeds `rollback_events`
 * automatically only for the safe, non-rollback case.
 *
 * Every JSON-string method follows the same rule {@link Coordinator} does —
 * read the shape from `match_driver_bridge.rs`'s `*_to_json` functions
 * rather than a duplicated TypeScript interface.
 *
 * ## The queue/drain seam
 *
 * `enqueueInbound` only ever appends to the wrapped
 * `gc_wasm::wasm_transport::WasmStarTransport`'s inbound queue — see that
 * module's and `crates/gc-wasm/src/net_inbox.rs`'s docs. `advance` is the
 * only reader (called once per fixed tick); it internally polls the
 * transport exactly once per call, so an item enqueued after one `advance`
 * call is not visible until the next.
 */
export interface MatchDriverBridge {
  initializeTransport(): void;
  /** Allocates a transport slot for `peerId`. Returns the assigned slot
   * number. */
  openPeer(peerId: string): number;
  /** Reports that the real connection to `peerId` (established by
   * `@gc/transport`) is up — signaling itself is out of this bridge's
   * scope, see the Rust module's doc. */
  setPeerConnected(peerId: string): void;
  setPeerDisconnected(peerId: string, detail: string): void;
  /** Queues one arrived envelope, already decoded by `@gc/transport` into
   * its structured fields. `channel` is `"control"`/`"input"`; `kind` is
   * `"input"`/`"event"`/`"state"`. Throws (a string) on an unrecognized
   * channel/kind, an unknown peer, or a structurally invalid envelope. */
  enqueueInbound(
    peerId: string,
    channel: "control" | "input",
    kind: "input" | "event" | "state",
    seq: number,
    tick: number | undefined,
    payload: Uint8Array,
  ): void;
  /** Drains every envelope queued to send, as JSON, oldest first. Call once
   * per tick, after {@link MatchDriverBridge.advance}, and actually
   * transmit each one via `@gc/transport`. */
  drainOutboundJson(): string;
  /** One fixed-tick driver step. `sampleWire` (if this peer authors a local
   * slot this tick) is a canonical `gc_sim::input_frame::InputSample` wire
   * (`encode_sample`/`decode_sample`'s format). Returns the batch as JSON.
   * Throws (a string) if `sampleWire` fails to decode. */
  advance(sampleWire?: string): string;
  statusJson(): string;
  terminalJson(): string;
  diagnosticsJson(): string;
  rollbackDiagnosticsJson(): string;
  rollbackAccountingJson(): string;
  retainedRollbackStepsJson(): string;
  /** This driver's own boundary-zero snapshot, as an opaque handle — for
   * building a standalone {@link RollbackEventsTimeline} against this same
   * driver's snapshot history (`newOnlineMatchPresentation`'s
   * `initialSnapshot`). */
  initialSnapshotHandle(): WasmMatchSnapshot;
  /** `gc_netcode::match_driver::snapshot`: looks up this driver's own
   * retained boundary-snapshot history at `boundaryTick` —
   * `MatchDriverPort.snapshot`. */
  snapshotLookup(boundaryTick: number): SnapshotLookup;
}

/** Constructs a {@link MatchDriverBridge}. `session` must be a freshly
 * constructed {@link SimSession} (boundary-zero, never stepped) — its
 * snapshot becomes the driver's `initial_snapshot`. `freezeJson`/
 * `manifestJson` are JSON encodings of `gc_netcode::coordinator::Freeze`/
 * `gc_netcode::protocol::Value` — typically exactly what a
 * {@link Coordinator}'s `start_match` action / `stateJson()` produced.
 * Throws (a string) if `role` is unrecognized or the JSON arguments fail to
 * parse/decode. */
export interface MatchDriverBridgeConstructor {
  new (
    session: SimSession,
    role: "host" | "guest",
    peerId: string,
    freezeJson: string,
    manifestJson: string,
    maxGuests: number | undefined,
  ): MatchDriverBridge;
}

/**
 * Mirrors `gc_wasm::session::FixedClock` (`crates/gc-wasm/src/session.rs`)
 * -- the render-driven tick-count decision (`gc_sim::fixed_clock::advance`'s
 * accumulator/catch-up/drop policy, v2/README.md §2.1), planned once per
 * render update from a `dt`. This exists specifically so no TypeScript
 * package has to re-derive that policy itself: call {@link FixedClock.advance}
 * with a render `dt` and run `step` that many times, in order.
 */
export interface FixedClock {
  /** Ticks this clock has authorized so far. */
  readonly tick: number;
  /**
   * Accumulate `renderDt` seconds and return how many ticks the caller
   * should run this render update. Throws (a string) if `renderDt` is not
   * finite and non-negative.
   */
  advance(renderDt: number): number;
  /**
   * The caller ran fewer ticks than the last `advance` call authorized
   * (e.g. a match finished mid-batch) -- zeroes the carried-over
   * accumulator, matching what stopping early means to
   * `gc_sim::fixed_clock::advance`'s own step callback.
   */
  stopEarly(): void;
}

/** Constructs a {@link FixedClock}. */
export interface FixedClockConstructor {
  new (): FixedClock;
}

/**
 * Mirrors `gc_wasm::tuning_bridge::WasmKnob`
 * (`crates/gc-wasm/src/tuning_bridge.rs`) — `packages/ui/src/tuning_panel.ts`'s
 * `Knob`. Field names are verbatim Rust field names, plain value fields
 * (not camelCase-renamed methods).
 */
export interface WasmKnob {
  readonly key: string;
  readonly label: string;
  readonly cat: string;
  readonly default: number;
  readonly min: number;
  readonly max: number;
  readonly step: number;
  free(): void;
}

/**
 * Mirrors `gc_wasm::tuning_bridge::WasmTuningPreset` —
 * `packages/ui/src/tuning_panel.ts`'s `TuningPreset`.
 */
export interface WasmTuningPreset {
  readonly id: string;
  readonly name: string;
  readonly blob: string;
  free(): void;
}

/**
 * Mirrors `gc_wasm::tuning_bridge::TuningRegistry` — a live registry of
 * `gc_sim::tuning` knob values, satisfying
 * `packages/ui/src/tuning_panel.ts`'s `TuningSource` method for method.
 */
export interface TuningRegistry {
  /** Distinct categories, in registry order. */
  categories(): string[];
  /** Every knob in one category, in registry order. */
  inCategory(cat: string): WasmKnob[];
  /** The current value of a knob. Panics (a Rust panic, not a `throw`) on
   * an unknown key — every caller reads a key it authored. */
  valueOf(key: string): number;
  /** Nudge a knob by `steps` (negative = down). Unknown keys are ignored. */
  nudge(key: string, steps: number): void;
  /** Reset one knob, or everything when `key` is omitted. Unknown keys are
   * ignored. */
  reset(key?: string): void;
  /** Whether a knob currently sits at its default value. */
  isDefault(key: string): boolean;
  /** One `KEY=value` line per non-default knob. */
  serialize(): string;
  /** Apply a serialized blob on top of defaults. Malformed lines are
   * skipped. */
  deserialize(blob: string): void;
  free(): void;
}

/** Constructs a {@link TuningRegistry}, at every knob's default value. */
export interface TuningRegistryConstructor {
  new (): TuningRegistry;
}

// ---------------------------------------------------------------------------
// `input_frame_bridge.rs` — `gc_sim::input_frame`'s per-slot sample codec.
// A sample crosses as its own canonical wire string (eight lowercase hex
// characters), never a handle or a JSON object — see that module's doc for
// why, and `SimSession.step`/`MatchDriverBridge.advance`'s own `sampleWire`
// parameter for the existing precedent this follows.
// ---------------------------------------------------------------------------

/** Every function `gc_wasm::input_frame_bridge` exports. Free functions, not
 * a class — see `SimHost`'s doc for why this package flattens everything
 * onto one loaded object rather than nesting per-Rust-module namespaces. */
export interface InputFrameBridge {
  /** `InputFramePort.EDGE_BITS`, as JSON: every canonical edge-action wire
   * name mapped to its bit (`{"shoot": 1, "pass": 2, ...}`). */
  inputFrameEdgeBitsJson(): string;
  /** Every canonical held-action wire name mapped to its bit, as JSON. */
  inputFrameHeldBitsJson(): string;
  /** `gc_sim::input_frame`'s bounds, as JSON: `version`, `slot_count`,
   * `home_slot_count`, `away_slot_count`, `move_scale`, `max_tick`,
   * `max_wire_bytes`. */
  inputFrameConstantsJson(): string;
  /** `InputFramePort.neutralSample`: the all-zero sample's canonical wire. */
  inputFrameNeutralSample(): string;
  /** `InputFramePort.newSample`: builds a validated sample from optional
   * overrides (unset fields default to zero) and returns its canonical
   * wire. Throws (a string) on an out-of-bound axis/mask or an invalid
   * equipment held/edge combination. */
  inputFrameNewSample(moveX?: number, moveY?: number, held?: number, edges?: number): string;
  /** Decodes a canonical sample wire back into its fields, as JSON
   * (`{"move_x", "move_y", "held", "edges"}`). Throws (a string) if `wire`
   * is not canonical. */
  inputFrameDecodeSampleJson(wire: string): string;
  /** Quantizes a raw `[-1, 1]` axis into a signed 8-bit value. Throws (a
   * string) if `rawAxis` is not finite. */
  inputFrameQuantizeAxis(rawAxis: number): number;
}

// ---------------------------------------------------------------------------
// `input_protocol_bridge.rs` — `gc_netcode::input_protocol`'s wire packets.
// Wire bytes cross as `Uint8Array` (never `string`) — see that module's doc
// and `packages/wasm/src/binary_string.ts` for converting to/from
// `@gc/transport`'s "binary string" `TransportMessage.payload` convention.
// ---------------------------------------------------------------------------

/**
 * Mirrors `gc_wasm::input_protocol_bridge::WasmInputPacket` — a decoded or
 * freshly-built `gc_netcode::input_protocol::Packet`. `sequence`/
 * `transport_tick` are `InputProtocolPacket`'s two declared fields
 * (`packages/online/src/net_diagnostics_fixture.ts`); the rest are exposed
 * too, at no extra cost, for a caller building diagnostics or a second
 * packet from the first. Field names are verbatim Rust field names, plain
 * value fields (not camelCase-renamed methods) — the same convention
 * {@link ControlMessageHeader}/{@link WasmKnob} already use.
 */
export interface WasmInputPacket {
  readonly sequence: number;
  readonly transport_tick: number;
  readonly first_input_tick: number;
  readonly confirmed_span: number;
  readonly session_id: string;
  readonly manifest_id: string;
  readonly sender_id: string;
  readonly packet_id: string;
  /** `"guest"` or `"host"`. */
  readonly kind: string;
  /** This packet's authority rows, as JSON (`[{"tick", "slot_index",
   * "sample"}, ...]`, `"sample"` an `inputFrame` canonical hex wire), in
   * canonical `(tick, slot_index)` order. */
  readonly rows_json: string;
  free(): void;
}

/** Every function `gc_wasm::input_protocol_bridge` exports. */
export interface InputProtocolBridge {
  /** `gc_netcode::input_protocol`'s bounds, as JSON: `version`,
   * `history_rows`, `retained_rows`, `fairness_delay_ticks`,
   * `max_guest_rows`, `host_window_rows`, `max_host_rows`, `record_bytes`,
   * `max_wire_bytes`, `min_wire_margin_bytes` — covers
   * `InputProtocolPort.FAIRNESS_DELAY_TICKS`/`HISTORY_ROWS`. */
  inputProtocolConstantsJson(): string;
  /** `InputProtocolPort.newGuest`: builds and validates a single-slot guest
   * bundle. `rowsJson` is a JSON array of `{"tick", "slot_index",
   * "sample"}` (`"sample"` an `inputFrame` canonical hex wire). Throws (a
   * string) if `rowsJson` fails to parse/decode, or the resulting packet
   * violates a structural invariant (this is how a forged bundle — the
   * wrong slot, a non-contiguous history — surfaces to a caller building
   * `net_diagnostics_fixture.ts`'s `forgedBundle`). */
  inputProtocolNewGuest(
    sessionId: string,
    manifestId: string,
    senderId: string,
    sequence: number,
    transportTick: number,
    firstInputTick: number,
    confirmedSpan: number | undefined,
    rowsJson: string,
  ): WasmInputPacket;
  /** The host's canonical, multi-slot authority batch. Not required by
   * `InputProtocolPort` (only `newGuest`/`encode` are declared there), but
   * the full module's other producing half. Same error behaviour as
   * {@link InputProtocolBridge.inputProtocolNewGuest}. */
  inputProtocolNewHost(
    sessionId: string,
    manifestId: string,
    senderId: string,
    sequence: number,
    transportTick: number,
    firstInputTick: number,
    confirmedSpan: number | undefined,
    rowsJson: string,
  ): WasmInputPacket;
  /** `InputProtocolPort.encode`: this packet's canonical wire bytes. Never
   * fails — a {@link WasmInputPacket} is always already valid. */
  inputProtocolEncode(packet: WasmInputPacket): Uint8Array;
  /** `gc_netcode::input_protocol::decode`: decodes and fully validates a
   * wire packet against its decode context. `wire` is the raw packet bytes.
   * Throws (a string) if `wire` fails to decode or does not match
   * `sessionId`/`manifestId`/`senderId`. */
  inputProtocolDecode(
    sessionId: string,
    manifestId: string,
    senderId: string,
    wire: Uint8Array,
  ): WasmInputPacket;
  /** `gc_netcode::input_protocol::packet_id`: the stable `fnv1a64` identity
   * a packet with this `kind`/`sessionId`/`senderId`/`sequence` must carry.
   * Throws (a string) if `kind` is not `"guest"`/`"host"`, or the other
   * fields are out of bounds. */
  inputProtocolPacketId(
    kind: "guest" | "host",
    sessionId: string,
    senderId: string,
    sequence: number,
  ): string;
  /** `gc_netcode::input_protocol::confirmed_tick`. */
  inputProtocolConfirmedTick(packet: WasmInputPacket): number;
  /** `gc_netcode::input_protocol::confirmed_span`. */
  inputProtocolConfirmedSpan(firstInputTick: number, confirmedTick: number): number;
  /** `gc_netcode::input_protocol::classify_duplicate`: `"idempotent"` for a
   * byte-identical resend. Throws (a string, a `packet_conflict`) if
   * `previous`/`incoming` share a sender/sequence identity with different
   * bytes, or do not share one at all. */
  inputProtocolClassifyDuplicate(previous: WasmInputPacket, incoming: WasmInputPacket): string;
  /** `gc_netcode::input_protocol::supersede_for_backpressure`. Throws (a
   * string, a `backpressure_gap`) if `newer` does not cover every row of
   * unsent authority `older` carried. */
  inputProtocolSupersedeForBackpressure(older: WasmInputPacket, newer: WasmInputPacket): WasmInputPacket;
}

// ---------------------------------------------------------------------------
// `match_driver_fixture_bridge.rs` — `gc_netcode::match_driver_fixture`,
// plus a `wasm-bindgen` wrapper over `gc_netcode::fake_star::FakeStarTransport`
// for the fixture's own in-process star.
// ---------------------------------------------------------------------------

/**
 * Mirrors `gc_wasm::match_driver_fixture_bridge::WasmFakeStarTransport` — a
 * real in-process star endpoint. See that Rust module's doc for how closely
 * this mirrors (and does not literally implement) `@gc/transport`'s
 * `StarTransportAdapter`: every fallible method here throws a `string`
 * rather than returning a `TransportResult<T>` discriminated union. A
 * caller wanting a real `StarTransportAdapter` needs a thin adapter over
 * this class.
 *
 * `send`/`broadcast`/`poll*` payloads are `Uint8Array` — see
 * `packages/wasm/src/binary_string.ts` for converting to/from
 * `@gc/transport`'s "binary string" convention.
 */
export interface WasmFakeStarTransport {
  initialize(): void;
  shutdown(): void;
  role(): "host" | "guest";
  capacity(): number;
  openPeer(peerId: string): number;
  closePeer(peerId: string, reason?: string): void;
  peerIds(): string[];
  peerState(peerId: string): string | undefined;
  /** Test seam: joins this (host) endpoint to `guest`'s already-open slot.
   * The fixture's own `matchDriverFixtureSession` already calls this
   * internally for every seated guest. */
  link(guest: WasmFakeStarTransport): void;
  /** Test seam: delivers everything currently buffered on this endpoint and
   * every endpoint linked to it. */
  pump(): void;
  requestOffer(peerId: string): void;
  acceptOffer(signal: string): void;
  acceptAnswer(peerId: string, signal: string): void;
  takeSignal(peerId: string): string | undefined;
  send(
    peerId: string,
    channel: "control" | "input",
    kind: "input" | "event" | "state",
    seq: number,
    tick: number | undefined,
    payload: Uint8Array,
  ): void;
  broadcast(
    channel: "control" | "input",
    kind: "input" | "event" | "state",
    seq: number,
    tick: number | undefined,
    payload: Uint8Array,
  ): number;
  /** Removes and returns up to one queued arrival, as JSON
   * (`crate::match_driver_bridge`'s `peer_message_to_json` shape), or
   * `null` if nothing is queued. */
  pollJson(): string;
  /** Removes and returns up to `limit` queued arrivals, as a JSON array,
   * oldest first. */
  pollBatchJson(limit?: number): string;
  /** Removes and returns up to one queued peer/star event, as JSON, or
   * `null` if nothing is queued. */
  pollEventJson(): string;
  state(): string;
  /** A full read of this endpoint's counters and every peer's, as JSON. */
  diagnosticsJson(): string;
  free(): void;
}

/**
 * Mirrors `gc_wasm::match_driver_fixture_bridge::WasmMatchDriverFixtureSession`
 * — `MatchDriverFixturePort.session`'s return.
 */
export interface WasmMatchDriverFixtureSession {
  /** This session's match mode wire string (`"1v1"`/`"2v2"`/`"4v4"`). */
  readonly mode: string;
  /** The frozen session manifest, as JSON. */
  manifestJson(): string;
  /** The frozen session state, as JSON — exactly what
   * {@link MatchDriverBridgeConstructor}'s `freezeJson` parameter expects. */
  freezeJson(): string;
  readonly hostPeerId: string;
  /** Every seated guest's peer id, in seating order. */
  guestPeerIds(): string[];
  /** The host endpoint of this session's in-process star. */
  hostTransport(): WasmFakeStarTransport;
  /** `peerId`'s guest endpoint, or `undefined` if `peerId` did not seat as
   * a guest in this session. */
  guestTransport(peerId: string): WasmFakeStarTransport | undefined;
  free(): void;
}

/** Every function `gc_wasm::match_driver_fixture_bridge` exports besides
 * `WasmFakeStarTransport`/`WasmMatchDriverFixtureSession` themselves. */
export interface MatchDriverFixtureBridge {
  /** This fixture's own constants, as JSON: `host_peer_id`, `countdown_id`,
   * `default_duration`, `default_seed`. */
  matchDriverFixtureConstantsJson(): string;
  matchDriverFixtureGuestPeerId(index: number): string;
  /** `MatchDriverFixturePort`'s `peer_ids` half: every peer id this session
   * seats, host first. Throws (a string) if `modeWire` is not a canonical
   * match mode. */
  matchDriverFixturePeerIds(modeWire: "1v1" | "2v2" | "4v4", humans?: number): string[];
  /** The frozen session state for `modeWire`, as JSON — paired with
   * {@link MatchDriverFixtureBridge.matchDriverFixtureManifestJson}, exactly
   * the `freezeJson`/`manifestJson` {@link MatchDriverBridgeConstructor}
   * requires, without constructing a full session. Throws (a string) if
   * `modeWire` is not a canonical match mode. */
  matchDriverFixtureFreezeJson(
    modeWire: "1v1" | "2v2" | "4v4",
    firstInputTick?: number,
    humans?: number,
  ): string;
  /** The frozen session manifest `matchDriverFixtureFreezeJson` is framed
   * against, as JSON. Throws (a string) if `modeWire` is not a canonical
   * match mode. */
  matchDriverFixtureManifestJson(modeWire: "1v1" | "2v2" | "4v4"): string;
  /** `MatchDriverFixturePort.initialSnapshot`: the pinned slot-mode
   * boundary zero, as an opaque handle. */
  matchDriverFixtureInitialSnapshot(
    duration?: number,
    combatActive?: boolean,
    seed?: number,
  ): WasmMatchSnapshot;
  /** `MatchDriverFixturePort.session`: builds the full connected fixture
   * session for `modeWire`. Throws (a string) if `modeWire` is not a
   * canonical match mode. */
  matchDriverFixtureSession(
    modeWire: "1v1" | "2v2" | "4v4",
    firstInputTick?: number,
    humans?: number,
  ): WasmMatchDriverFixtureSession;
}

/** The shape of `dist/pkg/gc_wasm.cjs`'s module exports. */
export interface GcWasmModule extends InputFrameBridge, InputProtocolBridge, MatchDriverFixtureBridge {
  readonly Session: SimSessionConstructor;
  readonly Coordinator: CoordinatorConstructor;
  readonly MatchDriverBridge: MatchDriverBridgeConstructor;
  readonly RollbackEventsTimeline: RollbackEventsTimelineConstructor;
  readonly FixedClock: FixedClockConstructor;
  readonly TuningRegistry: TuningRegistryConstructor;
  runDeterminismEvidence(): DeterminismEvidence;
  decodeControlMessageHeader(wire: string): ControlMessageHeader;
  protocolVocabularyId(): string;
  /** `gc_data::tuning_presets::ALL`, as {@link WasmTuningPreset}s, in panel
   * cycle order. */
  tuningPresets(): WasmTuningPreset[];
  readonly __wbg_raw: RawExports;
}
