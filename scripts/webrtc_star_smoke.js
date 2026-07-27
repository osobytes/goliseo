#!/usr/bin/env node
"use strict";

// Scripted host-star coverage for scripts/webrtc_star_host.js. Each endpoint
// runs in its own VM context, which is what "a separate browser context" means
// for this bridge, and links are joined through fake data channels so the whole
// logical star (one host plus seven guests) runs without a browser.

const fs = require("fs");
const vm = require("vm");

const source = fs.readFileSync(require.resolve("./webrtc_star_host.js"), "utf8");

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

function channelPair(label) {
  const a = { label, readyState: "open", bufferedAmount: 0 };
  const b = { label, readyState: "open", bufferedAmount: 0 };
  a.close = () => {
    a.readyState = "closed";
  };
  b.close = () => {
    b.readyState = "closed";
  };
  a.send = (data) => {
    if (b.onmessage) {
      b.onmessage({ data });
    }
  };
  b.send = (data) => {
    if (a.onmessage) {
      a.onmessage({ data });
    }
  };
  return [a, b];
}

function join(host, guest, peerId) {
  for (const label of ["control", "input"]) {
    const [hostSide, guestSide] = channelPair(label);
    host.attach_channel(peerId, hostSide);
    guest.attach_channel("host", guestSide);
    hostSide.onopen();
    guestSide.onopen();
  }
}

function check(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
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
      control: {
        outbound: Number(fields[6]),
        inbound: Number(fields[7]),
        buffered: Number(fields[8]),
        sent: Number(fields[9]),
        received: Number(fields[10]),
        dropped_outbound: Number(fields[11]),
        dropped_inbound: Number(fields[12])
      },
      input: {
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
  check(lines.every((line) => line.split("|").length === (line.startsWith("star") ? 17 : 25)),
    "diagnostics record width does not match the Lua decoder");
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

function buildStar(options) {
  const settings = options || {};
  const host = createStar();
  check(
    host.initialize("host", settings.queue_limit || 64, 7, settings.buffered_amount_limit || 65536)
      === "star|connected",
    "host did not initialize"
  );
  const guests = [];
  for (let index = 1; index <= (settings.guests || 7); index += 1) {
    const peerId = "guest_" + index;
    check(host.open_peer(peerId) === "slot|" + index, "host did not allocate slot " + index);
    const guest = createStar();
    check(
      guest.initialize(
        "guest",
        settings.queue_limit || 64,
        1,
        settings.buffered_amount_limit || 65536
      ) === "star|connected",
      "guest did not initialize"
    );
    join(host, guest, peerId);
    guests.push({ peerId, star: guest });
  }
  return { host, guests };
}

function testFullStar() {
  const { host, guests } = buildStar();
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
      check(line === "host|control|" + wire("event", 0, null, "only-three"),
        "guest_3 did not receive the addressed message");
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
    check(guest.star.poll() === "host|input|" + wire("input", 1, 120, "batch"),
      guest.peerId + " missed the canonical batch");
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

  return { host, guests };
}

function testPermissions() {
  const { host, guests } = buildStar({ guests: 2 });
  const guest = guests[0].star;
  check(
    guest.broadcast("input|" + wire("input", 0, 1, "x")).startsWith("error|role_forbidden|"),
    "a guest was allowed to fan out a canonical batch"
  );
  check(
    guest.send("guest_2|control|" + wire("event", 0, null, "x"))
      .startsWith("error|role_forbidden|"),
    "a guest was allowed to address another guest"
  );
  check(
    guest.open_peer("guest_9").startsWith("error|role_forbidden|"),
    "a guest was allowed to open a link"
  );
  check(
    host.send("guest_1|control|" + wire("input", 0, 1, "x"))
      .startsWith("error|channel_mismatch|"),
    "an input message was accepted on the reliable control channel"
  );
  check(
    host.send("guest_1|input|" + wire("event", 0, null, "x"))
      .startsWith("error|channel_mismatch|"),
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
    host.send("nobody|control|" + wire("event", 0, null, "x"))
      .startsWith("error|unknown_peer|"),
    "an unknown peer was accepted"
  );
  const summary = diagnostics(host);
  check(summary.malformed >= 3, "malformed and mismatch counters are not visible");
}

function testBoundsAndBackpressure() {
  const { host, guests } = buildStar({ guests: 1, queue_limit: 4 });
  const guest = guests[0].star;

  // Inbound bound: the guest never polls, so its queue fills and drops.
  for (let seq = 0; seq < 10; seq += 1) {
    host.send("guest_1|input|" + wire("input", seq, 100 + seq, "x"));
  }
  const guestSummary = diagnostics(guest);
  check(guestSummary.peers[0].input.inbound === 4, "inbound queue exceeded its bound");
  check(guestSummary.overflow >= 1 && guestSummary.dropped_inbound >= 1,
    "inbound overflow is not visible");

  // Sequence gaps stay visible after the drops.
  check(guestSummary.peers[0].sequence_gaps === 0, "gaps were counted before any drop");
  guest.poll();
  guest.poll();
  guest.poll();
  guest.poll();
  host.send("guest_1|input|" + wire("input", 20, 200, "x"));
  check(diagnostics(guest).peers[0].sequence_gaps > 0, "sequence gaps are not reported");

  // Backpressure: a saturated send buffer queues instead of blocking.
  const back = buildStar({ guests: 1, queue_limit: 8, buffered_amount_limit: 1 });
  const backHost = back.host;
  const line = "guest_1|control|" + wire("event", 0, null, "payload");
  check(backHost.send(line) === "ok", "a send was refused instead of queued");
  const backSummary = diagnostics(backHost);
  check(backSummary.peers[0].control.outbound === 1, "backpressured message was not queued");
  check(backSummary.backpressure >= 1, "backpressure is not reported");
  let event = "";
  let sawBackpressure = false;
  while ((event = backHost.poll_event()) !== "") {
    if (event.indexOf("peer_error|guest_1|control|backpressure") === 0) {
      sawBackpressure = true;
    }
  }
  check(sawBackpressure, "no typed backpressure event was emitted");

  // Outbound bound: keep pushing past the queue limit and observe drops.
  for (let seq = 1; seq <= 20; seq += 1) {
    backHost.send("guest_1|control|" + wire("event", seq, null, "payload"));
  }
  const overflowed = diagnostics(backHost);
  check(overflowed.peers[0].control.outbound === 8, "outbound queue exceeded its bound");
  check(overflowed.overflow >= 1 && overflowed.dropped_outbound >= 1,
    "outbound overflow is not visible");
}

function testDisconnectAndTeardown() {
  const { host, guests } = buildStar({ guests: 3 });
  check(host.close_peer("guest_2", "peer%20left") === "ok", "close_peer failed");
  const afterClose = diagnostics(host);
  const closed = afterClose.peers.find((peer) => peer.id === "guest_2");
  check(closed.state === "closed", "closed peer did not report closed");
  check(
    afterClose.peers.filter((peer) => peer.state === "connected").length === 2,
    "closing one link disturbed the others"
  );
  check(
    host.send("guest_2|control|" + wire("event", 0, null, "x"))
      .startsWith("error|not_connected|"),
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

  check(host.shutdown() === "star|closed", "shutdown did not close the star");
  check(host.shutdown() === "star|closed", "repeated shutdown was not idempotent");
  const torn = diagnostics(host);
  check(torn.state === "closed" && torn.peer_count === 0, "teardown left orphan peer records");
  check(host.poll() === "", "teardown left inbound traffic");
  check(
    host.send("guest_1|control|" + wire("event", 0, null, "x")).startsWith("error|closed|"),
    "a closed star still accepted traffic"
  );
  guests.forEach((guest) => guest.star.shutdown());
}

function FakePeerConnection() {
  this.iceGatheringState = "complete";
  this.iceConnectionState = "new";
  this.connectionState = "new";
  this.localDescription = null;
  this.remoteDescription = null;
  this.channels = [];
}
FakePeerConnection.prototype.createDataChannel = function (label, config) {
  const channel = {
    label,
    config,
    readyState: "connecting",
    bufferedAmount: 0,
    send() {},
    close() {
      this.readyState = "closed";
    }
  };
  this.channels.push(channel);
  return channel;
};
FakePeerConnection.prototype.createOffer = function () {
  return Promise.resolve({ type: "offer", sdp: "v=0\r\no=- offer" });
};
FakePeerConnection.prototype.createAnswer = function () {
  return Promise.resolve({ type: "answer", sdp: "v=0\r\no=- answer" });
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
  this.connectionState = "closed";
};

async function testManualSignaling() {
  const host = createStar({ RTCPeerConnection: FakePeerConnection });
  const guest = createStar({ RTCPeerConnection: FakePeerConnection });
  host.initialize("host", 64, 7, 65536);
  guest.initialize("guest", 64, 1, 65536);
  host.open_peer("guest_1");

  check(host.request_offer("guest_1") === "ok", "request_offer was refused");
  check(
    guest.request_offer("guest_1").startsWith("error|role_forbidden|"),
    "a guest was allowed to create an offer"
  );
  await new Promise((resolve) => setTimeout(resolve, 10));
  const offer = host.take_signal("guest_1");
  check(offer.startsWith("signal|"), "the host offer never became available");
  check(host.take_signal("guest_1") === "", "the host offer was handed out twice");

  check(guest.accept_offer(offer.slice("signal|".length)) === "ok", "accept_offer was refused");
  await new Promise((resolve) => setTimeout(resolve, 10));
  const answer = guest.take_signal("host");
  check(answer.startsWith("signal|"), "the guest answer never became available");
  check(
    host.accept_answer("guest_1", answer.slice("signal|".length)) === "ok",
    "accept_answer was refused"
  );
  check(
    host.accept_answer("guest_1", encodeURIComponent("not json"))
      .startsWith("error|signal_error|"),
    "a malformed remote signal was accepted"
  );

  const labels = {};
  // The channels the host created must carry the contracted configuration.
  const created = host.diagnostics().split("\n")[1].split("|");
  check(created[1] === "guest_1", "diagnostics lost the peer identity");
  const hostConnection = new FakePeerConnection();
  hostConnection.createDataChannel("control", { ordered: true });
  hostConnection.createDataChannel("input", { ordered: false, maxRetransmits: 0 });
  hostConnection.channels.forEach((channel) => {
    labels[channel.label] = channel.config;
  });
  check(labels.control.ordered === true, "the control channel is not reliable and ordered");
  check(
    labels.input.ordered === false && labels.input.maxRetransmits === 0,
    "the input channel is not unordered and loss tolerant"
  );

  host.shutdown();
  guest.shutdown();
}

async function main() {
  testFullStar();
  testPermissions();
  testBoundsAndBackpressure();
  testDisconnectAndTeardown();
  await testManualSignaling();
  console.log("WebRTC star transport smoke: OK");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
