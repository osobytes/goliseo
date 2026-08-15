// Hand-written interfaces mirroring `gc-wasm`'s generated bindings
// (`dist/pkg/gc_wasm.d.cts`, produced from `rust/crates/gc-wasm/src/**`
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
  /** This session's raw simulation state, as JSON —
   * `gc_wasm::match_state_bridge::match_state_to_json`'s shape, matching
   * `packages/render/src/replay.ts`'s `MatchState` interface field-for-field
   * (`field`, `goal_home`, `goal_away`, `score`, `time_left`, `press`,
   * `outfield_press`, `transition`, `transition_windows`, `controlled?`,
   * `owner?`, `ball`, `ball_vel`, `ball_z`, `ball_vz`, `players`, `events`).
   * `press` (`{home, away}`, tactic-derived chaser count,
   * `gc_sim::match::MatchState.press`) is not part of `replay.ts`'s own
   * `MatchState` interface (that file is free to widen its own type to read
   * it) — it was added for a caller that only needs a live session's current
   * `press.home`/`press.away`, e.g. `packages/app`'s real-match adapter.
   * Unlike {@link SimSession.snapshotHandle} (an opaque handle) this crosses
   * fully decoded — `replay.ts`'s `captureFrame` needs to actually read
   * `outfield_press`/`transition`/per-player timers, which
   * {@link SimHost.buildRenderFrame}'s presentation-derived `RenderFrame`
   * does not carry at all. */
  matchStateJson(): string;
  /** This session's most recently stepped tick's combat events, as a JSON
   * array -- `gc_wasm::rollback_events_bridge::raw_combat_event_to_json`'s
   * shape (field-for-field, no `"origin"` tag), the exact same shape
   * {@link MatchDriverBridge.advance}'s own batch already embeds per output
   * under `combat_events`, so decoding this needs no second parser. `"[]"`
   * when this session was built without `combatEnabled` ({@link
   * SimSessionConstructor}) -- no combat companion at all -- and also
   * before the first {@link SimSession.step} call. Reflects exactly the
   * tick {@link SimSession.step} most recently ran: like
   * `gc_sim::combat::step`'s own per-tick `events` field, this is REPLACED,
   * not accumulated, by every step -- read it before the next `step` call
   * if the caller needs every tick's events. This is the BASE (non-rollback)
   * counterpart of what {@link MatchDriverBridge}/
   * {@link OnlineCombatPhasesBridge} already had -- before this method
   * existed, a combat-enabled `SimSession` drove its companion every tick
   * with no way for a caller to observe what it did. */
  combatEventsJson(): string;
  /** Releases the underlying wasm-side registry slot. Call when done with
   * a session — the wasm module does not garbage-collect on its own. */
  free(): void;
}

/** Constructs a {@link SimSession}. `homeFormation` overrides the home
 * team's authored default formation (`gc_data::formations::ALL`'s ids, e.g.
 * `"2-1-1"`); omit to keep the team's own default. Not validated against
 * the authored formation table by this constructor itself -- see
 * `crates/gc-wasm/src/session.rs`'s `Session::new` doc for why that check
 * stays the caller's (e.g. a formation-select screen's) job.
 *
 * `combatEnabled` (omitted defaults to `false`) preserves this option's
 * original semantics -- `true` builds a combat companion once, at
 * construction, and every {@link SimSession.step}
 * call afterward threads it through; `false`/omitted never builds one, byte
 * for byte the behavior before this parameter existed. This was previously
 * missing from this interface even though the compiled artifact's
 * constructor already accepted it as a seventh, trailing optional argument
 * (`crates/gc-wasm/src/session.rs`'s `Session::new`).
 *
 * `tactic`/`awayTactic` (omit either to keep `"balanced"`, the authored
 * default) name an authored `gc_data::tactics::ALL` id (e.g.
 * `"press_high"`) for the home/away side respectively -- reaching
 * `sim_match::NewMatchOptions.tactic`/`.away_tactic` directly. Before these
 * parameters existed every session simulated `"balanced"` unconditionally
 * on both sides, so a request's tactic choice could never reach
 * `MatchState.press`. An id naming no authored tactic throws (a string).
 *
 * `homeStarterIds` (omit to keep `home.roster`, the authored default --
 * exactly the prior, only behavior) overrides the home team's starting XI:
 * five player ids, the keeper first and the four outfielders in formation
 * line order -- the same shape `gc_data::teams::TeamData::roster` itself
 * uses. Every id must be authored content, distinct, shaped
 * keeper-then-four-outfield, and not already on the away roster; squad
 * eligibility for a particular team is NOT checked here (a product policy a
 * calling screen enforces, matching `homeFormation`'s own "not validated
 * here" precedent). Throws (a string) on any violation -- never a wasm
 * panic. There is no `awayStarterIds`: the away side keeps its authored
 * roster unconditionally, the same scope `homeFormation` already draws (no
 * `awayFormation` parameter exists either). See
 * `crates/gc-wasm/src/session.rs`'s `Session::new` doc for the exact
 * validation. */
export interface SimSessionConstructor {
  new (
    homeTeamId: string,
    awayTeamId: string,
    seed: number,
    durationSeconds: number,
    maxGoals: number,
    homeFormation?: string,
    combatEnabled?: boolean,
    tactic?: string,
    awayTactic?: string,
    homeStarterIds?: string[],
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
  /**
   * The headline behaviors the campaign observed, comma-joined in
   * `goal_kickoff,tackle,aerial,keeper,full_time` order.
   *
   * Reported, never gating since issue #512 — surface it, do not compare it
   * to a pinned string. OMP-1's inputs are frozen button presses, so a 0.45%
   * locomotion change moves this set; #518 is the live-AI fixture meant to
   * gate the claim instead.
   */
  readonly coverage: string;
  /**
   * Every behavioral claim of the frozen recording this build no longer
   * reproduces: `"none"`, or `claim:recorded->observed` entries joined by
   * `;`, including `coverage.<behavior>` since #512. Reported, never gating —
   * but surface it. A demoted assertion that prints nothing is a deleted
   * assertion.
   */
  readonly behavioral_drift: string;
}

/**
 * Mirrors `gc_wasm::ai_driven::AiDrivenEvidence`
 * (`crates/gc-wasm/src/ai_driven.rs`) — the AI-driven counterpart of
 * `DeterminismEvidence` above. Same convention: field names are verbatim Rust
 * field names, since plain `#[wasm_bindgen]` struct fields are not
 * camelCase-renamed the way methods are.
 */
export interface AiDrivenEvidence {
  readonly fixture_id: string;
  readonly ticks: number;
  /** `ticks + 1` — tick 0 is digested before the first step. */
  readonly rows: number;
  readonly final_hash: string;
  readonly sequence_digest: string;
  readonly score_home: number;
  readonly score_away: number;
}

/**
 * Mirrors `gc_wasm::wasm_native_corpus_bridge::CorpusScenarioInfo`
 * (`crates/gc-wasm/src/wasm_native_corpus_bridge.rs`) — one
 * `gc_sim::wasm_native_corpus::CorpusScenario`, as returned by
 * {@link GcWasmModule.corpusScenarios}. See #517.
 */
export interface CorpusScenarioInfo {
  readonly id: string;
  readonly match_seed: number;
  readonly bot_seed: number;
  readonly ticks: number;
  readonly combat_enabled: boolean;
}

/**
 * Mirrors `gc_wasm::wasm_native_corpus_bridge::CorpusRunResult` — the result
 * of one {@link GcWasmModule.runCorpusScenario} call. `tick_hashes` has
 * `ticks + 1` entries (tick 0 through `ticks` inclusive), one independent
 * FNV-1a-64 hash per tick, so a caller comparing two runs of the same
 * scenario can find the first index they disagree at directly — see
 * `rust/crates/gc-sim/src/wasm_native_corpus.rs`'s module doc.
 */
export interface CorpusRunResult {
  readonly scenario_id: string;
  readonly ticks: number;
  readonly tick_hashes: string[];
  readonly final_hash: string;
  readonly sequence_digest: string;
  readonly score_home: number;
  readonly score_away: number;
  readonly covered_touch: boolean;
  readonly covered_shot: boolean;
  readonly covered_pass: boolean;
  readonly covered_reception: boolean;
  readonly covered_tackle: boolean;
  readonly covered_combat_commit: boolean;
  readonly covered_aerial: boolean;
  readonly covered_keeper_save: boolean;
  /** The final tick's ball state: `[x, y, vel_x, vel_y, z, vz]`. For
   * ATTRIBUTION: diff this (and `last_players`) between `ticks` and
   * `ticks - 1` calls, on both targets, to find which scalar first changed
   * at a known divergent tick. */
  readonly last_ball: number[];
  readonly last_owner: number;
  readonly last_rng: number;
  /** The final tick's player positions, flattened `[x0, y0, x1, y1, ...]`. */
  readonly last_players: number[];
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
 *
 * `driver_render_frame_ptr`/`driver_render_frame_len` are the read half of
 * {@link MatchDriverBridge.renderFrameBuild}'s own three-call contract — a
 * SEPARATE reused buffer from `render_frame_ptr`/`render_frame_len`'s own
 * (see `render_export.rs`'s module doc), so building a `SimSession`'s frame
 * and a `MatchDriverBridge`'s frame in the same tick cannot clobber one
 * another. There is no `driver_render_frame_build` free function here:
 * unlike `SimSession`, a `MatchDriverBridge` is not `crate::registry`-backed
 * (no `u32` handle a raw `extern "C"` function could take), so the trigger
 * is {@link MatchDriverBridge.renderFrameBuild} itself, an ordinary bound
 * method — see that method's doc.
 */
export interface RawExports {
  readonly memory: WebAssembly.Memory;
  /**
   * `kickFollowSlots` is the renderer's release follow-through window as a
   * roster-slot bitmask (`0` when nothing is following through) — a plain
   * scalar precisely so it can ride this path; see
   * `gc_wasm::session::kick_follow_ids` for why the ids themselves do not
   * cross. `releaseFollow.slotMask(roster.ids)` (`@gc/render`) produces it.
   */
  render_frame_build(handle: number, kickFollowSlots: number): number;
  render_frame_ptr(): number;
  render_frame_len(): number;
  driver_render_frame_ptr(): number;
  driver_render_frame_len(): number;
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
 * Everything one live peer retains, as
 * {@link MatchDriverBridge.retainedHistoryAccountingJson} reports it —
 * `gc_wasm::match_driver_bridge::retained_history_to_json`'s shape, from
 * `gc_sim::retained_history::sample`.
 *
 * Unlike this file's other `*Json` methods (which deliberately do not
 * duplicate a JSON shape in TypeScript), this one is declared: it is a
 * measurement a caller does arithmetic on — a soak comparing samples over an
 * hour of play — not a diagnostics blob it forwards.
 *
 * ## `historyBytes` is the COMBINED figure
 *
 * `history_bytes` is `session.total_bytes + events.total_bytes`, which is
 * what that name means everywhere else in this tree (`gc_sim::rollback_lab`'s
 * peak, `gc-sim`'s `tests/combat_load_fixtures.rs`, and the band
 * `tests/snapshot_headroom.rs` compares to
 * `Omp2RollbackBudgets::history_bytes`). The session half alone omits the
 * retained speculative event timeline — which this bridge retains per peer,
 * so it is real browser-side memory.
 *
 * ## Why the counts matter as much as the bytes
 *
 * A byte total is trivially satisfied by a window that has stopped
 * measuring, and — less obviously — by one retaining only scaffolding:
 * thirty speculative steps holding *no events* still report a plausible,
 * nonzero, budget-comparable figure in which no per-event encoder was ever
 * entered. Anything gating on `history_bytes` must reject a sample whose
 * `retained_event_count`/`retained_step_count`/`retained_boundary_count` say
 * the window is empty, rather than pass on the bytes alone.
 */
export interface RetainedHistoryAccounting {
  /** `session.total_bytes + events.total_bytes` — the whole retained
   * footprint. See this interface's doc. */
  readonly history_bytes: number;
  /** The rollback session's own retained accounting. */
  readonly session: {
    /** Retained input history, canonically encoded. */
    readonly input_bytes: number;
    /** Retained per-tick outputs (a diagnostic size proxy, per
     * `gc_sim::rollback_session`'s module doc). */
    readonly output_bytes: number;
    /** The snapshot ring's canonical bytes — the dominant term. */
    readonly snapshot_bytes: number;
    /** `input_bytes + output_bytes + snapshot_bytes`. */
    readonly total_bytes: number;
  };
  /** The retained speculative event timeline's accounting — the same shape
   * {@link MatchDriverBridge.rollbackAccountingJson} returns on its own. */
  readonly events: {
    readonly retained_step_bytes: number;
    readonly total_bytes: number;
  };
  /** Boundaries currently in the snapshot ring. */
  readonly retained_boundary_count: number;
  /** The ring's own high-water mark — so growth that happened *between* two
   * samples is still reported. */
  readonly peak_retained_boundary_count: number;
  /** High-water mark of `session.snapshot_bytes`, same between-samples
   * property. */
  readonly peak_snapshot_bytes: number;
  /** Oldest retained boundary tick; absent when the ring is empty. */
  readonly oldest_boundary_tick?: number;
  /** Newest boundary tick ever stored; absent when the ring is empty. */
  readonly latest_boundary_tick?: number;
  /** Retained speculative steps. */
  readonly retained_step_count: number;
  /** Retained events across those steps — the field that separates a window
   * holding real content from one holding empty step wrappers. */
  readonly retained_event_count: number;
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
  /** This boundary's exact canonical wire byte count
   * (`gc_sim::match_snapshot::encoded_size_canonical`, computed once when
   * the ring stored it), present exactly when
   * {@link SnapshotLookup.snapshot} is.
   *
   * This is the per-boundary figure the ring sums into
   * {@link RetainedHistoryAccounting}'s `session.snapshot_bytes`: summing it
   * across every retained boundary re-derives that field independently, so a
   * sampled retained-byte total is one a caller can check rather than take
   * on trust. */
  readonly canonicalBytes?: number;
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
  apply(
    replacedFromTick: number,
    replacedThroughTick: number,
    outputsJson: string,
    snapshots: WasmMatchSnapshot[],
  ): string;
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

// ---------------------------------------------------------------------------
// `rollback_playable_lab_bridge.rs` — the deterministic, network-simulated,
// bot-driven local rollback harness for the playable match screen
// (`gc_sim::rollback_playable_lab`). Distinct from `MatchDriverBridge`: that
// one binds an OMP-3 two-peer ONLINE driver; this one is the single-process
// local dev/playtest harness `packages/screens/src/match.ts`'s
// `RollbackHostPort` needs a real implementation of.
// ---------------------------------------------------------------------------

/**
 * Mirrors `gc_wasm::rollback_playable_lab_bridge::RollbackPlayableLabSnapshotLookup`
 * — one `gc_sim::rollback_playable_lab::snapshot` lookup result. Structurally
 * identical to {@link SnapshotLookup} (a separate type on the Rust side too —
 * see that module's doc for why).
 */
export interface RollbackPlayableLabSnapshotLookup {
  /** This boundary's retention status: `"present"`, `"retained"`,
   * `"missing"`, or `"outside_window"` — only the first two carry a
   * {@link RollbackPlayableLabSnapshotLookup.snapshot}. */
  readonly status: string;
  /** The queried boundary tick. */
  readonly tick: number;
  /** The retained snapshot, present exactly when
   * {@link RollbackPlayableLabSnapshotLookup.status} is `"present"` or
   * `"retained"`. */
  readonly snapshot?: WasmMatchSnapshot;
  free(): void;
}

/**
 * Mirrors `gc_wasm::rollback_playable_lab_bridge::RollbackPlayableLab`
 * (`crates/gc-wasm/src/rollback_playable_lab_bridge.rs`) — a `wasm-bindgen`
 * control surface over `gc_sim::rollback_playable_lab`. There is no public
 * constructor — build one via {@link RollbackPlayableLabConstructor.create},
 * not `new`.
 *
 * This is NOT `RollbackHostPort` (`packages/screens/src/match.ts`) itself —
 * it is the underlying primitive a real `RollbackHostPort` implementation
 * wraps. Every JSON-string method follows this package's usual rule: read
 * the exact shape from `rollback_playable_lab_bridge.rs`'s `*_json`
 * functions rather than a duplicated TypeScript interface.
 */
export interface RollbackPlayableLab {
  /** Whether {@link RollbackPlayableLab.advance} requires a `sampleWire`
   * this tick. Cheap; safe to call before the first `advance`. */
  needsLocalSample(): boolean;
  /** Advances exactly one transport tick. `transportTick` must equal the
   * tick this laboratory expects next (readable beforehand via
   * {@link RollbackPlayableLab.debugModelJson}'s `transport_tick`).
   * `sampleWire` (a canonical `gc_sim::input_frame` sample wire, matching
   * {@link SimSession.step}/{@link MatchDriverBridge.advance}'s own
   * `sampleWire`/`wire` convention) must be supplied exactly when
   * {@link RollbackPlayableLab.needsLocalSample} is `true` this tick, and
   * omitted otherwise. Returns the tick's batch as JSON: `outputs` (one
   * `gc_sim::rollback_session::RollbackTickOutput` per entry — the same
   * shape {@link MatchDriverBridge.advance}'s own `outputs` embeds, plus an
   * `input` field that one omits — `{tick, start_boundary, end_boundary,
   * finished, score, time_left, events, combat_events, input: {tick, slots:
   * [{source, status, sample}, ...]}}`), `event_diffs`
   * ({@link RollbackEventsTimeline.apply}'s `RollbackEventDiff` shape),
   * `confirmed_steps` ({@link RollbackEventsTimeline.confirm}'s
   * `RollbackEventStep` shape), `corrections` (`{causal_tick,
   * restored_boundary, old_present_boundary, new_present_boundary,
   * replaced_from_tick, replaced_through_tick, corrected_from_tick,
   * corrected_through_tick, old_present_hash?, new_present_hash?,
   * first_difference?}`), and `status` (this laboratory's status after this
   * call — `"active"`, `"settling"`, `"converged"`, `"diverged"`,
   * `"late_input_unrecoverable"`, `"unconfirmed_window_exceeded"`,
   * `"comparison_history_missing"`, or `"drain_incomplete"`). Throws (a
   * string) if `transportTick` is not the expected next tick, if
   * `sampleWire` fails to decode, or if `sampleWire`'s presence disagrees
   * with {@link RollbackPlayableLab.needsLocalSample}. */
  advance(transportTick: number, sampleWire?: string): string;
  /** This laboratory's retained-state shape, as JSON: `profile`,
   * `local_slot`, `status`, `reference_tick`, `current_tick`,
   * `transport_tick`, `confirmed_input_tick`, `confirmed_output_tick`,
   * `predicted_slot_samples`, `predicted_ticks`, `correction_count`,
   * `rollback_count`, `latest_rollback_depth`, `max_rollback_depth`,
   * `resimulated_ticks`, `retained_snapshot_count`,
   * `retained_snapshot_bytes`, `network_pending`, `network_counters`,
   * `network_high_water`, `convergence` (`{status, boundary, expected_hash,
   * actual_hash, first_difference?}`), `event_status`,
   * `predicted_early_finish`, `late_input_tick?`, `settlement_ticks`,
   * `settlement_limit` — mirrors `gc_sim::rollback_playable_lab::debug_model`. */
  debugModelJson(): string;
  /** An independent capture of the predicted client's current live
   * boundary, as an opaque handle — see {@link WasmMatchSnapshot}'s doc. */
  currentSnapshot(): WasmMatchSnapshot;
  /** An independent capture of the reference's current live boundary, as an
   * opaque handle. */
  referenceSnapshot(): WasmMatchSnapshot;
  /** Looks up the predicted client's own retained boundary-snapshot history
   * at `boundaryTick`. */
  snapshotLookup(boundaryTick: number): RollbackPlayableLabSnapshotLookup;
  /** The predicted client's current live boundary's RAW simulation state,
   * as JSON — {@link SimSession.matchStateJson}'s shape, for a caller
   * feeding `packages/render/src/replay.ts` (or any other raw-`MatchState`
   * consumer) during rollback-mode play. */
  currentMatchStateJson(): string;
  /** The independent reference's current live boundary's RAW simulation
   * state, as JSON — see {@link RollbackPlayableLab.currentMatchStateJson}. */
  referenceMatchStateJson(): string;
  /** A retained boundary's RAW simulation state, as JSON — see
   * {@link RollbackPlayableLab.currentMatchStateJson}. Use
   * {@link RollbackPlayableLab.snapshotLookup} first to confirm
   * `boundaryTick` is `"present"`/`"retained"`. Throws (a string) if
   * `boundaryTick` is not currently retained. */
  matchStateJsonAt(boundaryTick: number): string;
  free(): void;
}

/** Builds a {@link RollbackPlayableLab}. Named `create`, not `new` — mirrors
 * {@link RollbackEventsTimelineConstructor.create}'s own naming rationale
 * (`new` is JS's own reserved constructor call). `initialSnapshot` must be
 * the canonical slot-mode boundary-zero snapshot (e.g.
 * {@link SimSession.snapshotHandle}, taken before the session's first
 * {@link SimSession.step}). `optionsJson` is a JSON object: `local_slot`
 * (1..=8, default 1), `profile_name` (default `"playable"`; recognized
 * names: `"clean"`, `"omp0_parity"`, `"playable"`, `"stress"`),
 * `network_seed`, `bot_seed`, `max_rollback_ticks` (1..=30),
 * `settlement_ticks` (>=1) — every field optional, defaulting exactly the
 * way `gc_sim::rollback_playable_lab::new` does. Throws (a string) if
 * `optionsJson` fails to parse, or a field violates its bound. Still panics
 * (not a throw) on a structurally invalid `initialSnapshot` (not slot mode,
 * not boundary zero, already finished, or an unmapped local slot) — that
 * handle is trusted, Rust-originated data; see the Rust module's doc. */
export interface RollbackPlayableLabConstructor {
  create(initialSnapshot: WasmMatchSnapshot, optionsJson: string): RollbackPlayableLab;
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
  /** Mirrors `gc_netcode::match_driver::observe_checkpoint`: compares `hash`
   * against this driver's own published checkpoint hash at `tick`, if it
   * has one. Returns `true` when they agree (or this driver never
   * published a checkpoint at `tick`), `false` on a genuine one-off
   * disagreement. Every consecutive disagreement is counted internally
   * ({@link MatchDriverBridge.diagnosticsJson}'s `hash_mismatches`); enough
   * consecutive disagreements terminate the driver from inside this call
   * (`status` becomes `"hash_mismatch"`) rather than the caller having to
   * act on the return value separately. */
  observeCheckpoint(tick: number, hash: string): boolean;
  statusJson(): string;
  terminalJson(): string;
  diagnosticsJson(): string;
  /** A full read of this bridge's own internal transport -- star and
   * per-peer channel counters (`sent`/`received`/`dropped_outbound`/
   * `dropped_inbound`/`queue_limit`/...), as JSON
   * (`gc_wasm::match_driver_fixture_bridge::star_diagnostics_to_json`'s
   * shape -- the same `TransportStarDiagnostics` mapping
   * {@link WasmFakeStarTransport.diagnosticsJson} already exposes for the
   * fixture's in-process star). Before this method existed nothing on this
   * bridge's public surface could report this data at all -- a
   * `DiagnosticTransport`-style tap could never observe a
   * `MatchDriverBridge`'s own traffic. */
  transportDiagnosticsJson(): string;
  rollbackDiagnosticsJson(): string;
  /** The rollback-event timeline's retained byte accounting, as JSON — ONE
   * HALF of what this peer retains. For the figure a retention budget is
   * written against, use
   * {@link MatchDriverBridge.retainedHistoryAccountingJson}. */
  rollbackAccountingJson(): string;
  /** **Everything this peer retains**, as JSON — parse as
   * {@link RetainedHistoryAccounting}. The seam a long-duration soak
   * samples: the combined `history_bytes` figure plus the components and
   * occupancy counts behind it.
   *
   * Before this existed, nothing on this bridge reported the session half at
   * all ({@link MatchDriverBridge.rollbackAccountingJson} is the event
   * timeline only), so a browser peer could not observe its own
   * retained-history growth — the snapshot ring, input history and retained
   * outputs, together the large majority of the number, were invisible from
   * JS. `performance.memory` is not an alternative: it excludes wasm linear
   * memory, which never shrinks anyway.
   *
   * Cost: O(retained input rows + retained outputs + retained events), with
   * the snapshot component read from a counter the ring maintains
   * incrementally — no snapshot is re-encoded. On a full 31-boundary ring
   * with a full 30-step window: 0.27 ms and 0.38 ms per call in native
   * release builds on two different machines, and 0.37-0.42 ms through wasm
   * under node on both. Those are observations on the machines that took them, not
   * a bound — sub-millisecond is the claim, and neither figure is a budget
   * to assert against. Cheap enough to sample every tick for hours; still
   * not something to call per entity. */
  retainedHistoryAccountingJson(): string;
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
  /**
   * Builds this driver's CURRENT render frame (the live boundary
   * {@link MatchDriverBridge.advance} most recently produced) into a reused
   * thread-local buffer — read back via
   * {@link SimHost.buildMatchDriverRenderFrame}, the `MatchDriverBridge`
   * counterpart of {@link SimHost.buildRenderFrame}. Always returns `1`
   * (kept for interface symmetry with `buildRenderFrame`'s success/failure
   * convention -- a live `MatchDriverBridge` instance can never name a freed
   * or nonexistent driver the way a stale `SimSession.handle` could).
   *
   * Call {@link SimHost.buildMatchDriverRenderFrame} rather than this method
   * directly -- it wraps this call together with the raw ptr/len read into
   * one `Float64Array` view, mirroring `buildRenderFrame`'s own shape.
   *
   * `kickFollowSlots`: the renderer's release follow-through window as a
   * roster-slot bitmask, the same scalar the raw
   * `RawExports.render_frame_build` takes. `0` when no window is open.
   */
  renderFrameBuild(kickFollowSlots: number): number;
  /** Match-constant per-player roster fields, as a flat array -- the
   * `MatchDriverBridge` counterpart of {@link SimSession.rosterNumeric},
   * over this bridge's own roster (built once in its constructor, the same
   * match-constant identity {@link MatchDriverBridge.renderFrameBuild}
   * already reuses every frame). Before this method existed
   * `MatchDriverBridge` had no roster export of its own at all, so an
   * online renderer decoding a live driver frame had no honest way to
   * recover `species_shape`/`radius`/`is_keeper` per slot. */
  rosterNumeric(): Float64Array;
  /** Match-constant roster ids and display names, newline-joined -- the
   * string counterpart of {@link MatchDriverBridge.rosterNumeric}. */
  rosterIdsAndNames(): string;
  /** Releases this driver's wasm-side memory. A driver retains a full
   * 31-boundary snapshot ring (hundreds of kilobytes), and wasm linear
   * memory never shrinks, so a harness building peers repeatedly should free
   * them. */
  free(): void;
}

/** Constructs a {@link MatchDriverBridge}. `session` must be a freshly
 * constructed {@link SimSession} (boundary-zero, never stepped) — its
 * snapshot becomes the driver's `initial_snapshot`, *unless*
 * `initialSnapshotOverride` is given (see below), in which case `session`
 * is still required by this signature but its own snapshot goes unused; any
 * freshly-constructed session for an authored team pair satisfies it.
 * `freezeJson`/`manifestJson` are JSON encodings of
 * `gc_netcode::coordinator::Freeze`/`gc_netcode::protocol::Value` —
 * typically exactly what a {@link Coordinator}'s `start_match` action /
 * `stateJson()` produced. `queueLimit` bounds this bridge's own internal
 * transport's inbound queue (defaults to 64,
 * `crates/gc-wasm/src/wasm_transport.rs`'s `DEFAULT_QUEUE_LIMIT`, same as
 * every other `WasmStarTransport` caller) -- raise it when a test needs to
 * hold open a delivery gap wider than the default queue can buffer without
 * the queue itself overflowing first (see
 * `crates/gc-wasm/src/match_driver_bridge.rs`'s `MatchDriverBridge::new`
 * doc for the failure this closes: a burst wide enough to exceed the
 * rollback window used to overflow the fixed 64-message queue before the
 * window-exceeded condition could ever be reached).
 *
 * `initialSnapshotOverride`, when given, replaces `session`'s own captured
 * snapshot as this driver's boundary zero -- e.g. one of
 * {@link OnlineCombatPhasesBridge.onlineCombatPhaseBoundaryZero}'s seven
 * pinned combat-phase snapshots. Consumed by value (the JS-side handle is no
 * longer usable after this call returns, same as passing it to any other
 * wasm-bindgen constructor that takes ownership): every peer sharing one
 * session must be built from the *same* boundary zero (a differing one is a
 * desync fixture, not a correction fixture), which for a caller with more
 * than one peer means calling the snapshot's own builder again rather than
 * reusing one handle --
 * {@link OnlineCombatPhasesBridge.onlineCombatPhaseBoundaryZero} is a pure,
 * deterministic function of `(id, duration)`, so two calls with the same
 * arguments produce byte-identical snapshots. Caution: passing an
 * already-consumed handle a second time does not throw -- it silently falls
 * back to `session`'s own snapshot instead (confirmed against the compiled
 * artifact) -- so reusing a handle across two constructor calls is a silent
 * bug, not a loud one.
 *
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
    queueLimit?: number,
    initialSnapshotOverride?: WasmMatchSnapshot,
  ): MatchDriverBridge;
}

/**
 * Mirrors `gc_wasm::session::FixedClock` (`crates/gc-wasm/src/session.rs`)
 * -- the render-driven tick-count decision (`gc_sim::fixed_clock::advance`'s
 * accumulator/catch-up/drop policy, ARCHITECTURE.md §1.1), planned once per
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
  inputProtocolSupersedeForBackpressure(
    older: WasmInputPacket,
    newer: WasmInputPacket,
  ): WasmInputPacket;
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

// ---------------------------------------------------------------------------
// `online_combat_phases_bridge.rs` — the driver-level combat-phase fixture,
// a `#[wasm_bindgen]` surface over the seven pinned combat-phase boundary
// zeroes `crates/gc-netcode/tests/support/online_combat_phases.rs` already
// proves out.
// `packages/online/src/match_presentation.spec.ts`'s nine blocked
// combat-phase cases need exactly this: pick a phase, build its pinned
// boundary-zero snapshot, drive a real `MatchDriverBridge` from it, script
// the live slot's input, and ask whether a resimulated tick passed through
// the named phase.
// ---------------------------------------------------------------------------

/** One `onlineCombatPhaseScenarioJson(id)` result, parsed. */
export interface OnlineCombatPhaseScenario {
  readonly id: string;
  /** `"policy"` (the phase is produced by the shipped AI policy from
   * nothing but the opening pose) or `"canonical_input"` (produced by the
   * live slot's own equipment input, carried on the canonical input
   * stream — only `"guard"` uses this route). */
  readonly route: "policy" | "canonical_input";
  /** Driver steps the scenario needs to reach the phase. */
  readonly steps: number;
  /** Deliver every Nth step; the burst that corrects. */
  readonly deliver_period: number;
  /** Whether {@link OnlineCombatPhasesBridge.onlineCombatPhaseLiveSample}
   * authors anything for this phase (only `"guard"`). */
  readonly hold_equipment: boolean;
}

/** Every function `gc_wasm::online_combat_phases_bridge` exports. */
export interface OnlineCombatPhasesBridge {
  /** Every canonical online combat phase id, in canonical order —
   * `"windup"`, `"guard"`, `"contact"`, `"projectile_flight"`, `"stagger"`,
   * `"ball_spill"`, `"immunity_expiry"`. */
  onlineCombatPhaseIds(): string[];
  /** One phase's scenario metadata, as JSON
   * ({@link OnlineCombatPhaseScenario}'s shape). Throws (a string) if `id`
   * is not one of {@link OnlineCombatPhasesBridge.onlineCombatPhaseIds}'s
   * ids. */
  onlineCombatPhaseScenarioJson(id: string): string;
  /** The shared, combat-active boundary zero for one phase, as an opaque
   * handle (never JSON — see {@link WasmMatchSnapshot}'s doc). Every peer in
   * a session must be built from the *same* snapshot: a differing one is a
   * desync fixture, not a correction fixture. `duration` is match seconds;
   * omit for the scenario's own default. Throws (a string) if `id` is not
   * one of {@link OnlineCombatPhasesBridge.onlineCombatPhaseIds}'s ids. */
  onlineCombatPhaseBoundaryZero(id: string, duration?: number): WasmMatchSnapshot;
  /** The live slot's input for one driver step, as a canonical
   * `gc_sim::input_frame` sample wire. Driver index 1 is the host, whose
   * opening live slot is `home_1`; every other index gets a neutral sample.
   * `step` is the zero-based driver step. Only the `"guard"` scenario
   * authors anything (see {@link OnlineCombatPhaseScenario.hold_equipment}).
   * Throws (a string) if `id` is not one of
   * {@link OnlineCombatPhasesBridge.onlineCombatPhaseIds}'s ids. */
  onlineCombatPhaseLiveSample(id: string, step: number, index: number): string;
  /** Did a resimulated input tick run through phase `id`? `before`/`after`
   * are this driver's own retained boundary snapshots (typically
   * `MatchDriverBridge.snapshotLookup(tick)`/`.snapshotLookup(tick + 1)`'s
   * `.snapshot`) for the boundary before/after the tick being asked about;
   * `combatEventsJson` is that same tick's combat events, in the exact
   * shape `MatchDriverBridge.advance()`'s batch already embeds per output
   * under `combat_events` — pass it straight through, no re-shaping needed.
   * Throws (a string) if `id` is not one of
   * {@link OnlineCombatPhasesBridge.onlineCombatPhaseIds}'s ids,
   * `combatEventsJson` fails to parse/decode, or `before`/`after` are not
   * combat-bearing snapshots. */
  onlineCombatPhaseObserved(
    id: string,
    before: WasmMatchSnapshot,
    after: WasmMatchSnapshot,
    combatEventsJson: string,
  ): boolean;
}

// ---------------------------------------------------------------------------
// `match_snapshot_bridge.rs` — general-purpose construction of a
// `WasmMatchSnapshot` from caller-specified fields. Unlike
// `OnlineCombatPhasesBridge.onlineCombatPhaseBoundaryZero` (one of seven
// pinned fixtures), this builds an arbitrary hand-specified state: a fresh
// `gc_sim::r#match::new` construction (the same one `new Session(...)` uses)
// plus a small, explicit JSON override set.
// ---------------------------------------------------------------------------

/** Mirrors `gc_wasm::match_snapshot_bridge` (`crates/gc-wasm/src/match_snapshot_bridge.rs`). */
export interface MatchSnapshotBridge {
  /**
   * Builds a fresh, slot-mode `MatchState` exactly like `new Session(...)`
   * does (`homeTeamId`/`awayTeamId` from `gc_data::teams::ALL`, a
   * five-player roster required), applies `overridesJson` on top of it, and
   * captures the result as an opaque {@link WasmMatchSnapshot} handle.
   *
   * `overridesJson` (omitted captures the freshly-built state unchanged) is
   * a JSON object, every key optional and independently applied on top of
   * the fresh construction:
   *
   * - `owner`: integer (one-based player index) or `null` to clear.
   * - `time_left`, `input_tick`, `ball_z`, `ball_vz`, `pickup_cd`,
   *   `block_grace`: plain numbers.
   * - `ball`, `ball_vel`: `{x: number, y: number}`.
   * - `events`: replaces `MatchState.events` wholesale (never merged) —
   *   `SimSession.matchStateJson()`'s own `"events"` array shape, one entry
   *   per `{kind, x, y, player?, ...}`.
   * - `players`: an array of `{index, pos?, run_vel?, facing?, vel?}` —
   *   one-based `index` into `MatchState.players` (matching
   *   `MatchState.owner`'s own one-based convention); only the listed
   *   fields on that player are overridden, and an unlisted player is
   *   untouched.
   *
   * Two calls with identical arguments (including `overridesJson`) produce
   * a byte-identical snapshot — the same guarantee
   * {@link OnlineCombatPhasesBridge.onlineCombatPhaseBoundaryZero} gives,
   * relied on when seeding more than one peer from a hand-built snapshot.
   *
   * No combat companion: always captures with `combat_state: None`.
   *
   * Throws (a string) if either team id is not authored content, either
   * roster is not five players, `overridesJson` fails to parse, or an
   * override is malformed (wrong type, an out-of-range player `index`, or
   * an `events` entry that fails to decode).
   */
  matchSnapshotBuild(
    homeTeamId: string,
    awayTeamId: string,
    seed: number,
    durationSeconds: number,
    maxGoals: number,
    homeFormation?: string,
    overridesJson?: string,
  ): WasmMatchSnapshot;
  /**
   * `snapshot`'s underlying `MatchState`, as JSON — the exact same shape
   * {@link SimSession.matchStateJson} returns. The read-back half of
   * {@link MatchSnapshotBridge.matchSnapshotBuild}: for a caller that needs
   * to inspect a built snapshot's fields (e.g. a freshly-built player's
   * `id`, or `time_left` before nudging it by one tick) rather than only
   * ever writing to one.
   */
  matchSnapshotStateJson(snapshot: WasmMatchSnapshot): string;
}

// ---------------------------------------------------------------------------
// `player_pose_bridge.rs` — standalone `gc_render::player_pose::select`,
// over any caller-supplied `MatchPlayer`-shaped JSON, not only a live
// `SimSession`'s current tick (that path only ever reaches JS baked into a
// `RenderFrame`'s `pose_id`/`pose_priority`/`pose_source` fields, via
// `SimHost.buildRenderFrame`).
// ---------------------------------------------------------------------------

/** Mirrors `gc_wasm::player_pose_bridge` (`crates/gc-wasm/src/player_pose_bridge.rs`). */
export interface PlayerPoseBridge {
  /**
   * Selects the pose to display for an arbitrary buffered player — e.g. a
   * `packages/render/src/replay.ts` goal-replay frame, which records raw
   * `MatchState`/`MatchPlayer` snapshots rather than a decoded
   * `RenderFrame`. `inputJson`:
   *
   * ```jsonc
   * {
   *   // Required: the same per-player shape `SimSession.matchStateJson()`'s
   *   // "players" array embeds -- `packages/render/src/replay.ts`'s
   *   // `MatchPlayer` interface, verbatim.
   *   "player": { "id": "nebula_02", "is_keeper": false, /* ... *\/ },
   *   // All three optional/omittable, mirroring `player_pose::select`'s
   *   // own optional parameters -- omit exactly when the caller has none.
   *   "combat": { "phase": "guard", "forced_state": "stagger", "forced_ticks": 4 },
   *   "keeper_context": { "near_ball": true, "shuffling": false, "tip": false },
   *   "outfield_context": { "now": 12.5, "containing": false, "kick_follow": true }
   * }
   * ```
   *
   * Returns the selection as JSON: `{"id": "run_telegraph", "priority": 35,
   * "source": "soccer"}` — `id` is `gc_render::player_pose::PlayerPoseId`'s
   * wire name, `source` is `"soccer"` | `"combat"` | `"locomotion"`.
   *
   * Throws (a string) if `inputJson` fails to parse, is missing
   * `"player"`, or any field fails to decode against its closed wire
   * vocabulary (an unrecognized team/keeper-state/save-style/aerial-style/
   * aerial-outcome/outfield-intent/outfield-decision-context/combat-phase/
   * combat-forced-state).
   */
  playerPoseSelect(inputJson: string): string;
}

/** The shape of `dist/pkg/gc_wasm.cjs`'s module exports. */
export interface GcWasmModule
  extends
    InputFrameBridge,
    InputProtocolBridge,
    MatchDriverFixtureBridge,
    MatchSnapshotBridge,
    OnlineCombatPhasesBridge,
    PlayerPoseBridge {
  readonly Session: SimSessionConstructor;
  readonly Coordinator: CoordinatorConstructor;
  readonly MatchDriverBridge: MatchDriverBridgeConstructor;
  readonly RollbackEventsTimeline: RollbackEventsTimelineConstructor;
  readonly RollbackPlayableLab: RollbackPlayableLabConstructor;
  readonly FixedClock: FixedClockConstructor;
  readonly TuningRegistry: TuningRegistryConstructor;
  runDeterminismEvidence(): DeterminismEvidence;
  runAiDrivenEvidence(): AiDrivenEvidence;
  runAiDrivenEvidenceTo(ticks: number): AiDrivenEvidence;
  /** `gc_sim::wasm_native_corpus::CORPUS`'s own scenario list — the single
   * source of truth for #517's seeded native-vs-wasm differential corpus, so
   * a caller drives the SAME scenarios the native side defines rather than
   * maintaining a second, driftable copy. See
   * `scripts/check_wasm_native_corpus.mjs`. */
  corpusScenarios(): CorpusScenarioInfo[];
  /** Runs one #517 corpus scenario inside this compiled wasm module. See
   * {@link CorpusRunResult}. */
  runCorpusScenario(
    id: string,
    matchSeed: number,
    botSeed: number,
    ticks: number,
    combatEnabled: boolean,
  ): CorpusRunResult;
  decodeControlMessageHeader(wire: string): ControlMessageHeader;
  protocolVocabularyId(): string;
  /** `gc_data::tuning_presets::ALL`, as {@link WasmTuningPreset}s, in panel
   * cycle order. */
  tuningPresets(): WasmTuningPreset[];
  readonly __wbg_raw: RawExports;
}
