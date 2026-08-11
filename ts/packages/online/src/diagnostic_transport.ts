// A `StarTransportAdapter` that records what passed through it.
//
// The match driver takes its transport by injection, so a diagnostic tap can
// sit between the two without the driver knowing: every call is forwarded
// verbatim and every return value is passed back untouched. Nothing here
// retries, reorders, buffers, or drops, so the tap cannot change the
// behaviour it is there to explain.
//
// ## Why this decodes nothing
//
// A packet's newest authority tick is a *function of the delay policy*, not
// of its contents. The driver documents that a sample taken during driver
// step `T` is authority for input tick `first + T + DELAY`, and that the
// transport tick for step `T` is `T + DELAY`. Substituting gives
//
//     authority_input_tick = first_input_tick + transport_tick
//     sample_step          = transport_tick - DELAY
//
// so the envelope's own tick is enough. The tap therefore never re-decodes
// an input packet: the payload stays opaque, the hot path pays one object
// write, and the recorded lifecycle is derived from the stated policy
// rather than from a second parse that could disagree with the first.
// `row_count` is left unset for exactly this reason -- it is the one field
// that would need the parse.
//
// ## Why signalling is special-cased
//
// `requestOffer`, `acceptOffer`, `acceptAnswer`, and `takeSignal` carry SDP.
// SDP carries ICE ufrag/pwd and candidate lines with raw host and
// server-reflexive addresses. Those blobs are handed to `recordSignal`,
// which keeps a direction and a byte length and stores the content as an
// explicit redaction marker. The blob is not truncated into the record, not
// digested, and not logged.

import type {
  StarTransportAdapter,
  TransportChannel,
  TransportMessage,
  TransportPeerEvent,
  TransportPeerMessage,
  TransportPeerState,
  TransportResult,
  TransportRole,
  TransportStarDiagnostics,
  TransportState,
} from "@gc/transport";
import {
  type NetDiagnostics,
  type NetDiagnosticsEventKind,
  type NetDiagnosticsPacketDisposition,
  recordEvent,
  recordPacket,
  recordSignal,
  recordTeardown,
  recordTransport,
} from "./net_diagnostics.ts";

export interface DiagnosticTransportOptions {
  readonly transport: StarTransportAdapter;
  readonly recorder: NetDiagnostics;
  /** The local peer, used as the sender id on outbound records. */
  readonly peerId: string;
  readonly firstInputTick: number;
  readonly inputDelayTicks: number;
  /** Monotonic milliseconds; defaults to a zero clock. */
  readonly clock?: () => number;
  /** The receiver's own transport clock. */
  readonly transportTick?: () => number;
  /** Canonical outbound input wires to keep; 0 (default) keeps none. */
  readonly retainWires?: number;
}

function zeroClock(): number {
  return 0;
}

/** A `StarTransportAdapter` that taps traffic into a `NetDiagnostics` recorder. */
export class DiagnosticTransport implements StarTransportAdapter {
  private readonly _transport: StarTransportAdapter;
  private readonly _recorder: NetDiagnostics;
  private readonly _peerId: string;
  private readonly _first: number;
  private readonly _delay: number;
  private readonly _clock: () => number;
  private readonly _transportTick: (() => number) | undefined;
  private readonly _retainWires: number;
  // Bounded FIFO of canonical outbound input wires.
  private readonly _wires: string[] = [];
  private _wiresDropped = 0;
  private _closedPeers = 0;

  constructor(options: DiagnosticTransportOptions) {
    this._transport = options.transport;
    this._recorder = options.recorder;
    this._peerId = options.peerId;
    this._first = options.firstInputTick;
    this._delay = options.inputDelayTicks;
    this._clock = options.clock ?? zeroClock;
    this._transportTick = options.transportTick;
    this._retainWires = options.retainWires ?? 0;
  }

  // Canonical input wires this endpoint published, oldest first. Retention
  // is off unless asked for, because a wire ring is the only part of this
  // tap that costs real memory: at the protocol's 1 KiB envelope bound,
  // keeping 192 of them is 192 KiB and keeping a whole match is not bounded
  // at all.
  //
  // These are the *authored* wires, not decoded rows. They are exactly what
  // a desync package embeds, so the capture and the wire agree by
  // construction rather than by a second encoding that could drift.
  private retainWire(wire: string): void {
    if (this._retainWires <= 0) {
      return;
    }
    if (this._wires.length >= this._retainWires) {
      this._wires.shift();
      this._wiresDropped += 1;
    }
    this._wires.push(wire);
  }

  wires(): { readonly wires: readonly string[]; readonly dropped: number } {
    return { wires: [...this._wires], dropped: this._wiresDropped };
  }

  // The delay policy, applied to an envelope tick. Control messages carry no
  // tick, so they are never routed through here.
  private policyTicks(transportTick: number): { authorityInputTick: number; sampleStep: number } {
    return {
      authorityInputTick: this._first + transportTick,
      sampleStep: transportTick - this._delay,
    };
  }

  // `send_transport_tick` always comes from the envelope, because that is
  // the *sender's* clock and it travels with the packet.
  // `arrival_transport_tick` is the *receiver's* clock and is not in the
  // envelope, so it is recorded only when the caller supplies one. Reusing
  // the sender's tick for both would fabricate a zero-latency delivery on
  // every link that is not in-process.
  private recordEnvelope(
    senderId: string,
    message: TransportMessage,
    disposition: NetDiagnosticsPacketDisposition,
    inbound: boolean,
  ): void {
    const tick = message.tick;
    if (typeof tick !== "number") {
      return;
    }
    const { authorityInputTick, sampleStep } = this.policyTicks(tick);
    const arrival =
      inbound && this._transportTick !== undefined ? this._transportTick() : undefined;
    recordPacket(this._recorder, {
      sender_id: senderId,
      disposition,
      sequence: message.seq,
      payload_bytes: message.payload.length,
      sample_step: sampleStep,
      send_transport_tick: tick,
      authority_input_tick: authorityInputTick,
      ...(arrival !== undefined ? { arrival_transport_tick: arrival } : {}),
    });
  }

  // `state`, `code`, and `channel` are closed vocabularies from the
  // transport contract. `detail` is not: it carries `err`, a caller's close
  // reason, or a bridge's raw `String(error)`. It is redacted by
  // `recordEvent`, which is the single boundary every free-text field in
  // this system passes through -- doing it here as well would put two
  // owners on one rule.
  private note(event: {
    readonly kind: NetDiagnosticsEventKind;
    readonly peer_id?: string;
    readonly channel?: string;
    readonly state?: string;
    readonly code?: string;
    readonly detail?: string;
  }): void {
    recordEvent(this._recorder, { ...event, monotonic_ms: this._clock() });
  }

  // -------------------------------------------------------------------------
  // Forwarded surface
  // -------------------------------------------------------------------------

  initialize(): TransportResult<true> {
    return this._transport.initialize();
  }

  // Shutdown is where teardown completeness is decided, so the tap reads
  // the star's own view back afterwards: any link the star still reports,
  // and any queue depth it still holds, is an orphan rather than a clean
  // close.
  shutdown(): TransportResult<true> {
    const result = this._transport.shutdown();
    const diagnostics = this._transport.diagnostics();
    const orphaned: string[] = [];
    let residualOutbound = 0;
    let residualInbound = 0;
    for (const peer of diagnostics.peers) {
      if (peer.state !== "closed") {
        orphaned.push(peer.peer_id);
      }
      residualOutbound += peer.control.outbound_depth + peer.input.outbound_depth;
      residualInbound += peer.control.inbound_depth + peer.input.inbound_depth;
    }
    recordTeardown(this._recorder, {
      requested: true,
      closed_peers: this._closedPeers,
      orphaned_peers: orphaned,
      residual_outbound: residualOutbound,
      residual_inbound: residualInbound,
    });
    this.note({
      kind: "teardown",
      state: result.ok ? "closed" : "failed",
      ...(!result.ok ? { code: result.error.code, detail: result.error.message } : {}),
    });
    return result;
  }

  role(): TransportRole {
    return this._transport.role();
  }

  capacity(): number {
    return this._transport.capacity();
  }

  openPeer(peerId: string): TransportResult<number> {
    const result = this._transport.openPeer(peerId);
    this.note({
      kind: "peer_state",
      peer_id: peerId,
      state: result.ok ? "opened" : "rejected",
      ...(!result.ok ? { code: result.error.code, detail: result.error.message } : {}),
    });
    return result;
  }

  closePeer(peerId: string, reason?: string): TransportResult<true> {
    const result = this._transport.closePeer(peerId, reason);
    if (result.ok) {
      this._closedPeers += 1;
    }
    // `detail` prefers the caller's own close reason over the transport's
    // error, and does so independent of `ok` -- a reason handed to a
    // successful close is still worth recording.
    const detail = reason ?? (!result.ok ? result.error.message : undefined);
    this.note({
      kind: "peer_state",
      peer_id: peerId,
      state: result.ok ? "closed" : "close_failed",
      ...(!result.ok ? { code: result.error.code } : {}),
      ...(detail !== undefined ? { detail } : {}),
    });
    return result;
  }

  peerIds(): readonly string[] {
    return this._transport.peerIds();
  }

  peerState(peerId: string): TransportPeerState | null {
    return this._transport.peerState(peerId);
  }

  // -------------------------------------------------------------------------
  // Signalling: recorded, never retained
  // -------------------------------------------------------------------------

  requestOffer(peerId: string): TransportResult<true> {
    const result = this._transport.requestOffer(peerId);
    this.note({
      kind: "signal",
      peer_id: peerId,
      state: "offer_requested",
      ...(!result.ok ? { code: result.error.code, detail: result.error.message } : {}),
    });
    return result;
  }

  acceptOffer(signal: string): TransportResult<true> {
    recordSignal(this._recorder, this._peerId, "inbound", signal);
    return this._transport.acceptOffer(signal);
  }

  acceptAnswer(peerId: string, signal: string): TransportResult<true> {
    recordSignal(this._recorder, peerId, "inbound", signal);
    return this._transport.acceptAnswer(peerId, signal);
  }

  takeSignal(peerId: string): TransportResult<string | null> {
    const result = this._transport.takeSignal(peerId);
    if (result.ok && typeof result.value === "string") {
      recordSignal(this._recorder, peerId, "outbound", result.value);
    }
    return result;
  }

  // -------------------------------------------------------------------------
  // Traffic
  // -------------------------------------------------------------------------

  send(
    peerId: string,
    channel: TransportChannel,
    message: TransportMessage,
  ): TransportResult<true> {
    const result = this._transport.send(peerId, channel, message);
    if (channel === "input") {
      this.recordEnvelope(this._peerId, message, result.ok ? "sent" : "rejected", false);
      if (result.ok) {
        this.retainWire(message.payload);
      }
    }
    return result;
  }

  broadcast(channel: TransportChannel, message: TransportMessage): TransportResult<number> {
    const result = this._transport.broadcast(channel, message);
    if (channel === "input") {
      this.recordEnvelope(this._peerId, message, result.ok ? "sent" : "rejected", false);
      if (result.ok) {
        this.retainWire(message.payload);
      }
    }
    return result;
  }

  poll(): TransportResult<TransportPeerMessage | null> {
    const result = this._transport.poll();
    if (result.ok && result.value !== null && result.value.channel === "input") {
      this.recordEnvelope(result.value.peer_id, result.value.message, "arrived", true);
    }
    return result;
  }

  pollBatch(limit?: number): readonly TransportPeerMessage[] {
    const messages = this._transport.pollBatch(limit);
    for (const addressed of messages) {
      if (addressed.channel === "input") {
        this.recordEnvelope(addressed.peer_id, addressed.message, "arrived", true);
      }
    }
    return messages;
  }

  pollEvent(): TransportPeerEvent | null {
    const event = this._transport.pollEvent();
    if (event !== null) {
      this.note({
        kind: event.kind,
        ...(event.peer_id !== undefined ? { peer_id: event.peer_id } : {}),
        ...(event.channel !== undefined ? { channel: event.channel } : {}),
        ...(event.state !== undefined ? { state: event.state } : {}),
        ...(event.code !== undefined ? { code: event.code } : {}),
        ...(event.message !== undefined ? { detail: event.message } : {}),
      });
    }
    return event;
  }

  state(): TransportState {
    return this._transport.state();
  }

  // Reading transport diagnostics is also the moment to fold them into the
  // recorder: the star's counters are cumulative, so the newest read is the
  // whole truth and no accumulation of our own is correct on top of it.
  diagnostics(): TransportStarDiagnostics {
    const star = this._transport.diagnostics();
    recordTransport(this._recorder, star);
    return star;
  }

  // The wrapped adapter, for callers that legitimately need the real thing
  // (the fake star's `pump`, for instance, is a test affordance and not
  // part of the adapter contract).
  inner(): StarTransportAdapter {
    return this._transport;
  }
}
