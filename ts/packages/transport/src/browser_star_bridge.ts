// The real `window.GoliseoStarTransport` bridge `browser_star.ts` calls
// through its single eval seam. This reimplements
// `scripts/webrtc_star_host.js` (OMP-3's browser host-star transport) as a
// TypeScript module, adapted for two reasons:
//
// 1. That file has no v2 counterpart at all — `docs/online/transport_bridge.md`
//    documents `window.GoliseoStarTransport` as that script's own bridge,
//    embedded into `player.js` by `scripts/web_build.py`; nothing wires it
//    up for the v2 TypeScript/wasm app. `browser_star.ts`'s own header says
//    plainly: "That JS side has never been implemented."
//
// 2. Reimplementing surfaced a real defect in the original source, fixed
//    here rather than reproduced: `scripts/webrtc_star_host.js`'s
//    `escapeField`/`unescapeField` use `encodeURIComponent`/
//    `decodeURIComponent` (UTF-8 percent-encoding), but this module's own
//    `contract.ts` uses a *raw-byte* percent-encoding — one `%XX` triplet
//    per JS UTF-16 code unit (0-255 only), never re-interpreted as UTF-8.
//    `contract.ts`'s own header names the reason: "the spec exercises a
//    bare `\255`". A wire payload byte in 0x80-0xFF, once escaped by
//    `contract.encode` (`%FF`, one triplet), is *not* valid UTF-8 on its
//    own — `decodeURIComponent("%FF")` throws `URIError: URI malformed`.
//    The original bridge catches that exception and reports the envelope
//    as `malformed`, silently rejecting exactly the payload byte range
//    `contract.ts` explicitly supports. This module's `escapeField`/
//    `unescapeField` use the same raw-byte scheme as `contract.ts`'s own
//    `escape`/`unescape` instead, so a payload byte's escaped form always
//    round-trips regardless of what other bytes surround it.
//
// Every other behaviour (peer/channel lifecycle, manual signaling,
// deterministic `poll` cursor, backpressure, diagnostics encoding) is kept
// a deliberate line-for-line match with the original: this is the
// transport CONTRACT (`docs/online/transport_bridge.md`), not a redesign.

const VERSION = 1;
const MAX_GUESTS = 7;
const HOST_PEER_ID = "host";
const DEFAULT_QUEUE_LIMIT = 64;
const MAX_QUEUE_LIMIT = 256;
const MAX_PAYLOAD_BYTES = 1024;
const DEFAULT_BUFFERED_AMOUNT_LIMIT = 65536;
const MAX_BUFFERED_AMOUNT_LIMIT = 1048576;
const MAX_PEER_ID_BYTES = 128;
const MAX_SIGNAL_BYTES = 65536;
const ICE_TIMEOUT_MS = 5000;
// #612 part 3: a transient `connectionState === "disconnected"` (an ICE
// blip -- a dropped packet burst, a NAT rebinding) self-heals far more
// often than it means the link is gone. "failed"/"closed" mean that
// unambiguously and are reported immediately; "disconnected" instead gets
// this long to recover before the peer is declared lost -- see
// `ensurePeerConnection`'s `onconnectionstatechange`.
const DISCONNECT_GRACE_MS = 3000;
const PEER_ID_PATTERN = /^[0-9A-Za-z][0-9A-Za-z_-]*$/;
const CHANNEL_ORDER = ["control", "input"] as const;
type ChannelName = (typeof CHANNEL_ORDER)[number];
const CHANNEL_MESSAGE_TYPES: Readonly<Record<ChannelName, Readonly<Record<string, true>>>> = {
  control: { event: true, state: true },
  input: { input: true },
};
// Channel configuration is part of the transport contract: control is
// reliable and ordered, input is unordered and never retransmitted.
const CHANNEL_CONFIG: Readonly<Record<ChannelName, RTCDataChannelInit>> = {
  control: { ordered: true },
  input: { ordered: false, maxRetransmits: 0 },
};

// Raw-byte percent-encoding — one `%XX` triplet per UTF-16 code unit
// (0-255 only), matching `contract.ts`'s `escape`/`unescape` exactly. See
// this file's header for why `encodeURIComponent`/`decodeURIComponent`
// (the original bridge's choice) is wrong for this purpose.
function escapeField(value: string): string {
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

function unescapeField(value: string): string | null {
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

// A byte-string ("binary string") is one JS UTF-16 code unit per raw byte
// (0-255), the same convention `contract.ts` documents — so byte length is
// just code-unit count, not `TextEncoder`-measured UTF-8 length (that would
// double-count every byte 0x80-0xFF, which is exactly the range this file's
// header explains the original bridge got wrong).
function byteLength(value: string): number {
  return value.length;
}

interface ParsedWire {
  readonly error?: string;
  readonly detail?: string;
  readonly type?: string;
  readonly seq?: number;
  readonly tick?: number | null;
}

interface ChannelState {
  state: string;
  handle: RTCDataChannel | null;
  outbound: string[];
  inbound: string[];
  backpressured: boolean;
  sent: number;
  received: number;
  dropped_outbound: number;
  dropped_inbound: number;
  last_seq: number | null;
}

interface PeerState {
  id: string;
  slot: number;
  state: string;
  ice_state: string;
  pc: RTCPeerConnection | null;
  channels: Record<ChannelName, ChannelState>;
  signal: string | null;
  sequence_gaps: number;
  backpressure: number;
  malformed: number;
  last_error: string;
  /** A pending "report loss after `DISCONNECT_GRACE_MS`" timer, armed while
   * `connectionState` reads a transient "disconnected" -- `null` when none
   * is pending (never disconnected, already reported, or already
   * recovered). See `DISCONNECT_GRACE_MS`'s own doc. */
  disconnect_grace: number | null;
}

interface StarState {
  role: "host" | "guest";
  state: string;
  queue_limit: number;
  event_limit: number;
  max_guests: number;
  buffered_amount_limit: number;
  peers: Record<string, PeerState>;
  order: string[];
  events: string[];
  cursor: number;
  sent: number;
  received: number;
  dropped_outbound: number;
  dropped_inbound: number;
  malformed: number;
  unsupported_version: number;
  overflow: number;
  backpressure: number;
  last_error: string;
}

/** The bridge's public call surface — every method returns the pipe-framed
 * wire string `docs/online/transport_bridge.md` documents ("Generated
 * browser bridge" section). */
export interface GoliseoStarTransportBridge {
  initialize(role: string, queueLimit: number, maxGuests: number, bufferedLimit: number): string;
  shutdown(): string;
  open_peer(peerId: string): string;
  close_peer(peerId: string, reason: string): string;
  request_offer(peerId: string): string;
  accept_offer(signal: string): string;
  accept_answer(peerId: string, signal: string): string;
  take_signal(peerId: string): string;
  send(line: string): string;
  broadcast(line: string): string;
  poll(): string;
  poll_event(): string;
  diagnostics(): string;
}

/** One `RTCPeerConnection`'s ICE configuration -- `iceServers` (STUN/TURN,
 * #550) and an optional `iceTransportPolicy` (the "relay" force toggle,
 * #248's "config path only" scope). */
export interface StarBridgeIceConfig {
  readonly iceServers: readonly RTCIceServer[];
  readonly iceTransportPolicy?: RTCIceTransportPolicy;
}

/** Construction options for {@link newGoliseoStarTransportBridge}/
 * {@link installGoliseoStarTransport}. */
export interface StarBridgeOptions {
  /** Read fresh every time a peer connection is opened (not snapshotted at
   * install time), so a caller can update ICE servers/policy after a TURN
   * credential fetch resolves without reinstalling the bridge -- see
   * `@gc/app`'s `online_ports.ts`/`browser_main.ts`. Defaults to STUN-only
   * with no forced policy, matching this bridge's behavior before this
   * option existed. */
  readonly iceConfig?: () => StarBridgeIceConfig;
}

const NO_ICE_SERVERS: StarBridgeIceConfig = { iceServers: [] };

/** Builds one fresh bridge instance — never a `window` singleton on its
 * own; see {@link installGoliseoStarTransport} for that. Kept separate so a
 * test can build one without touching global state. */
export function newGoliseoStarTransportBridge(
  options: StarBridgeOptions = {},
): GoliseoStarTransportBridge {
  const iceConfig = options.iceConfig ?? ((): StarBridgeIceConfig => NO_ICE_SERVERS);
  const star: StarState = {
    role: "host",
    state: "new",
    queue_limit: DEFAULT_QUEUE_LIMIT,
    event_limit: Math.max(2, DEFAULT_QUEUE_LIMIT),
    max_guests: MAX_GUESTS,
    buffered_amount_limit: DEFAULT_BUFFERED_AMOUNT_LIMIT,
    peers: {},
    order: [],
    events: [],
    cursor: 0,
    sent: 0,
    received: 0,
    dropped_outbound: 0,
    dropped_inbound: 0,
    malformed: 0,
    unsupported_version: 0,
    overflow: 0,
    backpressure: 0,
    last_error: "",
  };

  function pushEvent(line: string): void {
    if (star.events.length >= star.event_limit) {
      star.events.shift();
      star.overflow += 1;
      star.last_error = "star event queue is full";
    }
    star.events.push(line);
  }

  function starState(next: string): void {
    star.state = next;
    pushEvent("star_state|" + next);
  }

  function peerState(peer: PeerState, next: string): void {
    peer.state = next;
    for (const name of CHANNEL_ORDER) {
      peer.channels[name].state = next;
    }
    pushEvent("peer_state|" + peer.id + "|" + next);
  }

  function countError(code: string, peer: PeerState | null): void {
    if (code === "malformed" || code === "payload_too_large" || code === "channel_mismatch") {
      star.malformed += 1;
      if (peer) {
        peer.malformed += 1;
      }
    } else if (code === "unsupported_version") {
      star.unsupported_version += 1;
    } else if (code === "overflow") {
      star.overflow += 1;
    } else if (code === "backpressure") {
      star.backpressure += 1;
      if (peer) {
        peer.backpressure += 1;
      }
    }
  }

  function starError(code: string, detail: string): string {
    star.last_error = detail || code;
    countError(code, null);
    pushEvent("star_error|" + code + "|" + escapeField(star.last_error));
    return "error|" + code + "|" + escapeField(star.last_error);
  }

  function peerError(
    peer: PeerState,
    channelName: string | null,
    code: string,
    detail: string,
  ): string {
    star.last_error = detail || code;
    peer.last_error = star.last_error;
    countError(code, peer);
    pushEvent(
      "peer_error|" +
        peer.id +
        "|" +
        (channelName ?? "") +
        "|" +
        code +
        "|" +
        escapeField(star.last_error),
    );
    return "error|" + code + "|" + escapeField(star.last_error);
  }

  function newChannel(): ChannelState {
    return {
      state: "connecting",
      handle: null,
      outbound: [],
      inbound: [],
      backpressured: false,
      sent: 0,
      received: 0,
      dropped_outbound: 0,
      dropped_inbound: 0,
      last_seq: null,
    };
  }

  function newPeer(id: string, slot: number): PeerState {
    return {
      id,
      slot,
      state: "connecting",
      ice_state: "new",
      pc: null,
      channels: { control: newChannel(), input: newChannel() },
      signal: null,
      sequence_gaps: 0,
      backpressure: 0,
      malformed: 0,
      last_error: "",
      disconnect_grace: null,
    };
  }

  /** Cancels a pending grace timer, if one is armed -- the connection
   * recovered, failed/closed outright (both reported immediately and make
   * a queued "disconnected" report stale), or the peer is being torn down
   * entirely. Safe to call whether or not one is pending. */
  function clearDisconnectGrace(peer: PeerState): void {
    if (peer.disconnect_grace !== null) {
      globalThis.clearTimeout(peer.disconnect_grace);
      peer.disconnect_grace = null;
    }
  }

  function bufferedAmount(channel: ChannelState): number {
    return channel.handle && typeof channel.handle.bufferedAmount === "number"
      ? channel.handle.bufferedAmount
      : 0;
  }

  // Envelope parsing mirrors `contract.ts`. Payload bytes stay opaque;
  // this bridge never inspects or rewrites them.
  function parseWire(wire: unknown): ParsedWire {
    if (typeof wire !== "string") {
      return { error: "malformed", detail: "wire is not a string" };
    }
    const fields = wire.split("|");
    if (fields.length !== 5) {
      return { error: "malformed", detail: "wire has invalid fields" };
    }
    if (!/^(0|[1-9][0-9]*)$/.test(fields[0] as string)) {
      return { error: "malformed", detail: "wire version is invalid" };
    }
    if (Number(fields[0]) !== VERSION) {
      return { error: "unsupported_version", detail: "wire version is unsupported" };
    }
    const type = unescapeField(fields[1] as string);
    if (type !== "input" && type !== "event" && type !== "state") {
      return { error: "malformed", detail: "wire type is invalid" };
    }
    if (!/^(0|[1-9][0-9]*)$/.test(fields[2] as string)) {
      return { error: "malformed", detail: "wire sequence is invalid" };
    }
    let tick: number | null = null;
    if (fields[3] !== "") {
      if (!/^(0|[1-9][0-9]*)$/.test(fields[3] as string)) {
        return { error: "malformed", detail: "wire tick is invalid" };
      }
      tick = Number(fields[3]);
    }
    if (type === "input" && tick === null) {
      return { error: "malformed", detail: "input wire is missing its tick" };
    }
    const payload = unescapeField(fields[4] as string);
    if (payload === null) {
      return { error: "malformed", detail: "wire payload escape is invalid" };
    }
    if (byteLength(payload) > MAX_PAYLOAD_BYTES) {
      return { error: "payload_too_large", detail: "wire payload exceeds the byte limit" };
    }
    return { type, seq: Number(fields[2]), tick };
  }

  function checkChannelMessage(channelName: string, parsed: ParsedWire): ParsedWire | null {
    const config = (CHANNEL_MESSAGE_TYPES as Record<string, Readonly<Record<string, true>>>)[
      channelName
    ];
    if (!config) {
      return { error: "channel_mismatch", detail: "channel must be control or input" };
    }
    if (!config[parsed.type as string]) {
      return {
        error: "channel_mismatch",
        detail:
          "the " + channelName + " channel does not carry " + String(parsed.type) + " messages",
      };
    }
    return null;
  }

  function validPeerId(id: unknown): id is string {
    return (
      typeof id === "string" &&
      id.length > 0 &&
      byteLength(id) <= MAX_PEER_ID_BYTES &&
      PEER_ID_PATTERN.test(id)
    );
  }

  // Sending is refused only when the bounded queue is full. A saturated send
  // buffer is backpressure: the message stays queued and drains later, so no
  // call blocks the caller's tick loop.
  function drain(peer: PeerState, channelName: ChannelName): void {
    const channel = peer.channels[channelName];
    const handle = channel.handle;
    if (!handle || handle.readyState !== "open") {
      return;
    }
    while (channel.outbound.length > 0) {
      const wire = channel.outbound[0] as string;
      if (bufferedAmount(channel) + byteLength(wire) > star.buffered_amount_limit) {
        if (!channel.backpressured) {
          channel.backpressured = true;
          peerError(peer, channelName, "backpressure", "channel send buffer is full");
        }
        return;
      }
      channel.outbound.shift();
      channel.backpressured = false;
      try {
        handle.send(wire);
      } catch (error) {
        channel.dropped_outbound += 1;
        star.dropped_outbound += 1;
        peerError(peer, channelName, "bridge_error", String(error));
        return;
      }
    }
  }

  function drainAll(): void {
    for (const id of star.order) {
      const peer = star.peers[id] as PeerState;
      for (const name of CHANNEL_ORDER) {
        drain(peer, name);
      }
    }
  }

  function enqueue(peer: PeerState, channelName: ChannelName, wire: string): string {
    const parsed = parseWire(wire);
    if (parsed.error) {
      return peerError(peer, channelName, parsed.error, parsed.detail ?? parsed.error);
    }
    const mismatch = checkChannelMessage(channelName, parsed);
    if (mismatch) {
      return peerError(peer, channelName, mismatch.error as string, mismatch.detail as string);
    }
    if (peer.state !== "connected") {
      return peerError(peer, channelName, "not_connected", "peer link is not connected");
    }
    const channel = peer.channels[channelName];
    if (channel.outbound.length >= star.queue_limit) {
      channel.dropped_outbound += 1;
      star.dropped_outbound += 1;
      return peerError(peer, channelName, "overflow", "peer outbound queue is full");
    }
    channel.outbound.push(wire);
    channel.sent += 1;
    star.sent += 1;
    drain(peer, channelName);
    return "ok";
  }

  function receive(peer: PeerState, channelName: ChannelName, wire: string): void {
    const channel = peer.channels[channelName];
    const parsed = parseWire(wire);
    if (parsed.error) {
      channel.dropped_inbound += 1;
      star.dropped_inbound += 1;
      peerError(peer, channelName, parsed.error, parsed.detail ?? parsed.error);
      return;
    }
    const mismatch = checkChannelMessage(channelName, parsed);
    if (mismatch) {
      channel.dropped_inbound += 1;
      star.dropped_inbound += 1;
      peerError(peer, channelName, mismatch.error as string, mismatch.detail as string);
      return;
    }
    if (channel.inbound.length >= star.queue_limit) {
      channel.dropped_inbound += 1;
      star.dropped_inbound += 1;
      peerError(peer, channelName, "overflow", "peer inbound queue is full");
      return;
    }
    const seq = parsed.seq as number;
    if (channel.last_seq !== null && seq > channel.last_seq + 1) {
      peer.sequence_gaps += seq - channel.last_seq - 1;
    }
    if (channel.last_seq === null || seq > channel.last_seq) {
      channel.last_seq = seq;
    }
    channel.inbound.push(peer.id + "|" + channelName + "|" + wire);
  }

  function attachChannel(peer: PeerState, handle: RTCDataChannel): void {
    const name = handle.label;
    if (name !== "control" && name !== "input") {
      handle.close();
      peerError(peer, null, "channel_mismatch", "unexpected data channel label");
      return;
    }
    const channelName = name;
    const channel = peer.channels[channelName];
    // Exactly two data channels per link, for the life of the link. Either
    // side of an established connection may call `createDataChannel` at any
    // time; a second channel on an occupied label would re-register
    // listeners without bound and silently take over the reference this
    // bridge sends through. Refuse it and close it.
    if (channel.handle) {
      try {
        handle.close();
      } catch (error) {
        star.last_error = String(error);
      }
      peerError(
        peer,
        channelName,
        "channel_mismatch",
        "duplicate " + channelName + " data channel refused",
      );
      return;
    }
    channel.handle = handle;
    handle.binaryType = "arraybuffer";
    // Resume draining well before the buffer empties, so backpressure costs
    // latency rather than a stalled channel.
    handle.bufferedAmountLowThreshold = Math.floor(star.buffered_amount_limit / 2);
    handle.onopen = () => {
      channel.state = "connected";
      if (
        peer.channels.control.state === "connected" &&
        peer.channels.input.state === "connected" &&
        peer.state !== "connected"
      ) {
        peerState(peer, "connected");
      }
      drain(peer, channelName);
    };
    handle.onbufferedamountlow = () => {
      drain(peer, channelName);
    };
    handle.onclose = () => {
      channel.state = "disconnected";
      if (peer.state === "connected") {
        peerState(peer, "disconnected");
        peerError(peer, channelName, "disconnected", channelName + " data channel closed");
      }
    };
    handle.onerror = () => {
      peerError(peer, channelName, "bridge_error", channelName + " data channel error");
    };
    handle.onmessage = (event: MessageEvent) => {
      receive(peer, channelName, typeof event.data === "string" ? event.data : "");
    };
  }

  function ensurePeerConnection(peer: PeerState): RTCPeerConnection | null {
    if (peer.pc) {
      return peer.pc;
    }
    // Present in a browser, absent under Node and in every headless test, so
    // it is read off `globalThis` as optional rather than through the DOM
    // lib's non-optional global declaration.
    const RTCPeerConnectionCtor = (
      globalThis as typeof globalThis & {
        readonly RTCPeerConnection?: typeof RTCPeerConnection;
      }
    ).RTCPeerConnection;
    if (!RTCPeerConnectionCtor) {
      peerError(peer, null, "bridge_unavailable", "RTCPeerConnection is not available");
      return null;
    }
    // Manual signaling (offer/answer) stays as-is; only the ICE server
    // list/policy is configurable -- see `StarBridgeOptions.iceConfig`'s
    // doc. Read fresh per connection, so a TURN credential fetch that
    // resolves after this bridge was installed still reaches the NEXT
    // peer connection this bridge opens.
    const config = iceConfig();
    const pc = new RTCPeerConnectionCtor({
      iceServers: [...config.iceServers],
      ...(config.iceTransportPolicy !== undefined
        ? { iceTransportPolicy: config.iceTransportPolicy }
        : {}),
    });
    peer.pc = pc;
    // These handlers close over `pc`, never `peer.pc`. `RTCPeerConnection
    // .close()` fires the state-change callbacks on a later task, by which
    // time `closePeer` has already set `peer.pc` to `null`.
    pc.oniceconnectionstatechange = () => {
      peer.ice_state = pc.iceConnectionState;
      if (peer.ice_state === "failed") {
        peerError(peer, null, "disconnected", "ICE negotiation failed");
        peerState(peer, "error");
      }
    };
    pc.onconnectionstatechange = () => {
      const connection = pc.connectionState;
      if (connection === "failed" || connection === "closed") {
        // Unambiguous: report immediately, and drop any pending transient-
        // disconnect grace -- it would only re-report the same loss late.
        clearDisconnectGrace(peer);
        if (peer.state !== "closed") {
          peerError(peer, null, "disconnected", "peer connection " + connection);
          peerState(peer, "disconnected");
        }
      } else if (connection === "disconnected" && peer.state === "connected") {
        // Transient -- see `DISCONNECT_GRACE_MS`'s own doc. One timer per
        // disconnected spell: a flicker of onconnectionstatechange firing
        // "disconnected" more than once before the grace elapses must not
        // shorten or restart the window.
        if (peer.disconnect_grace === null) {
          peer.disconnect_grace = globalThis.setTimeout(() => {
            peer.disconnect_grace = null;
            // Re-read live state rather than trust the closure: the peer
            // may have recovered, or already been closed/failed (and
            // reported) by the time this fires.
            if (pc.connectionState === "disconnected" && peer.state === "connected") {
              peerError(peer, null, "disconnected", "peer connection disconnected");
              peerState(peer, "disconnected");
            }
          }, DISCONNECT_GRACE_MS);
        }
      } else if (connection === "connected") {
        // Recovered before the grace elapsed -- the transient blip is over.
        clearDisconnectGrace(peer);
      }
    };
    pc.ondatachannel = (event: RTCDataChannelEvent) => {
      attachChannel(peer, event.channel);
    };
    return pc;
  }

  // Detach every listener before closing, so a late callback from a closed
  // connection or channel cannot touch a torn-down peer at all.
  function detach(target: object | null | undefined, names: readonly string[]): void {
    if (!target) {
      return;
    }
    // The callers pass DOM objects (`RTCPeerConnection`, `RTCDataChannel`)
    // and the names are their `on*` handler slots. One assertion here, at the
    // one place that writes them by name, rather than `any` across the whole
    // helper.
    const slots = target as Record<string, unknown>;
    for (const name of names) {
      slots[name] = null;
    }
  }

  function waitForIce(pc: RTCPeerConnection): Promise<void> {
    if (pc.iceGatheringState === "complete") {
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      let settled = false;
      const finish = () => {
        if (!settled) {
          settled = true;
          resolve();
        }
      };
      pc.onicegatheringstatechange = () => {
        if (pc.iceGatheringState === "complete") {
          finish();
        }
      };
      globalThis.setTimeout(finish, ICE_TIMEOUT_MS);
    });
  }

  function storeSignal(peer: PeerState, description: RTCSessionDescription | null): void {
    if (!description) {
      peerError(peer, null, "signal_error", "local description is unavailable");
      return;
    }
    const blob = JSON.stringify({ type: description.type, sdp: description.sdp });
    if (byteLength(blob) > MAX_SIGNAL_BYTES) {
      peerError(peer, null, "signal_error", "local signal exceeds the byte limit");
      return;
    }
    peer.signal = blob;
  }

  function parseSignal(peer: PeerState, raw: unknown): RTCSessionDescriptionInit | null {
    // Bound the encoded length before decoding, so an oversized blob is
    // rejected without allocating its decoded form first.
    if (typeof raw !== "string" || raw.length > MAX_SIGNAL_BYTES) {
      peerError(peer, null, "signal_error", "remote signal exceeds the byte limit");
      return null;
    }
    const text = unescapeField(raw);
    if (text === null || byteLength(text) > MAX_SIGNAL_BYTES) {
      peerError(peer, null, "signal_error", "remote signal is malformed");
      return null;
    }
    try {
      const value = JSON.parse(text) as { type?: string; sdp?: string };
      if (!value || (value.type !== "offer" && value.type !== "answer")) {
        peerError(peer, null, "signal_error", "remote signal is not an offer or answer");
        return null;
      }
      return value as RTCSessionDescriptionInit;
    } catch {
      peerError(peer, null, "signal_error", "remote signal is not JSON");
      return null;
    }
  }

  function closePeer(peer: PeerState, reason: string): void {
    clearDisconnectGrace(peer);
    for (const name of CHANNEL_ORDER) {
      const channel = peer.channels[name];
      star.dropped_outbound += channel.outbound.length;
      star.dropped_inbound += channel.inbound.length;
      channel.dropped_outbound += channel.outbound.length;
      channel.dropped_inbound += channel.inbound.length;
      channel.outbound = [];
      channel.inbound = [];
      if (channel.handle) {
        detach(channel.handle, [
          "onopen",
          "onclose",
          "onerror",
          "onmessage",
          "onbufferedamountlow",
        ]);
        try {
          channel.handle.close();
        } catch (error) {
          star.last_error = String(error);
        }
        channel.handle = null;
      }
    }
    if (peer.pc) {
      detach(peer.pc, [
        "oniceconnectionstatechange",
        "onconnectionstatechange",
        "onicegatheringstatechange",
        "ondatachannel",
      ]);
      try {
        peer.pc.close();
      } catch (error) {
        star.last_error = String(error);
      }
      peer.pc = null;
    }
    peer.ice_state = "closed";
    peer.signal = null;
    if (peer.state !== "closed") {
      peerState(peer, "closed");
      peerError(peer, null, "disconnected", reason || "peer closed");
    }
  }

  function requireConnected(): string | null {
    if (star.state === "new") {
      return starError("not_initialized", "star transport is not initialized");
    }
    if (star.state === "closed") {
      return starError("closed", "star transport is closed");
    }
    if (star.state !== "connected") {
      return starError("not_connected", "star transport is not connected");
    }
    return null;
  }

  function findPeer(id: string | null): PeerState | null {
    if (id === null) {
      return null;
    }
    return star.peers[id] ?? null;
  }

  function addPeer(id: string): PeerState {
    const peer = newPeer(id, star.order.length + 1);
    star.peers[id] = peer;
    star.order.push(id);
    pushEvent("peer_state|" + id + "|connecting");
    return peer;
  }

  function channelRecord(channel: ChannelState): (string | number)[] {
    return [
      channel.state,
      channel.outbound.length,
      channel.inbound.length,
      bufferedAmount(channel),
      channel.sent,
      channel.received,
      channel.dropped_outbound,
      channel.dropped_inbound,
    ];
  }

  return {
    initialize(role, queueLimit, maxGuests, bufferedLimit) {
      const requestedRole = unescapeField(role);
      if (requestedRole !== "host" && requestedRole !== "guest") {
        return starError("malformed", "role must be host or guest");
      }
      const limit = Number(queueLimit);
      const guests = Number(maxGuests);
      const buffered = Number(bufferedLimit);
      if (!Number.isSafeInteger(limit) || limit < 1 || limit > MAX_QUEUE_LIMIT) {
        return starError("malformed", "queue limit is invalid");
      }
      if (!Number.isSafeInteger(guests) || guests < 1 || guests > MAX_GUESTS) {
        return starError("malformed", "guest capacity is invalid");
      }
      if (!Number.isSafeInteger(buffered) || buffered < 1 || buffered > MAX_BUFFERED_AMOUNT_LIMIT) {
        return starError("malformed", "bufferedAmount limit is invalid");
      }
      if (star.state === "connected") {
        return "star|connected";
      }
      star.role = requestedRole;
      star.queue_limit = limit;
      star.event_limit = Math.max(2, limit);
      star.max_guests = requestedRole === "host" ? guests : 1;
      star.buffered_amount_limit = buffered;
      star.peers = {};
      star.order = [];
      star.cursor = 0;
      starState("connected");
      if (requestedRole === "guest") {
        addPeer(HOST_PEER_ID);
      }
      return "star|connected";
    },

    shutdown() {
      for (const id of star.order) {
        closePeer(star.peers[id] as PeerState, "star transport shutdown");
      }
      star.peers = {};
      star.order = [];
      star.cursor = 0;
      starState("closed");
      return "star|closed";
    },

    open_peer(peerId) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      if (star.role !== "host") {
        return starError("role_forbidden", "only a host opens guest links");
      }
      const id = unescapeField(peerId);
      if (!validPeerId(id)) {
        return starError("malformed", "peer id is invalid");
      }
      if (id === HOST_PEER_ID) {
        return starError("duplicate_peer", "the host peer id is reserved");
      }
      if (star.peers[id]) {
        return starError("duplicate_peer", "star peer is already open");
      }
      if (star.order.length >= star.max_guests) {
        return starError("capacity", "star transport is at guest capacity");
      }
      return "slot|" + addPeer(id).slot;
    },

    close_peer(peerId, reason) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      const peer = findPeer(unescapeField(peerId));
      if (!peer) {
        return starError("unknown_peer", "star transport has no peer with that id");
      }
      closePeer(peer, unescapeField(reason) || "peer closed");
      return "ok";
    },

    request_offer(peerId) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      if (star.role !== "host") {
        return starError("role_forbidden", "only a host creates offers");
      }
      const peer = findPeer(unescapeField(peerId));
      if (!peer) {
        return starError("unknown_peer", "star transport has no peer with that id");
      }
      const pc = ensurePeerConnection(peer);
      if (!pc) {
        return "error|bridge_unavailable|" + escapeField("RTCPeerConnection is not available");
      }
      for (const name of CHANNEL_ORDER) {
        if (!peer.channels[name].handle) {
          attachChannel(peer, pc.createDataChannel(name, CHANNEL_CONFIG[name]));
        }
      }
      Promise.resolve()
        .then(() => pc.createOffer())
        .then((offer) => pc.setLocalDescription(offer))
        .then(() => waitForIce(pc))
        .then(() => storeSignal(peer, pc.localDescription))
        .catch((error: unknown) => {
          peerError(peer, null, "signal_error", String(error));
        });
      return "ok";
    },

    accept_offer(signal) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      if (star.role !== "guest") {
        return starError("role_forbidden", "only a guest accepts an offer");
      }
      const peer = findPeer(HOST_PEER_ID) ?? addPeer(HOST_PEER_ID);
      const offer = parseSignal(peer, signal);
      if (!offer) {
        return "error|signal_error|" + escapeField("remote offer is invalid");
      }
      const pc = ensurePeerConnection(peer);
      if (!pc) {
        return "error|bridge_unavailable|" + escapeField("RTCPeerConnection is not available");
      }
      Promise.resolve()
        .then(() => pc.setRemoteDescription(offer))
        .then(() => pc.createAnswer())
        .then((answer) => pc.setLocalDescription(answer))
        .then(() => waitForIce(pc))
        .then(() => storeSignal(peer, pc.localDescription))
        .catch((error: unknown) => {
          peerError(peer, null, "signal_error", String(error));
        });
      return "ok";
    },

    accept_answer(peerId, signal) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      if (star.role !== "host") {
        return starError("role_forbidden", "only a host accepts an answer");
      }
      const peer = findPeer(unescapeField(peerId));
      if (!peer) {
        return starError("unknown_peer", "star transport has no peer with that id");
      }
      if (!peer.pc) {
        return peerError(peer, null, "signal_error", "peer has no pending offer");
      }
      const answer = parseSignal(peer, signal);
      if (!answer) {
        return "error|signal_error|" + escapeField("remote answer is invalid");
      }
      peer.pc.setRemoteDescription(answer).catch((error: unknown) => {
        peerError(peer, null, "signal_error", String(error));
      });
      return "ok";
    },

    take_signal(peerId) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      const peer = findPeer(unescapeField(peerId));
      if (!peer) {
        return starError("unknown_peer", "star transport has no peer with that id");
      }
      if (!peer.signal) {
        return "";
      }
      const blob = peer.signal;
      peer.signal = null;
      return "signal|" + escapeField(blob);
    },

    send(line) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      if (typeof line !== "string") {
        return starError("malformed", "addressed wire must be a string");
      }
      const first = line.indexOf("|");
      const second = line.indexOf("|", first + 1);
      if (first < 0 || second < 0) {
        return starError("malformed", "addressed wire has invalid fields");
      }
      const peerId = line.slice(0, first);
      const channelName = line.slice(first + 1, second);
      const wire = line.slice(second + 1);
      if (!validPeerId(peerId)) {
        return starError("malformed", "peer id is invalid");
      }
      if (star.role === "guest" && peerId !== HOST_PEER_ID) {
        return starError("role_forbidden", "a guest may only address the host");
      }
      const peer = findPeer(peerId);
      const known = (CHANNEL_MESSAGE_TYPES as Record<string, unknown>)[channelName]
        ? (channelName as ChannelName)
        : null;
      const parsed = parseWire(wire);
      if (parsed.error) {
        return peer
          ? peerError(peer, known, parsed.error, parsed.detail ?? parsed.error)
          : starError(parsed.error, parsed.detail ?? parsed.error);
      }
      const mismatch = checkChannelMessage(channelName, parsed);
      if (mismatch) {
        return peer
          ? peerError(peer, known, mismatch.error as string, mismatch.detail as string)
          : starError(mismatch.error as string, mismatch.detail as string);
      }
      if (!peer) {
        return starError("unknown_peer", "star transport has no peer with that id");
      }
      return enqueue(peer, channelName as ChannelName, wire);
    },

    broadcast(line) {
      const blocked = requireConnected();
      if (blocked) {
        return blocked;
      }
      if (star.role !== "host") {
        return starError("role_forbidden", "only a host fans out canonical batches");
      }
      if (typeof line !== "string") {
        return starError("malformed", "broadcast wire must be a string");
      }
      const first = line.indexOf("|");
      if (first < 0) {
        return starError("malformed", "broadcast wire has invalid fields");
      }
      const channelName = line.slice(0, first);
      const wire = line.slice(first + 1);
      if (!(CHANNEL_MESSAGE_TYPES as Record<string, unknown>)[channelName]) {
        return starError("channel_mismatch", "channel must be control or input");
      }
      const parsed = parseWire(wire);
      if (parsed.error) {
        return starError(parsed.error, parsed.detail ?? parsed.error);
      }
      const mismatch = checkChannelMessage(channelName, parsed);
      if (mismatch) {
        return starError(mismatch.error as string, mismatch.detail as string);
      }
      let delivered = 0;
      for (const id of star.order) {
        const peer = star.peers[id] as PeerState;
        if (
          peer.state === "connected" &&
          enqueue(peer, channelName as ChannelName, wire) === "ok"
        ) {
          delivered += 1;
        }
      }
      return "delivered|" + delivered;
    },

    // Deterministic drain: a persistent cursor walks (slot, channel-rank)
    // pairs and takes at most one message per pair. Browser callback
    // arrival order is deliberately not treated as simulation order.
    poll() {
      if (star.state !== "connected") {
        return "";
      }
      const slots = star.order.length;
      if (slots === 0) {
        return "";
      }
      const pairs = slots * CHANNEL_ORDER.length;
      for (let attempt = 0; attempt < pairs; attempt += 1) {
        const peerIndex = Math.floor(star.cursor / CHANNEL_ORDER.length) % slots;
        const channelIndex = star.cursor % CHANNEL_ORDER.length;
        star.cursor = (star.cursor + 1) % pairs;
        const peer = star.peers[star.order[peerIndex] as string] as PeerState;
        const channel = peer.channels[CHANNEL_ORDER[channelIndex] as ChannelName];
        if (channel.inbound.length > 0) {
          channel.received += 1;
          star.received += 1;
          return channel.inbound.shift() as string;
        }
      }
      return "";
    },

    poll_event() {
      drainAll();
      return star.events.length === 0 ? "" : (star.events.shift() as string);
    },

    diagnostics() {
      const lines = [
        [
          "star",
          star.role,
          star.state,
          star.max_guests,
          star.order.length,
          star.queue_limit,
          star.buffered_amount_limit,
          star.events.length,
          star.sent,
          star.received,
          star.dropped_outbound,
          star.dropped_inbound,
          star.malformed,
          star.unsupported_version,
          star.overflow,
          star.backpressure,
          escapeField(star.last_error),
        ].join("|"),
      ];
      for (const id of star.order) {
        const peer = star.peers[id] as PeerState;
        let fields: (string | number)[] = [
          "peer",
          peer.id,
          peer.slot,
          peer.state,
          escapeField(peer.ice_state),
        ];
        fields = fields
          .concat(channelRecord(peer.channels.control))
          .concat(channelRecord(peer.channels.input));
        fields.push(peer.sequence_gaps);
        fields.push(peer.backpressure);
        fields.push(peer.malformed);
        fields.push(escapeField(peer.last_error));
        lines.push(fields.join("|"));
      }
      return lines.join("\n");
    },
  };
}

declare global {
  interface Window {
    GoliseoStarTransport?: GoliseoStarTransportBridge;
  }
}

/** Installs a real, `RTCPeerConnection`-backed `window.GoliseoStarTransport`
 * — the seam `browser_star.ts`'s `BrowserStarTransport` calls through (its
 * `defaultEval` evaluates `window.GoliseoStarTransport.<method>(...)`
 * textually). Idempotent: a bridge already installed on `target` is left
 * alone, matching the original `window.GoliseoStarTransport =
 * window.GoliseoStarTransport || (...)` guard. */
export function installGoliseoStarTransport(
  target: typeof globalThis = globalThis,
  options: StarBridgeOptions = {},
): GoliseoStarTransportBridge {
  const host = target as unknown as Window;
  if (!host.GoliseoStarTransport) {
    host.GoliseoStarTransport = newGoliseoStarTransportBridge(options);
  }
  return host.GoliseoStarTransport;
}

/** The default `eval` seam a real page hands to `BrowserStarTransport`:
 * every command `browser_star.ts` builds
 * (`window.GoliseoStarTransport.<method>(<args>)`) is valid JS source once
 * the bridge is installed, so a real `eval` call is the whole
 * implementation. `(0, eval)` forces indirect (global) eval so the
 * command cannot see or touch this module's own locals. */
export function browserStarEval(
  command: string,
): readonly [result: string | null, error: string | null] {
  try {
    // Indirect eval (calling through a reference rather than the literal
    // `eval(...)` form) runs in global scope, so the command cannot see or
    // touch this module's own locals.
    // eslint-disable-next-line no-eval
    const indirectEval: (code: string) => unknown = globalThis.eval;
    const result = indirectEval(command);
    if (result === undefined || result === null) {
      return [null, "browser star bridge returned no result"];
    }
    // `result` is whatever the evaluated command returned, and this
    // stringification IS the transport contract
    // (docs/online/transport_bridge.md). Narrowing it would change what the
    // bridge reports for a non-string result -- a contract change, not a lint
    // fix, so it is deliberately left for its own change.
    // eslint-disable-next-line @typescript-eslint/no-base-to-string
    return [String(result), null];
  } catch (error) {
    return [null, String(error)];
  }
}
