// Pure in-process host-star transport. It carries the reference semantics for
// the browser adapter: one host endpoint, up to seven independently addressed
// guest links, a reliable ordered control channel and a lossy unordered input
// channel per link, bounded queues, simulated `bufferedAmount` backpressure,
// typed events, and deterministic poll batching. It owns no simulation
// authority and never inspects a payload.

import { ok, err } from "@gc/core";
import * as contract from "./contract.ts";
import type {
  StarTransportAdapter,
  TransportAddressedMessage,
  TransportChannel,
  TransportErrorCode,
  TransportMessage,
  TransportPeerDiagnostics,
  TransportPeerEvent,
  TransportPeerMessage,
  TransportPeerState,
  TransportResult,
  TransportRole,
  TransportState,
  TransportStarDiagnostics,
} from "./contract.ts";
import type { FakeRelayWireCounters } from "./fake_relay.ts";

export interface FakeStarTransportOptions {
  readonly role?: TransportRole;
  /** Guest link identity; ignored for a host. */
  readonly peer_id?: string;
  readonly queue_limit?: number;
  readonly max_guests?: number;
  readonly buffered_amount_limit?: number;
  /** Share one across the endpoints of a single star. */
  readonly rendezvous?: FakeStarRendezvous;
}

interface FakeStarChannel {
  state: TransportPeerState;
  outbound: TransportAddressedMessage[];
  inbound: TransportPeerMessage[];
  backpressured: boolean;
  bufferedAmount: number;
  sent: number;
  received: number;
  droppedOutbound: number;
  droppedInbound: number;
  lastSeq: number | null;
}

interface FakeStarPeer {
  peerId: string;
  slot: number;
  state: TransportPeerState;
  iceState: string;
  channels: Record<TransportChannel, FakeStarChannel>;
  arrivalSeq: number;
  signal: string | null;
  sequenceGaps: number;
  backpressure: number;
  malformed: number;
  lastError: string | null;
  link: FakeStarTransport | null;
  linkPeerId: string | null;
}

/**
 * In-process signaling rendezvous, scoped to ONE logical star. A real star
 * exchanges SDP blobs by hand between two browser contexts; here the blob is
 * an opaque token that resolves back to the offering host and the answering
 * guest. Endpoints only reach each other's tokens when they were handed the
 * same rendezvous, so two stars in one process cannot cross-talk by
 * accident. An endpoint constructed without one gets a private rendezvous,
 * which makes sharing an explicit, greppable decision rather than the
 * default.
 */
export interface FakeStarRendezvous {
  offers: Map<string, { readonly host: FakeStarTransport; readonly peerId: string }>;
  answers: Map<string, FakeStarTransport>;
  sequence: number;
}

export function newRendezvous(): FakeStarRendezvous {
  return { offers: new Map(), answers: new Map(), sequence: 0 };
}

function failure<T>(code: TransportErrorCode, message: string): TransportResult<T> {
  return err({ message, code });
}

function newChannel(): FakeStarChannel {
  return {
    state: "connecting",
    outbound: [],
    inbound: [],
    backpressured: false,
    bufferedAmount: 0,
    sent: 0,
    received: 0,
    droppedOutbound: 0,
    droppedInbound: 0,
    lastSeq: null,
  };
}

/** Pure in-process host-star transport: one host, up to seven guest links. */
export class FakeStarTransport implements StarTransportAdapter {
  private readonly _role: TransportRole;
  private readonly _peerId: string;
  private _state: TransportState = "new";
  private readonly _queueLimit: number;
  private readonly _eventLimit: number;
  private readonly _maxGuests: number;
  private readonly _bufferedAmountLimit: number;
  private readonly _rendezvous: FakeStarRendezvous;
  private _peers = new Map<string, FakeStarPeer>();
  private _order: string[] = [];
  private _events: TransportPeerEvent[] = [];
  private _cursor = 0;
  private _sent = 0;
  private _received = 0;
  private _droppedOutbound = 0;
  private _droppedInbound = 0;
  private _malformed = 0;
  private _unsupportedVersion = 0;
  private _overflow = 0;
  private _backpressure = 0;
  private _uplinkBytes = 0;
  private _downlinkBytes = 0;
  private _inputUplinkBytes = 0;
  private _inputDownlinkBytes = 0;
  private _uplinkUnits = 0;
  private _downlinkFrames = 0;
  private _lastError: string | null = null;

  constructor(options: FakeStarTransportOptions = {}) {
    const role = options.role ?? "host";
    if (role !== "host" && role !== "guest") {
      throw new Error("fake star transport role must be host or guest");
    }
    const queueLimit = options.queue_limit ?? contract.DEFAULT_QUEUE_LIMIT;
    if (
      queueLimit !== Math.floor(queueLimit) ||
      queueLimit <= 0 ||
      queueLimit > contract.MAX_QUEUE_LIMIT
    ) {
      throw new Error("fake star transport queue_limit is outside the supported range");
    }
    const maxGuests = options.max_guests ?? contract.MAX_GUESTS;
    if (maxGuests !== Math.floor(maxGuests) || maxGuests <= 0 || maxGuests > contract.MAX_GUESTS) {
      throw new Error("fake star transport max_guests is outside the supported range");
    }
    const bufferedAmountLimit =
      options.buffered_amount_limit ?? contract.DEFAULT_BUFFERED_AMOUNT_LIMIT;
    if (
      bufferedAmountLimit !== Math.floor(bufferedAmountLimit) ||
      bufferedAmountLimit <= 0 ||
      bufferedAmountLimit > contract.MAX_BUFFERED_AMOUNT_LIMIT
    ) {
      throw new Error("fake star transport buffered_amount_limit is outside the supported range");
    }
    const peerId = options.peer_id ?? "guest";
    const peerIdResult = contract.validatePeerId(peerId);
    if (!peerIdResult.ok) {
      throw new Error(peerIdResult.error.message);
    }
    this._role = role;
    this._peerId = peerId;
    this._queueLimit = queueLimit;
    this._eventLimit = Math.max(2, queueLimit);
    this._maxGuests = role === "host" ? maxGuests : 1;
    this._bufferedAmountLimit = bufferedAmountLimit;
    this._rendezvous = options.rendezvous ?? newRendezvous();
  }

  private _pushEvent(event: TransportPeerEvent): void {
    if (this._events.length >= this._eventLimit) {
      this._events.shift();
      this._overflow += 1;
      this._lastError = "fake star transport event queue is full";
    }
    this._events.push(event);
  }

  private _recordError(
    code: TransportErrorCode,
    message: string,
    peer?: FakeStarPeer,
    channel?: TransportChannel,
  ): void {
    this._lastError = message;
    if (code === "malformed" || code === "payload_too_large" || code === "channel_mismatch") {
      this._malformed += 1;
      if (peer) {
        peer.malformed += 1;
      }
    } else if (code === "unsupported_version") {
      this._unsupportedVersion += 1;
    } else if (code === "overflow") {
      this._overflow += 1;
    } else if (code === "backpressure") {
      this._backpressure += 1;
      if (peer) {
        peer.backpressure += 1;
      }
    }
    if (peer) {
      peer.lastError = message;
      this._pushEvent({
        kind: "peer_error",
        peer_id: peer.peerId,
        ...(channel !== undefined ? { channel } : {}),
        code,
        message,
      });
    } else {
      this._pushEvent({ kind: "star_error", code, message });
    }
  }

  private _setPeerState(peer: FakeStarPeer, state: TransportPeerState): void {
    peer.state = state;
    for (const channel of contract.CHANNEL_ORDER) {
      peer.channels[channel].state = state;
    }
    this._pushEvent({ kind: "peer_state", peer_id: peer.peerId, state });
  }

  private _setState(state: TransportState): void {
    this._state = state;
    this._pushEvent({ kind: "star_state", state });
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

  initialize(): TransportResult<true> {
    if (this._state === "connected") {
      return ok(true);
    }
    this._setState("connected");
    if (this._role === "guest" && !this._peers.has(contract.HOST_PEER_ID)) {
      this._addPeer(contract.HOST_PEER_ID);
    }
    return ok(true);
  }

  private _addPeer(peerId: string): number {
    const slot = this._order.length + 1;
    const peer: FakeStarPeer = {
      peerId,
      slot,
      state: "connecting",
      iceState: "new",
      channels: { control: newChannel(), input: newChannel() },
      arrivalSeq: 0,
      signal: null,
      sequenceGaps: 0,
      backpressure: 0,
      malformed: 0,
      lastError: null,
      link: null,
      linkPeerId: null,
    };
    this._peers.set(peerId, peer);
    this._order[slot - 1] = peerId;
    this._pushEvent({ kind: "peer_state", peer_id: peerId, state: "connecting" });
    return slot;
  }

  /** Host-only. Allocates one guest slot with its own control and input channels. */
  openPeer(peerId: string): TransportResult<number> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      return failure("role_forbidden", "only a host opens guest links");
    }
    const idResult = contract.validatePeerId(peerId);
    if (!idResult.ok) {
      this._recordError(idResult.error.code, idResult.error.message);
      return idResult;
    }
    if (peerId === contract.HOST_PEER_ID) {
      this._recordError("duplicate_peer", "the host peer id is reserved");
      return failure("duplicate_peer", "the host peer id is reserved");
    }
    if (this._peers.has(peerId)) {
      this._recordError("duplicate_peer", "star peer is already open");
      return failure("duplicate_peer", "star peer is already open");
    }
    if (this._order.length >= this._maxGuests) {
      this._recordError("capacity", "star transport is at guest capacity");
      return failure("capacity", "star transport is at guest capacity");
    }
    return ok(this._addPeer(peerId));
  }

  /**
   * Test seam: joins a guest endpoint to an already-opened host slot. The
   * browser adapter reaches the same state through manual offer/answer
   * exchange.
   */
  link(guest: FakeStarTransport): TransportResult<true> {
    if (this._role !== "host" || guest._role !== "guest") {
      return failure("role_forbidden", "link joins one host endpoint to one guest endpoint");
    }
    const peer = this._peers.get(guest._peerId);
    if (!peer) {
      return failure("unknown_peer", "star host has no slot for that guest");
    }
    const hostPeer = guest._peers.get(contract.HOST_PEER_ID);
    if (!hostPeer) {
      return failure("not_connected", "guest endpoint is not initialized");
    }
    peer.link = guest;
    peer.linkPeerId = contract.HOST_PEER_ID;
    hostPeer.link = this;
    hostPeer.linkPeerId = guest._peerId;
    peer.iceState = "connected";
    hostPeer.iceState = "connected";
    this._setPeerState(peer, "connected");
    guest._setPeerState(hostPeer, "connected");
    return ok(true);
  }

  /**
   * Manual signaling, in process. The browser adapter drives the same four
   * calls against real SDP; here the blob is an opaque token that resolves
   * back to the offering host, so a lobby can script the full handshake
   * without a browser.
   */
  requestOffer(peerId: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      this._recordError("role_forbidden", "only a host creates offers");
      return failure("role_forbidden", "only a host creates offers");
    }
    const peerResult = this._peer(peerId);
    if (!peerResult.ok) {
      this._recordError(peerResult.error.code, peerResult.error.message);
      return peerResult;
    }
    const peer = peerResult.value;
    const rendezvous = this._rendezvous;
    rendezvous.sequence += 1;
    const token = `offer:${rendezvous.sequence}`;
    rendezvous.offers.set(token, { host: this, peerId });
    peer.signal = token;
    peer.iceState = "checking";
    return ok(true);
  }

  acceptOffer(signal: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "guest") {
      this._recordError("role_forbidden", "only a guest accepts an offer");
      return failure("role_forbidden", "only a guest accepts an offer");
    }
    const offer = typeof signal === "string" ? this._rendezvous.offers.get(signal) : undefined;
    if (!offer) {
      this._recordError("signal_error", "remote offer is not a known fake star offer");
      return failure("signal_error", "remote offer is not a known fake star offer");
    }
    const hostPeer = this._peers.get(contract.HOST_PEER_ID);
    if (!hostPeer) {
      return failure("not_connected", "guest endpoint is not initialized");
    }
    this._rendezvous.answers.set(signal, this);
    hostPeer.signal = "answer:" + signal;
    hostPeer.iceState = "checking";
    return ok(true);
  }

  acceptAnswer(peerId: string, signal: string): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      this._recordError("role_forbidden", "only a host accepts an answer");
      return failure("role_forbidden", "only a host accepts an answer");
    }
    const peerResult = this._peer(peerId);
    if (!peerResult.ok) {
      this._recordError(peerResult.error.code, peerResult.error.message);
      return peerResult;
    }
    const peer = peerResult.value;
    const rendezvous = this._rendezvous;
    const tokenMatch = typeof signal === "string" ? /^answer:(.+)$/.exec(signal) : null;
    const token = tokenMatch ? tokenMatch[1] : undefined;
    const offer = token !== undefined ? rendezvous.offers.get(token) : undefined;
    if (!offer || offer.host !== this || offer.peerId !== peerId) {
      this._recordError("signal_error", "remote answer does not match a pending offer");
      return failure("signal_error", "remote answer does not match a pending offer");
    }
    const guest = token !== undefined ? rendezvous.answers.get(token) : undefined;
    if (!guest) {
      this._recordError("signal_error", "no guest endpoint accepted that offer");
      return failure("signal_error", "no guest endpoint accepted that offer");
    }
    if (token !== undefined) {
      rendezvous.offers.delete(token);
      rendezvous.answers.delete(token);
    }
    peer.signal = null;
    return this.link(guest);
  }

  /** Returns the locally produced blob once, or nil while none is pending. */
  takeSignal(peerId: string): TransportResult<string | null> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const peerResult = this._peer(peerId);
    if (!peerResult.ok) {
      return peerResult;
    }
    const peer = peerResult.value;
    const signal = peer.signal;
    peer.signal = null;
    return ok(signal);
  }

  private _peer(peerId: string): TransportResult<FakeStarPeer> {
    const peer = this._peers.get(peerId);
    if (!peer) {
      return failure("unknown_peer", "star transport has no peer with that id");
    }
    return ok(peer);
  }

  private _dropPeerQueues(peer: FakeStarPeer): void {
    for (const channelName of contract.CHANNEL_ORDER) {
      const channel = peer.channels[channelName];
      this._droppedOutbound += channel.outbound.length;
      this._droppedInbound += channel.inbound.length;
      channel.droppedOutbound += channel.outbound.length;
      channel.droppedInbound += channel.inbound.length;
      channel.outbound = [];
      channel.inbound = [];
      channel.bufferedAmount = 0;
    }
  }

  /**
   * Closes one link without disturbing the other guests. Repeated calls are
   * idempotent once the peer reports `closed`.
   */
  closePeer(peerId: string, reason?: string): TransportResult<true> {
    const peerResult = this._peer(peerId);
    if (!peerResult.ok) {
      return peerResult;
    }
    const peer = peerResult.value;
    if (peer.state === "closed") {
      return ok(true);
    }
    const detail = reason ?? "star peer closed";
    this._dropPeerQueues(peer);
    peer.iceState = "closed";
    const link = peer.link;
    const linkPeerId = peer.linkPeerId;
    peer.link = null;
    peer.linkPeerId = null;
    this._setPeerState(peer, "closed");
    this._recordError("disconnected", detail, peer);
    if (link && linkPeerId) {
      const remote = link._peers.get(linkPeerId);
      if (remote && remote.state !== "closed") {
        remote.link = null;
        remote.linkPeerId = null;
        link._dropPeerQueues(remote);
        remote.iceState = "closed";
        link._setPeerState(remote, "disconnected");
        link._recordError("disconnected", detail, remote);
      }
    }
    return ok(true);
  }

  shutdown(): TransportResult<true> {
    if (this._state === "closed") {
      return ok(true);
    }
    for (const peerId of this._order) {
      this.closePeer(peerId, "star transport shutdown");
    }
    // Teardown releases every slot, so a repeated shutdown and any later
    // initialize start from an empty star instead of orphan links.
    this._peers = new Map();
    this._order = [];
    this._cursor = 0;
    this._setState("closed");
    return ok(true);
  }

  private _enqueue(
    peer: FakeStarPeer,
    channelName: TransportChannel,
    message: TransportMessage,
  ): TransportResult<true> {
    const validated = contract.validateChannelMessage(channelName, message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message, peer);
      return validated;
    }
    if (peer.state !== "connected") {
      this._recordError("not_connected", "star peer link is not connected", peer, channelName);
      return failure("not_connected", "star peer link is not connected");
    }
    const channel = peer.channels[channelName];
    const wireResult = contract.encode(message);
    if (!wireResult.ok) {
      throw new Error(wireResult.error.message);
    }
    const wire = wireResult.value;
    // The queue bound is the only place a send is refused. A full send
    // buffer is backpressure, not a rejection: it leaves messages queued
    // until the channel drains or the bounded queue overflows.
    if (channel.outbound.length >= this._queueLimit) {
      channel.droppedOutbound += 1;
      this._droppedOutbound += 1;
      this._recordError("overflow", "star peer outbound queue is full", peer, channelName);
      return failure("overflow", "star peer outbound queue is full");
    }
    channel.outbound.push({
      peer_id: peer.linkPeerId ?? peer.peerId,
      channel: channelName,
      message: contract.copy(message),
    });
    channel.bufferedAmount += wire.length;
    channel.sent += 1;
    this._sent += 1;
    // One count per queued copy. A host `broadcast` reaches here once per
    // guest link, which is exactly the fan-out amplification a relay
    // removes and the reason this counter exists at all.
    this._uplinkUnits += 1;
    this._uplinkBytes += wire.length;
    if (channelName === "input") {
      this._inputUplinkBytes += wire.length;
    }
    return ok(true);
  }

  /** Host: address one guest. Guest: the only legal target is the host link. */
  send(
    peerId: string,
    channel: TransportChannel,
    message: TransportMessage,
  ): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role === "guest" && peerId !== contract.HOST_PEER_ID) {
      this._recordError("role_forbidden", "a guest may only address the host");
      return failure("role_forbidden", "a guest may only address the host");
    }
    // Message shape decides the returned code before peer resolution,
    // because the browser adapter cannot authoritatively resolve a peer
    // without asking the bridge. The peer is still looked up first so a
    // shape fault aimed at a known link stays attributed to that link in its
    // diagnostics.
    const peer = this._peers.get(peerId);
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message, peer);
      return validated;
    }
    if (!peer) {
      this._recordError("unknown_peer", "star transport has no peer with that id");
      return failure("unknown_peer", "star transport has no peer with that id");
    }
    return this._enqueue(peer, channel, message);
  }

  /**
   * Host-only fan-out. Returns how many links accepted the message;
   * per-peer failures stay visible through peer events and diagnostics.
   */
  broadcast(channel: TransportChannel, message: TransportMessage): TransportResult<number> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    if (this._role !== "host") {
      this._recordError("role_forbidden", "only a host fans out canonical batches");
      return failure("role_forbidden", "only a host fans out canonical batches");
    }
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message);
      return validated;
    }
    let delivered = 0;
    for (const peerId of this._order) {
      const peer = this._peers.get(peerId);
      if (peer && peer.state === "connected") {
        if (this._enqueue(peer, channel, message).ok) {
          delivered += 1;
        }
      }
    }
    return ok(delivered);
  }

  private _receive(
    peer: FakeStarPeer,
    channelName: TransportChannel,
    addressed: TransportAddressedMessage,
    wireBytes?: number,
  ): void {
    const channel = peer.channels[channelName];
    this._downlinkFrames += 1;
    let receivedBytes = wireBytes;
    if (receivedBytes === undefined) {
      const encoded = contract.encode(addressed.message);
      if (!encoded.ok) {
        throw new Error(encoded.error.message);
      }
      receivedBytes = encoded.value.length;
    }
    this._downlinkBytes += receivedBytes;
    if (channelName === "input") {
      this._inputDownlinkBytes += receivedBytes;
    }
    if (channel.inbound.length >= this._queueLimit) {
      channel.droppedInbound += 1;
      this._droppedInbound += 1;
      this._recordError("overflow", "star peer inbound queue is full", peer, channelName);
      return;
    }
    const seq = addressed.message.seq;
    if (channel.lastSeq !== null && seq > channel.lastSeq + 1) {
      peer.sequenceGaps += seq - channel.lastSeq - 1;
    }
    if (channel.lastSeq === null || seq > channel.lastSeq) {
      channel.lastSeq = seq;
    }
    // `arrival_seq` is stamped at poll time, not here. It then means the
    // same thing on both adapters: the ordinal of this message among what
    // the caller has drained for this peer. The browser adapter cannot
    // observe receive order at all, so receive-time numbering would
    // diverge.
    channel.inbound.push({
      peer_id: peer.peerId,
      channel: channelName,
      arrival_seq: 0,
      message: addressed.message,
    });
  }

  /**
   * Test seam: delivers everything currently buffered on this endpoint and
   * on every endpoint linked to it, then clears the simulated
   * `bufferedAmount`. The browser adapter has no equivalent because the
   * browser drains its own data channels asynchronously.
   */
  pump(): void {
    const endpoints: FakeStarTransport[] = [this];
    const seen = new Set<FakeStarTransport>([this]);
    for (const peerId of this._order) {
      const link = this._peers.get(peerId)?.link;
      if (link && !seen.has(link)) {
        seen.add(link);
        endpoints.push(link);
      }
    }
    for (const endpoint of endpoints) {
      endpoint._flush();
    }
  }

  /**
   * Drains each channel until its send budget is exhausted.
   * `buffered_amount_limit` stands in for a data channel's `bufferedAmount`
   * ceiling: once a pass reaches it, the remainder stays queued and the peer
   * reports backpressure instead of blocking.
   */
  private _flush(): void {
    for (const peerId of this._order) {
      const peer = this._peers.get(peerId);
      if (!peer) {
        continue;
      }
      for (const channelName of contract.CHANNEL_ORDER) {
        const channel = peer.channels[channelName];
        const link = peer.link;
        const linkPeerId = peer.linkPeerId;
        let budget = this._bufferedAmountLimit;
        while (channel.outbound.length > 0) {
          const addressed = channel.outbound[0] as TransportAddressedMessage;
          const encoded = contract.encode(addressed.message);
          if (!encoded.ok) {
            throw new Error(encoded.error.message);
          }
          const size = encoded.value.length;
          if (size > budget) {
            // Latch the report so a saturated buffer costs one event, not
            // one per drain pass. It clears once the channel moves.
            if (!channel.backpressured) {
              channel.backpressured = true;
              this._recordError(
                "backpressure",
                "star peer channel send buffer is full",
                peer,
                channelName,
              );
            }
            break;
          }
          channel.outbound.shift();
          channel.backpressured = false;
          budget -= size;
          channel.bufferedAmount = Math.max(0, channel.bufferedAmount - size);
          const remote = link && linkPeerId ? link._peers.get(linkPeerId) : undefined;
          if (link && remote && peer.state === "connected") {
            link._receive(remote, channelName, addressed, size);
          } else {
            channel.droppedOutbound += 1;
            this._droppedOutbound += 1;
          }
        }
      }
    }
  }

  /**
   * Deterministic drain: a persistent cursor walks (slot, channel-rank)
   * pairs and takes at most one message per pair before moving on, so no
   * peer can starve another. The JavaScript bridge advances the same
   * cursor. This is an ordering rule at this boundary, not a claim that
   * browser callback arrival order is simulation order; the session layer
   * above still reorders by tick.
   */
  private _take(): TransportPeerMessage | null {
    const slots = this._order.length;
    const channels = contract.CHANNEL_ORDER.length;
    if (slots === 0) {
      return null;
    }
    for (let iteration = 0; iteration < slots * channels; iteration += 1) {
      const peerIndex = Math.floor(this._cursor / channels) % slots;
      const channelIndex = this._cursor % channels;
      this._cursor = (this._cursor + 1) % (slots * channels);
      const peerId = this._order[peerIndex] as string;
      const peer = this._peers.get(peerId) as FakeStarPeer;
      const channelName = contract.CHANNEL_ORDER[channelIndex] as TransportChannel;
      const channel = peer.channels[channelName];
      if (channel.inbound.length > 0) {
        const entry = channel.inbound.shift() as TransportPeerMessage;
        channel.received += 1;
        this._received += 1;
        peer.arrivalSeq += 1;
        return { ...entry, arrival_seq: peer.arrivalSeq };
      }
    }
    return null;
  }

  pollBatch(limit?: number): readonly TransportPeerMessage[] {
    const budget = limit ?? contract.DEFAULT_POLL_BATCH;
    const batch: TransportPeerMessage[] = [];
    if (this._state !== "connected") {
      return batch;
    }
    while (batch.length < budget) {
      const entry = this._take();
      if (!entry) {
        return batch;
      }
      batch.push(entry);
    }
    return batch;
  }

  poll(): TransportResult<TransportPeerMessage | null> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    return ok(this.pollBatch(1)[0] ?? null);
  }

  pollEvent(): TransportPeerEvent | null {
    return this._events.shift() ?? null;
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
    return this._peers.get(peerId)?.state ?? null;
  }

  state(): TransportState {
    return this._state;
  }

  /**
   * Encoded envelope wires this endpoint put on its links and took off
   * them, one count per copy. A host `broadcast` costs one copy per guest
   * link here and one copy in total on `fake_relay`; these counters are
   * what makes that comparable without either adapter having to be trusted
   * about it.
   */
  wireBytes(): readonly [uplinkBytes: number, downlinkBytes: number] {
    return [this._uplinkBytes, this._downlinkBytes];
  }

  wireCounters(): FakeRelayWireCounters {
    return {
      uplink_bytes: this._uplinkBytes,
      downlink_bytes: this._downlinkBytes,
      uplink_units: this._uplinkUnits,
      input_uplink_bytes: this._inputUplinkBytes,
      input_downlink_bytes: this._inputDownlinkBytes,
      // A star pays no addressing on the wire: each guest link is its own
      // data channel, so the origin of an arrival is the channel it arrived
      // on. The framed figure is therefore the envelope figure.
      downlink_framed_bytes: this._downlinkBytes,
      downlink_frames: this._downlinkFrames,
      // A star delivers one envelope per message; there is no framing layer
      // above the envelope to pay for.
      frame_overhead_bytes: 0,
    };
  }

  diagnostics(): TransportStarDiagnostics {
    const peers: TransportPeerDiagnostics[] = [];
    for (const peerId of this._order) {
      const peer = this._peers.get(peerId);
      if (!peer) {
        continue;
      }
      peers.push({
        peer_id: peer.peerId,
        slot: peer.slot,
        state: peer.state,
        ice_state: peer.iceState,
        control: channelDiagnostics(peer.channels.control),
        input: channelDiagnostics(peer.channels.input),
        sequence_gaps: peer.sequenceGaps,
        backpressure: peer.backpressure,
        malformed: peer.malformed,
        last_error: peer.lastError,
      });
    }
    return {
      role: this._role,
      state: this._state,
      capacity: this._maxGuests,
      peer_count: this._order.length,
      queue_limit: this._queueLimit,
      buffered_amount_limit: this._bufferedAmountLimit,
      event_depth: this._events.length,
      sent: this._sent,
      received: this._received,
      dropped_outbound: this._droppedOutbound,
      dropped_inbound: this._droppedInbound,
      malformed: this._malformed,
      unsupported_version: this._unsupportedVersion,
      overflow: this._overflow,
      backpressure: this._backpressure,
      last_error: this._lastError,
      peers,
    };
  }
}

function channelDiagnostics(channel: FakeStarChannel): TransportPeerDiagnostics["control"] {
  return {
    state: channel.state,
    outbound_depth: channel.outbound.length,
    inbound_depth: channel.inbound.length,
    buffered_amount: channel.bufferedAmount,
    sent: channel.sent,
    received: channel.received,
    dropped_outbound: channel.droppedOutbound,
    dropped_inbound: channel.droppedInbound,
  };
}
