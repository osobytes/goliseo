// In-memory loopback transport. Ported from game/transport/fake.lua.

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

export interface FakeTransportOptions {
  readonly queue_limit?: number;
}

/** In-memory loopback transport: everything enqueued is immediately deliverable. */
export class FakeTransport implements TransportAdapter {
  private _state: TransportState = "new";
  private readonly _queueLimit: number;
  private readonly _eventLimit: number;
  private _outbound: TransportMessage[] = [];
  private _inbound: TransportMessage[] = [];
  private _events: TransportEvent[] = [];
  private _droppedOutbound = 0;
  private _droppedInbound = 0;
  private _malformed = 0;
  private _unsupportedVersion = 0;
  private _overflow = 0;
  private _sent = 0;
  private _received = 0;
  private _lastError: string | null = null;

  constructor(options: FakeTransportOptions = {}) {
    const queueLimit = options.queue_limit ?? contract.DEFAULT_QUEUE_LIMIT;
    if (
      queueLimit !== Math.floor(queueLimit) ||
      queueLimit <= 0 ||
      queueLimit > contract.MAX_QUEUE_LIMIT
    ) {
      throw new Error("fake transport queue_limit is outside the supported range");
    }
    this._queueLimit = queueLimit;
    this._eventLimit = Math.max(2, queueLimit);
  }

  private _pushEvent(event: TransportEvent): void {
    if (this._events.length >= this._eventLimit) {
      this._events.shift();
      this._overflow += 1;
      this._lastError = "fake transport event queue is full";
    }
    this._events.push(event);
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
    this._pushEvent({ kind: "error", code, message });
  }

  private _setState(value: TransportState): void {
    this._state = value;
    this._pushEvent({ kind: "state", state: value });
  }

  initialize(): TransportResult<true> {
    if (this._state === "connected") {
      return ok(true);
    }
    this._setState("connected");
    return ok(true);
  }

  shutdown(): TransportResult<true> {
    if (this._state === "closed") {
      return ok(true);
    }
    this._droppedOutbound += this._outbound.length;
    this._droppedInbound += this._inbound.length;
    this._outbound = [];
    this._inbound = [];
    this._setState("closed");
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

  private _deliver(): void {
    while (this._outbound.length > 0 && this._inbound.length < this._queueLimit) {
      const next = this._outbound.shift();
      if (next !== undefined) {
        this._inbound.push(next);
      }
    }
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
    if (this._outbound.length >= this._queueLimit) {
      this._recordError("overflow", "fake transport outbound queue is full");
      this._droppedOutbound += 1;
      return err({ message: "fake transport outbound queue is full", code: "overflow" });
    }
    this._outbound.push(contract.copy(message));
    this._sent += 1;
    this._deliver();
    return ok(true);
  }

  send(message: TransportMessage): TransportResult<true> {
    return this.enqueue(message);
  }

  /** Test seam: injects a message directly onto the inbound queue. */
  inject(message: TransportMessage): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const validated = contract.validate(message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message);
      return validated;
    }
    if (this._inbound.length >= this._queueLimit) {
      this._recordError("overflow", "fake transport inbound queue is full");
      this._droppedInbound += 1;
      return err({ message: "fake transport inbound queue is full", code: "overflow" });
    }
    this._inbound.push(contract.copy(message));
    return ok(true);
  }

  poll(): TransportResult<TransportMessage | null> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    this._deliver();
    const message = this._inbound.shift();
    if (message === undefined) {
      return ok(null);
    }
    this._received += 1;
    this._deliver();
    return ok(message);
  }

  pollEvent(): TransportEvent | null {
    return this._events.shift() ?? null;
  }

  disconnect(reason?: string): TransportResult<true> {
    if (this._state !== "connected") {
      return err({ message: "transport is not connected", code: "not_connected" });
    }
    this._droppedOutbound += this._outbound.length;
    this._droppedInbound += this._inbound.length;
    this._outbound = [];
    this._inbound = [];
    this._setState("disconnected");
    this._recordError("disconnected", reason ?? "fake transport disconnected");
    return ok(true);
  }

  state(): TransportState {
    return this._state;
  }

  diagnostics(): TransportDiagnostics {
    return {
      state: this._state,
      queue_limit: this._queueLimit,
      outbound_depth: this._outbound.length,
      inbound_depth: this._inbound.length,
      event_depth: this._events.length,
      dropped_outbound: this._droppedOutbound,
      dropped_inbound: this._droppedInbound,
      malformed: this._malformed,
      unsupported_version: this._unsupportedVersion,
      overflow: this._overflow,
      sent: this._sent,
      received: this._received,
      last_error: this._lastError,
    };
  }
}
