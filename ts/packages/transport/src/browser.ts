// Real browser transport, driven through a single bounded-string eval seam
// against the DOM `window` bridge.

import { ok, err } from "@gc/core";
import * as contract from "./contract.ts";
import type {
  TransportAdapter,
  TransportDiagnostics,
  TransportErrorCode,
  TransportEvent,
  TransportMessage,
  TransportResult,
  TransportState,
} from "./contract.ts";

/**
 * Test seam; defaults to evaluating `window.GoliseoTransportBridge`.
 * Returns a `[result, error]` pair so a mock is free to explain a `null`
 * result with a message.
 */
export type EvalFn = (command: string) => readonly [result: string | null, error: string | null];

export interface BrowserTransportOptions {
  readonly queue_limit?: number;
  readonly eval?: EvalFn;
}

function isBrowserState(value: string): value is TransportState {
  return (
    value === "new" ||
    value === "connected" ||
    value === "disconnected" ||
    value === "closed" ||
    value === "error"
  );
}

function splitFields(value: string): string[] {
  const fields: string[] = [];
  let start = 0;
  for (;;) {
    const separator = value.indexOf("|", start);
    if (separator === -1) {
      fields.push(value.slice(start));
      return fields;
    }
    fields.push(value.slice(start, separator));
    start = separator + 1;
  }
}

function parseState(result: string): { readonly state: TransportState | null; readonly error?: string } {
  const fields = splitFields(result);
  const tag = fields[0];
  const rawState = fields[1];
  if (tag !== "state" || rawState === undefined) {
    return { state: null, error: "browser bridge returned an invalid state response" };
  }
  if (!isBrowserState(rawState)) {
    return { state: null, error: "browser bridge returned an unknown state" };
  }
  return { state: rawState };
}

interface ParsedResult {
  readonly ok: boolean;
  readonly code?: TransportErrorCode;
  readonly detail?: string;
}

function parseResult(result: string): ParsedResult {
  if (result === "ok") {
    return { ok: true };
  }
  const fields = splitFields(result);
  const tag = fields[0];
  const code = fields[1];
  if (tag !== "error" || code === undefined) {
    return { ok: false, code: "bridge_error", detail: "browser bridge returned an invalid operation response" };
  }
  return { ok: false, code: code as TransportErrorCode, detail: fields[2] ?? code };
}

/** Real browser transport: real WebRTC/WebSocket behind a bounded eval seam. */
export class BrowserTransport implements TransportAdapter {
  private _state: TransportState = "new";
  private readonly _queueLimit: number;
  private readonly _eval: EvalFn;
  private _eventDepth = 0;
  private _droppedOutbound = 0;
  private _droppedInbound = 0;
  private _malformed = 0;
  private _unsupportedVersion = 0;
  private _overflow = 0;
  private _sent = 0;
  private _received = 0;
  private _lastError: string | null = null;

  constructor(options: BrowserTransportOptions = {}) {
    const queueLimit = options.queue_limit ?? contract.DEFAULT_QUEUE_LIMIT;
    if (
      queueLimit !== Math.floor(queueLimit) ||
      queueLimit <= 0 ||
      queueLimit > contract.MAX_QUEUE_LIMIT
    ) {
      throw new Error("browser transport queue_limit is outside the supported range");
    }
    this._queueLimit = queueLimit;
    this._eval = options.eval ?? BrowserTransport._defaultEval;
  }

  private static _defaultEval(_command: string): readonly [result: string | null, error: string | null] {
    // The real DOM bridge (`window.GoliseoTransportBridge`) is wired up by
    // the app shell at runtime; this package implements transport behavior
    // only, not those bindings, so the default seam has nothing to call.
    return [null, "window.GoliseoTransportBridge is not available"];
  }

  private _recordError(code: TransportErrorCode, message: string): void {
    this._lastError = message;
    if (code === "malformed" || code === "payload_too_large") {
      this._malformed += 1;
    } else if (code === "unsupported_version") {
      this._unsupportedVersion += 1;
    } else if (code === "overflow") {
      this._overflow += 1;
    }
  }

  private _call(name: string, argument?: string): { readonly result: string | null; readonly error?: string } {
    const command = `window.GoliseoTransportBridge.${name}(${argument ?? ""})`;
    const [result, evalError] = this._eval(command);
    if (result === null) {
      const message = evalError ?? "browser bridge returned no result";
      this._recordError("bridge_error", message);
      return { result: null, error: message };
    }
    return { result };
  }

  initialize(): TransportResult<true> {
    if (this._state === "connected") {
      return ok(true);
    }
    const { result, error } = this._call("initialize", String(this._queueLimit));
    if (result === null) {
      return err({ message: error ?? "browser bridge returned no result", code: "bridge_error" });
    }
    const { state, error: stateError } = parseState(result);
    if (state === null) {
      const message = stateError ?? "browser bridge returned an invalid state";
      this._recordError("bridge_error", message);
      return err({ message, code: "bridge_error" });
    }
    this._state = state;
    if (state !== "connected") {
      return err({ message: "browser transport did not connect", code: "not_connected" });
    }
    return ok(true);
  }

  shutdown(): TransportResult<true> {
    if (this._state === "closed") {
      return ok(true);
    }
    const { result, error } = this._call("shutdown");
    if (result === null) {
      return err({ message: error ?? "browser bridge returned no result", code: "bridge_error" });
    }
    const { state, error: stateError } = parseState(result);
    if (state === null) {
      const message = stateError ?? "browser bridge returned an invalid state";
      this._recordError("bridge_error", message);
      return err({ message, code: "bridge_error" });
    }
    this._state = state;
    if (state !== "closed") {
      return err({ message: "browser transport did not close", code: "bridge_error" });
    }
    return ok(true);
  }

  private _requireConnected(): TransportResult<true> {
    if (this._state === "new") {
      return err({ message: "transport is not initialized", code: "not_initialized" });
    }
    if (this._state === "closed") {
      return err({ message: "transport is closed", code: "closed" });
    }
    if (this._state !== "connected") {
      return err({ message: "transport is not connected", code: "not_connected" });
    }
    return ok(true);
  }

  enqueue(message: TransportMessage): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const validated = contract.validate(message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message);
      return validated;
    }
    const wireResult = contract.encode(message);
    if (!wireResult.ok) {
      // `validate` above already guarantees `encode` succeeds.
      throw new Error(wireResult.error.message);
    }
    const { result, error: callError } = this._call("enqueue", `'${wireResult.value}'`);
    if (result === null) {
      return err({ message: callError ?? "browser bridge returned no result", code: "bridge_error" });
    }
    const parsed = parseResult(result);
    if (!parsed.ok) {
      const code = parsed.code ?? "bridge_error";
      const message = parsed.detail ?? "browser bridge rejected a message";
      this._recordError(code, message);
      if (code === "overflow") {
        this._droppedOutbound += 1;
      }
      return err({ message, code });
    }
    this._sent += 1;
    return ok(true);
  }

  send(message: TransportMessage): TransportResult<true> {
    return this.enqueue(message);
  }

  poll(): TransportResult<TransportMessage | null> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const { result, error: callError } = this._call("poll");
    if (result === null) {
      return err({ message: callError ?? "browser bridge returned no result", code: "bridge_error" });
    }
    if (result === "") {
      return ok(null);
    }
    const decoded = contract.decode(result);
    if (!decoded.ok) {
      this._recordError(decoded.error.code, decoded.error.message);
      this._droppedInbound += 1;
      return decoded;
    }
    this._received += 1;
    return ok(decoded.value);
  }

  pollEvent(): TransportEvent | null {
    const { result } = this._call("poll_event");
    if (result === null || result === "") {
      return null;
    }
    const fields = splitFields(result);
    const tag = fields[0];
    if (tag === "state" && fields[1] !== undefined && fields[1] !== "") {
      const state = fields[1];
      if (isBrowserState(state)) {
        this._state = state;
        return { kind: "state", state };
      }
    }
    if (tag === "error" && fields[1] !== undefined && fields[1] !== "") {
      this._eventDepth = Math.max(0, this._eventDepth - 1);
      return { kind: "error", code: fields[1] as TransportErrorCode };
    }
    this._recordError("bridge_error", "browser bridge returned an invalid event response");
    return { kind: "error", code: "bridge_error", message: result };
  }

  disconnect(reason?: string): TransportResult<true> {
    const argument =
      reason !== undefined
        ? `'${reason.replace(/[^A-Za-z0-9\-._~]/g, (character) => "%" + (character.charCodeAt(0) & 0xff).toString(16).toUpperCase().padStart(2, "0"))}'`
        : undefined;
    const { result, error } = this._call("disconnect", argument);
    if (result === null) {
      return err({ message: error ?? "browser bridge returned no result", code: "bridge_error" });
    }
    const { state, error: stateError } = parseState(result);
    if (state === null) {
      const message = stateError ?? "browser bridge returned an invalid state";
      this._recordError("bridge_error", message);
      return err({ message, code: "bridge_error" });
    }
    this._state = state;
    return ok(true);
  }

  state(): TransportState {
    return this._state;
  }

  diagnostics(): TransportDiagnostics {
    const { result, error } = this._call("diagnostics");
    if (result === null) {
      return this._diagnosticsFallback(error ?? null);
    }
    const fields = splitFields(result);
    if (fields.length !== 13) {
      this._recordError("bridge_error", "browser bridge returned invalid diagnostics");
      return this._diagnosticsFallback(null);
    }
    const numbers: number[] = [];
    for (let index = 1; index <= 11; index += 1) {
      const raw = fields[index];
      const value = raw === undefined ? NaN : Number(raw);
      if (!Number.isFinite(value)) {
        this._recordError("bridge_error", "browser bridge diagnostics contain invalid numbers");
        return this._diagnosticsFallback(null);
      }
      numbers[index] = value;
    }
    const stateField = fields[0];
    if (stateField === undefined || !isBrowserState(stateField)) {
      this._recordError("bridge_error", "browser bridge returned invalid diagnostics");
      return this._diagnosticsFallback(null);
    }
    this._state = stateField;
    const lastErrorField = fields[12];
    return {
      state: stateField,
      queue_limit: numbers[1] as number,
      outbound_depth: numbers[2] as number,
      inbound_depth: numbers[3] as number,
      event_depth: numbers[4] as number,
      dropped_outbound: numbers[5] as number,
      dropped_inbound: numbers[6] as number,
      malformed: numbers[7] as number,
      unsupported_version: numbers[8] as number,
      overflow: numbers[9] as number,
      sent: numbers[10] as number,
      received: numbers[11] as number,
      last_error: lastErrorField !== undefined && lastErrorField !== "" ? lastErrorField : null,
    };
  }

  private _diagnosticsFallback(error: string | null): TransportDiagnostics {
    return {
      state: this._state,
      queue_limit: this._queueLimit,
      outbound_depth: 0,
      inbound_depth: 0,
      event_depth: this._eventDepth,
      dropped_outbound: this._droppedOutbound,
      dropped_inbound: this._droppedInbound,
      malformed: this._malformed,
      unsupported_version: this._unsupportedVersion,
      overflow: this._overflow,
      sent: this._sent,
      received: this._received,
      last_error: error ?? this._lastError,
    };
  }
}
