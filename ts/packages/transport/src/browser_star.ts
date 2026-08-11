// Browser host-star adapter. Every WebRTC object stays behind
// `window.GoliseoStarTransport`; this module only exchanges bounded ASCII
// strings through a single eval seam (the DOM analogue of the LÖVE
// `love.js.eval` hook the Lua source targets), so no JavaScript/DOM value
// ever escapes typed diagnostics. Nothing here is a simulation authority.
//
// Ported from game/transport/browser_star.lua.

import { ok, err } from "@gc/core";
import * as contract from "./contract.ts";
import type {
  StarTransportAdapter,
  TransportChannel,
  TransportErrorCode,
  TransportPeerEvent,
  TransportPeerMessage,
  TransportPeerState,
  TransportMessage,
  TransportResult,
  TransportRole,
  TransportState,
  TransportStarDiagnostics,
} from "./contract.ts";

const BRIDGE = "window.GoliseoStarTransport.";

/**
 * Test seam; defaults to evaluating `window.GoliseoStarTransport`. Returns a
 * `[result, error]` pair, mirroring Lua's `local result, err =
 * self._eval(command)` capturing both of the callee's returns.
 */
export type StarEvalFn = (command: string) => readonly [result: string | null, error: string | null];

export interface BrowserStarTransportOptions {
  readonly role?: TransportRole;
  readonly queue_limit?: number;
  readonly max_guests?: number;
  readonly buffered_amount_limit?: number;
  readonly eval?: StarEvalFn;
}

function failure<T>(code: TransportErrorCode, message: string): TransportResult<T> {
  return err({ message, code });
}

function defaultEval(_command: string): readonly [result: string | null, error: string | null] {
  // The real DOM bridge (`window.GoliseoStarTransport`) is wired up by the
  // app shell at runtime; this milestone only ports behaviour, not the
  // bindings, so the default seam has nothing to call.
  return [null, "window.GoliseoStarTransport is not available"];
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

function quote(value: string): string {
  return "'" + escape(value) + "'";
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

function bounded(value: number, name: string, maximum: number): number {
  if (value !== Math.floor(value) || value <= 0 || value > maximum) {
    throw new Error(`browser star transport ${name} is outside the supported range`);
  }
  return value;
}

function isTransportState(value: string): value is TransportState {
  return (
    value === "new" ||
    value === "connected" ||
    value === "disconnected" ||
    value === "closed" ||
    value === "error"
  );
}

/** Browser host-star adapter, driven through a single bridge eval seam. */
export class BrowserStarTransport implements StarTransportAdapter {
  private readonly _role: TransportRole;
  private _state: TransportState = "new";
  private readonly _queueLimit: number;
  private readonly _maxGuests: number;
  private readonly _bufferedAmountLimit: number;
  private readonly _eval: StarEvalFn;
  private _arrivals = new Map<string, number>();
  private _peerStates = new Map<string, TransportPeerState>();
  private _order: string[] = [];
  private _events: TransportPeerEvent[] = [];
  private readonly _eventLimit: number;
  private _localError = false;
  private _lastError: string | null = null;

  constructor(options: BrowserStarTransportOptions = {}) {
    const role = options.role ?? "host";
    if (role !== "host" && role !== "guest") {
      throw new Error("browser star transport role must be host or guest");
    }
    this._role = role;
    this._queueLimit = bounded(
      options.queue_limit ?? contract.DEFAULT_QUEUE_LIMIT,
      "queue_limit",
      contract.MAX_QUEUE_LIMIT
    );
    this._maxGuests =
      role === "host"
        ? bounded(options.max_guests ?? contract.MAX_GUESTS, "max_guests", contract.MAX_GUESTS)
        : 1;
    this._bufferedAmountLimit = bounded(
      options.buffered_amount_limit ?? contract.DEFAULT_BUFFERED_AMOUNT_LIMIT,
      "buffered_amount_limit",
      contract.MAX_BUFFERED_AMOUNT_LIMIT
    );
    this._eval = options.eval ?? defaultEval;
    this._eventLimit = Math.max(2, options.queue_limit ?? contract.DEFAULT_QUEUE_LIMIT);
  }

  // A fault this adapter rejects locally (role misuse, lifecycle misuse, a
  // message the bridge would never see) has to be as observable as one the
  // bridge reports, or the two adapters disagree about what happened.
  private _recordError<T>(
    code: TransportErrorCode,
    message: string,
    peerId?: string,
    channel?: TransportChannel
  ): TransportResult<T> {
    this._lastError = message;
    this._localError = true;
    if (this._events.length >= this._eventLimit) {
      this._events.shift();
    }
    if (peerId !== undefined) {
      this._events.push({
        kind: "peer_error",
        peer_id: peerId,
        ...(channel !== undefined ? { channel } : {}),
        code,
        message,
      });
    } else {
      this._events.push({ kind: "star_error", code, message });
    }
    return failure(code, message);
  }

  private _call(name: string, ...args: readonly string[]): { readonly result: string | null; readonly error?: string } {
    const command = `${BRIDGE}${name}(${args.join(",")})`;
    const [result, evalError] = this._eval(command);
    if (result === null) {
      const message = evalError ?? "browser star bridge returned no result";
      this._lastError = message;
      return { result: null, error: message };
    }
    return { result };
  }

  // Bridge results are either `ok`, `<tag>|<value>`, or `error|<code>|<detail>`.
  private _parse(result: string): TransportResult<string[]> {
    const fields = split(result, "|");
    const tag = fields[0];
    if (tag !== "error") {
      return ok(fields);
    }
    const code = fields[1];
    const detail = unescape(fields[2] ?? "") ?? code;
    if (typeof code !== "string" || code === "") {
      this._lastError = "browser star bridge returned an invalid error response";
      return failure("bridge_error", this._lastError);
    }
    this._lastError = detail ?? code;
    return failure(code as TransportErrorCode, detail ?? code);
  }

  initialize(): TransportResult<true> {
    if (this._state === "connected") {
      return ok(true);
    }
    const { result, error } = this._call(
      "initialize",
      quote(this._role),
      String(this._queueLimit),
      String(this._maxGuests),
      String(this._bufferedAmountLimit)
    );
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    const fields = parsed.value;
    if (fields[0] !== "star" || fields[1] !== "connected") {
      return failure("not_connected", "browser star transport did not connect");
    }
    this._state = "connected";
    if (this._role === "guest") {
      this._order = [contract.HOST_PEER_ID];
      this._peerStates.set(contract.HOST_PEER_ID, "connecting");
    }
    return ok(true);
  }

  shutdown(): TransportResult<true> {
    if (this._state === "closed") {
      return ok(true);
    }
    const { result, error } = this._call("shutdown");
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    const fields = parsed.value;
    if (fields[0] !== "star" || fields[1] !== "closed") {
      return failure("bridge_error", "browser star transport did not close");
    }
    this._state = "closed";
    this._order = [];
    this._peerStates = new Map();
    this._arrivals = new Map();
    return ok(true);
  }

  // Returns the peer id only if this adapter has actually opened that link,
  // so a caller-supplied id for an unknown peer never gets tagged onto an
  // event.
  private _openedPeer(peerId: string): string | undefined {
    return this._order.includes(peerId) ? peerId : undefined;
  }

  private _requireConnected(): TransportResult<true> {
    if (this._state === "new") {
      return failure("not_initialized", "star transport is not initialized");
    }
    if (this._state === "closed") {
      return failure("closed", "star transport is closed");
    }
    if (this._state !== "connected") {
      return failure("not_connected", "star transport is not connected");
    }
    return ok(true);
  }

  openPeer(peerId: string): TransportResult<number> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      return this._recordError("role_forbidden", "only a host opens guest links");
    }
    const idResult = contract.validatePeerId(peerId);
    if (!idResult.ok) {
      return idResult;
    }
    const { result, error } = this._call("open_peer", quote(peerId));
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    const fields = parsed.value;
    const slot = fields[0] === "slot" && fields[1] !== undefined ? Number(fields[1]) : NaN;
    if (!Number.isFinite(slot)) {
      return failure("bridge_error", "browser star bridge returned an invalid slot");
    }
    this._order.push(peerId);
    this._peerStates.set(peerId, "connecting");
    return ok(slot);
  }

  closePeer(peerId: string, reason?: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const idResult = contract.validatePeerId(peerId);
    if (!idResult.ok) {
      return this._recordError(idResult.error.code, idResult.error.message);
    }
    const { result, error } = this._call("close_peer", quote(peerId), quote(reason ?? "closed"));
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    this._peerStates.set(peerId, "closed");
    return ok(true);
  }

  // Manual signaling. Offer and answer creation are asynchronous in the
  // browser, so these calls only start a job; `takeSignal` returns the SDP
  // blob on a later frame. Nothing here waits on the network inside an
  // update loop.
  requestOffer(peerId: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      return this._recordError("role_forbidden", "only a host creates offers");
    }
    const idResult = contract.validatePeerId(peerId);
    if (!idResult.ok) {
      return this._recordError(idResult.error.code, idResult.error.message);
    }
    return this._command("request_offer", quote(peerId));
  }

  acceptOffer(signal: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "guest") {
      return this._recordError("role_forbidden", "only a guest accepts an offer");
    }
    return this._command("accept_offer", quote(signal));
  }

  acceptAnswer(peerId: string, signal: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      return this._recordError("role_forbidden", "only a host accepts an answer");
    }
    const idResult = contract.validatePeerId(peerId);
    if (!idResult.ok) {
      return this._recordError(idResult.error.code, idResult.error.message);
    }
    return this._command("accept_answer", quote(peerId), quote(signal));
  }

  private _command(name: string, ...args: readonly string[]): TransportResult<true> {
    const { result, error } = this._call(name, ...args);
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    return ok(true);
  }

  // Returns the locally generated SDP blob for a peer once the browser has
  // finished gathering, or nil while the job is still pending.
  takeSignal(peerId: string): TransportResult<string | null> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const { result, error } = this._call("take_signal", quote(peerId));
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    if (result === "") {
      return ok(null);
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    const fields = parsed.value;
    if (fields[0] !== "signal") {
      return failure("bridge_error", "browser star bridge returned an invalid signal");
    }
    const signal = unescape(fields[1] ?? "");
    if (signal === null) {
      return failure("bridge_error", "browser star bridge signal escape is invalid");
    }
    return ok(signal);
  }

  send(peerId: string, channel: TransportChannel, message: TransportMessage): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role === "guest" && peerId !== contract.HOST_PEER_ID) {
      return this._recordError("role_forbidden", "a guest may only address the host");
    }
    // Validation order is part of the contract: message shape first, peer
    // resolution second. Only the bridge knows which links exist, so an
    // adapter must never claim `unknown_peer` from its own cached peer
    // table; shape is the one fault every layer can judge identically.
    const lineResult = contract.encodeAddressed(peerId, channel, message);
    if (!lineResult.ok) {
      // Tag the event against the link only when it is actually open. For a
      // peer that was never opened the bridge and the fake both queue an
      // untagged `star_error`, and this has to match them. The lookup only
      // picks the event's attribution; the returned code is unaffected.
      const openedPeer = this._openedPeer(peerId);
      const knownChannel = contract.CHANNEL_CONFIG[channel] !== undefined ? channel : undefined;
      return this._recordError(lineResult.error.code, lineResult.error.message, openedPeer, knownChannel);
    }
    return this._command("send", `'${lineResult.value}'`);
  }

  broadcast(channel: TransportChannel, message: TransportMessage): TransportResult<number> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      return this._recordError("role_forbidden", "only a host fans out canonical batches");
    }
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      return this._recordError(validated.error.code, validated.error.message);
    }
    const wireResult = contract.encode(message);
    if (!wireResult.ok) {
      throw new Error(wireResult.error.message);
    }
    const { result, error } = this._call("broadcast", `'${channel}|${wireResult.value}'`);
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    const parsed = this._parse(result);
    if (!parsed.ok) {
      return parsed;
    }
    const fields = parsed.value;
    const delivered = fields[0] === "delivered" && fields[1] !== undefined ? Number(fields[1]) : NaN;
    if (!Number.isFinite(delivered)) {
      return failure("bridge_error", "browser star bridge returned an invalid fan-out count");
    }
    return ok(delivered);
  }

  poll(): TransportResult<TransportPeerMessage | null> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const { result, error } = this._call("poll");
    if (result === null) {
      return failure("bridge_error", error ?? "browser star bridge is unavailable");
    }
    if (result === "") {
      return ok(null);
    }
    const addressedResult = contract.decodeAddressed(result);
    if (!addressedResult.ok) {
      this._lastError = addressedResult.error.message;
      return addressedResult;
    }
    const addressed = addressedResult.value;
    const arrival = (this._arrivals.get(addressed.peer_id) ?? 0) + 1;
    this._arrivals.set(addressed.peer_id, arrival);
    return ok({
      peer_id: addressed.peer_id,
      channel: addressed.channel,
      arrival_seq: arrival,
      message: addressed.message,
    });
  }

  // The bridge drains its per-peer queues in slot order, control before
  // input, one message per queue per pass. This relays that fixed order; it
  // is not a claim that browser callback arrival order is simulation order.
  pollBatch(limit?: number): readonly TransportPeerMessage[] {
    const budget = limit ?? contract.DEFAULT_POLL_BATCH;
    const batch: TransportPeerMessage[] = [];
    while (batch.length < budget) {
      const entry = this.poll();
      if (!entry.ok || entry.value === null) {
        return batch;
      }
      batch.push(entry.value);
    }
    return batch;
  }

  pollEvent(): TransportPeerEvent | null {
    // Locally rejected calls are queued here; drain them before the
    // bridge's own events so the caller sees every fault this adapter
    // produced.
    const local = this._events.shift();
    if (local !== undefined) {
      return local;
    }
    const { result } = this._call("poll_event");
    if (result === null || result === "") {
      return null;
    }
    const fields = split(result, "|");
    const tag = fields[0];
    if (tag === "star_state" && fields[1] !== undefined) {
      const state = fields[1];
      if (isTransportState(state)) {
        this._state = state;
        return { kind: "star_state", state };
      }
    }
    if (tag === "peer_state" && fields[1] !== undefined && fields[2] !== undefined) {
      const peerId = fields[1];
      const state = fields[2] as TransportPeerState;
      this._peerStates.set(peerId, state);
      return { kind: "peer_state", peer_id: peerId, state };
    }
    if (tag === "star_error" && fields[1] !== undefined) {
      const code = fields[1] as TransportErrorCode;
      const detail = unescape(fields[2] ?? "");
      this._lastError = detail;
      return { kind: "star_error", code, ...(detail !== null ? { message: detail } : {}) };
    }
    if (tag === "peer_error" && fields[1] !== undefined && fields[3] !== undefined) {
      const code = fields[3] as TransportErrorCode;
      const channel = fields[2] !== undefined && fields[2] !== "" ? (fields[2] as TransportChannel) : undefined;
      const detail = unescape(fields[4] ?? "");
      this._lastError = detail;
      return {
        kind: "peer_error",
        peer_id: fields[1],
        ...(channel !== undefined ? { channel } : {}),
        code,
        ...(detail !== null ? { message: detail } : {}),
      };
    }
    const invalidEventMessage = "browser star bridge returned an invalid event response";
    this._lastError = invalidEventMessage;
    return { kind: "star_error", code: "bridge_error", message: invalidEventMessage };
  }

  role(): TransportRole {
    return this._role;
  }

  capacity(): number {
    return this._maxGuests;
  }

  peerIds(): readonly string[] {
    return [...this._order];
  }

  peerState(peerId: string): TransportPeerState | null {
    return this._peerStates.get(peerId) ?? null;
  }

  state(): TransportState {
    return this._state;
  }

  private _fallbackDiagnostics(): TransportStarDiagnostics {
    return {
      role: this._role,
      state: this._state,
      capacity: this._maxGuests,
      peer_count: this._order.length,
      queue_limit: this._queueLimit,
      buffered_amount_limit: this._bufferedAmountLimit,
      event_depth: 0,
      sent: 0,
      received: 0,
      dropped_outbound: 0,
      dropped_inbound: 0,
      malformed: 0,
      unsupported_version: 0,
      overflow: 0,
      backpressure: 0,
      last_error: this._lastError,
      peers: [],
    };
  }

  diagnostics(): TransportStarDiagnostics {
    const { result, error } = this._call("diagnostics");
    if (result === null) {
      this._lastError = error ?? "browser star bridge is unavailable";
      return this._fallbackDiagnostics();
    }
    const decoded = contract.decodeStarDiagnostics(result);
    if (!decoded.ok) {
      this._lastError = decoded.error.message ?? "browser star diagnostics are invalid";
      return this._fallbackDiagnostics();
    }
    const diagnostics = decoded.value;
    this._state = diagnostics.state;
    this._order = [];
    for (const peer of diagnostics.peers) {
      this._order.push(peer.peer_id);
      this._peerStates.set(peer.peer_id, peer.state);
    }
    // A fault this adapter rejected locally never reached the bridge, so the
    // bridge's `last_error` would silently hide it. The local one is newer.
    let lastError = diagnostics.last_error;
    if (this._localError) {
      lastError = this._lastError;
      this._localError = false;
    }
    return { ...diagnostics, event_depth: diagnostics.event_depth + this._events.length, last_error: lastError };
  }
}
