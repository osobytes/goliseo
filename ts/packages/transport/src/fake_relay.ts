// Pure in-process **relay** transport: a third `StarTransportAdapter`
// implementation alongside `fake_star` and `browser_star`, built to measure
// the OMP-4 relay topology in the #169 fault harness before any server
// exists.
//
// # What makes it a relay rather than a star
//
// In `fake_star` one endpoint *is* the hub: a host `broadcast` enqueues one
// copy per guest link on the host's own uplink, and a guest may address
// nobody but the host. Here every endpoint holds exactly **one** physical
// link, to the room, and the room is not a player:
//
//  * `broadcast` costs **one** uplink copy no matter how many members
//    receive it. That single difference is the whole bandwidth claim under
//    test.
//  * Any member may address any other member. No role is privileged, and
//    `role()` is carried only because the adapter contract and the session
//    layer above still name one. The room enforces nothing on it.
//  * There is no manual signaling. `requestOffer`, `acceptOffer`,
//    `acceptAnswer` and `takeSignal` cannot mean anything when the far end
//    is a server the client dials, so they refuse rather than pretend. The
//    room is the rendezvous.
//
// # The room never parses a game packet
//
// This is the property the topology decision rests on, so it is structural
// here rather than promised. An endpoint encodes its own outbound message
// into the contract's peer-addressed wire (`origin|channel|<envelope>`)
// *before* handing it up. `FakeRelayRoom` only ever appends those opaque
// strings to a per-destination list, concatenates the list with a newline
// once per forward pass, and hands the frame down. It calls no decoder,
// reads no field, and knows nothing about slots, ticks, ownership, or
// canonicalisation. The receiving endpoint splits the frame and decodes
// each line.
//
// The only thing the room reads is the **destination set** attached to each
// unit by the sending adapter, which is link identity — the same thing a
// real relay reads from the data channel a packet arrived on plus the room
// it belongs to.
//
// A member never receives its own line back: `forward` skips the origin.
//
// # Byte accounting
//
// `uplinkBytes` and `downlinkBytes` count encoded envelope wires, one count
// per copy that actually crosses the link, which is exactly how `fake_star`
// counts. On a star a seven-guest `broadcast` is seven uplink copies; here
// it is one. `frameOverheadBytes` is kept separately so a comparison never
// has to guess whether framing was folded into the payload figure.

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

export interface FakeRelayRoomCounters {
  /** Framed downlink messages the room emitted. */
  frames: number;
  /** Opaque lines forwarded, counted once per destination. */
  lines: number;
  /** Lines with no connected destination left. */
  dropped: number;
}

/**
 * One room. Members reach each other only through a room they were all
 * handed, so two rooms in one process cannot cross-talk — the same
 * explicitness `fake_star.newRendezvous` buys for a single logical star.
 */
export interface FakeRelayRoom {
  /** Join order; the only ordering the room has. */
  members: string[];
  endpoints: Map<string, FakeRelayTransport>;
  /** Destination peer id -> opaque lines this pass. */
  inbox: Map<string, string[]>;
  counters: FakeRelayRoomCounters;
}

export function newRoom(): FakeRelayRoom {
  return {
    members: [],
    endpoints: new Map(),
    inbox: new Map(),
    counters: { frames: 0, lines: 0, dropped: 0 },
  };
}

export interface FakeRelayTransportOptions {
  /** Carried for the session layer; the room ignores it. */
  readonly role?: TransportRole;
  /** Room-unique member identity. */
  readonly peer_id: string;
  /** Share one across every member of a single room. */
  readonly room?: FakeRelayRoom;
  readonly queue_limit?: number;
  readonly max_peers?: number;
  readonly buffered_amount_limit?: number;
}

interface FakeRelayChannel {
  state: TransportPeerState;
  inbound: TransportPeerMessage[];
  received: number;
  droppedInbound: number;
  lastSeq: number | null;
}

interface FakeRelayPeer {
  peerId: string;
  slot: number;
  state: TransportPeerState;
  iceState: string;
  channels: Record<TransportChannel, FakeRelayChannel>;
  arrivalSeq: number;
  sequenceGaps: number;
  backpressure: number;
  malformed: number;
  /** Uplink units addressed to this member and released. */
  sent: number;
  droppedOutbound: number;
  lastError: string | null;
}

interface FakeRelayUnit {
  channel: TransportChannel;
  /** Member ids resolved at send time; canonical order. */
  targets: string[];
  targetSet: Set<string>;
  /** `origin|channel|<envelope wire>`; opaque to the room. */
  line: string;
  /** Encoded envelope wire length, excluding the frame header. */
  bytes: number;
}

interface FakeRelayUplink {
  units: FakeRelayUnit[];
  bufferedAmount: number;
  backpressured: boolean;
  sent: number;
  droppedOutbound: number;
}

function failure<T>(code: TransportErrorCode, message: string): TransportResult<T> {
  return err({ message, code });
}

function newChannel(): FakeRelayChannel {
  return { state: "connecting", inbound: [], received: 0, droppedInbound: 0, lastSeq: null };
}

function newUplink(): FakeRelayUplink {
  return { units: [], bufferedAmount: 0, backpressured: false, sent: 0, droppedOutbound: 0 };
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

/** Pure in-process relay transport: one link per member, to a shared room. */
export class FakeRelayTransport implements StarTransportAdapter {
  private readonly _role: TransportRole;
  private readonly _peerId: string;
  private _state: TransportState = "new";
  private readonly _queueLimit: number;
  private readonly _eventLimit: number;
  private readonly _maxPeers: number;
  private readonly _bufferedAmountLimit: number;
  private readonly _room: FakeRelayRoom;
  private _peers = new Map<string, FakeRelayPeer>();
  private _order: string[] = [];
  private _uplink: Record<TransportChannel, FakeRelayUplink> = {
    control: newUplink(),
    input: newUplink(),
  };
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
  private _downlinkFramedBytes = 0;
  private _uplinkUnits = 0;
  private _downlinkFrames = 0;
  private _frameOverhead = 0;
  private _lastError: string | null = null;

  constructor(options: FakeRelayTransportOptions) {
    const role = options.role ?? "guest";
    if (role !== "host" && role !== "guest") {
      throw new Error("fake relay transport role must be host or guest");
    }
    const queueLimit = options.queue_limit ?? contract.DEFAULT_QUEUE_LIMIT;
    if (
      queueLimit !== Math.floor(queueLimit) ||
      queueLimit <= 0 ||
      queueLimit > contract.MAX_QUEUE_LIMIT
    ) {
      throw new Error("fake relay transport queue_limit is outside the supported range");
    }
    const maxPeers = options.max_peers ?? contract.MAX_GUESTS;
    if (maxPeers !== Math.floor(maxPeers) || maxPeers <= 0 || maxPeers > contract.MAX_GUESTS) {
      throw new Error("fake relay transport max_peers is outside the supported range");
    }
    const bufferedAmountLimit =
      options.buffered_amount_limit ?? contract.DEFAULT_BUFFERED_AMOUNT_LIMIT;
    if (
      bufferedAmountLimit !== Math.floor(bufferedAmountLimit) ||
      bufferedAmountLimit <= 0 ||
      bufferedAmountLimit > contract.MAX_BUFFERED_AMOUNT_LIMIT
    ) {
      throw new Error("fake relay transport buffered_amount_limit is outside the supported range");
    }
    if (!options.peer_id) {
      throw new Error("a fake relay member needs its own peer id");
    }
    const peerIdResult = contract.validatePeerId(options.peer_id);
    if (!peerIdResult.ok) {
      throw new Error(peerIdResult.error.message);
    }
    this._role = role;
    this._peerId = options.peer_id;
    this._queueLimit = queueLimit;
    this._eventLimit = Math.max(2, queueLimit);
    this._maxPeers = maxPeers;
    this._bufferedAmountLimit = bufferedAmountLimit;
    this._room = options.room ?? newRoom();
  }

  private _pushEvent(event: TransportPeerEvent): void {
    if (this._events.length >= this._eventLimit) {
      this._events.shift();
      this._overflow += 1;
      this._lastError = "fake relay transport event queue is full";
    }
    this._events.push(event);
  }

  private _recordError(
    code: TransportErrorCode,
    message: string,
    peer?: FakeRelayPeer,
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

  private _setPeerState(peer: FakeRelayPeer, state: TransportPeerState): void {
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
      return failure("not_initialized", "relay transport is not initialized");
    }
    if (this._state === "closed") {
      return failure("closed", "relay transport is closed");
    }
    if (this._state !== "connected") {
      return failure("not_connected", "relay transport is not connected");
    }
    return ok(true);
  }

  private _addPeer(peerId: string): number {
    const slot = this._order.length + 1;
    const peer: FakeRelayPeer = {
      peerId,
      slot,
      state: "connecting",
      iceState: "new",
      channels: { control: newChannel(), input: newChannel() },
      arrivalSeq: 0,
      sequenceGaps: 0,
      backpressure: 0,
      malformed: 0,
      sent: 0,
      droppedOutbound: 0,
      lastError: null,
    };
    this._peers.set(peerId, peer);
    this._order[slot - 1] = peerId;
    this._pushEvent({ kind: "peer_state", peer_id: peerId, state: "connecting" });
    return slot;
  }

  /**
   * Joining the room is the whole handshake. Every member already in the
   * room gains a link to the newcomer and the newcomer gains one to each of
   * them, in join order, so slot numbering is a deterministic function of
   * arrival and not of any hash order.
   */
  initialize(): TransportResult<true> {
    if (this._state === "connected") {
      return ok(true);
    }
    const room = this._room;
    const existing = room.endpoints.get(this._peerId);
    if (existing !== undefined && existing !== this) {
      return failure("duplicate_peer", "the relay room already holds that member id");
    }
    this._setState("connected");
    room.endpoints.set(this._peerId, this);
    if (!room.inbox.has(this._peerId)) {
      room.inbox.set(this._peerId, []);
    }
    if (!room.members.includes(this._peerId)) {
      room.members.push(this._peerId);
    }
    for (const memberId of room.members) {
      if (memberId !== this._peerId) {
        const other = room.endpoints.get(memberId);
        if (other !== undefined && other._state === "connected") {
          this._connectTo(other);
          other._connectTo(this);
        }
      }
    }
    return ok(true);
  }

  /**
   * Bring one directed link up. Both endpoints call this on each other,
   * which is what makes a member's peer table symmetric without either
   * side being "the" opener.
   */
  private _connectTo(other: FakeRelayTransport): void {
    let peer = this._peers.get(other._peerId);
    if (peer === undefined) {
      if (this._order.length >= this._maxPeers) {
        this._recordError("capacity", "relay room membership is at capacity");
        return;
      }
      this._addPeer(other._peerId);
      // The assertion is NOT redundant, whatever eslint says here. The lint
      // runs on typescript@6 (the last release with a JS compiler API -- see
      // ts/tools/lint/tseslint.mjs) and the build runs on the pinned
      // typescript@7, and the two disagree about the control flow through
      // `_addPeer`: without this, `tsc --build --force` fails with TS18048
      // three lines below. When they stop disagreeing, delete both this
      // comment and the directive.
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion
      peer = this._peers.get(other._peerId) as FakeRelayPeer;
    }
    if (peer.state === "connected") {
      return;
    }
    peer.iceState = "connected";
    this._setPeerState(peer, "connected");
  }

  private _peer(peerId: string): TransportResult<FakeRelayPeer> {
    const peer = this._peers.get(peerId);
    if (!peer) {
      return failure("unknown_peer", "relay transport has no member with that id");
    }
    return ok(peer);
  }

  /**
   * Declaring a member explicitly. A relay has no privileged opener, so
   * unlike the star this is not host-only: it is here because the adapter
   * contract names it, and because a caller may want a link before the far
   * member has joined.
   */
  openPeer(peerId: string): TransportResult<number> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const idResult = contract.validatePeerId(peerId);
    if (!idResult.ok) {
      this._recordError(idResult.error.code, idResult.error.message);
      return idResult;
    }
    if (peerId === this._peerId) {
      this._recordError("duplicate_peer", "a relay member cannot address itself");
      return failure("duplicate_peer", "a relay member cannot address itself");
    }
    if (this._peers.has(peerId)) {
      this._recordError("duplicate_peer", "relay member is already open");
      return failure("duplicate_peer", "relay member is already open");
    }
    if (this._order.length >= this._maxPeers) {
      this._recordError("capacity", "relay room membership is at capacity");
      return failure("capacity", "relay room membership is at capacity");
    }
    return ok(this._addPeer(peerId));
  }

  private _dropPeerQueues(peer: FakeRelayPeer): void {
    for (const channelName of contract.CHANNEL_ORDER) {
      const channel = peer.channels[channelName];
      this._droppedInbound += channel.inbound.length;
      channel.droppedInbound += channel.inbound.length;
      channel.inbound = [];
    }
    // Uplink units addressed only to this member have nowhere left to go.
    // They are dropped here rather than at flush so teardown drains,
    // exactly as the star drops a closed link's outbound queue.
    for (const channelName of contract.CHANNEL_ORDER) {
      const uplink = this._uplink[channelName];
      const kept: FakeRelayUnit[] = [];
      for (const unit of uplink.units) {
        if (unit.targetSet.has(peer.peerId)) {
          unit.targetSet.delete(peer.peerId);
          unit.targets = unit.targets.filter((target) => target !== peer.peerId);
        }
        if (unit.targets.length > 0) {
          kept.push(unit);
        } else {
          uplink.bufferedAmount = Math.max(0, uplink.bufferedAmount - unit.bytes);
          uplink.droppedOutbound += 1;
          peer.droppedOutbound += 1;
          this._droppedOutbound += 1;
        }
      }
      uplink.units = kept;
    }
  }

  /**
   * Closes one member link without disturbing the rest of the room. The
   * remote member is told, because a relay does know when a client's link
   * to it drops.
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
    const detail = reason ?? "relay member closed";
    this._dropPeerQueues(peer);
    peer.iceState = "closed";
    this._setPeerState(peer, "closed");
    this._recordError("disconnected", detail, peer);
    const other = this._room.endpoints.get(peerId);
    if (other !== undefined && other !== this) {
      const remote = other._peers.get(this._peerId);
      if (remote !== undefined && remote.state !== "closed") {
        other._dropPeerQueues(remote);
        remote.iceState = "closed";
        other._setPeerState(remote, "disconnected");
        other._recordError("disconnected", detail, remote);
      }
    }
    return ok(true);
  }

  shutdown(): TransportResult<true> {
    if (this._state === "closed") {
      return ok(true);
    }
    for (const peerId of this._order) {
      this.closePeer(peerId, "relay transport shutdown");
    }
    this._peers = new Map();
    this._order = [];
    this._cursor = 0;
    this._uplink = { control: newUplink(), input: newUplink() };
    const room = this._room;
    room.endpoints.delete(this._peerId);
    room.inbox.delete(this._peerId);
    room.members = room.members.filter((memberId) => memberId !== this._peerId);
    this._setState("closed");
    return ok(true);
  }

  // ---------------------------------------------------------------------
  // The uplink
  // ---------------------------------------------------------------------

  /**
   * One unit on the single link to the room, whatever its destination set.
   * This is the whole topological difference from `fake_star`, where the
   * same call costs one queued copy per destination.
   */
  private _enqueue(
    channelName: TransportChannel,
    message: TransportMessage,
    targets: string[],
    attributed?: FakeRelayPeer,
  ): TransportResult<true> {
    const uplink = this._uplink[channelName];
    if (uplink.units.length >= this._queueLimit) {
      uplink.droppedOutbound += 1;
      this._droppedOutbound += 1;
      this._recordError("overflow", "relay uplink queue is full", attributed, channelName);
      return failure("overflow", "relay uplink queue is full");
    }
    const wireResult = contract.encode(message);
    if (!wireResult.ok) {
      throw new Error(wireResult.error.message);
    }
    const lineResult = contract.encodeAddressed(this._peerId, channelName, message);
    if (!lineResult.ok) {
      throw new Error(lineResult.error.message);
    }
    const wire = wireResult.value;
    const line = lineResult.value;
    const targetSet = new Set(targets);
    uplink.units.push({ channel: channelName, targets, targetSet, line, bytes: wire.length });
    uplink.bufferedAmount += wire.length;
    uplink.sent += 1;
    this._sent += 1;
    this._uplinkUnits += 1;
    this._uplinkBytes += wire.length;
    if (channelName === "input") {
      this._inputUplinkBytes += wire.length;
    }
    this._frameOverhead += line.length - wire.length;
    return ok(true);
  }

  /**
   * Address one member. Any member may address any other: the relay has no
   * privileged direction, which is precisely what removes the sequencer.
   */
  send(
    peerId: string,
    channel: TransportChannel,
    message: TransportMessage,
  ): TransportResult<true> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const peer = this._peers.get(peerId);
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message, peer);
      return validated;
    }
    if (!peer) {
      this._recordError("unknown_peer", "relay transport has no member with that id");
      return failure("unknown_peer", "relay transport has no member with that id");
    }
    if (peer.state !== "connected") {
      this._recordError("not_connected", "relay member link is not connected", peer, channel);
      return failure("not_connected", "relay member link is not connected");
    }
    return this._enqueue(channel, message, [peerId], peer);
  }

  /**
   * Fan-out for the price of one upload. Returns how many members the room
   * will frame this to; per-member failures stay visible through events
   * and diagnostics, exactly as on the star.
   */
  broadcast(channel: TransportChannel, message: TransportMessage): TransportResult<number> {
    const connected = this._requireConnected();
    if (!connected.ok) {
      return connected;
    }
    const validated = contract.validateChannelMessage(channel, message);
    if (!validated.ok) {
      this._recordError(validated.error.code, validated.error.message);
      return validated;
    }
    const targets: string[] = [];
    for (const peerId of this._order) {
      if (this._peers.get(peerId)?.state === "connected") {
        targets.push(peerId);
      }
    }
    if (targets.length === 0) {
      return ok(0);
    }
    const enqueued = this._enqueue(channel, message, targets);
    if (!enqueued.ok) {
      return enqueued;
    }
    return ok(targets.length);
  }

  // ---------------------------------------------------------------------
  // The room
  // ---------------------------------------------------------------------

  /**
   * Test seam, mirroring `fake_star`'s `pump`. Flush every member's uplink
   * into the room in join order, then let the room frame one message per
   * destination.
   */
  static pumpRoom(room: FakeRelayRoom): void {
    for (const memberId of room.members) {
      const endpoint = room.endpoints.get(memberId);
      if (endpoint !== undefined) {
        endpoint._flush();
      }
    }
    FakeRelayTransport.forwardRoom(room);
  }

  pump(): void {
    FakeRelayTransport.pumpRoom(this._room);
  }

  /**
   * The relay itself. It concatenates the opaque lines it received for
   * each destination this pass and hands the destination one frame. It
   * decodes nothing and it never returns a line to its own origin — origin
   * is decided by which member handed the line up, never by reading it.
   */
  static forwardRoom(room: FakeRelayRoom): void {
    for (const memberId of room.members) {
      const lines = room.inbox.get(memberId);
      if (lines !== undefined && lines.length > 0) {
        const frame = lines.join("\n");
        room.inbox.set(memberId, []);
        room.counters.frames += 1;
        const endpoint = room.endpoints.get(memberId);
        if (endpoint !== undefined) {
          endpoint._receiveFrame(frame);
        } else {
          room.counters.dropped += lines.length;
        }
      }
    }
  }

  /**
   * Drain the single uplink under the same `bufferedAmount` model the star
   * uses, with one difference that is the point: the budget is per *link*,
   * and this endpoint has one link however many members are listening.
   */
  private _flush(): void {
    const room = this._room;
    for (const channelName of contract.CHANNEL_ORDER) {
      const uplink = this._uplink[channelName];
      let budget = this._bufferedAmountLimit;
      while (uplink.units.length > 0) {
        const unit = uplink.units[0] as FakeRelayUnit;
        if (unit.bytes > budget) {
          if (!uplink.backpressured) {
            uplink.backpressured = true;
            this._backpressure += 1;
            this._lastError = "relay uplink send buffer is full";
            for (const target of unit.targets) {
              const peer = this._peers.get(target);
              if (peer !== undefined) {
                peer.backpressure += 1;
                peer.lastError = this._lastError;
              }
            }
            const primaryTarget = unit.targets[0];
            this._pushEvent({
              kind: "peer_error",
              ...(primaryTarget !== undefined ? { peer_id: primaryTarget } : {}),
              channel: channelName,
              code: "backpressure",
              message: this._lastError,
            });
          }
          break;
        }
        uplink.units.shift();
        uplink.backpressured = false;
        budget -= unit.bytes;
        uplink.bufferedAmount = Math.max(0, uplink.bufferedAmount - unit.bytes);
        let delivered = 0;
        for (const target of unit.targets) {
          const peer = this._peers.get(target);
          const remote = room.endpoints.get(target);
          if (peer !== undefined && peer.state === "connected" && remote !== undefined) {
            const inbox = room.inbox.get(target);
            if (inbox !== undefined) {
              inbox.push(unit.line);
              room.counters.lines += 1;
              peer.sent += 1;
              delivered += 1;
            }
          }
        }
        if (delivered === 0) {
          uplink.droppedOutbound += 1;
          this._droppedOutbound += 1;
          room.counters.dropped += 1;
        }
      }
    }
  }

  /**
   * Split the frame the room handed down and decode each line back into an
   * addressed envelope. The origin comes from the line the *sender* wrote,
   * and is checked against this endpoint's own member table: a line from a
   * member this endpoint does not hold a link to is counted, not
   * delivered.
   */
  private _receiveFrame(frame: string): void {
    this._downlinkFrames += 1;
    // The exact byte count that crossed the link, addressing and
    // separators included. `_downlinkBytes` below counts envelope wires
    // only, so that it is comparable with the star's figure; this one is
    // what a real relay would actually have to send, because a framing
    // relay has to name the origin of every line it forwards while a star
    // gets origin for free from the dedicated per-peer data channel.
    this._downlinkFramedBytes += frame.length;
    for (const line of split(frame, "\n")) {
      const addressedResult = contract.decodeAddressed(line);
      if (!addressedResult.ok) {
        this._recordError(addressedResult.error.code, addressedResult.error.message);
        continue;
      }
      const addressed = addressedResult.value;
      const peer = this._peers.get(addressed.peer_id);
      if (peer === undefined || peer.state !== "connected") {
        this._droppedInbound += 1;
        continue;
      }
      const wireBytes = line.length - addressed.peer_id.length - addressed.channel.length - 2;
      this._downlinkBytes += wireBytes;
      if (addressed.channel === "input") {
        this._inputDownlinkBytes += wireBytes;
      }
      this._receive(peer, addressed.channel, addressed);
    }
  }

  private _receive(
    peer: FakeRelayPeer,
    channelName: TransportChannel,
    addressed: TransportAddressedMessage,
  ): void {
    const channel = peer.channels[channelName];
    if (channel.inbound.length >= this._queueLimit) {
      channel.droppedInbound += 1;
      this._droppedInbound += 1;
      this._recordError("overflow", "relay member inbound queue is full", peer, channelName);
      return;
    }
    const seq = addressed.message.seq;
    if (channel.lastSeq !== null && seq > channel.lastSeq + 1) {
      peer.sequenceGaps += seq - channel.lastSeq - 1;
    }
    if (channel.lastSeq === null || seq > channel.lastSeq) {
      channel.lastSeq = seq;
    }
    // `arrival_seq` is stamped at poll time, for the same reason the star
    // does it there: it must mean the same thing on every adapter.
    channel.inbound.push({
      peer_id: peer.peerId,
      channel: channelName,
      arrival_seq: 0,
      message: addressed.message,
    });
  }

  // ---------------------------------------------------------------------
  // Draining
  // ---------------------------------------------------------------------

  /**
   * The same persistent (slot, channel-rank) cursor the star uses, so
   * release order is a property of the contract rather than of the
   * topology and the two adapters can be compared without that as a
   * variable.
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
      const peer = this._peers.get(peerId) as FakeRelayPeer;
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

  // ---------------------------------------------------------------------
  // Signaling: refused, not faked
  // ---------------------------------------------------------------------

  /**
   * A relay client dials a server with a known address. There is no offer
   * to hand to a human, no answer to paste back, and no ICE state worth
   * reporting on an ICE-lite endpoint. Returning a plausible token would
   * let a caller believe the manual handshake still exists; refusing makes
   * the four methods that the topology deletes visible at the seam
   * instead.
   */
  private static _noSignaling(): TransportResult<never> {
    return failure(
      "signal_error",
      "a relay endpoint has no manual signaling; the room is the rendezvous",
    );
  }

  requestOffer(_peerId: string): TransportResult<true> {
    this._recordError("signal_error", "relay endpoints do not create offers");
    return FakeRelayTransport._noSignaling();
  }

  acceptOffer(_signal: string): TransportResult<true> {
    this._recordError("signal_error", "relay endpoints do not accept offers");
    return FakeRelayTransport._noSignaling();
  }

  acceptAnswer(_peerId: string, _signal: string): TransportResult<true> {
    this._recordError("signal_error", "relay endpoints do not accept answers");
    return FakeRelayTransport._noSignaling();
  }

  takeSignal(_peerId: string): TransportResult<string | null> {
    return ok(null);
  }

  // ---------------------------------------------------------------------
  // Reporting
  // ---------------------------------------------------------------------

  role(): TransportRole {
    return this._role;
  }

  capacity(): number {
    return this._maxPeers;
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
   * Encoded envelope wires this endpoint put on its uplink and took off
   * its downlink, one count per copy that crossed the link. On a star a
   * `broadcast` to seven guests is seven uplink copies; here it is one,
   * and that difference is the measurement the relay topology decision
   * turns on.
   */
  wireBytes(): readonly [uplinkBytes: number, downlinkBytes: number] {
    return [this._uplinkBytes, this._downlinkBytes];
  }

  wireCounters(): FakeRelayWireCounters {
    return {
      uplink_bytes: this._uplinkBytes,
      downlink_bytes: this._downlinkBytes,
      input_uplink_bytes: this._inputUplinkBytes,
      input_downlink_bytes: this._inputDownlinkBytes,
      downlink_framed_bytes: this._downlinkFramedBytes,
      uplink_units: this._uplinkUnits,
      downlink_frames: this._downlinkFrames,
      frame_overhead_bytes: this._frameOverhead,
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
        control: this._channelDiagnostics(peer, "control"),
        input: this._channelDiagnostics(peer, "input"),
        sequence_gaps: peer.sequenceGaps,
        backpressure: peer.backpressure,
        malformed: peer.malformed,
        last_error: peer.lastError,
      });
    }
    return {
      role: this._role,
      state: this._state,
      capacity: this._maxPeers,
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

  /**
   * The uplink is shared, so a member's outbound view is the units still
   * queued that are addressed to it. Byte figures follow the same rule,
   * which keeps the depth gate meaningful without pretending the copies
   * are real.
   */
  private _channelDiagnostics(
    peer: FakeRelayPeer,
    channelName: TransportChannel,
  ): TransportPeerDiagnostics["control"] {
    const channel = peer.channels[channelName];
    const uplink = this._uplink[channelName];
    let depth = 0;
    let buffered = 0;
    for (const unit of uplink.units) {
      if (unit.targetSet.has(peer.peerId)) {
        depth += 1;
        buffered += unit.bytes;
      }
    }
    return {
      state: channel.state,
      outbound_depth: depth,
      inbound_depth: channel.inbound.length,
      buffered_amount: buffered,
      sent: peer.sent,
      received: channel.received,
      dropped_outbound: peer.droppedOutbound,
      dropped_inbound: channel.droppedInbound,
    };
  }
}

export interface FakeRelayWireCounters {
  readonly uplink_bytes: number;
  readonly downlink_bytes: number;
  /** The `input` channel alone, which is the per-tick match cost. */
  readonly input_uplink_bytes: number;
  readonly input_downlink_bytes: number;
  /** What actually crossed the link: addressing and separators included. */
  readonly downlink_framed_bytes: number;
  readonly uplink_units: number;
  readonly downlink_frames: number;
  readonly frame_overhead_bytes: number;
}

// Every method the star adapter contract names is implemented above.
// Asserted at module load so a contract addition fails loudly rather than
// at the first call.
const REQUIRED_METHODS = [
  "initialize",
  "shutdown",
  "role",
  "capacity",
  "openPeer",
  "closePeer",
  "peerIds",
  "peerState",
  "requestOffer",
  "acceptOffer",
  "acceptAnswer",
  "takeSignal",
  "send",
  "broadcast",
  "poll",
  "pollBatch",
  "pollEvent",
  "state",
  "diagnostics",
] as const;
for (const name of REQUIRED_METHODS) {
  if (
    typeof (FakeRelayTransport.prototype as unknown as Record<string, unknown>)[name] !== "function"
  ) {
    throw new Error(`fake relay transport is missing ${name}`);
  }
}
