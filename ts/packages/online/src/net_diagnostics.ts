// Ported from game/online/net_diagnostics.lua.
//
// Bounded, privacy-safe OMP-3 network diagnostics.
//
// This module is a *recorder*, not an instrument. It never reaches into the
// match driver, the coordinator, or a transport: callers hand it the values
// those modules already publish, and it shapes, bounds, validates, and
// exports them. That is deliberate -- the driver and the star are settled,
// and a diagnostic that can perturb the thing it measures is not a
// diagnostic.
//
// ## The one distinction this module exists to keep
//
// Canonical simulation evidence and wall-clock runtime observation are
// stored in separate sections of the schema, and `diagnostics_schema`
// refuses at shape-construction time to let either borrow the other's
// vocabulary. In prose:
//
// * **Canonical** is what the simulation says: input ticks, boundaries,
//   hashes, rollback depth, resimulated ticks, the retained floor, the first
//   differing path. Two peers that agree here agree about the match. It is
//   reproducible.
// * **Runtime** is what the machine and the wire did while that happened:
//   RTT, jitter, queue depth, `bufferedAmount`, ICE transitions, drops. It is
//   a measurement. It is *not* reproducible and is never simulation truth.
// * **Anchors** are the only bridge, and each one carries its own mapping
//   error.
//
// ## Privacy
//
// Nothing here stores SDP, ICE credentials, raw addresses, clipboard
// contents, direct identifiers, or any playtest payload. Signalling blobs
// are recorded as a byte length and an explicit redaction marker and never
// as content -- see `recordSignal`. Every id is validated against a grammar
// whose charset excludes `@`, `:`, `/`, and `\`, so an email address, a URL,
// a path, and an IPv6 literal cannot be stored as an id even by mistake, and
// a dotted quad is rejected by an explicit check on top of that.
//
// Collection is always bounded and in memory. Export is opt-in and off by
// default: holding diagnostics is cheap and local, writing them somewhere a
// human might paste them is a decision.
//
// ## Boundary note: `game.online.protocol`
//
// The Lua original has exactly one call into Rust-owned code:
// `protocol.decode(payload)` inside `record_control`, to recover a session
// control message's `kind`/`sequence`/`message_id` from its wire payload.
// `game.online.protocol` is `crates/gc-netcode` (v2/README.md §2.1), so it
// cannot be imported here. The decoder is threaded through as an explicit
// `decodeControlMessage` field on `NetDiagnosticsOptions` instead -- the
// same injection pattern `@gc/presentation/src/combat.ts` uses for content
// tables, applied to a function rather than data.

import { type Result, ok, err } from "@gc/core";
import type {
  TransportChannelDiagnostics,
  TransportPeerMessage,
  TransportPeerState,
  TransportRole,
  TransportStarDiagnostics,
  TransportState,
} from "@gc/transport";
import { MAX_GUESTS } from "@gc/transport";
import * as schema from "./diagnostics_schema.ts";
import type { DiagnosticsField, DiagnosticsShape } from "./diagnostics_schema.ts";

// ---------------------------------------------------------------------------
// Closed vocabularies owned elsewhere (declared locally; see combat.ts for
// the pattern this follows)
// ---------------------------------------------------------------------------

/** `game.online.protocol`'s `SessionRole` (Rust-owned; `crates/gc-netcode`). */
export type SessionRole = "host" | "guest";

/** `game.online.match_manifest`'s `SessionMatchMode` (Rust-owned). */
export type MatchMode = "1v1" | "2v2" | "4v4";

/** `game.online.match_manifest`'s `CombatMatchStatus` (Rust-owned). */
export type CombatMatchStatus = "provisional_114" | "accepted_proceed" | "accepted_revision";

/** `game.online.match_driver`'s `MatchDriverStatus` (Rust-owned; `crates/gc-netcode`). */
export type MatchDriverStatus =
  | "active"
  | "completed"
  | "settle_timeout"
  | "confirmation_stalled"
  | "late_input"
  | "hash_mismatch"
  | "ownership_violation"
  | "authority_conflict"
  | "input_channel_failure"
  | "transport_lost";

/** `game.online.coordinator`'s `CoordinatorNetcodeFailure` (Rust-owned). */
export type CoordinatorNetcodeFailure = "input_channel" | "late_input" | "desync";

/** `game.online.protocol`'s `SessionMessageKind` (Rust-owned). */
export type SessionMessageKind =
  | "handshake"
  | "manifest_proposal"
  | "manifest_accept"
  | "peer_assignment"
  | "slot_assignment"
  | "ready"
  | "pair_preference"
  | "pair_preference_result"
  | "countdown"
  | "start"
  | "match_phase"
  | "hash_report"
  | "result_ack"
  | "abort"
  | "disconnect";

export type NetDiagnosticsRetention = "complete" | "truncated";
type NetDiagnosticsKeep = "newest" | "oldest";
export type NetDiagnosticsSignalDirection = "outbound" | "inbound";
export type NetDiagnosticsPacketDisposition =
  | "authored"
  | "sent"
  | "arrived"
  | "deferred"
  | "duplicate"
  | "rejected";
export type NetDiagnosticsEventKind =
  | "peer_state"
  | "peer_error"
  | "star_state"
  | "star_error"
  | "signal"
  | "lifecycle"
  | "teardown";

// ---------------------------------------------------------------------------
// Cross-layer data this module only reads (Rust-owned; only the fields this
// module actually touches are declared -- see combat.ts for the pattern)
// ---------------------------------------------------------------------------

/** `game.online.match_manifest`'s `SessionManifest` (Rust-owned). */
export interface SessionManifest {
  readonly session_id: string;
  readonly match_mode: MatchMode;
  readonly combat_status: CombatMatchStatus;
  readonly build_id: string;
  readonly source_id: string;
  readonly content_id: string;
  readonly tuning_id: string;
  readonly match_config_id: string;
  readonly fixture_id: string;
  readonly arena_id: string;
  readonly combat_rules_id: string;
  readonly gameplay_ai_policy_id: string;
  readonly protocol_version: number;
  readonly input_version: number;
  readonly snapshot_version: number;
  readonly tape_version: number;
  readonly combat_schema_version: number;
  readonly seed: number;
  readonly tick_rate: number;
  readonly duration_ticks: number;
  readonly max_goals: number;
}

/** `game.online.coordinator`'s `CoordinatorFreeze` (Rust-owned). */
export interface CoordinatorFreeze {
  readonly manifest_id: string;
  readonly assignment_id: string;
  readonly countdown_id: string;
  readonly first_input_tick: number;
}

/** `game.online.match_driver`'s `MatchDriverTerminal` (Rust-owned). */
export interface MatchDriverTerminal {
  readonly status: MatchDriverStatus;
  readonly failure?: CoordinatorNetcodeFailure;
  readonly detail: string;
  readonly tick?: number;
}

/** `game.online.match_driver`'s `MatchDriverDiagnostics` (Rust-owned). */
export interface MatchDriverDiagnostics {
  readonly status: MatchDriverStatus;
  readonly present_input_tick: number;
  readonly confirmed_input_tick: number;
  readonly confirmed_output_tick: number;
  readonly retained_floor_tick: number;
  readonly rollback_count: number;
  readonly correction_count: number;
  readonly predicted_slot_samples: number;
  readonly max_rollback_depth: number;
  readonly hash_mismatches: number;
  readonly checkpoint_count: number;
  readonly late_input_tick?: number;
  readonly control_slot?: string;
  readonly owned: readonly string[];
  readonly authored: readonly string[];
  readonly terminal?: MatchDriverTerminal;
}

/** A single free-form event field's value, as `sim/*.lua` emits it. */
export type DiagnosticEventFieldValue = string | number | boolean;

/**
 * `sim.match.MatchEvent`/`sim.combat.CombatEvent` (Rust-owned). This module
 * only ever reads a fixed, named subset of fields generically (see
 * `MATCH_EVENT_FIELDS`/`COMBAT_EVENT_FIELDS` below) to build an identity
 * digest, so the full shape -- which belongs to `crates/gc-sim` -- is left
 * open rather than duplicated.
 */
export type DiagnosticEvent = Readonly<Record<string, DiagnosticEventFieldValue | undefined>>;

/** `game.online.match_driver`'s `RollbackTickOutput` (Rust-owned). */
export interface RollbackTickOutput {
  readonly tick: number;
  readonly events: readonly DiagnosticEvent[];
  readonly combat_events?: readonly DiagnosticEvent[];
}

/** `game.online.match_driver`'s `MatchDriverCheckpoint` (Rust-owned). */
export interface MatchDriverCheckpoint {
  readonly tick: number;
  readonly hash: string;
  readonly live: Readonly<Record<string, string>>;
}

/** `game.online.match_driver`'s `MatchDriverBatch` (Rust-owned). */
export interface MatchDriverBatch {
  readonly input_tick: number;
  readonly applied_rows: number;
  readonly reconciliations: number;
  readonly sent_packets: number;
  readonly outputs: readonly RollbackTickOutput[];
  readonly checkpoints: readonly MatchDriverCheckpoint[];
  readonly control: readonly TransportPeerMessage[];
}

/** The decoded half of `game.online.protocol`'s `SessionControlMessage`. */
export interface ProtocolControlMessage {
  readonly kind: SessionMessageKind;
  readonly sequence: number;
  readonly message_id: string;
}

/** `game.online.protocol.decode`, injected -- see the module doc comment. */
export type ProtocolDecoder = (payload: string) => ProtocolControlMessage | null;

// ---------------------------------------------------------------------------
// Recorder shapes
// ---------------------------------------------------------------------------

export interface NetDiagnosticsLimits {
  readonly checkpoints: number;
  readonly packets: number;
  readonly control: number;
  readonly events: number;
  readonly signals: number;
  readonly mismatches: number;
  readonly anchors: number;
  readonly latency_peers: number;
}

export interface NetDiagnosticsOptions {
  readonly role: SessionRole;
  readonly peer_id: string;
  readonly manifest: SessionManifest;
  readonly freeze: CoordinatorFreeze;
  readonly input_delay_ticks?: number;
  readonly hash_interval_ticks?: number;
  readonly max_rollback_ticks?: number;
  readonly network_profile_digest?: string;
  readonly limits?: NetDiagnosticsLimits;
  readonly export_opt_in?: boolean;
  readonly decodeControlMessage: ProtocolDecoder;
}

interface NetDiagnosticsRing<T> {
  items: Array<T | undefined>;
  limit: number;
  head: number; // 0-based next write position (Lua's ring was 1-based).
  count: number;
  dropped: number;
  keep: NetDiagnosticsKeep;
}

interface NetDiagnosticsSessionRecord {
  readonly session_id: string;
  readonly peer_id: string;
  readonly role: SessionRole;
  readonly match_mode: MatchMode;
  readonly combat_status: CombatMatchStatus;
  readonly manifest_id: string;
  readonly assignment_id: string;
  readonly countdown_id: string;
  readonly build_id: string;
  readonly source_id: string;
  readonly content_id: string;
  readonly tuning_id: string;
  readonly match_config_id: string;
  readonly fixture_id: string;
  readonly arena_id: string;
  readonly combat_rules_id: string;
  readonly gameplay_ai_policy_id: string;
  readonly network_profile_digest?: string;
  readonly protocol_version: number;
  readonly input_version: number;
  readonly snapshot_version: number;
  readonly tape_version: number;
  readonly combat_schema_version: number;
  readonly seed: number;
  readonly tick_rate: number;
  readonly duration_ticks: number;
  readonly max_goals: number;
}

interface NetDiagnosticsSimulationState {
  status: MatchDriverStatus;
  step_count: number;
  first_input_tick: number;
  present_input_tick: number;
  confirmed_input_tick: number;
  confirmed_output_tick: number;
  retained_floor_tick: number;
  input_delay_ticks: number;
  hash_interval_ticks: number;
  max_rollback_ticks: number;
  rollback_count: number;
  correction_count: number;
  predicted_slot_samples: number;
  resimulated_ticks: number;
  max_rollback_depth: number;
  hash_mismatch_streak: number;
  checkpoint_count: number;
  // Mutable pass-through fields: `T | undefined` rather than `T?`, since
  // they are reassigned imperatively from an equally-optional source field
  // (`exactOptionalPropertyTypes` forbids assigning a possibly-`undefined`
  // value into an optional property). Absent-vs-`undefined` makes no
  // difference to `diagnostics_schema.validate`, which reads them by
  // indexing and treats `undefined` as "not present" either way.
  late_input_tick: number | undefined;
  control_slot: string | undefined;
  owned: string[];
  authored: string[];
  terminal_status: MatchDriverStatus | undefined;
  terminal_failure: CoordinatorNetcodeFailure | undefined;
  terminal_detail: string | undefined;
  terminal_tick: number | undefined;
}

interface NetDiagnosticsDeliveryCounts {
  authored: number;
  published: number;
  sent: number;
  arrived: number;
  applied_rows: number;
  deferred: number;
  duplicate: number;
  rejected: number;
  reconciliations: number;
}

interface NetDiagnosticsEventCounts {
  emitted: number;
  added: number;
  revoked: number;
  replaced: number;
  unchanged: number;
  resimulated_tick_count: number;
}

interface NetDiagnosticsWorstCorrection {
  readonly causal_tick: number;
  readonly through_tick: number;
  readonly depth: number;
  readonly resimulated_ticks: number;
}

export interface NetDiagnosticsCheckpointRecord {
  readonly tick: number;
  readonly hash: string;
  readonly live: Readonly<Record<string, string>>;
}

export interface NetDiagnosticsMismatchRecord {
  readonly tick: number;
  readonly peer_id: string;
  readonly local_hash: string;
  readonly remote_hash: string;
  readonly first_difference_path?: string;
}

export interface NetDiagnosticsPacketRecord {
  readonly sender_id: string;
  readonly disposition: NetDiagnosticsPacketDisposition;
  readonly sequence: number;
  readonly payload_bytes: number;
  readonly row_count?: number;
  readonly sample_step: number;
  readonly send_transport_tick: number;
  readonly authority_input_tick: number;
  arrival_transport_tick?: number;
  apply_input_tick?: number;
}

export interface NetDiagnosticsControlRecord {
  readonly ordinal: number;
  readonly kind: SessionMessageKind;
  readonly peer_id: string;
  readonly sequence: number;
  readonly message_id: string;
}

export interface NetDiagnosticsEventRecord {
  readonly ordinal: number;
  readonly kind: NetDiagnosticsEventKind;
  readonly monotonic_ms: number;
  readonly peer_id?: string;
  readonly channel?: string;
  readonly state?: string;
  readonly code?: string;
  readonly detail?: string;
}

export interface NetDiagnosticsSignalRecord {
  readonly ordinal: number;
  readonly peer_id: string;
  readonly direction: NetDiagnosticsSignalDirection;
  readonly byte_length: number;
  readonly content: string;
}

interface NetDiagnosticsLatencyEntry {
  peer_id: string;
  sample_count: number;
  rtt_ms_min?: number;
  rtt_ms_max?: number;
  rtt_ms_last?: number;
  jitter_ms_max?: number;
  jitter_ms_last?: number;
  monotonic_ms_first?: number;
  monotonic_ms_last?: number;
}

interface NetDiagnosticsStarSnapshot {
  readonly role: TransportRole;
  readonly state: TransportState;
  readonly capacity: number;
  readonly peer_count: number;
  readonly queue_limit: number;
  readonly buffered_amount_limit: number;
  readonly event_depth: number;
  readonly sent: number;
  readonly received: number;
  readonly dropped_outbound: number;
  readonly dropped_inbound: number;
  readonly malformed: number;
  readonly unsupported_version: number;
  readonly overflow: number;
  readonly backpressure: number;
  /** Absent (never a present `null`) when this snapshot carries no error --
   * mirrors the omit-vs-null convention `exportArtifact` already uses for
   * `star`/`worst`/`pressure`/`teardown` themselves (each `undefined`, never
   * a present `null`, when absent). `last_error` used to be the one
   * exception: always present, `null` when clean. The schema's own
   * `{ optional: true }` declaration for this field only ever treated an
   * *absent* key as satisfying "optional" (`validateField`'s record
   * dispatch checks `childValue === undefined`, never `=== null`), so a
   * clean run's literal `null` failed export validation with
   * "last_error must be a string" -- confirmed empirically driving a real
   * `MatchDriverBridge.transportDiagnosticsJson()` snapshot through
   * `recordTransport` on a clean run. */
  readonly last_error?: string;
}

interface NetDiagnosticsPeerSnapshot {
  readonly peer_id: string;
  readonly slot: number;
  readonly state: TransportPeerState;
  readonly ice_state: string;
  readonly control: TransportChannelDiagnostics;
  readonly input: TransportChannelDiagnostics;
  readonly sequence_gaps: number;
  readonly backpressure: number;
  readonly malformed: number;
  /** See {@link NetDiagnosticsStarSnapshot.last_error}'s doc. */
  readonly last_error?: string;
}

interface NetDiagnosticsPressure {
  samples: number;
  peak_outbound_depth: number;
  peak_inbound_depth: number;
  peak_buffered_amount: number;
  peak_event_depth: number;
  peak_peer_count: number;
  backpressure: number;
  peer_backpressure: number;
  overflow: number;
  dropped_outbound: number;
  dropped_inbound: number;
}

interface NetDiagnosticsTeardownState {
  readonly requested: boolean;
  readonly complete: boolean;
  readonly closed_peers: number;
  readonly residual_outbound: number;
  readonly residual_inbound: number;
  readonly orphaned_peers: readonly string[];
}

export interface NetDiagnostics {
  readonly session: NetDiagnosticsSessionRecord;
  readonly limits: NetDiagnosticsLimits;
  export_opt_in: boolean;
  rejected: number;
  readonly checkpoints: NetDiagnosticsRing<NetDiagnosticsCheckpointRecord>;
  readonly packets: NetDiagnosticsRing<NetDiagnosticsPacketRecord>;
  readonly control: NetDiagnosticsRing<NetDiagnosticsControlRecord>;
  readonly events: NetDiagnosticsRing<NetDiagnosticsEventRecord>;
  readonly signals: NetDiagnosticsRing<NetDiagnosticsSignalRecord>;
  readonly mismatches: NetDiagnosticsRing<NetDiagnosticsMismatchRecord>;
  readonly anchors: NetDiagnosticsRing<{
    readonly input_tick: number;
    readonly monotonic_ms: number;
    readonly mapping_error_ms: number;
  }>;
  readonly latency: Map<string, NetDiagnosticsLatencyEntry>;
  readonly latency_order: string[];
  readonly simulation: NetDiagnosticsSimulationState;
  readonly delivery: NetDiagnosticsDeliveryCounts;
  readonly events_counts: NetDiagnosticsEventCounts;
  worst: NetDiagnosticsWorstCorrection | null;
  star: NetDiagnosticsStarSnapshot | null;
  peers: NetDiagnosticsPeerSnapshot[];
  pressure: NetDiagnosticsPressure | null;
  teardown: NetDiagnosticsTeardownState | null;
  readonly tick_digests: Map<number, string>;
  tick_digest_floor: number;
  pending_apply: NetDiagnosticsPacketRecord[];
  ordinal: number;
  readonly decodeControlMessage: ProtocolDecoder;
}

export const SCHEMA_VERSION = 1;

// Caps are deliberately small. A diagnostic ring exists so a failure that
// happened seconds ago is still explainable; it is not a log, and a
// diagnostic that grows with match length would be the first thing to blame
// for a stall.
export const DEFAULT_LIMITS: NetDiagnosticsLimits = {
  checkpoints: 64,
  packets: 256,
  control: 64,
  events: 128,
  signals: 32,
  mismatches: 16,
  anchors: 32,
  latency_peers: MAX_GUESTS + 1,
};

export const MAX_DETAIL_BYTES = 240;

const ROLES = schema.enumValues(["host", "guest"]);
const MATCH_MODES = schema.enumValues(["1v1", "2v2", "4v4"]);
const COMBAT_STATUS = schema.enumValues([
  "provisional_114",
  "accepted_proceed",
  "accepted_revision",
]);
const DRIVER_STATUS = schema.enumValues([
  "active",
  "completed",
  "settle_timeout",
  "confirmation_stalled",
  "late_input",
  "hash_mismatch",
  "ownership_violation",
  "authority_conflict",
  "input_channel_failure",
  "transport_lost",
]);
const NETCODE_FAILURE = schema.enumValues(["input_channel", "late_input", "desync"]);
const RETENTION = schema.enumValues([schema.COMPLETE, schema.TRUNCATED]);
const TRANSPORT_STATE = schema.enumValues(["new", "connected", "disconnected", "closed", "error"]);
const PEER_STATE = schema.enumValues([
  "new",
  "connecting",
  "connected",
  "disconnected",
  "closed",
  "error",
]);
const CHANNELS = schema.enumValues(["control", "input"]);
const SIGNAL_DIRECTION = schema.enumValues(["outbound", "inbound"]);
const EVENT_KIND = schema.enumValues([
  "peer_state",
  "peer_error",
  "star_state",
  "star_error",
  "signal",
  "lifecycle",
  "teardown",
]);
const CONTROL_KIND = schema.enumValues([
  "handshake",
  "manifest_proposal",
  "manifest_accept",
  "peer_assignment",
  "slot_assignment",
  "ready",
  "pair_preference",
  "pair_preference_result",
  "countdown",
  "start",
  "match_phase",
  "hash_report",
  "result_ack",
  "abort",
  "disconnect",
]);
const PACKET_DISPOSITION = schema.enumValues([
  "authored",
  "sent",
  "arrived",
  "deferred",
  "duplicate",
  "rejected",
]);

function integerField(name: string): DiagnosticsField {
  return { name, kind: "integer", min: 0 };
}

function signedField(name: string): DiagnosticsField {
  return { name, kind: "integer" };
}

function optionalInteger(name: string): DiagnosticsField {
  return { name, kind: "integer", optional: true };
}

function optionalNumber(name: string): DiagnosticsField {
  return { name, kind: "number", optional: true };
}

const CHANNEL_FIELDS: DiagnosticsField[] = [
  { name: "state", kind: "enum", values: PEER_STATE },
  integerField("outbound_depth"),
  integerField("inbound_depth"),
  integerField("buffered_amount"),
  integerField("sent"),
  integerField("received"),
  integerField("dropped_outbound"),
  integerField("dropped_inbound"),
];

export const SESSION_SHAPE: DiagnosticsField[] = [
  { name: "session_id", kind: "id" },
  { name: "peer_id", kind: "id" },
  { name: "role", kind: "enum", values: ROLES },
  { name: "match_mode", kind: "enum", values: MATCH_MODES },
  { name: "combat_status", kind: "enum", values: COMBAT_STATUS },
  { name: "manifest_id", kind: "hash" },
  { name: "assignment_id", kind: "hash" },
  { name: "countdown_id", kind: "id" },
  { name: "build_id", kind: "id" },
  { name: "source_id", kind: "id" },
  { name: "content_id", kind: "id" },
  { name: "tuning_id", kind: "id" },
  { name: "match_config_id", kind: "id" },
  { name: "fixture_id", kind: "id" },
  { name: "arena_id", kind: "id" },
  { name: "combat_rules_id", kind: "id" },
  { name: "gameplay_ai_policy_id", kind: "id" },
  { name: "network_profile_digest", kind: "hash", optional: true },
  integerField("protocol_version"),
  integerField("input_version"),
  integerField("snapshot_version"),
  integerField("tape_version"),
  integerField("combat_schema_version"),
  integerField("seed"),
  integerField("tick_rate"),
  integerField("duration_ticks"),
  integerField("max_goals"),
];

// The exported artifact. Section order here is the canonical export order,
// and `diagnostics_schema.encode` follows declared order, so two exports of
// the same values are byte-identical regardless of object key order.
export const EXPORT: DiagnosticsShape = schema.record("net_diagnostics", undefined, [
  { name: "schema_version", kind: "integer", domain: "identity", min: 1 },
  { name: "digest_algorithm", kind: "string", domain: "identity" },
  { name: "session", kind: "record", domain: "identity", fields: SESSION_SHAPE },
  {
    name: "collection",
    kind: "record",
    domain: "identity",
    fields: [
      { name: "export_opt_in", kind: "boolean" },
      integerField("rejected_values"),
      schema.nested("caps", [
        integerField("checkpoints"),
        integerField("packets"),
        integerField("control"),
        integerField("events"),
        integerField("signals"),
        integerField("mismatches"),
        integerField("anchors"),
        integerField("latency_peers"),
      ]),
      schema.nested("dropped", [
        integerField("checkpoints"),
        integerField("packets"),
        integerField("control"),
        integerField("events"),
        integerField("signals"),
        integerField("mismatches"),
        integerField("anchors"),
      ]),
      schema.nested("retention", [
        { name: "checkpoints", kind: "enum", values: RETENTION },
        { name: "packets", kind: "enum", values: RETENTION },
        { name: "control", kind: "enum", values: RETENTION },
        { name: "events", kind: "enum", values: RETENTION },
        { name: "signals", kind: "enum", values: RETENTION },
        { name: "mismatches", kind: "enum", values: RETENTION },
        { name: "anchors", kind: "enum", values: RETENTION },
      ]),
    ],
  },
  {
    name: "canonical",
    kind: "record",
    domain: "canonical",
    fields: [
      schema.nested("simulation", [
        { name: "status", kind: "enum", values: DRIVER_STATUS },
        { name: "terminal_status", kind: "enum", values: DRIVER_STATUS, optional: true },
        { name: "terminal_failure", kind: "enum", values: NETCODE_FAILURE, optional: true },
        { name: "terminal_detail", kind: "text", optional: true },
        optionalInteger("terminal_tick"),
        integerField("step_count"),
        signedField("first_input_tick"),
        signedField("present_input_tick"),
        signedField("confirmed_input_tick"),
        signedField("confirmed_output_tick"),
        signedField("retained_floor_tick"),
        integerField("input_delay_ticks"),
        integerField("hash_interval_ticks"),
        integerField("max_rollback_ticks"),
        integerField("rollback_count"),
        integerField("correction_count"),
        integerField("predicted_slot_samples"),
        integerField("resimulated_ticks"),
        integerField("max_rollback_depth"),
        integerField("hash_mismatch_streak"),
        integerField("checkpoint_count"),
        optionalInteger("late_input_tick"),
        { name: "control_slot", kind: "id", optional: true },
        { name: "owned", kind: "array", element: { kind: "id" } },
        { name: "authored", kind: "array", element: { kind: "id" } },
      ]),
      {
        name: "worst_correction",
        kind: "record",
        optional: true,
        fields: [
          signedField("causal_tick"),
          signedField("through_tick"),
          integerField("depth"),
          integerField("resimulated_ticks"),
        ],
      },
      schema.nested("events", [
        integerField("emitted"),
        integerField("added"),
        integerField("revoked"),
        integerField("replaced"),
        integerField("unchanged"),
        integerField("resimulated_tick_count"),
      ]),
      schema.nested("delivery", [
        integerField("authored"),
        integerField("published"),
        integerField("sent"),
        integerField("arrived"),
        integerField("applied_rows"),
        integerField("deferred"),
        integerField("duplicate"),
        integerField("rejected"),
        integerField("reconciliations"),
        {
          name: "packets",
          kind: "array",
          element: {
            kind: "record",
            fields: [
              { name: "sender_id", kind: "id" },
              { name: "disposition", kind: "enum", values: PACKET_DISPOSITION },
              integerField("sequence"),
              integerField("payload_bytes"),
              signedField("sample_step"),
              signedField("send_transport_tick"),
              signedField("authority_input_tick"),
              optionalInteger("arrival_transport_tick"),
              optionalInteger("apply_input_tick"),
              optionalInteger("row_count"),
            ],
          },
        },
      ]),
      {
        name: "checkpoints",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            signedField("tick"),
            { name: "hash", kind: "hash" },
            { name: "live", kind: "map", element: { kind: "id" } },
          ],
        },
      },
      {
        name: "mismatches",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            signedField("tick"),
            { name: "peer_id", kind: "id" },
            { name: "local_hash", kind: "hash" },
            { name: "remote_hash", kind: "hash" },
            { name: "first_difference_path", kind: "string", optional: true },
          ],
        },
      },
      // Control traffic is canonical, not runtime: a session control message
      // is identified by a sender-local sequence and a derived message id,
      // both reproducible, and the order they were accepted in is part of
      // what an offline reproduction has to replay.
      {
        name: "control",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            integerField("ordinal"),
            { name: "kind", kind: "enum", values: CONTROL_KIND },
            { name: "peer_id", kind: "id" },
            integerField("sequence"),
            // Not an `id`: the derived message id is length-prefixed and
            // contains `:`, which the join-key charset excludes on purpose.
            // It stays an opaque bounded string.
            { name: "message_id", kind: "string", max_length: 320 },
          ],
        },
      },
    ],
  },
  {
    name: "runtime",
    kind: "record",
    domain: "runtime",
    fields: [
      {
        name: "star",
        kind: "record",
        optional: true,
        fields: [
          { name: "role", kind: "enum", values: ROLES },
          { name: "state", kind: "enum", values: TRANSPORT_STATE },
          integerField("capacity"),
          integerField("peer_count"),
          integerField("queue_limit"),
          integerField("buffered_amount_limit"),
          integerField("event_depth"),
          integerField("sent"),
          integerField("received"),
          integerField("dropped_outbound"),
          integerField("dropped_inbound"),
          integerField("malformed"),
          integerField("unsupported_version"),
          integerField("overflow"),
          integerField("backpressure"),
          { name: "last_error", kind: "text", optional: true },
        ],
      },
      {
        name: "peers",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            { name: "peer_id", kind: "id" },
            integerField("slot"),
            { name: "state", kind: "enum", values: PEER_STATE },
            { name: "ice_state", kind: "string" },
            { name: "control", kind: "record", fields: CHANNEL_FIELDS },
            { name: "input", kind: "record", fields: CHANNEL_FIELDS },
            integerField("sequence_gaps"),
            integerField("backpressure"),
            integerField("malformed"),
            { name: "last_error", kind: "text", optional: true },
          ],
        },
      },
      {
        name: "pressure",
        kind: "record",
        optional: true,
        fields: [
          integerField("samples"),
          integerField("peak_outbound_depth"),
          integerField("peak_inbound_depth"),
          integerField("peak_buffered_amount"),
          integerField("peak_event_depth"),
          integerField("peak_peer_count"),
          integerField("backpressure"),
          integerField("peer_backpressure"),
          integerField("overflow"),
          integerField("dropped_outbound"),
          integerField("dropped_inbound"),
        ],
      },
      {
        name: "latency",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            { name: "peer_id", kind: "id" },
            integerField("sample_count"),
            optionalNumber("rtt_ms_min"),
            optionalNumber("rtt_ms_max"),
            optionalNumber("rtt_ms_last"),
            optionalNumber("jitter_ms_max"),
            optionalNumber("jitter_ms_last"),
            optionalNumber("monotonic_ms_first"),
            optionalNumber("monotonic_ms_last"),
          ],
        },
      },
      {
        name: "events",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            integerField("ordinal"),
            { name: "kind", kind: "enum", values: EVENT_KIND },
            { name: "monotonic_ms", kind: "number" },
            { name: "peer_id", kind: "id", optional: true },
            { name: "channel", kind: "enum", values: CHANNELS, optional: true },
            { name: "state", kind: "string", optional: true },
            { name: "code", kind: "string", optional: true },
            { name: "detail", kind: "text", optional: true },
          ],
        },
      },
      {
        name: "signals",
        kind: "array",
        element: {
          kind: "record",
          fields: [
            integerField("ordinal"),
            { name: "peer_id", kind: "id" },
            { name: "direction", kind: "enum", values: SIGNAL_DIRECTION },
            integerField("byte_length"),
            { name: "content", kind: "string" },
          ],
        },
      },
      {
        name: "teardown",
        kind: "record",
        optional: true,
        fields: [
          { name: "requested", kind: "boolean" },
          { name: "complete", kind: "boolean" },
          integerField("closed_peers"),
          integerField("residual_outbound"),
          integerField("residual_inbound"),
          { name: "orphaned_peers", kind: "array", element: { kind: "id" } },
        ],
      },
    ],
  },
  {
    name: "anchors",
    kind: "array",
    domain: "anchor",
    element: {
      kind: "record",
      fields: [
        signedField("input_tick"),
        { name: "monotonic_ms", kind: "number" },
        { name: "mapping_error_ms", kind: "number" },
      ],
    },
  },
]);

// ---------------------------------------------------------------------------
// Bounded rings
// ---------------------------------------------------------------------------

function newRing<T>(limit: number, keep: NetDiagnosticsKeep): NetDiagnosticsRing<T> {
  return { items: [], limit, head: 0, count: 0, dropped: 0, keep };
}

// `newest` keeps the tail: what just happened explains a failure that just
// happened. `oldest` keeps the head and is used where the *first* occurrence
// is the causal one -- a hash mismatch stream, where later entries are
// echoes.
function ringPush<T>(ring: NetDiagnosticsRing<T>, item: T): void {
  if (ring.limit <= 0) {
    ring.dropped += 1;
    return;
  }
  if (ring.keep === "oldest") {
    if (ring.count >= ring.limit) {
      ring.dropped += 1;
      return;
    }
    ring.items[ring.count] = item;
    ring.count += 1;
    return;
  }
  if (ring.count >= ring.limit) {
    ring.dropped += 1;
  } else {
    ring.count += 1;
  }
  ring.items[ring.head] = item;
  ring.head = (ring.head + 1) % ring.limit;
}

function ringItems<T>(ring: NetDiagnosticsRing<T>): T[] {
  const out: T[] = [];
  if (ring.keep === "oldest" || ring.count < ring.limit) {
    for (let index = 0; index < ring.count; index += 1) {
      const item = ring.items[index];
      if (item !== undefined) {
        out.push(item);
      }
    }
    return out;
  }
  for (let offset = 0; offset < ring.count; offset += 1) {
    const item = ring.items[(ring.head + offset) % ring.limit];
    if (item !== undefined) {
      out.push(item);
    }
  }
  return out;
}

function ringRetention(ring: NetDiagnosticsRing<unknown>): NetDiagnosticsRetention {
  return ring.dropped > 0 ? schema.TRUNCATED : schema.COMPLETE;
}

// ---------------------------------------------------------------------------
// Value guards
// ---------------------------------------------------------------------------

function isFinite_(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isInteger(value: unknown): value is number {
  return isFinite_(value) && value === Math.floor(value);
}

// Every free-text field in this module goes through here. Length is the
// lesser half of the job: the text originates from a transport, a browser
// bridge, or a raw DOM exception, so it is checked for network and identity
// material and replaced whole when it looks like it carries any. See
// `diagnostics_schema.isSensitiveText`.
function boundedDetail(text: unknown): string | null {
  return schema.redactFreeText(text, MAX_DETAIL_BYTES);
}

function reject(recorder: NetDiagnostics, message: string): Result<never, string> {
  recorder.rejected += 1;
  return err(message);
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

export function newNetDiagnostics(options: NetDiagnosticsOptions): NetDiagnostics {
  const limits = options.limits ?? DEFAULT_LIMITS;
  const manifest = options.manifest;
  const freeze = options.freeze;
  const session: NetDiagnosticsSessionRecord = {
    session_id: manifest.session_id,
    peer_id: options.peer_id,
    role: options.role,
    match_mode: manifest.match_mode,
    combat_status: manifest.combat_status,
    manifest_id: freeze.manifest_id,
    assignment_id: freeze.assignment_id,
    countdown_id: freeze.countdown_id,
    build_id: manifest.build_id,
    source_id: manifest.source_id,
    content_id: manifest.content_id,
    tuning_id: manifest.tuning_id,
    match_config_id: manifest.match_config_id,
    fixture_id: manifest.fixture_id,
    arena_id: manifest.arena_id,
    combat_rules_id: manifest.combat_rules_id,
    gameplay_ai_policy_id: manifest.gameplay_ai_policy_id,
    protocol_version: manifest.protocol_version,
    input_version: manifest.input_version,
    snapshot_version: manifest.snapshot_version,
    tape_version: manifest.tape_version,
    combat_schema_version: manifest.combat_schema_version,
    seed: manifest.seed,
    tick_rate: manifest.tick_rate,
    duration_ticks: manifest.duration_ticks,
    max_goals: manifest.max_goals,
    ...(options.network_profile_digest !== undefined
      ? { network_profile_digest: options.network_profile_digest }
      : {}),
  };
  return {
    session,
    limits: { ...limits },
    export_opt_in: options.export_opt_in === true,
    rejected: 0,
    checkpoints: newRing(limits.checkpoints, "newest"),
    packets: newRing(limits.packets, "newest"),
    control: newRing(limits.control, "newest"),
    events: newRing(limits.events, "newest"),
    signals: newRing(limits.signals, "newest"),
    mismatches: newRing(limits.mismatches, "oldest"),
    anchors: newRing(limits.anchors, "newest"),
    latency: new Map(),
    latency_order: [],
    simulation: {
      status: "active",
      step_count: 0,
      first_input_tick: freeze.first_input_tick,
      present_input_tick: freeze.first_input_tick,
      confirmed_input_tick: freeze.first_input_tick - 1,
      confirmed_output_tick: freeze.first_input_tick - 1,
      retained_floor_tick: freeze.first_input_tick,
      input_delay_ticks: options.input_delay_ticks ?? 0,
      hash_interval_ticks: options.hash_interval_ticks ?? 0,
      max_rollback_ticks: options.max_rollback_ticks ?? 0,
      rollback_count: 0,
      correction_count: 0,
      predicted_slot_samples: 0,
      resimulated_ticks: 0,
      max_rollback_depth: 0,
      hash_mismatch_streak: 0,
      checkpoint_count: 0,
      late_input_tick: undefined,
      control_slot: undefined,
      owned: [],
      authored: [],
      terminal_status: undefined,
      terminal_failure: undefined,
      terminal_detail: undefined,
      terminal_tick: undefined,
    },
    delivery: {
      authored: 0,
      published: 0,
      sent: 0,
      arrived: 0,
      applied_rows: 0,
      deferred: 0,
      duplicate: 0,
      rejected: 0,
      reconciliations: 0,
    },
    events_counts: {
      emitted: 0,
      added: 0,
      revoked: 0,
      replaced: 0,
      unchanged: 0,
      resimulated_tick_count: 0,
    },
    worst: null,
    star: null,
    peers: [],
    pressure: null,
    teardown: null,
    tick_digests: new Map(),
    tick_digest_floor: freeze.first_input_tick,
    pending_apply: [],
    ordinal: 0,
    decodeControlMessage: options.decodeControlMessage,
  };
}

// Export is a decision, not a default. Collection is always on and always
// bounded; producing an artifact a human can paste is separately opted into.
export function optInExport(recorder: NetDiagnostics): void {
  recorder.export_opt_in = true;
}

export function optOutExport(recorder: NetDiagnostics): void {
  recorder.export_opt_in = false;
}

// ---------------------------------------------------------------------------
// Canonical recording
// ---------------------------------------------------------------------------

const MATCH_EVENT_FIELDS = [
  "kind",
  "x",
  "y",
  "player",
  "save_style",
  "style",
  "outcome",
  "jumping",
  "difficulty",
  "shot_type",
  "keeper_state",
  "keeper_depth",
  "on_target",
] as const;

// The digest has to cover every field that tells two rows apart, or a
// resimulation that changed one is reported as reconciling. A rejected
// request differs from another refusal of the same player only by its
// reason, and one terminal differs from another only by which terminal it
// is.
const COMBAT_EVENT_FIELDS = [
  "kind",
  "tick",
  "family_id",
  "source_index",
  "target_index",
  "source_sequence",
  "result",
  "outcome",
  "reason",
  "terminal",
  "x",
  "y",
  "interruption_ticks",
  "displacement_px",
] as const;

function pushEventFields(parts: string[], event: DiagnosticEvent, fields: readonly string[]): void {
  for (const name of fields) {
    const value = event[name];
    if (value === undefined) {
      parts.push("-");
    } else if (typeof value === "number") {
      // `%.17g`, not `toPrecision(17)` -- see `diagnostics_schema.ts`'s
      // `formatG17` doc: `toPrecision` pads trailing zeros where `%g`
      // strips them, which would silently change this digest.
      parts.push(schema.formatG17(value));
    } else {
      parts.push(String(value));
    }
  }
}

// The identity of one tick's event set. A rollback that replaces a tick and
// produces the same events is `unchanged`; one that produces different
// events is `replaced`. Both are resimulation, and only the second is a
// reconciliation a consumer has to care about.
function eventDigest(output: RollbackTickOutput): string {
  const parts: string[] = [String(output.events.length)];
  for (const event of output.events) {
    pushEventFields(parts, event, MATCH_EVENT_FIELDS);
  }
  const combat = output.combat_events ?? [];
  parts.push(String(combat.length));
  for (const event of combat) {
    pushEventFields(parts, event, COMBAT_EVENT_FIELDS);
  }
  return schema.tupleDigest("tick_events", parts);
}

function pruneTickDigests(recorder: NetDiagnostics, floor: number): void {
  if (floor <= recorder.tick_digest_floor) {
    return;
  }
  for (let tick = recorder.tick_digest_floor; tick < floor; tick += 1) {
    recorder.tick_digests.delete(tick);
  }
  recorder.tick_digest_floor = floor;
}

// Fold one driver step. `diagnostics` is `match_driver.diagnostics(driver)`
// and `batch` is the `MatchDriverBatch` the same `advance` returned; nothing
// here reaches into the driver.
export function recordStep(
  recorder: NetDiagnostics,
  diagnostics: MatchDriverDiagnostics,
  batch: MatchDriverBatch
): Result<true, string> {
  if (!isInteger(diagnostics.present_input_tick) || !isInteger(batch.input_tick)) {
    return reject(recorder, "a driver step must carry finite integer ticks");
  }
  const simulation = recorder.simulation;
  simulation.status = diagnostics.status;
  simulation.step_count += 1;
  simulation.present_input_tick = diagnostics.present_input_tick;
  simulation.confirmed_input_tick = diagnostics.confirmed_input_tick;
  simulation.confirmed_output_tick = diagnostics.confirmed_output_tick;
  simulation.retained_floor_tick = diagnostics.retained_floor_tick;
  simulation.rollback_count = diagnostics.rollback_count;
  simulation.correction_count = diagnostics.correction_count;
  simulation.predicted_slot_samples = diagnostics.predicted_slot_samples;
  simulation.max_rollback_depth = diagnostics.max_rollback_depth;
  simulation.hash_mismatch_streak = diagnostics.hash_mismatches;
  simulation.checkpoint_count = diagnostics.checkpoint_count;
  simulation.late_input_tick = diagnostics.late_input_tick;
  simulation.control_slot = diagnostics.control_slot;
  simulation.owned = [...diagnostics.owned];
  simulation.authored = [...diagnostics.authored];
  const terminal = diagnostics.terminal;
  if (terminal !== undefined) {
    simulation.terminal_status = terminal.status;
    simulation.terminal_failure = terminal.failure;
    const detail = boundedDetail(terminal.detail);
    simulation.terminal_detail = detail ?? undefined;
    simulation.terminal_tick = terminal.tick;
  }

  const delivery = recorder.delivery;
  delivery.applied_rows += batch.applied_rows;
  delivery.reconciliations += batch.reconciliations;
  // `published` is the driver's own count of the packets it handed to the
  // transport; `sent` is what an observer actually saw leave. They are kept
  // apart rather than summed: adding them would double-count whenever a tap
  // is installed, and a disagreement between them is itself a finding.
  delivery.published += batch.sent_packets;

  // Resimulation and event reconciliation are read off the outputs the batch
  // already carries: a rollback re-emits every corrected tick before this
  // step's own output, so a tick seen twice is a resimulated tick by
  // definition rather than by a counter the driver would have to keep.
  const counts = recorder.events_counts;
  let resimulated = 0;
  let firstCorrected = Infinity;
  let lastCorrected = -Infinity;
  for (const output of batch.outputs) {
    const tick = output.tick;
    const digest = eventDigest(output);
    const previous = recorder.tick_digests.get(tick);
    const eventCount = output.events.length + (output.combat_events?.length ?? 0);
    counts.emitted += eventCount;
    if (previous === undefined) {
      counts.added += 1;
    } else {
      resimulated += 1;
      firstCorrected = Math.min(firstCorrected, tick);
      lastCorrected = Math.max(lastCorrected, tick);
      if (previous === digest) {
        counts.unchanged += 1;
      } else if (eventCount === 0) {
        counts.revoked += 1;
      } else {
        counts.replaced += 1;
      }
    }
    recorder.tick_digests.set(tick, digest);
  }
  if (resimulated > 0) {
    simulation.resimulated_ticks += resimulated;
    counts.resimulated_tick_count += resimulated;
    const depth = lastCorrected - firstCorrected + 1;
    if (recorder.worst === null || depth > recorder.worst.depth) {
      recorder.worst = {
        causal_tick: firstCorrected,
        through_tick: lastCorrected,
        depth,
        resimulated_ticks: resimulated,
      };
    }
  }
  pruneTickDigests(recorder, diagnostics.retained_floor_tick);

  const pending = recorder.pending_apply;
  recorder.pending_apply = [];
  for (const record of pending) {
    record.apply_input_tick = batch.input_tick;
  }

  for (const checkpoint of batch.checkpoints) {
    recordCheckpoint(recorder, checkpoint);
  }
  return ok(true);
}

export function recordCheckpoint(
  recorder: NetDiagnostics,
  checkpoint: MatchDriverCheckpoint
): Result<true, string> {
  if (!isInteger(checkpoint.tick)) {
    return reject(recorder, "a checkpoint needs a finite integer tick");
  }
  if (typeof checkpoint.hash !== "string") {
    return reject(recorder, "a checkpoint needs a hash");
  }
  ringPush(recorder.checkpoints, {
    tick: checkpoint.tick,
    hash: checkpoint.hash,
    live: { ...checkpoint.live },
  });
  return ok(true);
}

// A boundary where a peer's published hash disagreed with ours. The first
// difference path is optional and is only ever present when the capture
// holds both snapshots -- in a real session it does not, and claiming
// otherwise would be the kind of false precision this schema exists to
// avoid.
export function recordMismatch(
  recorder: NetDiagnostics,
  mismatch: NetDiagnosticsMismatchRecord
): Result<true, string> {
  if (!isInteger(mismatch.tick)) {
    return reject(recorder, "a mismatch needs a finite integer tick");
  }
  ringPush(recorder.mismatches, {
    tick: mismatch.tick,
    peer_id: mismatch.peer_id,
    local_hash: mismatch.local_hash,
    remote_hash: mismatch.remote_hash,
    ...(mismatch.first_difference_path !== undefined
      ? { first_difference_path: mismatch.first_difference_path }
      : {}),
  });
  return ok(true);
}

export interface NetDiagnosticsPacketInput {
  readonly sender_id: string;
  readonly disposition: NetDiagnosticsPacketDisposition;
  readonly sequence: number;
  readonly payload_bytes: number;
  readonly sample_step: number;
  readonly send_transport_tick: number;
  readonly authority_input_tick: number;
  readonly arrival_transport_tick?: number;
  readonly apply_input_tick?: number;
  readonly row_count?: number;
}

export function recordPacket(
  recorder: NetDiagnostics,
  packet: NetDiagnosticsPacketInput
): Result<true, string> {
  const required = [
    packet.sequence,
    packet.payload_bytes,
    packet.sample_step,
    packet.send_transport_tick,
    packet.authority_input_tick,
  ];
  for (const value of required) {
    if (!isInteger(value)) {
      return reject(recorder, "a packet record needs finite integer ticks and counts");
    }
  }
  if (packet.arrival_transport_tick !== undefined && !isInteger(packet.arrival_transport_tick)) {
    return reject(recorder, "a packet arrival tick must be a finite integer");
  }
  if (packet.apply_input_tick !== undefined && !isInteger(packet.apply_input_tick)) {
    return reject(recorder, "a packet apply tick must be a finite integer");
  }
  if (packet.row_count !== undefined && !isInteger(packet.row_count)) {
    return reject(recorder, "a packet row count must be a finite integer");
  }
  const delivery = recorder.delivery;
  const disposition = packet.disposition;
  if (disposition === "authored") {
    delivery.authored += 1;
  } else if (disposition === "sent") {
    delivery.sent += 1;
  } else if (disposition === "arrived") {
    delivery.arrived += 1;
  } else if (disposition === "deferred") {
    delivery.deferred += 1;
  } else if (disposition === "duplicate") {
    delivery.duplicate += 1;
  } else if (disposition === "rejected") {
    delivery.rejected += 1;
  }
  const record: NetDiagnosticsPacketRecord = {
    sender_id: packet.sender_id,
    disposition,
    sequence: packet.sequence,
    payload_bytes: packet.payload_bytes,
    sample_step: packet.sample_step,
    send_transport_tick: packet.send_transport_tick,
    authority_input_tick: packet.authority_input_tick,
    ...(packet.row_count !== undefined ? { row_count: packet.row_count } : {}),
    ...(packet.arrival_transport_tick !== undefined
      ? { arrival_transport_tick: packet.arrival_transport_tick }
      : {}),
    ...(packet.apply_input_tick !== undefined ? { apply_input_tick: packet.apply_input_tick } : {}),
  };
  ringPush(recorder.packets, record);
  // An arrival is applied by the same `advance` that polled it, so the apply
  // tick is stamped by the next `recordStep` rather than guessed here. The
  // lead it exposes -- authority tick minus apply tick -- is the number a
  // prediction burst is actually about.
  if (disposition === "arrived" && record.apply_input_tick === undefined) {
    recorder.pending_apply.push(record);
  }
  return ok(true);
}

// One accepted session control message, in the order it was handed back.
// The driver returns control traffic on the batch rather than consuming it,
// so this takes the same `TransportPeerMessage` the coordinator will read --
// it observes the mail, it does not open it on anyone's behalf.
export function recordControl(
  recorder: NetDiagnostics,
  addressed: TransportPeerMessage
): Result<true, string> {
  const message = recorder.decodeControlMessage(addressed.message.payload);
  if (message === null) {
    return reject(recorder, "control payload did not decode as a session message");
  }
  recorder.ordinal += 1;
  ringPush(recorder.control, {
    ordinal: recorder.ordinal,
    kind: message.kind,
    peer_id: addressed.peer_id,
    sequence: message.sequence,
    message_id: message.message_id,
  });
  return ok(true);
}

// ---------------------------------------------------------------------------
// Runtime observation
// ---------------------------------------------------------------------------

function copyChannel(channel: TransportChannelDiagnostics): TransportChannelDiagnostics {
  return {
    state: channel.state,
    outbound_depth: channel.outbound_depth,
    inbound_depth: channel.inbound_depth,
    buffered_amount: channel.buffered_amount,
    sent: channel.sent,
    received: channel.received,
    dropped_outbound: channel.dropped_outbound,
    dropped_inbound: channel.dropped_inbound,
  };
}

// Transport diagnostics are a *snapshot of counters*, so the latest replaces
// the previous rather than accumulating: the star already counts
// cumulatively and adding our own sum on top would double-count.
//
// Queue **depths** are the exception, and they are why `runtime.pressure`
// exists. A depth is an instantaneous level, not a counter, and the last
// snapshot a run takes is almost always a quiescent one -- taken after the
// final pump, or after teardown drained everything. Reading
// `peers[*].control.outbound_depth` out of the export therefore answers "how
// deep was the queue when we stopped looking", which is nearly always zero,
// and a resource gate written against it cannot fail. `pressure` folds a
// genuine running peak across every snapshot, alongside the cumulative
// backpressure and overflow latches that a depth sample can miss entirely: a
// channel that hit its `bufferedAmount` ceiling and drained again between
// two observations leaves no depth behind, only a latch.
// `boundedDetail` is `string | null` (redaction/absence collapse to the same
// `null`), but the exported shape's `last_error` is optional -- present or
// absent, never a present `null` (see `NetDiagnosticsStarSnapshot.last_error`'s
// doc for the export-validation failure this fixes). This is the seam that
// converts between the two, the same way `exportArtifact` already does for
// `star`/`worst`/`pressure`/`teardown` themselves.
function optionalDetail(text: unknown): { readonly last_error?: string } {
  const value = boundedDetail(text);
  return value !== null ? { last_error: value } : {};
}

export function recordTransport(
  recorder: NetDiagnostics,
  star: TransportStarDiagnostics
): Result<true, string> {
  recorder.star = {
    role: star.role,
    state: star.state,
    capacity: star.capacity,
    peer_count: star.peer_count,
    queue_limit: star.queue_limit,
    buffered_amount_limit: star.buffered_amount_limit,
    event_depth: star.event_depth,
    sent: star.sent,
    received: star.received,
    dropped_outbound: star.dropped_outbound,
    dropped_inbound: star.dropped_inbound,
    malformed: star.malformed,
    unsupported_version: star.unsupported_version,
    overflow: star.overflow,
    backpressure: star.backpressure,
    ...optionalDetail(star.last_error),
  };
  const peers: NetDiagnosticsPeerSnapshot[] = star.peers.map((peer) => ({
    peer_id: peer.peer_id,
    slot: peer.slot,
    state: peer.state,
    ice_state: peer.ice_state,
    control: copyChannel(peer.control),
    input: copyChannel(peer.input),
    sequence_gaps: peer.sequence_gaps,
    backpressure: peer.backpressure,
    malformed: peer.malformed,
    ...optionalDetail(peer.last_error),
  }));
  recorder.peers = peers;

  const pressure: NetDiagnosticsPressure = recorder.pressure ?? {
    samples: 0,
    peak_outbound_depth: 0,
    peak_inbound_depth: 0,
    peak_buffered_amount: 0,
    peak_event_depth: 0,
    peak_peer_count: 0,
    backpressure: 0,
    peer_backpressure: 0,
    overflow: 0,
    dropped_outbound: 0,
    dropped_inbound: 0,
  };
  pressure.samples += 1;
  pressure.peak_event_depth = Math.max(pressure.peak_event_depth, star.event_depth);
  pressure.peak_peer_count = Math.max(pressure.peak_peer_count, star.peer_count);
  // The star's own counters are cumulative and monotonic, so a running
  // maximum is the same value while the star lives and survives a
  // re-initialized one.
  pressure.backpressure = Math.max(pressure.backpressure, star.backpressure);
  pressure.overflow = Math.max(pressure.overflow, star.overflow);
  pressure.dropped_outbound = Math.max(pressure.dropped_outbound, star.dropped_outbound);
  pressure.dropped_inbound = Math.max(pressure.dropped_inbound, star.dropped_inbound);
  for (const peer of peers) {
    pressure.peer_backpressure = Math.max(pressure.peer_backpressure, peer.backpressure);
    for (const channel of [peer.control, peer.input]) {
      pressure.peak_outbound_depth = Math.max(pressure.peak_outbound_depth, channel.outbound_depth);
      pressure.peak_inbound_depth = Math.max(pressure.peak_inbound_depth, channel.inbound_depth);
      pressure.peak_buffered_amount = Math.max(pressure.peak_buffered_amount, channel.buffered_amount);
    }
  }
  recorder.pressure = pressure;
  return ok(true);
}

export interface NetDiagnosticsRuntimeSample {
  readonly peer_id: string;
  readonly monotonic_ms: number;
  readonly rtt_ms?: number;
  readonly jitter_ms?: number;
}

// A wall-clock quality sample. Nothing here is ever consulted as simulation
// truth; it lives in the runtime section and the schema will not let it
// borrow a tick's vocabulary.
export function recordRuntimeSample(
  recorder: NetDiagnostics,
  sample: NetDiagnosticsRuntimeSample
): Result<true, string> {
  if (!isFinite_(sample.monotonic_ms) || sample.monotonic_ms < 0) {
    return reject(recorder, "a runtime sample needs a finite monotonic observation");
  }
  if (sample.rtt_ms !== undefined && (!isFinite_(sample.rtt_ms) || sample.rtt_ms < 0)) {
    return reject(recorder, "an rtt observation must be finite and non-negative");
  }
  if (sample.jitter_ms !== undefined && (!isFinite_(sample.jitter_ms) || sample.jitter_ms < 0)) {
    return reject(recorder, "a jitter observation must be finite and non-negative");
  }
  let entry = recorder.latency.get(sample.peer_id);
  if (entry === undefined) {
    if (recorder.latency_order.length >= recorder.limits.latency_peers) {
      return reject(recorder, "runtime samples exceeded the tracked peer cap");
    }
    entry = { peer_id: sample.peer_id, sample_count: 0 };
    recorder.latency.set(sample.peer_id, entry);
    recorder.latency_order.push(sample.peer_id);
  }
  entry.sample_count += 1;
  entry.monotonic_ms_first = entry.monotonic_ms_first ?? sample.monotonic_ms;
  entry.monotonic_ms_last = sample.monotonic_ms;
  if (sample.rtt_ms !== undefined) {
    entry.rtt_ms_last = sample.rtt_ms;
    entry.rtt_ms_min = entry.rtt_ms_min !== undefined ? Math.min(entry.rtt_ms_min, sample.rtt_ms) : sample.rtt_ms;
    entry.rtt_ms_max = entry.rtt_ms_max !== undefined ? Math.max(entry.rtt_ms_max, sample.rtt_ms) : sample.rtt_ms;
  }
  if (sample.jitter_ms !== undefined) {
    entry.jitter_ms_last = sample.jitter_ms;
    entry.jitter_ms_max =
      entry.jitter_ms_max !== undefined ? Math.max(entry.jitter_ms_max, sample.jitter_ms) : sample.jitter_ms;
  }
  return ok(true);
}

export interface NetDiagnosticsAnchorInput {
  readonly input_tick: number;
  readonly monotonic_ms: number;
  readonly mapping_error_ms: number;
}

// The only place the two clocks are allowed to meet, and it has to say how
// badly they might disagree.
export function recordAnchor(
  recorder: NetDiagnostics,
  anchor: NetDiagnosticsAnchorInput
): Result<true, string> {
  if (!isInteger(anchor.input_tick)) {
    return reject(recorder, "an anchor needs a finite integer input tick");
  }
  if (!isFinite_(anchor.monotonic_ms) || anchor.monotonic_ms < 0) {
    return reject(recorder, "an anchor needs a finite monotonic observation");
  }
  if (!isFinite_(anchor.mapping_error_ms) || anchor.mapping_error_ms < 0) {
    return reject(recorder, "an anchor must declare a finite non-negative mapping error");
  }
  ringPush(recorder.anchors, {
    input_tick: anchor.input_tick,
    monotonic_ms: anchor.monotonic_ms,
    mapping_error_ms: anchor.mapping_error_ms,
  });
  return ok(true);
}

export interface NetDiagnosticsEventInput {
  readonly kind: NetDiagnosticsEventKind;
  readonly monotonic_ms: number;
  readonly peer_id?: string;
  readonly channel?: string;
  readonly state?: string;
  readonly code?: string;
  readonly detail?: string;
}

export function recordEvent(
  recorder: NetDiagnostics,
  event: NetDiagnosticsEventInput
): Result<true, string> {
  if (!isFinite_(event.monotonic_ms) || event.monotonic_ms < 0) {
    return reject(recorder, "a runtime event needs a finite monotonic observation");
  }
  recorder.ordinal += 1;
  const detail = boundedDetail(event.detail);
  ringPush(recorder.events, {
    ordinal: recorder.ordinal,
    kind: event.kind,
    monotonic_ms: event.monotonic_ms,
    ...(event.peer_id !== undefined ? { peer_id: event.peer_id } : {}),
    ...(event.channel !== undefined ? { channel: event.channel } : {}),
    ...(event.state !== undefined ? { state: event.state } : {}),
    ...(event.code !== undefined ? { code: event.code } : {}),
    ...(detail !== null ? { detail } : {}),
  });
  return ok(true);
}

// Signalling blobs carry SDP and ICE credentials, and SDP candidate lines
// carry raw host and server-reflexive addresses. The blob itself is
// therefore never stored -- not truncated, not digested, not hashed. What is
// useful for debugging is that an exchange happened, in which direction, and
// how large it was, and that is exactly what this keeps.
export function recordSignal(
  recorder: NetDiagnostics,
  peerId: string,
  direction: NetDiagnosticsSignalDirection,
  signal: string
): Result<true, string> {
  recorder.ordinal += 1;
  ringPush(recorder.signals, {
    ordinal: recorder.ordinal,
    peer_id: peerId,
    direction,
    byte_length: signal.length,
    content: schema.REDACTED,
  });
  return ok(true);
}

export interface NetDiagnosticsTeardownInput {
  readonly requested?: boolean;
  readonly closed_peers: number;
  readonly orphaned_peers?: readonly string[];
  readonly residual_outbound?: number;
  readonly residual_inbound?: number;
}

// Teardown completeness. `orphaned_peers` are links the star still reports
// after shutdown was requested; residual queue depth is what never drained.
export function recordTeardown(
  recorder: NetDiagnostics,
  teardown: NetDiagnosticsTeardownInput
): Result<true, string> {
  if (!isInteger(teardown.closed_peers)) {
    return reject(recorder, "teardown needs a finite closed peer count");
  }
  const orphaned = [...(teardown.orphaned_peers ?? [])];
  const residualOutbound = teardown.residual_outbound ?? 0;
  const residualInbound = teardown.residual_inbound ?? 0;
  recorder.teardown = {
    requested: teardown.requested === true,
    complete: orphaned.length === 0 && residualOutbound === 0 && residualInbound === 0,
    closed_peers: teardown.closed_peers,
    residual_outbound: residualOutbound,
    residual_inbound: residualInbound,
    orphaned_peers: orphaned,
  };
  return ok(true);
}

// ---------------------------------------------------------------------------
// Reading back
// ---------------------------------------------------------------------------

function copySession(recorder: NetDiagnostics): Record<string, unknown> {
  return { ...recorder.session };
}

function latencyRows(recorder: NetDiagnostics): NetDiagnosticsLatencyEntry[] {
  // Sorted, never insertion-ordered: an export whose row order depends on
  // which peer happened to answer first is not stable across runs.
  const ids = [...recorder.latency_order].sort();
  const rows: NetDiagnosticsLatencyEntry[] = [];
  for (const peerId of ids) {
    const entry = recorder.latency.get(peerId);
    if (entry !== undefined) {
      rows.push({ ...entry });
    }
  }
  return rows;
}

// The full artifact. Rejects unless export was opted into: this is the seam
// between "we are holding diagnostics" and "we are producing something that
// can leave the machine".
export function exportArtifact(recorder: NetDiagnostics): Result<Record<string, unknown>, string> {
  if (!recorder.export_opt_in) {
    return err("diagnostic export was not opted into");
  }
  const artifact: Record<string, unknown> = {
    schema_version: SCHEMA_VERSION,
    digest_algorithm: schema.DIGEST,
    session: copySession(recorder),
    collection: {
      export_opt_in: true,
      rejected_values: recorder.rejected,
      caps: { ...recorder.limits },
      dropped: {
        checkpoints: recorder.checkpoints.dropped,
        packets: recorder.packets.dropped,
        control: recorder.control.dropped,
        events: recorder.events.dropped,
        signals: recorder.signals.dropped,
        mismatches: recorder.mismatches.dropped,
        anchors: recorder.anchors.dropped,
      },
      retention: {
        checkpoints: ringRetention(recorder.checkpoints),
        packets: ringRetention(recorder.packets),
        control: ringRetention(recorder.control),
        events: ringRetention(recorder.events),
        signals: ringRetention(recorder.signals),
        mismatches: ringRetention(recorder.mismatches),
        anchors: ringRetention(recorder.anchors),
      },
    },
    canonical: {
      simulation: { ...recorder.simulation },
      worst_correction: recorder.worst !== null ? { ...recorder.worst } : undefined,
      events: { ...recorder.events_counts },
      delivery: { ...recorder.delivery, packets: ringItems(recorder.packets) },
      checkpoints: ringItems(recorder.checkpoints),
      mismatches: ringItems(recorder.mismatches),
      control: ringItems(recorder.control),
    },
    runtime: {
      star: recorder.star !== null ? { ...recorder.star } : undefined,
      peers: recorder.peers,
      pressure: recorder.pressure !== null ? { ...recorder.pressure } : undefined,
      latency: latencyRows(recorder),
      events: ringItems(recorder.events),
      signals: ringItems(recorder.signals),
      teardown: recorder.teardown !== null ? { ...recorder.teardown } : undefined,
    },
    anchors: ringItems(recorder.anchors),
  };
  const validated = schema.validate(EXPORT, artifact);
  if (!validated.ok) {
    return err(validated.error);
  }
  return ok(artifact);
}

export function encode(recorder: NetDiagnostics): Result<string, string> {
  const artifact = exportArtifact(recorder);
  if (!artifact.ok) {
    return artifact;
  }
  return schema.encode(EXPORT, artifact.value);
}

export function digest(recorder: NetDiagnostics): Result<string, string> {
  const artifact = exportArtifact(recorder);
  if (!artifact.ok) {
    return artifact;
  }
  return schema.digest(EXPORT, artifact.value);
}

// The deterministic half of the artifact: identity plus canonical evidence,
// with runtime observation and clock anchors projected out.
//
// This exists because the two halves deserve different claims. Two runs of
// the same fixture over the same delivery schedule produce byte-identical
// canonical evidence, and that is worth pinning. The same two runs produce
// *different* RTT, jitter, and monotonic timestamps on any real machine, and
// pinning those would either be a lie or a test that only passes because the
// harness froze the clock. So: byte-identity here, invariants over there.
export const CANONICAL_PROJECTION = schema.project(EXPORT, { identity: true, canonical: true });

export function canonicalBytes(recorder: NetDiagnostics): Result<string, string> {
  const artifact = exportArtifact(recorder);
  if (!artifact.ok) {
    return artifact;
  }
  const value = artifact.value;
  return schema.encode(CANONICAL_PROJECTION, {
    schema_version: value.schema_version,
    digest_algorithm: value.digest_algorithm,
    session: value.session,
    collection: value.collection,
    canonical: value.canonical,
  });
}

export function canonicalDigest(recorder: NetDiagnostics): Result<string, string> {
  const bytes = canonicalBytes(recorder);
  if (!bytes.ok) {
    return bytes;
  }
  return ok(schema.tupleDigest("net_diagnostics_canonical", [bytes.value]));
}

// A short human-readable read-out for an in-game debug overlay. It reads
// only fields the export already carries, it never needs the export opt-in
// (nothing leaves the machine), and it keeps the two clocks in separate
// lines with separate units so a screenshot cannot blur them either.
export function summary(recorder: NetDiagnostics): string[] {
  const simulation = recorder.simulation;
  const delivery = recorder.delivery;
  const lines: string[] = [
    `session ${recorder.session.manifest_id}  peer ${recorder.session.peer_id}  ${recorder.session.role}  ${recorder.session.match_mode}`,
    `sim  tick ${simulation.present_input_tick}  confirmed ${simulation.confirmed_output_tick}  floor ${simulation.retained_floor_tick}  delay ${simulation.input_delay_ticks}`,
    `sim  rollbacks ${simulation.rollback_count}  worst depth ${simulation.max_rollback_depth}  resimulated ${simulation.resimulated_ticks}  corrections ${simulation.correction_count}`,
    `sim  checkpoints ${simulation.checkpoint_count}  mismatch streak ${simulation.hash_mismatch_streak}  status ${simulation.status}`,
    `wire  sent ${delivery.sent}  arrived ${delivery.arrived}  applied rows ${delivery.applied_rows}  reconciliations ${delivery.reconciliations}`,
  ];
  const star = recorder.star;
  if (star !== null) {
    lines.push(
      `wire  ${star.state}  peers ${star.peer_count}  dropped ${star.dropped_outbound}/${star.dropped_inbound}  backpressure ${star.backpressure}`
    );
  }
  for (const row of latencyRows(recorder)) {
    lines.push(
      `clock  ${row.peer_id}  rtt ${row.rtt_ms_last !== undefined ? row.rtt_ms_last.toFixed(1) : "n/a"} ms  jitter ${
        row.jitter_ms_last !== undefined ? row.jitter_ms_last.toFixed(1) : "n/a"
      } ms  (observed, not simulation)`
    );
  }
  const truncated: string[] = [];
  const rings: readonly [string, NetDiagnosticsRing<unknown>][] = [
    ["checkpoints", recorder.checkpoints],
    ["packets", recorder.packets],
    ["control", recorder.control],
    ["events", recorder.events],
    ["signals", recorder.signals],
    ["mismatches", recorder.mismatches],
    ["anchors", recorder.anchors],
  ];
  for (const [name, ring] of rings) {
    if (ring.dropped > 0) {
      truncated.push(`${name} -${ring.dropped}`);
    }
  }
  if (truncated.length > 0) {
    lines.push("truncated  " + truncated.join("  "));
  }
  if (recorder.rejected > 0) {
    lines.push(`rejected  ${recorder.rejected} invalid values`);
  }
  return lines;
}
