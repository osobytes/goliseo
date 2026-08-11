// Carries lobby control traffic over a star transport.
//
// The session protocol bounds a control wire at 8,192 bytes while the
// transport envelope bounds one payload at 1,024, so a manifest proposal
// (about 2.5 kB) cannot travel as a single message. The transport treats
// payload contents as opaque and says nothing about splitting them, so this
// module adds the bounded framing the transport bridge document leaves to
// its caller: a control wire is split into at most `MAX_FRAMES` ordered
// chunks over the reliable, ordered control channel and reassembled on
// arrival. Framing is deliberately dumb -- no retransmission, no
// interleaving, no session state -- because the control channel already
// guarantees order and delivery.
//
// It also drives the manual signaling handshake and translates transport
// traffic into the events the pure lobby model consumes. Nothing here
// decides lobby policy: it executes effects and reports facts.
//
// ## Boundary note: `game.online.protocol.MAX_WIRE_BYTES`
//
// `game.online.protocol` is Rust-owned (`crates/gc-netcode`, ARCHITECTURE.md
// §1.1) and cannot be imported here. `MAX_WIRE_BYTES` (8192) is a plain
// integer bound this module uses only to reject an oversized wire before
// framing it -- not an algorithm, not a content table -- so it is
// duplicated as a local constant rather than threaded through as an
// injected parameter. If the protocol's bound ever changes, this constant
// needs a matching update.

import { type Result, ok, err } from "@gc/core";
import {
  HOST_PEER_ID,
  newMessage,
  type StarTransportAdapter,
  type TransportErrorCode,
} from "@gc/transport";

const PROTOCOL_MAX_WIRE_BYTES = 8192;

export interface LobbyFrameBuffer {
  /** Frames expected for the wire in progress, 0 when idle. */
  count: number;
  /** Next frame index expected, 1-based (matches the wire's own frame numbering). */
  next: number;
  /** Accumulated payload bytes, bounded by `PROTOCOL_MAX_WIRE_BYTES`. */
  bytes: number;
  chunks: string[];
}

export type LobbyLinkEvent =
  | { readonly kind: "signal"; readonly peer_id: string; readonly signal: string }
  | { readonly kind: "peer_connected"; readonly peer_id: string }
  | { readonly kind: "link_lost"; readonly link_id: string; readonly detail?: string }
  | { readonly kind: "control"; readonly link_id: string; readonly wire: string }
  | {
      readonly kind: "link_error";
      readonly link_id?: string;
      readonly code?: TransportErrorCode;
      readonly detail?: string;
    };

/** An effect the pure lobby model asked the transport layer to carry out. */
export type LobbyEffect =
  | { readonly kind: "open_peer"; readonly peer_id: string }
  | { readonly kind: "request_offer"; readonly peer_id: string }
  | { readonly kind: "accept_offer"; readonly signal: string }
  | { readonly kind: "accept_answer"; readonly peer_id: string; readonly signal: string }
  | { readonly kind: "send"; readonly link_id: string; readonly wire: string }
  | { readonly kind: "close"; readonly link_id: string; readonly detail?: string }
  | { readonly kind: "shutdown" }
  // Every other lobby-model effect kind (e.g. `clipboard`, `start_match`,
  // `leave`) is the caller's own concern; `apply` is a no-op for anything
  // it does not recognize, returning success rather than failing on an
  // effect kind it does not own.
  | { readonly kind: string };

export const FRAME_PREFIX = "GCLF;1;";
export const MAX_CHUNK_BYTES = 960;
export const MAX_FRAMES = 9;

const FRAME_PATTERN = /^GCLF;1;(\d+);(\d+);([\s\S]*)$/;

export function frame(wire: string): Result<readonly string[], string> {
  if (wire.length === 0) {
    return err("a control wire must be a non-empty string");
  }
  if (wire.length > PROTOCOL_MAX_WIRE_BYTES) {
    return err("control wire exceeds the protocol bound");
  }
  const count = Math.ceil(wire.length / MAX_CHUNK_BYTES);
  if (count > MAX_FRAMES) {
    return err("control wire needs more frames than the bound allows");
  }
  const frames: string[] = [];
  for (let index = 1; index <= count; index += 1) {
    const first = (index - 1) * MAX_CHUNK_BYTES;
    const last = Math.min(wire.length, first + MAX_CHUNK_BYTES);
    frames.push(`${FRAME_PREFIX}${index};${count};${wire.slice(first, last)}`);
  }
  return ok(frames);
}

export function newFrameBuffer(): LobbyFrameBuffer {
  return { count: 0, next: 1, bytes: 0, chunks: [] };
}

// Feeds one payload into a reassembly buffer. Returns the complete wire once
// the final frame lands, `null` while more are expected, and an error when
// the stream is malformed -- which the caller must treat as a protocol
// violation rather than silently resynchronising.
//
// There is deliberately no reassembly timeout. A peer that sends frame 1 of
// 3 and stops leaves its own buffer half full; that costs at most
// `PROTOCOL_MAX_WIRE_BYTES` of memory, is per-peer, and blocks only that
// peer's control channel until a malformed frame resets the buffer or the
// link drops. On a reliable ordered channel a stall means the peer is gone,
// which the transport reports on its own, so a timer would add a second
// liveness authority for no reachable failure it does not already cover.
export function absorb(buffer: LobbyFrameBuffer, payload: string): Result<string | null, string> {
  const match = FRAME_PATTERN.exec(payload);
  if (match === null) {
    return err("control frame is not canonical");
  }
  const [, rawIndex, rawCount, chunk] = match as unknown as [string, string, string, string];
  const index = Math.floor(Number(rawIndex));
  const count = Math.floor(Number(rawCount));
  if (count < 1 || count > MAX_FRAMES) {
    return err("control frame count is outside the bound");
  }
  if (index < 1 || index > count) {
    return err("control frame index is outside its own stream");
  }
  if (chunk.length > MAX_CHUNK_BYTES) {
    return err("control frame chunk exceeds the bound");
  }
  if (buffer.count === 0) {
    if (index !== 1) {
      return err("control stream started mid-wire");
    }
    buffer.count = count;
  } else if (count !== buffer.count || index !== buffer.next) {
    return err("control frame arrived out of order");
  }
  buffer.bytes += chunk.length;
  if (buffer.bytes > PROTOCOL_MAX_WIRE_BYTES) {
    return err("reassembled control wire exceeds the protocol bound");
  }
  buffer.chunks.push(chunk);
  if (index < count) {
    buffer.next = index + 1;
    return ok(null);
  }
  const wire = buffer.chunks.join("");
  buffer.count = 0;
  buffer.next = 1;
  buffer.bytes = 0;
  buffer.chunks = [];
  return ok(wire);
}

/** Carries lobby control traffic over a star transport. */
export class LobbyLink {
  readonly star: StarTransportAdapter;
  private readonly _buffers = new Map<string, LobbyFrameBuffer>();
  private readonly _sequence = new Map<string, number>();
  /** Peers whose local signal blob is awaited. */
  private readonly _pending = new Set<string>();
  /** Links closed on the next pump. */
  private _closing: { readonly link_id: string; readonly detail?: string }[] = [];

  constructor(star: StarTransportAdapter) {
    this.star = star;
  }

  private buffer(peerId: string): LobbyFrameBuffer {
    let buffer = this._buffers.get(peerId);
    if (buffer === undefined) {
      buffer = newFrameBuffer();
      this._buffers.set(peerId, buffer);
    }
    return buffer;
  }

  private nextSequence(peerId: string): number {
    const sequence = this._sequence.get(peerId) ?? 0;
    this._sequence.set(peerId, sequence + 1);
    return sequence;
  }

  send(peerId: string, wire: string): Result<true, string> {
    const frames = frame(wire);
    if (!frames.ok) {
      return frames;
    }
    for (const payload of frames.value) {
      const message = newMessage({ type: "event", seq: this.nextSequence(peerId), payload });
      if (!message.ok) {
        return err(message.error.message);
      }
      const sent = this.star.send(peerId, "control", message.value);
      if (!sent.ok) {
        return err(sent.error.message);
      }
    }
    return ok(true);
  }

  // Executes one lobby effect. Transport failures are returned rather than
  // raised: a rejected paste is an ordinary lobby outcome, not a crash.
  apply(effect: LobbyEffect): Result<true, string> {
    const kind = effect.kind;
    if (kind === "open_peer") {
      const opened = this.star.openPeer((effect as { peer_id: string }).peer_id);
      if (!opened.ok) {
        return err(opened.error.message);
      }
      return ok(true);
    } else if (kind === "request_offer") {
      const peerId = (effect as { peer_id: string }).peer_id;
      this._pending.add(peerId);
      const result = this.star.requestOffer(peerId);
      return result.ok ? ok(true) : err(result.error.message);
    } else if (kind === "accept_offer") {
      this._pending.add(HOST_PEER_ID);
      const result = this.star.acceptOffer((effect as { signal: string }).signal);
      return result.ok ? ok(true) : err(result.error.message);
    } else if (kind === "accept_answer") {
      const typed = effect as { peer_id: string; signal: string };
      const result = this.star.acceptAnswer(typed.peer_id, typed.signal);
      return result.ok ? ok(true) : err(result.error.message);
    } else if (kind === "send") {
      const typed = effect as { link_id: string; wire: string };
      return this.send(typed.link_id, typed.wire);
    } else if (kind === "close") {
      // A coordinator announces on a link and closes it in the same batch.
      // An adapter is only required to *queue* the announcement, so closing
      // immediately can discard it. The close is therefore deferred by one
      // pump, which every adapter has to complete anyway.
      const typed = effect as { link_id: string; detail?: string };
      this._closing.push({
        link_id: typed.link_id,
        ...(typed.detail !== undefined ? { detail: typed.detail } : {}),
      });
      return ok(true);
    } else if (kind === "shutdown") {
      const result = this.star.shutdown();
      return result.ok ? ok(true) : err(result.error.message);
    }
    return ok(true);
  }

  private drainSignals(events: LobbyLinkEvent[]): void {
    for (const peerId of this.star.peerIds()) {
      if (this._pending.has(peerId)) {
        const signal = this.star.takeSignal(peerId);
        if (signal.ok && signal.value !== null) {
          this._pending.delete(peerId);
          events.push({ kind: "signal", peer_id: peerId, signal: signal.value });
        }
      }
    }
  }

  private drainEvents(events: LobbyLinkEvent[]): void {
    let event = this.star.pollEvent();
    while (event !== null) {
      if (event.kind === "peer_state" && event.peer_id !== undefined) {
        if (event.state === "connected") {
          events.push({ kind: "peer_connected", peer_id: event.peer_id });
        } else if (
          event.state === "error" ||
          event.state === "closed" ||
          event.state === "disconnected"
        ) {
          events.push({
            kind: "link_lost",
            link_id: event.peer_id,
            ...(event.message !== undefined ? { detail: event.message } : {}),
          });
        }
      } else if (event.kind === "peer_error" || event.kind === "star_error") {
        events.push({
          kind: "link_error",
          ...(event.peer_id !== undefined ? { link_id: event.peer_id } : {}),
          ...(event.code !== undefined ? { code: event.code } : {}),
          ...(event.message !== undefined ? { detail: event.message } : {}),
        });
      }
      event = this.star.pollEvent();
    }
  }

  private drainMessages(events: LobbyLinkEvent[]): void {
    for (const received of this.star.pollBatch()) {
      if (received.channel === "control") {
        const result = absorb(this.buffer(received.peer_id), received.message.payload);
        if (result.ok && result.value !== null) {
          events.push({ kind: "control", link_id: received.peer_id, wire: result.value });
        } else if (!result.ok) {
          this._buffers.set(received.peer_id, newFrameBuffer());
          events.push({
            kind: "link_error",
            link_id: received.peer_id,
            code: "malformed",
            detail: result.error,
          });
        }
      }
    }
  }

  // One pump of the transport: locally produced signaling blobs first, then
  // connection events, then reassembled control wires. The order is fixed
  // so a scripted lobby flow is reproducible.
  poll(): LobbyLinkEvent[] {
    const events: LobbyLinkEvent[] = [];
    for (const closing of this._closing) {
      this.star.closePeer(closing.link_id, closing.detail);
      this._buffers.delete(closing.link_id);
    }
    this._closing = [];
    this.drainSignals(events);
    this.drainEvents(events);
    this.drainMessages(events);
    return events;
  }
}

export function newLobbyLink(star: StarTransportAdapter): LobbyLink {
  return new LobbyLink(star);
}
