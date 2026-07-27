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
  return window.GalacticCupStarTransport;
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

function createRtc() {
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
    const peer = this._peer;
    if (peer) {
      fire(peer, "onmessage", { data });
    }
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
    this.iceGatheringState = "complete";
    this.iceConnectionState = "new";
    this.connectionState = "new";
    this.localDescription = null;
    this.remoteDescription = null;
    this.localChannels = [];
    this.remoteChannels = [];
    instances.push(this);
  }
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
  FakePeerConnection.prototype.connectTo = function (other) {
    this.localChannels.forEach((local) => {
      const remote = new FakeChannel(local.label, local.config);
      local._peer = remote;
      remote._peer = local;
      other.remoteChannels.push(remote);
      fire(other, "ondatachannel", { channel: remote });
      local.readyState = "open";
      remote.readyState = "open";
      fire(local, "onopen", {});
      fire(remote, "onopen", {});
    });
    this.connectionState = "connected";
    this.iceConnectionState = "connected";
    other.connectionState = "connected";
    other.iceConnectionState = "connected";
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

async function connect(host, guest, peerId, rtc) {
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
  hostPc.connectTo(guestPc);
  await delay(5);
  return { hostPc, guestPc };
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
    const links = await connect(host, guest, peerId, rtc);
    guests.push({ peerId, star: guest, hostPc: links.hostPc, guestPc: links.guestPc });
  }
  return { host, guests, rtc };
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

async function testBackpressureLatchRearms() {
  const { host, guests } = await buildStar({
    guests: 1,
    queue_limit: 16,
    buffered_amount_limit: 200
  });
  const control = guests[0].hostPc.localChannels.find((channel) => channel.label === "control");
  drainEvents(host);

  const line = "guest_1|control|" + wire("event", 0, null, "payload");

  // 1. Healthy: the message reaches the data channel immediately.
  check(host.send(line) === "ok", "a healthy send was refused");
  check(control.sent.length === 1, "a healthy send did not reach the data channel");
  check(diagnostics(host).peers[0].control.outbound === 0, "a healthy send stayed queued");

  // 2. Saturate the send buffer. The message queues and one event is emitted.
  control.bufferedAmount = 1000;
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

  // 4. Relieve the buffer. Draining empties the queue and clears the latch.
  control.bufferedAmount = 0;
  drainEvents(host);
  check(control.sent.length === 3, "the queued messages did not send after relief");
  check(diagnostics(host).peers[0].control.outbound === 0, "the outbound queue did not drain");

  // 5. Re-saturate: a genuine second congestion must be reported again.
  control.bufferedAmount = 1000;
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
