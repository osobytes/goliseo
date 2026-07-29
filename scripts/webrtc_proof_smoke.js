#!/usr/bin/env node
"use strict";

const fs = require("fs");
const vm = require("vm");

const source = fs.readFileSync(require.resolve("./webrtc_proof_host.js"), "utf8");
const window = {
  performance: { now: () => Date.now() },
  TextEncoder,
  console: { info() {} },
  setTimeout,
  clearTimeout,
  setInterval,
  clearInterval
};
vm.runInNewContext(source, {
  window,
  Promise,
  Number,
  Date,
  JSON,
  encodeURIComponent,
  decodeURIComponent,
  unescape,
  Math
});

const api = window.GoliseoWebRTCProof;

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function channelPair(label) {
  const first = { label, readyState: "open", bufferedAmount: 0 };
  const second = { label, readyState: "open", bufferedAmount: 0 };
  first.send = (data) => setTimeout(() => second.onmessage && second.onmessage({ data }), 0);
  second.send = (data) => setTimeout(() => first.onmessage && first.onmessage({ data }), 0);
  return [first, second];
}

function proofPair(hostOptions, guestOptions) {
  const host = api.create({ role: "host", build_id: "same", ...hostOptions });
  const guest = api.create({ role: "guest", build_id: "same", ...guestOptions });
  for (const label of ["control", "input"]) {
    const [hostChannel, guestChannel] = channelPair(label);
    host.attachChannel(hostChannel);
    guest.attachChannel(guestChannel);
    hostChannel.onopen();
    guestChannel.onopen();
  }
  return { host, guest };
}

function stop(pair) {
  pair.host.disconnect("smoke_complete");
  pair.guest.disconnect("smoke_complete");
}

async function main() {
  const baseline = proofPair({}, {});
  await delay(25);
  if (!baseline.host.handshake_complete || !baseline.guest.handshake_complete) {
    throw new Error("baseline handshake did not complete");
  }
  baseline.host.start_input({ hz: 60, duration_ms: 100 });
  baseline.guest.start_input({ hz: 60, duration_ms: 100 });
  await delay(180);
  const baselineHost = baseline.host.diagnostics();
  const baselineGuest = baseline.guest.diagnostics();
  if (
    baselineHost.sent < 5 || baselineHost.sent > 7 ||
    baselineGuest.sent < 5 || baselineGuest.sent > 7
  ) {
    throw new Error("60 Hz cadence left the expected 100 ms sample range");
  }
  if (baselineHost.received === 0 || baselineGuest.received === 0) {
    throw new Error("baseline input was not exchanged");
  }
  if (
    baselineHost.input_duration_s <= 0 ||
    baselineHost.queue_p95 === null ||
    baselineGuest.queue_p95 === null
  ) {
    throw new Error("traffic-window rate or queue percentiles are missing");
  }
  stop(baseline);

  const shapedOptions = {
    network_profile: "shaped",
    one_way_delay_ms: 5,
    input_loss_percent: 100
  };
  const shaped = proofPair(
    { ...shapedOptions, loss_seed: 17 },
    { ...shapedOptions, loss_seed: 29 }
  );
  await delay(40);
  if (!shaped.host.handshake_complete || !shaped.guest.handshake_complete) {
    throw new Error("shaped handshake did not complete");
  }
  shaped.host.start_input({ hz: 60, duration_ms: 100 });
  shaped.guest.start_input({ hz: 60, duration_ms: 100 });
  await delay(200);
  const shapedHost = shaped.host.diagnostics();
  const shapedGuest = shaped.guest.diagnostics();
  if (shapedHost.shaper_dropped === 0 || shapedGuest.shaper_dropped === 0) {
    throw new Error("configured input loss was not observed");
  }
  if (shapedHost.received !== 0 || shapedGuest.received !== 0) {
    throw new Error("100% input-loss profile delivered an input");
  }
  if (shapedHost.network_profile !== "shaped" || shapedHost.one_way_delay_ms !== 5) {
    throw new Error("shaper diagnostics are incomplete");
  }
  stop(shaped);

  const mismatch = proofPair({}, { build_id: "mismatch" });
  await delay(25);
  if (
    !mismatch.host.last_error ||
    mismatch.host.last_error.code !== "build_mismatch" ||
    !mismatch.guest.last_error ||
    mismatch.guest.last_error.code !== "build_mismatch"
  ) {
    throw new Error("build mismatch was not rejected");
  }
  stop(mismatch);

  const malformed = api.create({ role: "host", build_id: "same" });
  malformed.control = { readyState: "open", bufferedAmount: 0, send() {} };
  malformed.receiveControl(api.encode("event", 0, null, JSON.stringify({ kind: "unknown" })));
  if (!malformed.last_error || malformed.last_error.code !== "malformed") {
    throw new Error("unknown control message was not rejected");
  }

  const oversized = api.create({ role: "host", build_id: "same" });
  oversized.control = { readyState: "open", bufferedAmount: 0, send() {} };
  oversized.receiveControl("x".repeat(513));
  if (!oversized.last_error || oversized.last_error.code !== "message_too_large") {
    throw new Error("oversized control message was not rejected");
  }

  console.log("WebRTC proof smoke: OK");
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
