#!/usr/bin/env node
"use strict";

// Scripted host-star coverage for scripts/webrtc_star_host.js. Each endpoint
// runs in its own VM context, which is what "a separate browser context" means
// for this bridge, and links are established through the real manual
// offer/answer path against an RTCPeerConnection double.
//
// The double deliberately reproduces browser behaviour the bridge has to
// survive: close() fires its state-change callbacks on a LATER task, ICE and
// connection failures are drivable, and remote data channels arrive through
// `ondatachannel` rather than being injected. Every callback the bridge
// registers is dispatched through `fire`, which records anything it throws, so
// a late callback touching a torn-down peer fails the run instead of passing
// silently.
//
// Three more axes exist because a forgiving double certifies code a browser
// breaks:
//
//   * `bufferedAmount` is real state. send() grows it, the transport drains it,
//     and `onbufferedamountlow` fires -- on a later task -- only when the value
//     CROSSES `bufferedAmountLowThreshold` downwards. That is a browser's
//     primary drain-resumption path, and it is the one the bridge relies on.
//   * data channels open in a caller-chosen order, synchronously or on later
//     tasks. A browser guarantees neither the ordering nor the synchronicity.
//   * `iceGatheringState` can start "gathering" and transition later, or never
//     complete at all, so both the `onicegatheringstatechange` path and the
//     ICE timeout fallback are reachable.

const fs = require("fs");
const vm = require("vm");

const source = fs.readFileSync(require.resolve("./webrtc_star_host.js"), "utf8");

const callbackErrors = [];

function fire(target, name, argument) {
  const handler = target[name];
  if (typeof handler !== "function") {
    return;
  }
  try {
    handler(argument);
  } catch (error) {
    callbackErrors.push(name + ": " + (error && error.message ? error.message : String(error)));
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function check(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

function createStar(extra) {
  const window = {
    TextEncoder,
    setTimeout,
    clearTimeout,
    console: { info() {} }
  };
  Object.assign(window, extra || {});
  vm.runInNewContext(source, {
    window,
    Promise,
    Number,
    Math,
    Object,
    String,
    JSON,
    encodeURIComponent,
    decodeURIComponent,
    unescape
  });
  return window.GoliseoStarTransport;
}

function wire(type, seq, tick, payload) {
  return [
    1,
    encodeURIComponent(type),
    seq,
    tick === null ? "" : tick,
    encodeURIComponent(payload)
  ].join("|");
}

// ---------------------------------------------------------------------------
// RTCPeerConnection double
// ---------------------------------------------------------------------------

function createRtc(options) {
  const settings = options || {};
  const instances = [];

  function FakeChannel(label, config) {
    this.label = label;
    this.config = config;
    this.readyState = "connecting";
    this.bufferedAmount = 0;
    this.bufferedAmountLowThreshold = 0;
    this.sent = [];
    this._peer = null;
  }
  FakeChannel.prototype.send = function (data) {
    if (this.readyState !== "open") {
      throw new Error("InvalidStateError: data channel is not open");
    }
    this.sent.push(data);
    // A real send is buffered, not instantaneous: the byte count lands in
    // bufferedAmount and only leaves it as the user agent puts bytes on the
    // wire (see `releaseBytes`).
    this.bufferedAmount += Buffer.byteLength(String(data), "utf8");
    const peer = this._peer;
    if (peer) {
      fire(peer, "onmessage", { data });
    }
  };
  // Pretend the user agent put `bytes` on the wire. This is the ONLY way
  // bufferedAmount ever falls, and it is what makes onbufferedamountlow fire:
  // per spec the event is queued when the value transitions from above the
  // threshold to at-or-below it, on a later task.
  FakeChannel.prototype.releaseBytes = function (bytes) {
    const before = this.bufferedAmount;
    const amount = bytes === undefined ? before : bytes;
    this.bufferedAmount = Math.max(0, before - amount);
    const crossed =
      before > this.bufferedAmountLowThreshold &&
      this.bufferedAmount <= this.bufferedAmountLowThreshold;
    if (crossed) {
      setTimeout(() => fire(this, "onbufferedamountlow", {}), 0);
    }
  };
  // Congest the channel without any crossing: bufferedAmount only ever rises
  // here, so no onbufferedamountlow can be attributed to setting it up.
  FakeChannel.prototype.congestTo = function (bytes) {
    check(bytes >= this.bufferedAmount, "congestTo must not lower bufferedAmount");
    this.bufferedAmount = bytes;
  };
  FakeChannel.prototype.close = function () {
    if (this.readyState === "closed") {
      return;
    }
    this.readyState = "closed";
    // Real data channels fire onclose on a later task.
    setTimeout(() => fire(this, "onclose", {}), 0);
  };

  function FakePeerConnection(config) {
    this.config = config;
    // "complete" keeps waitForIce on its synchronous fast path. The gathering
    // modes below are what a real connection does: state starts short of
    // complete and transitions later, or never does.
    this.iceGatheringState =
      settings.ice_gathering === undefined ? "complete" : settings.ice_gathering;
    this.iceConnectionState = "new";
    this.connectionState = "new";
    this.localDescription = null;
    this.remoteDescription = null;
    this.localChannels = [];
    this.remoteChannels = [];
    instances.push(this);
  }
  // Finish ICE gathering the way a browser does: mutate the state, then fire
  // onicegatheringstatechange on a later task.
  FakePeerConnection.prototype.completeIceGathering = function () {
    this.iceGatheringState = "complete";
    setTimeout(() => fire(this, "onicegatheringstatechange", {}), 0);
  };
  FakePeerConnection.prototype.createDataChannel = function (label, config) {
    const channel = new FakeChannel(label, config);
    this.localChannels.push(channel);
    return channel;
  };
  FakePeerConnection.prototype.createOffer = function () {
    return Promise.resolve({ type: "offer", sdp: "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\na=offer" });
  };
  FakePeerConnection.prototype.createAnswer = function () {
    return Promise.resolve({ type: "answer", sdp: "v=0\r\no=- 2 2 IN IP4 127.0.0.1\r\na=answer" });
  };
  FakePeerConnection.prototype.setLocalDescription = function (description) {
    this.localDescription = description;
    return Promise.resolve();
  };
  FakePeerConnection.prototype.setRemoteDescription = function (description) {
    this.remoteDescription = description;
    return Promise.resolve();
  };
  FakePeerConnection.prototype.close = function () {
    if (this.connectionState === "closed") {
      return;
    }
    this.connectionState = "closed";
    this.iceConnectionState = "closed";
    // Per spec these fire on a later task, after the caller has already
    // dropped its reference to the connection.
    setTimeout(() => {
      fire(this, "onconnectionstatechange", {});
      fire(this, "oniceconnectionstatechange", {});
    }, 0);
  };
  FakePeerConnection.prototype.failIce = function () {
    this.iceConnectionState = "failed";
    setTimeout(() => fire(this, "oniceconnectionstatechange", {}), 0);
  };
  FakePeerConnection.prototype.failConnection = function () {
    this.connectionState = "failed";
    setTimeout(() => fire(this, "onconnectionstatechange", {}), 0);
  };
  // Deliver this connection's locally created channels to `other` the way a
  // browser does: through ondatachannel, then onopen on both ends.
  //
  // A browser guarantees neither the order in which the two channels open nor
  // that they open during this call, so both are the caller's choice.
  // `options.order` lists the labels to open, first to last; with
  // `options.open === "deferred"` nothing opens here and the returned array of
  // thunks lets the test run each open on a later task of its own choosing.
  // The returned array is in `order`, so `openings[0]` is the first to open.
  FakePeerConnection.prototype.connectTo = function (other, options) {
    const settings = options || {};
    const order = settings.order || this.localChannels.map((channel) => channel.label);
    check(
      order.length === this.localChannels.length,
      "connectTo was given an order that does not cover every local channel"
    );
    const pairs = order.map((label) => {
      const local = this.localChannels.find((channel) => channel.label === label);
      check(local !== undefined, "connectTo was asked to open an unknown channel: " + label);
      const remote = new FakeChannel(local.label, local.config);
      local._peer = remote;
      remote._peer = local;
      other.remoteChannels.push(remote);
      fire(other, "ondatachannel", { channel: remote });
      return { local, remote };
    });
    this.connectionState = "connected";
    this.iceConnectionState = "connected";
    other.connectionState = "connected";
    other.iceConnectionState = "connected";
    const openings = pairs.map((pair) => () => {
      pair.local.readyState = "open";
      pair.remote.readyState = "open";
      fire(pair.local, "onopen", {});
      fire(pair.remote, "onopen", {});
    });
    if (settings.open !== "deferred") {
      openings.forEach((open) => open());
    }
    return openings;
  };
  // Open an extra channel from this side onto an established connection.
  FakePeerConnection.prototype.pushExtraChannel = function (other, label) {
    const remote = new FakeChannel(label, { ordered: true });
    other.remoteChannels.push(remote);
    fire(other, "ondatachannel", { channel: remote });
    return remote;
  };

  return { FakePeerConnection, instances };
}

// ---------------------------------------------------------------------------
// Star construction over the real signaling path
// ---------------------------------------------------------------------------

async function connect(host, guest, peerId, rtc, link) {
  const first = rtc.instances.length;
  check(host.request_offer(peerId) === "ok", "request_offer was refused");
  await delay(5);
  const offer = host.take_signal(peerId);
  check(offer.startsWith("signal|"), "the host offer never became available");

  check(guest.accept_offer(offer.slice("signal|".length)) === "ok", "accept_offer was refused");
  await delay(5);
  const answer = guest.take_signal("host");
  check(answer.startsWith("signal|"), "the guest answer never became available");
  check(
    host.accept_answer(peerId, answer.slice("signal|".length)) === "ok",
    "accept_answer was refused"
  );
  await delay(5);

  const hostPc = rtc.instances[first];
  const guestPc = rtc.instances[first + 1];
  const openings = hostPc.connectTo(guestPc, link);
  await delay(5);
  return { hostPc, guestPc, openings };
}

async function buildStar(options) {
  const settings = options || {};
  const queueLimit = settings.queue_limit || 64;
  const bufferedLimit = settings.buffered_amount_limit || 65536;
  const rtc = createRtc();
  const host = createStar({ RTCPeerConnection: rtc.FakePeerConnection });
  check(
    host.initialize("host", queueLimit, 7, bufferedLimit) === "star|connected",
    "host did not initialize"
  );
  const guests = [];
  for (let index = 1; index <= (settings.guests || 7); index += 1) {
    const peerId = "guest_" + index;
    check(host.open_peer(peerId) === "slot|" + index, "host did not allocate slot " + index);
    const guest = createStar({ RTCPeerConnection: rtc.FakePeerConnection });
    check(
      guest.initialize("guest", queueLimit, 1, bufferedLimit) === "star|connected",
      "guest did not initialize"
    );
    const links = await connect(host, guest, peerId, rtc, settings.link);
    guests.push({
      peerId,
      star: guest,
      hostPc: links.hostPc,
      guestPc: links.guestPc,
      openings: links.openings
    });
  }
  return { host, guests, rtc };
}

// The channel the host bridge actually sends through on a given link.
function hostChannel(link, label) {
  const channel = link.hostPc.localChannels.find((candidate) => candidate.label === label);
  check(channel !== undefined, "the bridge never created a " + label + " channel");
  return channel;
}

function peerRecord(star, peerId) {
  const record = diagnostics(star).peers.find((peer) => peer.id === peerId);
  check(record !== undefined, "diagnostics do not report " + peerId);
  return record;
}

// Run one deferred channel open on a later task, the way a browser delivers an
// open event, and let the bridge's synchronous reaction settle.
function openLater(open) {
  return new Promise((resolve) => {
    setTimeout(() => {
      open();
      resolve();
    }, 1);
  });
}

function diagnostics(star) {
  const lines = star.diagnostics().split("\n");
  const head = lines[0].split("|");
  const peers = lines.slice(1).map((line) => {
    const fields = line.split("|");
    return {
      id: fields[1],
      slot: Number(fields[2]),
      state: fields[3],
      ice_state: decodeURIComponent(fields[4]),
      control: {
        state: fields[5],
        outbound: Number(fields[6]),
        inbound: Number(fields[7]),
        buffered: Number(fields[8]),
        sent: Number(fields[9]),
        received: Number(fields[10]),
        dropped_outbound: Number(fields[11]),
        dropped_inbound: Number(fields[12])
      },
      input: {
        state: fields[13],
        outbound: Number(fields[14]),
        inbound: Number(fields[15]),
        buffered: Number(fields[16]),
        sent: Number(fields[17]),
        received: Number(fields[18]),
        dropped_outbound: Number(fields[19]),
        dropped_inbound: Number(fields[20])
      },
      sequence_gaps: Number(fields[21]),
      backpressure: Number(fields[22]),
      malformed: Number(fields[23]),
      last_error: decodeURIComponent(fields[24])
    };
  });
  check(
    lines.every((line) => line.split("|").length === (line.startsWith("star") ? 17 : 25)),
    "diagnostics record width does not match the Lua decoder"
  );
  return {
    role: head[1],
    state: head[2],
    capacity: Number(head[3]),
    peer_count: Number(head[4]),
    queue_limit: Number(head[5]),
    buffered_amount_limit: Number(head[6]),
    dropped_outbound: Number(head[10]),
    dropped_inbound: Number(head[11]),
    malformed: Number(head[12]),
    overflow: Number(head[14]),
    backpressure: Number(head[15]),
    peers
  };
}

function drainEvents(star) {
  const events = [];
  let event = "";
  while ((event = star.poll_event()) !== "") {
    events.push(event);
  }
  return events;
}

function backpressureEvents(events) {
  return events.filter(
    (event) => event.indexOf("peer_error|guest_1|control|backpressure") === 0
  );
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

async function testFullStar() {
  const { host, guests } = await buildStar();
  check(host.open_peer("guest_8").startsWith("error|capacity|"), "capacity was not enforced");
  check(
    host.open_peer("guest_1").startsWith("error|duplicate_peer|"),
    "duplicate peer id was not rejected"
  );
  check(host.open_peer("host").startsWith("error|duplicate_peer|"), "host id is not reserved");
  check(host.open_peer("bad id").startsWith("error|malformed|"), "peer id charset is not enforced");

  const summary = diagnostics(host);
  check(summary.peer_count === 7 && summary.capacity === 7, "host does not hold seven guests");
  check(
    summary.peers.every((peer) => peer.state === "connected"),
    "not every guest link reached connected"
  );

  // Independent addressing: only the third guest sees this control message.
  check(
    host.send("guest_3|control|" + wire("event", 0, null, "only-three")) === "ok",
    "addressed control send failed"
  );
  guests.forEach((guest) => {
    const line = guest.star.poll();
    if (guest.peerId === "guest_3") {
      check(
        line === "host|control|" + wire("event", 0, null, "only-three"),
        "guest_3 did not receive the addressed message"
      );
    } else {
      check(line === "", guest.peerId + " received a message addressed to another peer");
    }
  });

  // Host fan-out reaches every connected link exactly once.
  check(
    host.broadcast("input|" + wire("input", 1, 120, "batch")) === "delivered|7",
    "host fan-out did not reach seven guests"
  );
  guests.forEach((guest) => {
    check(
      guest.star.poll() === "host|input|" + wire("input", 1, 120, "batch"),
      guest.peerId + " missed the canonical batch"
    );
    check(guest.star.poll() === "", guest.peerId + " received a duplicate batch");
  });

  // Guests are attributed by link identity, never by payload.
  guests.forEach((guest, index) => {
    check(
      guest.star.send("host|input|" + wire("input", 2, 121, "g" + index)) === "ok",
      "guest input send failed"
    );
  });
  const seen = new Set();
  for (let index = 0; index < 7; index += 1) {
    const line = host.poll();
    const peerId = line.split("|")[0];
    check(!seen.has(peerId), "host drained the same peer twice in one pass");
    seen.add(peerId);
  }
  check(seen.size === 7, "host did not attribute every guest independently");
  check(host.poll() === "", "host inbound queues were not drained");
}

async function testChannelConfiguration() {
  const rtc = createRtc();
  const host = createStar({ RTCPeerConnection: rtc.FakePeerConnection });
  host.initialize("host", 64, 7, 65536);
  host.open_peer("guest_1");
  host.request_offer("guest_1");
  await delay(5);

  // Assert on the channels the bridge actually created, not on a hand-written
  // config handed to a throwaway connection.
  const hostPc = rtc.instances[0];
  check(hostPc !== undefined, "request_offer did not create a peer connection");
  check(hostPc.localChannels.length === 2, "the bridge did not create exactly two channels");
  const byLabel = {};
  hostPc.localChannels.forEach((channel) => {
    byLabel[channel.label] = channel.config;
  });
  check(byLabel.control !== undefined, "the bridge did not create a control channel");
  check(byLabel.input !== undefined, "the bridge did not create an input channel");
  check(byLabel.control.ordered === true, "the control channel is not reliable and ordered");
  check(
    byLabel.control.maxRetransmits === undefined,
    "the control channel must not limit retransmits"
  );
  check(
    byLabel.input.ordered === false && byLabel.input.maxRetransmits === 0,
    "the input channel is not unordered and loss tolerant"
  );
  host.shutdown();
  await delay(10);
}

async function testDuplicateChannelRefused() {
  const { host, guests } = await buildStar({ guests: 1 });
  const link = guests[0];
  drainEvents(host);

  // A connected peer opens a second channel on an occupied label. The bridge
  // must refuse and close it rather than re-registering listeners.
  const extra = link.guestPc.pushExtraChannel(link.hostPc, "control");
  await delay(5);
  const events = drainEvents(host);
  check(
    events.some((event) => event.indexOf("peer_error|guest_1|control|channel_mismatch") === 0),
    "a duplicate control channel was not refused"
  );
  check(extra.readyState === "closed", "the refused duplicate channel was not closed");

  // The original channel is still the one the bridge sends through.
  const original = link.hostPc.localChannels.find((channel) => channel.label === "control");
  const before = original.sent.length;
  check(
    host.send("guest_1|control|" + wire("event", 9, null, "still-mine")) === "ok",
    "the link stopped accepting traffic after the duplicate"
  );
  check(original.sent.length === before + 1, "traffic did not go through the original channel");
  check(extra.sent.length === 0, "traffic leaked to the refused channel");
}

async function testPermissions() {
  const { host, guests } = await buildStar({ guests: 2 });
  const guest = guests[0].star;
  check(
    guest.broadcast("input|" + wire("input", 0, 1, "x")).startsWith("error|role_forbidden|"),
    "a guest was allowed to fan out a canonical batch"
  );
  check(
    guest
      .send("guest_2|control|" + wire("event", 0, null, "x"))
      .startsWith("error|role_forbidden|"),
    "a guest was allowed to address another guest"
  );
  check(
    guest.open_peer("guest_9").startsWith("error|role_forbidden|"),
    "a guest was allowed to open a link"
  );
  check(
    guest.request_offer("guest_2").startsWith("error|role_forbidden|"),
    "a guest was allowed to create an offer"
  );
  check(
    host.accept_offer("x").startsWith("error|role_forbidden|"),
    "a host was allowed to accept an offer"
  );
  check(
    host.send("guest_1|control|" + wire("input", 0, 1, "x")).startsWith("error|channel_mismatch|"),
    "an input message was accepted on the reliable control channel"
  );
  check(
    host.send("guest_1|input|" + wire("event", 0, null, "x")).startsWith("error|channel_mismatch|"),
    "a control message was accepted on the lossy input channel"
  );
  check(
    host.send("guest_1|control|1|event|nope|x").startsWith("error|malformed|"),
    "a malformed wire was accepted"
  );
  check(
    host.send("guest_1|control|2|event|0||x").startsWith("error|unsupported_version|"),
    "an unsupported envelope version was accepted"
  );
  check(
    host.send("nobody|control|" + wire("event", 0, null, "x")).startsWith("error|unknown_peer|"),
    "an unknown peer was accepted"
  );
  // Validation order is pinned: message shape is judged before the peer is
  // resolved, because the Lua adapters cannot resolve a peer without asking
  // this bridge. A combined fault therefore reports channel_mismatch on all
  // three layers rather than depending on which one you asked.
  check(
    host.send("nobody|control|" + wire("input", 0, 1, "x")).startsWith("error|channel_mismatch|"),
    "a combined unknown-peer plus channel-mismatch fault did not report channel_mismatch"
  );
  const summary = diagnostics(host);
  check(summary.malformed >= 3, "malformed and mismatch counters are not visible");
}

async function testBoundsAndOverflow() {
  const { host, guests } = await buildStar({ guests: 1, queue_limit: 4 });
  const guest = guests[0].star;

  // Inbound bound: the guest never polls, so its queue fills and drops.
  for (let seq = 0; seq < 10; seq += 1) {
    host.send("guest_1|input|" + wire("input", seq, 100 + seq, "x"));
  }
  const guestSummary = diagnostics(guest);
  check(guestSummary.peers[0].input.inbound === 4, "inbound queue exceeded its bound");
  check(
    guestSummary.overflow >= 1 && guestSummary.dropped_inbound >= 1,
    "inbound overflow is not visible"
  );
  check(guestSummary.peers[0].sequence_gaps === 0, "gaps were counted before any drop");

  for (let index = 0; index < 4; index += 1) {
    guest.poll();
  }
  host.send("guest_1|input|" + wire("input", 20, 200, "x"));
  check(diagnostics(guest).peers[0].sequence_gaps > 0, "sequence gaps are not reported");
}

// The poll-driven fallback, measured on its own. Congestion is relieved here
// by assigning bufferedAmount directly -- deliberately NOT a browser
// operation, and the one thing in this file that skips the low-water event --
// so that everything this case proves is attributable to poll_event's
// drainAll. The browser's own resumption path is testBufferedAmountLowDrain.
async function testBackpressureLatchRearms() {
  const { host, guests } = await buildStar({
    guests: 1,
    queue_limit: 16,
    buffered_amount_limit: 200
  });
  const control = hostChannel(guests[0], "control");
  drainEvents(host);

  const line = "guest_1|control|" + wire("event", 0, null, "payload");
  const wireBytes = Buffer.byteLength(wire("event", 0, null, "payload"), "utf8");

  // 1. Healthy: the message reaches the data channel immediately.
  check(host.send(line) === "ok", "a healthy send was refused");
  check(control.sent.length === 1, "a healthy send did not reach the data channel");
  check(diagnostics(host).peers[0].control.outbound === 0, "a healthy send stayed queued");
  check(
    control.bufferedAmount === wireBytes,
    "a sent message did not land in the channel send buffer"
  );

  // 2. Saturate the send buffer. The message queues and one event is emitted.
  control.congestTo(1000);
  check(host.send(line) === "ok", "a backpressured send was refused instead of queued");
  check(control.sent.length === 1, "a message was pushed into a saturated buffer");
  check(diagnostics(host).peers[0].control.outbound === 1, "the message was not queued");
  const first = backpressureEvents(drainEvents(host));
  check(first.length === 1, "expected one backpressure event, got " + first.length);

  // 3. Still saturated: the latch holds, no event storm.
  host.send(line);
  host.poll_event();
  check(
    backpressureEvents(drainEvents(host)).length === 0,
    "a held backpressure condition emitted a repeat event"
  );
  check(diagnostics(host).backpressure === 1, "the backpressure counter double-counted");

  // 4. Relieve the buffer silently. Only polling can notice, and it does.
  control.bufferedAmount = 0;
  await delay(5);
  check(control.sent.length === 1, "a silent relief drained without anything polling");
  drainEvents(host);
  check(control.sent.length === 3, "the queued messages did not send after relief");
  check(diagnostics(host).peers[0].control.outbound === 0, "the outbound queue did not drain");

  // 5. Re-saturate: a genuine second congestion must be reported again.
  control.congestTo(1000);
  check(host.send(line) === "ok", "a re-saturated send was refused");
  const second = backpressureEvents(drainEvents(host));
  check(
    second.length === 1,
    "the backpressure latch did not re-arm: expected a second event, got " + second.length
  );
  check(diagnostics(host).backpressure === 2, "the second congestion was not counted");

  // The outbound bound still applies while congested.
  for (let seq = 1; seq <= 40; seq += 1) {
    host.send("guest_1|control|" + wire("event", seq, null, "payload"));
  }
  const overflowed = diagnostics(host);
  check(overflowed.peers[0].control.outbound === 16, "outbound queue exceeded its bound");
  check(
    overflowed.overflow >= 1 && overflowed.dropped_outbound >= 1,
    "outbound overflow is not visible"
  );
}

// The browser's own drain-resumption path, end to end and without polling:
// the bridge asks to be woken at half its bufferedAmount limit, the channel
// crosses that threshold downwards, onbufferedamountlow fires on a later task,
// and THAT is what empties the outbound queue. Nothing in this case calls
// poll_event between congestion and relief.
async function testBufferedAmountLowDrain() {
  const bufferedLimit = 200;
  const { host, guests } = await buildStar({
    guests: 1,
    queue_limit: 16,
    buffered_amount_limit: bufferedLimit
  });
  const control = hostChannel(guests[0], "control");
  const input = hostChannel(guests[0], "input");
  drainEvents(host);

  // 1. The threshold is part of the contract, not an implementation detail:
  //    half the configured limit, on every channel of the link, so a channel
  //    resumes well before its send buffer empties.
  const threshold = Math.floor(bufferedLimit / 2);
  check(
    control.bufferedAmountLowThreshold === threshold,
    "the control low-water threshold is " +
      control.bufferedAmountLowThreshold +
      ", expected half the " +
      bufferedLimit +
      "-byte limit (" +
      threshold +
      ")"
  );
  check(
    input.bufferedAmountLowThreshold === threshold,
    "the input low-water threshold is " +
      input.bufferedAmountLowThreshold +
      ", expected half the " +
      bufferedLimit +
      "-byte limit (" +
      threshold +
      ")"
  );

  // 2. Congest well past the limit, then queue a message behind it.
  const line = "guest_1|control|" + wire("event", 4, null, "resumed");
  control.congestTo(1000);
  const baseline = control.sent.length;
  check(host.send(line) === "ok", "a backpressured send was refused instead of queued");
  check(control.sent.length === baseline, "a message was pushed into a saturated buffer");
  check(peerRecord(host, "guest_1").control.outbound === 1, "the message was not queued");

  // 3. Drain the buffer down to just ABOVE the threshold. No crossing means no
  //    event, and with nothing polling there is nothing else to resume the
  //    queue. A threshold set any higher than half the limit fires here.
  control.releaseBytes(1000 - (threshold + 1));
  check(
    control.bufferedAmount === threshold + 1,
    "the fixture did not land one byte above the threshold"
  );
  await delay(5);
  check(
    control.sent.length === baseline,
    "the outbound queue drained before the buffer crossed the low-water threshold"
  );
  check(
    peerRecord(host, "guest_1").control.outbound === 1,
    "the outbound queue emptied before the buffer crossed the low-water threshold"
  );

  // 4. Cross it. The event is queued on a later task, exactly like a browser,
  //    so the drain must not have happened yet when releaseBytes returns.
  control.releaseBytes(1);
  check(control.bufferedAmount === threshold, "the fixture did not land on the threshold");
  check(
    control.sent.length === baseline,
    "the drain resumed synchronously instead of from the low-water event"
  );
  await delay(5);
  check(
    control.sent.length === baseline + 1,
    "onbufferedamountlow did not resume the drain: the queued message never reached the channel"
  );
  check(
    peerRecord(host, "guest_1").control.outbound === 0,
    "onbufferedamountlow did not empty the outbound queue"
  );

  // 5. The message the browser resumed actually arrives at the far end, and
  //    the latch cleared with it.
  check(
    guests[0].star.poll() === "host|control|" + wire("event", 4, null, "resumed"),
    "the guest never received the message the low-water event released"
  );
  check(
    backpressureEvents(drainEvents(host)).length === 1,
    "the single congestion was not reported exactly once"
  );
  check(
    host.send("guest_1|control|" + wire("event", 5, null, "after")) === "ok",
    "the link stopped accepting traffic after a low-water resumption"
  );
  check(
    control.sent.length === baseline + 2,
    "a healthy send after resumption did not reach the channel"
  );
}

// A browser guarantees neither which data channel opens first nor that either
// opens during the call that created it. The bridge must hold the peer at
// "connecting" until BOTH are open, whichever order they arrive in, and must
// then carry traffic on both.
async function testChannelOpenOrdering() {
  const orders = [["control", "input"], ["input", "control"]];
  for (const order of orders) {
    const label = order.join(" then ");
    const { host, guests } = await buildStar({
      guests: 1,
      link: { order, open: "deferred" }
    });
    const link = guests[0];
    check(
      peerRecord(host, "guest_1").state === "connecting",
      "the peer was connected before either channel opened (" + label + ")"
    );

    // Each open lands on its own later task, the way a browser delivers one.
    await openLater(link.openings[0]);
    check(
      peerRecord(host, "guest_1").state === "connecting",
      "the peer was called connected after only " + order[0] + " opened (" + label + ")"
    );
    check(
      peerRecord(link.star, "host").state === "connecting",
      "the guest called the link connected after only " + order[0] + " opened (" + label + ")"
    );
    const early = host.send("guest_1|control|" + wire("event", 0, null, "early"));
    check(
      early.startsWith("error|not_connected|"),
      "a half-open link accepted traffic (" + label + "): " + early
    );

    await openLater(link.openings[1]);
    check(
      peerRecord(host, "guest_1").state === "connected",
      "the peer never connected after both channels opened (" + label + ")"
    );

    // Both channels carry traffic, in both directions, whichever opened first.
    check(
      host.send("guest_1|control|" + wire("event", 1, null, "ctl")) === "ok",
      "the control channel did not carry traffic (" + label + ")"
    );
    check(
      host.send("guest_1|input|" + wire("input", 1, 30, "inp")) === "ok",
      "the input channel did not carry traffic (" + label + ")"
    );
    const received = [link.star.poll(), link.star.poll()].sort();
    check(
      received[0] === "host|control|" + wire("event", 1, null, "ctl") &&
        received[1] === "host|input|" + wire("input", 1, 30, "inp"),
      "the guest did not receive both messages (" + label + "): " + received.join(" , ")
    );
    check(
      link.star.send("host|input|" + wire("input", 2, 31, "back")) === "ok",
      "the guest could not send back (" + label + ")"
    );
    check(
      host.poll() === "guest_1|input|" + wire("input", 2, 31, "back"),
      "the host did not receive the guest reply (" + label + ")"
    );
    host.shutdown();
    link.star.shutdown();
    await delay(10);
  }

  // Both opens scheduled up front on staggered later tasks, with nothing
  // stepping them: the link still settles on its own.
  const { host, guests } = await buildStar({
    guests: 1,
    link: { order: ["input", "control"], open: "deferred" }
  });
  const link = guests[0];
  setTimeout(link.openings[0], 1);
  setTimeout(link.openings[1], 8);
  await delay(25);
  check(
    peerRecord(host, "guest_1").state === "connected",
    "an unattended staggered open never reached connected"
  );
  host.shutdown();
  link.star.shutdown();
  await delay(10);
}

// A star whose window.setTimeout is stubbed, so waitForIce's fallback timer is
// inspectable and fireable on demand instead of really waiting out
// ICE_TIMEOUT_MS -- and so no live five-second timer outlives the case.
// waitForIce is the bridge's only user of window.setTimeout.
function createStarWithTimers(rtc) {
  const timers = [];
  const star = createStar({
    RTCPeerConnection: rtc.FakePeerConnection,
    setTimeout: (handler, ms) => {
      timers.push({ handler, ms });
      return timers.length;
    },
    clearTimeout: () => {}
  });
  return { star, timers };
}

// waitForIce's async branch. A real connection is rarely "complete" by the
// time setLocalDescription resolves, so the offer is published either from
// onicegatheringstatechange or from the ICE timeout fallback.
async function testIceGatheringTransition() {
  const rtc = createRtc({ ice_gathering: "gathering" });
  const { star: host, timers } = createStarWithTimers(rtc);
  check(host.initialize("host", 64, 7, 65536) === "star|connected", "host did not initialize");
  check(host.open_peer("guest_1") === "slot|1", "host did not allocate a slot");
  check(host.request_offer("guest_1") === "ok", "request_offer was refused");
  await delay(5);

  const pc = rtc.instances[0];
  check(pc !== undefined, "request_offer did not create a peer connection");
  check(pc.iceGatheringState === "gathering", "the fixture did not start mid-gathering");
  check(
    host.take_signal("guest_1") === "",
    "the offer was published before ICE gathering completed"
  );

  pc.completeIceGathering();
  await delay(5);
  check(
    host.take_signal("guest_1").startsWith("signal|"),
    "onicegatheringstatechange did not release the offer"
  );
  // The state change won the race: the fallback was armed but never fired.
  check(timers.length === 1, "waitForIce armed " + timers.length + " fallback timers, expected 1");
  host.shutdown();
  await delay(10);
}

// The same branch, taken by the fallback instead: gathering never completes.
async function testIceGatheringTimeoutFallback() {
  const rtc = createRtc({ ice_gathering: "gathering" });
  const { star: host, timers } = createStarWithTimers(rtc);
  check(host.initialize("host", 64, 7, 65536) === "star|connected", "host did not initialize");
  check(host.open_peer("guest_1") === "slot|1", "host did not allocate a slot");
  check(host.request_offer("guest_1") === "ok", "request_offer was refused");
  await delay(5);

  // Gathering never completes, so nothing but the fallback can publish.
  check(
    rtc.instances[0].iceGatheringState === "gathering",
    "the fixture completed gathering on its own"
  );
  check(host.take_signal("guest_1") === "", "the offer was published without a gathering signal");
  check(timers.length === 1, "waitForIce armed " + timers.length + " fallback timers, expected 1");
  check(
    timers[0].ms === 5000,
    "the ICE fallback is armed at " + timers[0].ms + "ms, expected ICE_TIMEOUT_MS (5000)"
  );

  timers[0].handler();
  await delay(5);
  check(
    host.take_signal("guest_1").startsWith("signal|"),
    "the ICE timeout fallback never published the offer"
  );

  // The late transition is harmless: the promise is already settled and the
  // handler must not publish a second signal.
  rtc.instances[0].completeIceGathering();
  await delay(5);
  check(host.take_signal("guest_1") === "", "a late gathering transition republished the offer");
  host.shutdown();
  await delay(10);
}

async function testIceAndConnectionFailure() {
  const ice = await buildStar({ guests: 2 });
  drainEvents(ice.host);
  ice.guests[0].hostPc.failIce();
  await delay(5);

  const iceEvents = drainEvents(ice.host);
  check(
    iceEvents.some(
      (event) =>
        event.indexOf("peer_error|guest_1||disconnected") === 0 &&
        decodeURIComponent(event.split("|")[4]).indexOf("ICE negotiation failed") >= 0
    ),
    "an ICE failure did not surface a typed peer error"
  );
  check(
    iceEvents.some((event) => event === "peer_state|guest_1|error"),
    "an ICE failure did not move the peer to error"
  );
  const iceSummary = diagnostics(ice.host);
  const failed = iceSummary.peers.find((peer) => peer.id === "guest_1");
  check(failed.state === "error", "diagnostics do not report the failed peer state");
  check(failed.ice_state === "failed", "diagnostics do not report the failed ICE state");
  check(
    failed.last_error.indexOf("ICE negotiation failed") >= 0,
    "the peer last_error does not name the ICE failure"
  );
  check(
    iceSummary.peers.find((peer) => peer.id === "guest_2").state === "connected",
    "an ICE failure on one link disturbed another"
  );
  check(
    ice.host
      .send("guest_1|control|" + wire("event", 0, null, "x"))
      .startsWith("error|not_connected|"),
    "a failed link still accepted traffic"
  );

  // A connection-level failure is reported the same way.
  const connection = await buildStar({ guests: 1 });
  drainEvents(connection.host);
  connection.guests[0].hostPc.failConnection();
  await delay(5);
  const connectionEvents = drainEvents(connection.host);
  check(
    connectionEvents.some((event) => event.indexOf("peer_error|guest_1||disconnected") === 0),
    "a connection failure did not surface a typed peer error"
  );
  check(
    connectionEvents.some((event) => event === "peer_state|guest_1|disconnected"),
    "a connection failure did not move the peer to disconnected"
  );
}

async function testDisconnectAndTeardown() {
  const { host, guests } = await buildStar({ guests: 3 });
  check(host.close_peer("guest_2", "peer%20left") === "ok", "close_peer failed");
  // Late close callbacks land here; `fire` records anything they throw.
  await delay(10);

  const afterClose = diagnostics(host);
  const closed = afterClose.peers.find((peer) => peer.id === "guest_2");
  check(closed.state === "closed", "closed peer did not report closed");
  check(
    afterClose.peers.filter((peer) => peer.state === "connected").length === 2,
    "closing one link disturbed the others"
  );
  check(
    host.send("guest_2|control|" + wire("event", 0, null, "x")).startsWith("error|not_connected|"),
    "a closed link still accepted traffic"
  );
  check(
    host.send("guest_1|control|" + wire("event", 0, null, "x")) === "ok",
    "a surviving link stopped accepting traffic"
  );
  check(
    host.broadcast("input|" + wire("input", 5, 50, "x")) === "delivered|2",
    "fan-out did not skip the closed link"
  );
  check(host.close_peer("guest_2", "again") === "ok", "repeated close_peer was not idempotent");

  check(host.shutdown() === "star|closed", "shutdown did not close the star");
  check(host.shutdown() === "star|closed", "repeated shutdown was not idempotent");
  await delay(10);

  const torn = diagnostics(host);
  check(torn.state === "closed" && torn.peer_count === 0, "teardown left orphan peer records");
  check(host.poll() === "", "teardown left inbound traffic");
  check(
    host.send("guest_1|control|" + wire("event", 0, null, "x")).startsWith("error|closed|"),
    "a closed star still accepted traffic"
  );
  guests.forEach((guest) => guest.star.shutdown());
  await delay(10);
}

async function testSignalingFaults() {
  const rtc = createRtc();
  const host = createStar({ RTCPeerConnection: rtc.FakePeerConnection });
  const guest = createStar({ RTCPeerConnection: rtc.FakePeerConnection });
  host.initialize("host", 64, 7, 65536);
  guest.initialize("guest", 64, 1, 65536);
  host.open_peer("guest_1");

  check(
    host.request_offer("nobody").startsWith("error|unknown_peer|"),
    "request_offer accepted an unknown peer"
  );
  check(
    host.accept_answer("guest_1", encodeURIComponent("not json")).startsWith("error|signal_error|"),
    "an answer was accepted before an offer existed"
  );

  host.request_offer("guest_1");
  await delay(5);
  check(host.take_signal("guest_1").startsWith("signal|"), "no offer was produced");
  check(host.take_signal("guest_1") === "", "the offer was handed out twice");

  check(
    host.accept_answer("guest_1", encodeURIComponent("not json")).startsWith("error|signal_error|"),
    "a non-JSON remote signal was accepted"
  );
  check(
    host
      .accept_answer("guest_1", encodeURIComponent(JSON.stringify({ type: "nonsense", sdp: "x" })))
      .startsWith("error|signal_error|"),
    "a signal that is neither offer nor answer was accepted"
  );
  check(
    guest.accept_offer(encodeURIComponent("{ broken")).startsWith("error|signal_error|"),
    "a malformed remote offer was accepted"
  );
  check(
    guest.accept_offer(encodeURIComponent("x".repeat(70000))).startsWith("error|signal_error|"),
    "an oversized remote signal was accepted"
  );
  host.shutdown();
  guest.shutdown();
  await delay(10);
}

async function main() {
  await testFullStar();
  await testChannelConfiguration();
  await testDuplicateChannelRefused();
  await testPermissions();
  await testBoundsAndOverflow();
  await testBackpressureLatchRearms();
  await testBufferedAmountLowDrain();
  await testChannelOpenOrdering();
  await testIceGatheringTransition();
  await testIceGatheringTimeoutFallback();
  await testIceAndConnectionFailure();
  await testDisconnectAndTeardown();
  await testSignalingFaults();

  // Any exception thrown by a bridge callback -- including a late callback
  // from a connection the bridge has already torn down -- fails the run.
  if (callbackErrors.length > 0) {
    throw new Error(
      "bridge callbacks threw " +
        callbackErrors.length +
        " error(s): " +
        callbackErrors.slice(0, 5).join("; ")
    );
  }
  console.log("WebRTC star transport smoke: OK");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
