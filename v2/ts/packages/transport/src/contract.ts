// The transport interface every implementation satisfies: envelope shape,
// wire encoding, and the peer-addressed framing the star and relay adapters
// share. Ported from game/transport/contract.lua.

import { type Result, ok, err } from "@gc/core";

export type TransportMessageType = "input" | "event" | "state";
export type TransportState = "new" | "connected" | "disconnected" | "closed" | "error";
export type TransportRole = "host" | "guest";
export type TransportChannel = "control" | "input";
export type TransportPeerState =
  | "new"
  | "connecting"
  | "connected"
  | "disconnected"
  | "closed"
  | "error";
export type TransportErrorCode =
  | "not_initialized"
  | "not_connected"
  | "closed"
  | "malformed"
  | "unsupported_version"
  | "payload_too_large"
  | "overflow"
  | "bridge_unavailable"
  | "bridge_error"
  | "disconnected"
  | "capacity"
  | "unknown_peer"
  | "duplicate_peer"
  | "role_forbidden"
  | "channel_mismatch"
  | "backpressure"
  | "signal_error";

/**
 * The failure half of every transport operation: a human-readable message
 * plus the typed code callers branch on. Lua returns this as `nil, err,
 * code`; here it is the error arm of a `Result`.
 */
export interface TransportFailure {
  readonly message: string;
  readonly code: TransportErrorCode;
}

/** A transport operation's outcome: `T` on success, a typed failure on not. */
export type TransportResult<T> = Result<T, TransportFailure>;

export interface TransportMessage {
  readonly version: number;
  readonly type: TransportMessageType;
  /** Monotonic transport sequence, starting at zero. */
  readonly seq: number;
  /** Required for input messages; omitted for control messages. */
  readonly tick?: number;
  /** UTF-8 payload owned by the next protocol layer. */
  readonly payload: string;
}

export interface TransportMessageOptions {
  readonly version?: number;
  readonly type: TransportMessageType;
  readonly seq: number;
  readonly tick?: number;
  readonly payload: string;
}

export interface TransportEvent {
  readonly kind: "state" | "error";
  readonly state?: TransportState;
  readonly code?: TransportErrorCode;
  readonly message?: string;
}

export interface TransportDiagnostics {
  readonly state: TransportState;
  readonly queue_limit: number;
  readonly outbound_depth: number;
  readonly inbound_depth: number;
  readonly event_depth: number;
  readonly dropped_outbound: number;
  readonly dropped_inbound: number;
  readonly malformed: number;
  readonly unsupported_version: number;
  readonly overflow: number;
  readonly sent: number;
  readonly received: number;
  readonly last_error: string | null;
}

export interface TransportChannelDiagnostics {
  readonly state: TransportPeerState;
  readonly outbound_depth: number;
  readonly inbound_depth: number;
  readonly buffered_amount: number;
  readonly sent: number;
  readonly received: number;
  readonly dropped_outbound: number;
  readonly dropped_inbound: number;
}

export interface TransportPeerDiagnostics {
  readonly peer_id: string;
  /** Stable star slot, 1..MAX_GUESTS on a host, always 1 on a guest. */
  readonly slot: number;
  readonly state: TransportPeerState;
  readonly ice_state: string;
  readonly control: TransportChannelDiagnostics;
  readonly input: TransportChannelDiagnostics;
  readonly sequence_gaps: number;
  readonly backpressure: number;
  readonly malformed: number;
  readonly last_error: string | null;
}

export interface TransportStarDiagnostics {
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
  readonly last_error: string | null;
  /** Ordered by slot, never by arrival. */
  readonly peers: readonly TransportPeerDiagnostics[];
}

export interface TransportAddressedMessage {
  /** Link identity chosen by the star, never read from the payload. */
  readonly peer_id: string;
  readonly channel: TransportChannel;
  readonly message: TransportMessage;
}

export interface TransportPeerMessage extends TransportAddressedMessage {
  /** Per-peer transport receive counter; not a simulation tick. */
  readonly arrival_seq: number;
}

export interface TransportPeerEvent {
  readonly kind: "peer_state" | "peer_error" | "star_state" | "star_error";
  readonly peer_id?: string;
  readonly channel?: TransportChannel;
  readonly state?: TransportPeerState;
  readonly code?: TransportErrorCode;
  readonly message?: string;
}

/**
 * The host-star adapter contract: one host endpoint, up to `MAX_GUESTS`
 * independently addressed guest links. `fake_star`, `fake_relay`, and
 * `browser_star` all satisfy this.
 */
export interface StarTransportAdapter {
  initialize(): TransportResult<true>;
  shutdown(): TransportResult<true>;
  role(): TransportRole;
  capacity(): number;
  openPeer(peerId: string): TransportResult<number>;
  closePeer(peerId: string, reason?: string): TransportResult<true>;
  peerIds(): readonly string[];
  peerState(peerId: string): TransportPeerState | null;
  // Manual signaling. Offer/answer creation is asynchronous in the browser,
  // so these only start the work; `takeSignal` yields the blob on a later
  // frame.
  requestOffer(peerId: string): TransportResult<true>;
  acceptOffer(signal: string): TransportResult<true>;
  acceptAnswer(peerId: string, signal: string): TransportResult<true>;
  takeSignal(peerId: string): TransportResult<string | null>;
  send(peerId: string, channel: TransportChannel, message: TransportMessage): TransportResult<true>;
  broadcast(channel: TransportChannel, message: TransportMessage): TransportResult<number>;
  poll(): TransportResult<TransportPeerMessage | null>;
  pollBatch(limit?: number): readonly TransportPeerMessage[];
  pollEvent(): TransportPeerEvent | null;
  state(): TransportState;
  diagnostics(): TransportStarDiagnostics;
}

/** The point-to-point adapter contract: one link, one queue each way. */
export interface TransportAdapter {
  initialize(): TransportResult<true>;
  shutdown(): TransportResult<true>;
  enqueue(message: TransportMessage): TransportResult<true>;
  send(message: TransportMessage): TransportResult<true>;
  poll(): TransportResult<TransportMessage | null>;
  pollEvent(): TransportEvent | null;
  state(): TransportState;
  diagnostics(): TransportDiagnostics;
}

export const VERSION = 1;
export const DEFAULT_QUEUE_LIMIT = 64;
export const MAX_QUEUE_LIMIT = 256;
export const MAX_PAYLOAD_BYTES = 1024;

// Host-star bounds. One host endpoint addresses at most seven guest links.
export const MAX_GUESTS = 7;
export const HOST_PEER_ID = "host";
export const MAX_PEER_ID_BYTES = 128;
export const PEER_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]*$/;
export const DEFAULT_BUFFERED_AMOUNT_LIMIT = 65536;
export const MAX_BUFFERED_AMOUNT_LIMIT = 1048576;
export const DEFAULT_POLL_BATCH = 32;

export interface ChannelConfig {
  readonly label: TransportChannel;
  readonly ordered: boolean;
  readonly reliable?: boolean;
  readonly maxRetransmits?: number;
  readonly rank: number;
}

// Channel configuration is part of the contract: the control channel is
// reliable and ordered, the input channel is unordered and never
// retransmits. `rank` fixes the deterministic drain order at this boundary.
export const CHANNEL_CONFIG: Readonly<Record<TransportChannel, ChannelConfig>> = {
  control: { label: "control", ordered: true, reliable: true, rank: 1 },
  input: { label: "input", ordered: false, maxRetransmits: 0, rank: 2 },
};
export const CHANNEL_ORDER: readonly TransportChannel[] = ["control", "input"];

// Channel/message-type pairing. Control carries lifecycle and session
// control payloads; input carries tick-numbered input packets and nothing
// else.
export const CHANNEL_MESSAGE_TYPES: Readonly<
  Record<TransportChannel, Readonly<Partial<Record<TransportMessageType, true>>>>
> = {
  control: { event: true, state: true },
  input: { input: true },
};

const MESSAGE_TYPES: Readonly<Record<TransportMessageType, true>> = {
  input: true,
  event: true,
  state: true,
};

function isInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value === Math.floor(value);
}

function failure<T>(code: TransportErrorCode, message: string): TransportResult<T> {
  return err({ message, code });
}

/** Structurally the same shape `TransportMessage` requires, prior to validation. */
export interface UnvalidatedMessage {
  readonly version?: unknown;
  readonly type?: unknown;
  readonly seq?: unknown;
  readonly tick?: unknown;
  readonly payload?: unknown;
}

export function validate(message: unknown): TransportResult<true> {
  if (typeof message !== "object" || message === null) {
    return failure("malformed", "transport message must be a table");
  }
  const candidate = message as UnvalidatedMessage;
  if (!isInteger(candidate.version)) {
    return failure("malformed", "transport message version must be an integer");
  }
  if (candidate.version !== VERSION) {
    return failure("unsupported_version", "unsupported transport message version");
  }
  if (typeof candidate.type !== "string" || !(candidate.type in MESSAGE_TYPES)) {
    return failure("malformed", "transport message type is invalid");
  }
  if (!isInteger(candidate.seq) || candidate.seq < 0) {
    return failure("malformed", "transport message seq must be a non-negative integer");
  }
  if (candidate.type === "input") {
    if (!isInteger(candidate.tick) || candidate.tick < 0) {
      return failure("malformed", "input message tick must be a non-negative integer");
    }
  } else if (candidate.tick !== undefined && (!isInteger(candidate.tick) || candidate.tick < 0)) {
    return failure("malformed", "transport message tick must be a non-negative integer");
  }
  if (typeof candidate.payload !== "string") {
    return failure("malformed", "transport message payload must be a string");
  }
  if (byteLength(candidate.payload) > MAX_PAYLOAD_BYTES) {
    return failure("payload_too_large", "transport message payload exceeds the byte limit");
  }
  return ok(true);
}

// Lua strings are raw byte sequences, not Unicode text — `#string` counts
// bytes, and a payload may carry any byte value, valid UTF-8 or not (the
// spec exercises a bare `\255`). The wire layer is ported the same way: a JS
// string here is a "binary string", one JS UTF-16 code unit per byte (code
// points 0-255 only), the same convention `Buffer.from(s, "binary")` uses.
// The layer above this one is responsible for UTF-8 encoding/decoding its
// own payload text before it reaches `contract`.
function byteLength(value: string): number {
  return value.length;
}

export function newMessage(options: TransportMessageOptions): TransportResult<TransportMessage> {
  const message: TransportMessage = {
    version: options.version ?? VERSION,
    type: options.type,
    seq: options.seq,
    ...(options.tick !== undefined ? { tick: options.tick } : {}),
    payload: options.payload,
  };
  const validated = validate(message);
  if (!validated.ok) {
    return err(validated.error);
  }
  return ok(message);
}

/** Throws if `message` fails validation — mirrors the Lua `assert(ok, err)`. */
export function copy(message: TransportMessage): TransportMessage {
  const validated = validate(message);
  if (!validated.ok) {
    throw new Error(validated.error.message);
  }
  return {
    version: message.version,
    type: message.type,
    seq: message.seq,
    ...(message.tick !== undefined ? { tick: message.tick } : {}),
    payload: message.payload,
  };
}

function escape(value: string): string {
  let out = "";
  for (let index = 0; index < value.length; index += 1) {
    const char = value[index] as string;
    if (/^[A-Za-z0-9\-._~]$/.test(char)) {
      out += char;
    } else {
      const byte = value.charCodeAt(index) & 0xff;
      out += "%" + byte.toString(16).toUpperCase().padStart(2, "0");
    }
  }
  return out;
}

function unescape(value: string): string | null {
  let out = "";
  let index = 0;
  while (index < value.length) {
    const character = value[index];
    if (character === "%") {
      const hex = value.slice(index + 1, index + 3);
      if (hex.length !== 2 || !/^[0-9A-Fa-f]{2}$/.test(hex)) {
        return null;
      }
      out += String.fromCharCode(parseInt(hex, 16));
      index += 3;
    } else {
      out += character;
      index += 1;
    }
  }
  return out;
}

function parseInteger(value: string): number {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    throw new Error(`transport wire field is not a number: ${value}`);
  }
  return number;
}

export function encode(message: TransportMessage): TransportResult<string> {
  const validated = validate(message);
  if (!validated.ok) {
    return err(validated.error);
  }
  return ok(
    [
      String(message.version),
      escape(message.type),
      String(message.seq),
      message.tick !== undefined ? String(message.tick) : "",
      escape(message.payload),
    ].join("|")
  );
}

export function decode(wire: unknown): TransportResult<TransportMessage> {
  if (typeof wire !== "string") {
    return failure("malformed", "transport wire message must be a string");
  }
  const match = /^([^|]*)\|([^|]*)\|([^|]*)\|([^|]*)\|([^|]*)$/.exec(wire);
  if (!match) {
    return failure("malformed", "transport wire message has invalid fields");
  }
  const [, rawVersion, rawType, rawSeq, rawTick, rawPayload] = match as unknown as [
    string,
    string,
    string,
    string,
    string,
    string,
  ];
  if (!/^\d+$/.test(rawVersion) || !/^\d+$/.test(rawSeq)) {
    return failure("malformed", "transport wire message numbers are invalid");
  }
  if (rawTick !== "" && !/^\d+$/.test(rawTick)) {
    return failure("malformed", "transport wire message tick is invalid");
  }
  const messageType = unescape(rawType);
  const payload = unescape(rawPayload);
  if (messageType === null || payload === null) {
    return failure("malformed", "transport wire escape is invalid");
  }
  const tick = rawTick !== "" ? parseInteger(rawTick) : undefined;
  return newMessage({
    version: parseInteger(rawVersion),
    type: messageType as TransportMessageType,
    seq: parseInteger(rawSeq),
    ...(tick !== undefined ? { tick } : {}),
    payload,
  });
}

function split(value: string, separator: string): string[] {
  const fields: string[] = [];
  let start = 0;
  for (;;) {
    const index = value.indexOf(separator, start);
    if (index === -1) {
      fields.push(value.slice(start));
      return fields;
    }
    fields.push(value.slice(start, index));
    start = index + separator.length;
  }
}

export function validatePeerId(peerId: unknown): TransportResult<true> {
  if (typeof peerId !== "string" || peerId === "") {
    return failure("malformed", "transport peer id must be a non-empty string");
  }
  if (byteLength(peerId) > MAX_PEER_ID_BYTES) {
    return failure("malformed", "transport peer id exceeds the byte limit");
  }
  if (!PEER_ID_PATTERN.test(peerId)) {
    return failure("malformed", "transport peer id contains unsupported characters");
  }
  return ok(true);
}

export function validateChannel(channel: unknown): TransportResult<true> {
  if (typeof channel !== "string" || !(channel in CHANNEL_CONFIG)) {
    return failure("channel_mismatch", "transport channel must be control or input");
  }
  return ok(true);
}

// Enforces the channel/message-type pairing before anything reaches a data
// channel, so an input packet can never take the reliable ordered path and a
// lifecycle message can never take the lossy path.
export function validateChannelMessage(
  channel: unknown,
  message: unknown
): TransportResult<true> {
  const channelResult = validateChannel(channel);
  if (!channelResult.ok) {
    return channelResult;
  }
  const messageResult = validate(message);
  if (!messageResult.ok) {
    return messageResult;
  }
  const typedChannel = channel as TransportChannel;
  const typedMessage = message as TransportMessage;
  if (!CHANNEL_MESSAGE_TYPES[typedChannel][typedMessage.type]) {
    return failure(
      "channel_mismatch",
      `transport ${typedChannel} channel does not carry ${typedMessage.type} messages`
    );
  }
  return ok(true);
}

// Peer-addressed wire framing: `peer_id|channel|<envelope wire>`. Peer ids
// and channel names never contain a pipe, so the envelope tail is recovered
// by splitting only the first two separators.
export function encodeAddressed(
  peerId: string,
  channel: TransportChannel,
  message: TransportMessage
): TransportResult<string> {
  const peerResult = validatePeerId(peerId);
  if (!peerResult.ok) {
    return peerResult;
  }
  const channelMessageResult = validateChannelMessage(channel, message);
  if (!channelMessageResult.ok) {
    return channelMessageResult;
  }
  const wireResult = encode(message);
  if (!wireResult.ok) {
    return wireResult;
  }
  return ok(`${peerId}|${channel}|${wireResult.value}`);
}

export function decodeAddressed(line: unknown): TransportResult<TransportAddressedMessage> {
  if (typeof line !== "string") {
    return failure("malformed", "transport addressed wire must be a string");
  }
  const match = /^([^|]*)\|([^|]*)\|([\s\S]*)$/.exec(line);
  if (!match) {
    return failure("malformed", "transport addressed wire has invalid fields");
  }
  const [, peerId, channel, wire] = match as unknown as [string, string, string, string];
  const peerResult = validatePeerId(peerId);
  if (!peerResult.ok) {
    return peerResult;
  }
  const channelResult = validateChannel(channel);
  if (!channelResult.ok) {
    return channelResult;
  }
  const messageResult = decode(wire);
  if (!messageResult.ok) {
    return messageResult;
  }
  const typedChannel = channel as TransportChannel;
  const message = messageResult.value;
  if (!CHANNEL_MESSAGE_TYPES[typedChannel][message.type]) {
    return failure(
      "channel_mismatch",
      `transport ${typedChannel} channel does not carry ${message.type} messages`
    );
  }
  return ok({ peer_id: peerId, channel: typedChannel, message });
}

function channelFields(diagnostics: TransportChannelDiagnostics): string[] {
  return [
    diagnostics.state,
    String(diagnostics.outbound_depth),
    String(diagnostics.inbound_depth),
    String(diagnostics.buffered_amount),
    String(diagnostics.sent),
    String(diagnostics.received),
    String(diagnostics.dropped_outbound),
    String(diagnostics.dropped_inbound),
  ];
}

// Line-oriented star diagnostics: one `star` record followed by one `peer`
// record per slot. The JavaScript bridge emits the same shape.
export function encodeStarDiagnostics(diagnostics: TransportStarDiagnostics): string {
  const lines = [
    [
      "star",
      diagnostics.role,
      diagnostics.state,
      String(diagnostics.capacity),
      String(diagnostics.peer_count),
      String(diagnostics.queue_limit),
      String(diagnostics.buffered_amount_limit),
      String(diagnostics.event_depth),
      String(diagnostics.sent),
      String(diagnostics.received),
      String(diagnostics.dropped_outbound),
      String(diagnostics.dropped_inbound),
      String(diagnostics.malformed),
      String(diagnostics.unsupported_version),
      String(diagnostics.overflow),
      String(diagnostics.backpressure),
      escape(diagnostics.last_error ?? ""),
    ].join("|"),
  ];
  for (const peer of diagnostics.peers) {
    const fields = [
      "peer",
      peer.peer_id,
      String(peer.slot),
      peer.state,
      escape(peer.ice_state),
      ...channelFields(peer.control),
      ...channelFields(peer.input),
      String(peer.sequence_gaps),
      String(peer.backpressure),
      String(peer.malformed),
      escape(peer.last_error ?? ""),
    ];
    lines.push(fields.join("|"));
  }
  return lines.join("\n");
}

const STAR_FIELDS = 17;
const PEER_FIELDS = 25;

function parseChannel(fields: readonly string[], offset: number): TransportChannelDiagnostics | null {
  const numbers: number[] = [];
  for (let index = 1; index <= 7; index += 1) {
    const raw = fields[offset + index];
    const value = raw === undefined ? NaN : Number(raw);
    if (!Number.isFinite(value) || value !== Math.floor(value)) {
      return null;
    }
    numbers[index] = value;
  }
  const state = fields[offset] as TransportPeerState | undefined;
  if (state === undefined) {
    return null;
  }
  return {
    state,
    outbound_depth: numbers[1] as number,
    inbound_depth: numbers[2] as number,
    buffered_amount: numbers[3] as number,
    sent: numbers[4] as number,
    received: numbers[5] as number,
    dropped_outbound: numbers[6] as number,
    dropped_inbound: numbers[7] as number,
  };
}

export function decodeStarDiagnostics(text: unknown): TransportResult<TransportStarDiagnostics> {
  if (typeof text !== "string" || text === "") {
    return failure("bridge_error", "star diagnostics must be a non-empty string");
  }
  const lines = split(text, "\n");
  const firstLine = lines[0];
  if (firstLine === undefined) {
    return failure("bridge_error", "star diagnostics header is invalid");
  }
  const star = split(firstLine, "|");
  if (star[0] !== "star" || star.length !== STAR_FIELDS) {
    return failure("bridge_error", "star diagnostics header is invalid");
  }
  const numbers: number[] = [];
  for (let index = 3; index <= 15; index += 1) {
    const raw = star[index];
    const value = raw === undefined ? NaN : Number(raw);
    if (!Number.isFinite(value) || value !== Math.floor(value)) {
      return failure("bridge_error", "star diagnostics contain invalid numbers");
    }
    numbers[index] = value;
  }
  const lastErrorRaw = unescape(star[16] ?? "");
  const role = star[1] as TransportRole | undefined;
  const state = star[2] as TransportState | undefined;
  if (role === undefined || state === undefined) {
    return failure("bridge_error", "star diagnostics header is invalid");
  }
  const diagnostics: TransportStarDiagnostics = {
    role,
    state,
    capacity: numbers[3] as number,
    peer_count: numbers[4] as number,
    queue_limit: numbers[5] as number,
    buffered_amount_limit: numbers[6] as number,
    event_depth: numbers[7] as number,
    sent: numbers[8] as number,
    received: numbers[9] as number,
    dropped_outbound: numbers[10] as number,
    dropped_inbound: numbers[11] as number,
    malformed: numbers[12] as number,
    unsupported_version: numbers[13] as number,
    overflow: numbers[14] as number,
    backpressure: numbers[15] as number,
    last_error: lastErrorRaw !== null && lastErrorRaw !== "" ? lastErrorRaw : null,
    peers: [],
  };
  const peers: TransportPeerDiagnostics[] = [];
  for (let index = 1; index < lines.length; index += 1) {
    const line = lines[index];
    if (line === undefined) {
      continue;
    }
    const peer = split(line, "|");
    if (peer[0] !== "peer" || peer.length !== PEER_FIELDS) {
      return failure("bridge_error", "star diagnostics peer record is invalid");
    }
    const slotRaw = peer[2];
    const slot = slotRaw === undefined ? NaN : Number(slotRaw);
    const control = parseChannel(peer, 5);
    const input = parseChannel(peer, 13);
    const sequenceGapsRaw = peer[21];
    const backpressureRaw = peer[22];
    const malformedRaw = peer[23];
    const sequenceGaps = sequenceGapsRaw === undefined ? NaN : Number(sequenceGapsRaw);
    const backpressure = backpressureRaw === undefined ? NaN : Number(backpressureRaw);
    const malformed = malformedRaw === undefined ? NaN : Number(malformedRaw);
    if (!Number.isFinite(slot) || control === null || input === null) {
      return failure("bridge_error", "star diagnostics peer channels are invalid");
    }
    if (!Number.isFinite(sequenceGaps) || !Number.isFinite(backpressure) || !Number.isFinite(malformed)) {
      return failure("bridge_error", "star diagnostics peer counters are invalid");
    }
    const peerError = unescape(peer[24] ?? "");
    const peerState = peer[3] as TransportPeerState | undefined;
    const peerId = peer[1];
    if (peerState === undefined || peerId === undefined) {
      return failure("bridge_error", "star diagnostics peer record is invalid");
    }
    peers.push({
      peer_id: peerId,
      slot,
      state: peerState,
      ice_state: unescape(peer[4] ?? "") ?? "",
      control,
      input,
      sequence_gaps: sequenceGaps,
      backpressure,
      malformed,
      last_error: peerError !== null && peerError !== "" ? peerError : null,
    });
  }
  return ok({ ...diagnostics, peers });
}
