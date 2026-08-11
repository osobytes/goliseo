// Ported from game/webrtc_proof.lua.
//
// Pure protocol and diagnostics helpers for the OMP-0 WebRTC proof. Browser
// APIs stay outside this module; it is reusable by a later game-facing
// adapter without importing DOM or WebRTC types.

import { type Result, err, ok } from "@gc/core";

export type WebRTCProofRole = "host" | "guest";
export type WebRTCProofErrorCode =
  | "malformed"
  | "unsupported_version"
  | "build_mismatch"
  | "role_mismatch"
  | "input_too_large"
  | "non_monotonic_tick";

export interface WebRTCProofFailure {
  readonly message: string;
  readonly code: WebRTCProofErrorCode;
}

export interface WebRTCHandshake {
  readonly kind: "handshake";
  readonly version: number;
  readonly build_id: string;
  readonly role: WebRTCProofRole;
}

export interface WebRTCInput {
  readonly kind: "input";
  readonly version: number;
  readonly seq: number;
  readonly tick: number;
  readonly payload: string;
  readonly history: readonly number[];
}

export interface WebRTCProofDiagnostics {
  readonly startedAt: number;
  sent: number;
  received: number;
  uniqueTicks: number;
  historyRetransmits: number;
  sequenceGaps: number;
  outOfOrder: number;
  dropped: number;
  queueDepth: number;
  maxQueueDepth: number;
  rttSamples: number[];
  jitterSamples: number[];
  lastReceivedSeq?: number;
  lastReceivedTick?: number;
  readonly seenTicks: Map<number, boolean>;
}

export interface WebRTCProofSummary {
  readonly durationS: number;
  readonly sendRate: number;
  readonly receiveRate: number;
  readonly sent: number;
  readonly received: number;
  readonly uniqueTicks: number;
  readonly historyRetransmits: number;
  readonly sequenceGaps: number;
  readonly outOfOrder: number;
  readonly dropped: number;
  readonly queueDepth: number;
  readonly maxQueueDepth: number;
  readonly rttP50Ms?: number;
  readonly rttP95Ms?: number;
  readonly rttMaxMs?: number;
  readonly jitterP50Ms?: number;
  readonly jitterP95Ms?: number;
  readonly jitterMaxMs?: number;
}

export const VERSION = 1;
export const MAX_HISTORY = 6;
export const MAX_INPUT_BYTES = 128;

const ROLES: ReadonlySet<string> = new Set(["host", "guest"]);

function isInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value === Math.floor(value);
}

function failure(code: WebRTCProofErrorCode, message: string): Result<never, WebRTCProofFailure> {
  return err({ message, code });
}

function percentile(values: readonly number[], fraction: number): number | undefined {
  if (values.length === 0) {
    return undefined;
  }
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.max(1, Math.ceil(sorted.length * fraction)) - 1;
  return sorted[index];
}

function maximum(values: readonly number[]): number | undefined {
  let result: number | undefined;
  for (const value of values) {
    if (result === undefined || value > result) {
      result = value;
    }
  }
  return result;
}

export function newHandshake(role: unknown, buildId: unknown): Result<WebRTCHandshake, WebRTCProofFailure> {
  if (typeof role !== "string" || !ROLES.has(role)) {
    return failure("role_mismatch", "WebRTC proof role must be host or guest");
  }
  if (typeof buildId !== "string" || buildId === "") {
    return failure("malformed", "WebRTC proof build_id must be a non-empty string");
  }
  return ok({ kind: "handshake", version: VERSION, build_id: buildId, role: role as WebRTCProofRole });
}

export function validateHandshake(
  message: unknown,
  expectedVersion?: number,
  expectedBuildId?: string,
  expectedPeerRole?: WebRTCProofRole,
): Result<true, WebRTCProofFailure> {
  if (typeof message !== "object" || message === null || (message as { kind?: unknown }).kind !== "handshake") {
    return failure("malformed", "WebRTC proof handshake is malformed");
  }
  const handshake = message as Record<string, unknown>;
  if (!isInteger(handshake.version)) {
    return failure("malformed", "WebRTC proof handshake version must be an integer");
  }
  if (handshake.version !== (expectedVersion ?? VERSION)) {
    return failure("unsupported_version", "WebRTC proof protocol version is unsupported");
  }
  if (typeof handshake.build_id !== "string" || handshake.build_id === "") {
    return failure("malformed", "WebRTC proof handshake build_id is missing");
  }
  if (expectedBuildId !== undefined && handshake.build_id !== expectedBuildId) {
    return failure("build_mismatch", "WebRTC proof build identity does not match");
  }
  if (typeof handshake.role !== "string" || !ROLES.has(handshake.role)) {
    return failure("role_mismatch", "WebRTC proof handshake role is invalid");
  }
  if (expectedPeerRole !== undefined && handshake.role !== expectedPeerRole) {
    return failure("role_mismatch", "WebRTC proof peer role does not match");
  }
  return ok(true);
}

export function newInput(
  seq: unknown,
  tick: unknown,
  payload: unknown,
  history?: unknown,
): Result<WebRTCInput, WebRTCProofFailure> {
  if (!isInteger(seq) || seq < 0 || !isInteger(tick) || tick < 0) {
    return failure("malformed", "WebRTC proof input sequence and tick must be non-negative integers");
  }
  if (typeof payload !== "string") {
    return failure("malformed", "WebRTC proof input payload must be a string");
  }
  if (payload.length > MAX_INPUT_BYTES) {
    return failure("input_too_large", "WebRTC proof input payload exceeds 128 bytes");
  }
  const resolvedHistory = history ?? [];
  if (!Array.isArray(resolvedHistory) || resolvedHistory.length > MAX_HISTORY) {
    return failure("malformed", "WebRTC proof input history exceeds six ticks");
  }
  for (const historyTick of resolvedHistory) {
    if (!isInteger(historyTick) || historyTick < 0 || historyTick >= tick) {
      return failure("malformed", "WebRTC proof input history must contain earlier ticks");
    }
  }
  return ok({ kind: "input", version: VERSION, seq, tick, payload, history: resolvedHistory as number[] });
}

export function validateInput(message: unknown): Result<true, WebRTCProofFailure> {
  if (typeof message !== "object" || message === null || (message as { kind?: unknown }).kind !== "input") {
    return failure("malformed", "WebRTC proof input is malformed");
  }
  const input = message as Record<string, unknown>;
  if (input.version !== VERSION) {
    return failure("unsupported_version", "WebRTC proof input version is unsupported");
  }
  const result = newInput(input.seq, input.tick, input.payload, input.history);
  if (!result.ok) {
    return result;
  }
  return ok(true);
}

export function newDiagnostics(now: number): WebRTCProofDiagnostics {
  return {
    startedAt: now,
    sent: 0,
    received: 0,
    uniqueTicks: 0,
    historyRetransmits: 0,
    sequenceGaps: 0,
    outOfOrder: 0,
    dropped: 0,
    queueDepth: 0,
    maxQueueDepth: 0,
    rttSamples: [],
    jitterSamples: [],
    seenTicks: new Map(),
  };
}

export function recordSent(diagnostics: WebRTCProofDiagnostics, _now: number, queueDepth: number): void {
  diagnostics.sent += 1;
  diagnostics.queueDepth = queueDepth;
  diagnostics.maxQueueDepth = Math.max(diagnostics.maxQueueDepth, queueDepth);
}

export function recordReceived(diagnostics: WebRTCProofDiagnostics, _now: number, input: WebRTCInput): void {
  diagnostics.received += 1;
  if (diagnostics.lastReceivedSeq !== undefined) {
    if (input.seq > diagnostics.lastReceivedSeq + 1) {
      diagnostics.sequenceGaps += input.seq - diagnostics.lastReceivedSeq - 1;
    } else if (input.seq <= diagnostics.lastReceivedSeq) {
      diagnostics.outOfOrder += 1;
    }
  }
  const history = input.history;
  for (const historyTick of history) {
    if (!diagnostics.seenTicks.get(historyTick)) {
      diagnostics.seenTicks.set(historyTick, true);
      diagnostics.uniqueTicks += 1;
      diagnostics.historyRetransmits += 1;
    }
  }
  if (!diagnostics.seenTicks.get(input.tick)) {
    diagnostics.seenTicks.set(input.tick, true);
    diagnostics.uniqueTicks += 1;
  }
  diagnostics.lastReceivedSeq = Math.max(diagnostics.lastReceivedSeq ?? input.seq, input.seq);
  diagnostics.lastReceivedTick = Math.max(diagnostics.lastReceivedTick ?? input.tick, input.tick);
}

export function recordRtt(diagnostics: WebRTCProofDiagnostics, rttMs: number): void {
  if (rttMs < 0) {
    return;
  }
  const previous = diagnostics.rttSamples[diagnostics.rttSamples.length - 1];
  diagnostics.rttSamples.push(rttMs);
  if (previous !== undefined) {
    diagnostics.jitterSamples.push(Math.abs(rttMs - previous));
  }
}

export function summary(diagnostics: WebRTCProofDiagnostics, now: number): WebRTCProofSummary {
  const duration = Math.max(0, now - diagnostics.startedAt);
  const rttP50Ms = percentile(diagnostics.rttSamples, 0.5);
  const rttP95Ms = percentile(diagnostics.rttSamples, 0.95);
  const rttMaxMs = maximum(diagnostics.rttSamples);
  const jitterP50Ms = percentile(diagnostics.jitterSamples, 0.5);
  const jitterP95Ms = percentile(diagnostics.jitterSamples, 0.95);
  const jitterMaxMs = maximum(diagnostics.jitterSamples);
  return {
    durationS: duration,
    sendRate: diagnostics.sent / Math.max(duration, 0.001),
    receiveRate: diagnostics.received / Math.max(duration, 0.001),
    sent: diagnostics.sent,
    received: diagnostics.received,
    uniqueTicks: diagnostics.uniqueTicks,
    historyRetransmits: diagnostics.historyRetransmits,
    sequenceGaps: diagnostics.sequenceGaps,
    outOfOrder: diagnostics.outOfOrder,
    dropped: diagnostics.dropped,
    queueDepth: diagnostics.queueDepth,
    maxQueueDepth: diagnostics.maxQueueDepth,
    ...(rttP50Ms !== undefined ? { rttP50Ms } : {}),
    ...(rttP95Ms !== undefined ? { rttP95Ms } : {}),
    ...(rttMaxMs !== undefined ? { rttMaxMs } : {}),
    ...(jitterP50Ms !== undefined ? { jitterP50Ms } : {}),
    ...(jitterP95Ms !== undefined ? { jitterP95Ms } : {}),
    ...(jitterMaxMs !== undefined ? { jitterMaxMs } : {}),
  };
}

export const webrtcProof = {
  VERSION,
  MAX_HISTORY,
  MAX_INPUT_BYTES,
  newHandshake,
  validateHandshake,
  newInput,
  validateInput,
  newDiagnostics,
  recordSent,
  recordReceived,
  recordRtt,
  summary,
};
