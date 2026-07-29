#!/usr/bin/env node
"use strict";

const fs = require("fs");
const vm = require("vm");

const playerPath = process.argv[2];
if (!playerPath) {
  throw new Error("usage: transport_bridge_smoke.js <generated-player.js>");
}

const source = fs.readFileSync(playerPath, "utf8");
const proofMarker = source.indexOf("/* OMP-0 manual WebRTC proof host.");
if (proofMarker < 0) {
  throw new Error("generated player is missing the WebRTC proof marker");
}

const pending = [];
const window = {
  TextEncoder,
  queueMicrotask(callback) {
    pending.push(callback);
  },
  setTimeout(callback) {
    pending.push(callback);
  }
};
vm.runInNewContext(source.slice(0, proofMarker) + "\n})();", {
  window,
  TextEncoder,
  Number,
  String,
  encodeURIComponent,
  decodeURIComponent,
  unescape
});

const bridge = window.GoliseoTransportBridge;
const wire = "1|input|1|1|move";

if (bridge.initialize(1) !== "state|connected") {
  throw new Error("bridge did not initialize");
}
if (bridge.poll_event() !== "state|connected") {
  throw new Error("bridge did not expose the connected state");
}
if (bridge.enqueue(wire) !== "ok") {
  throw new Error("bridge did not enqueue the test message");
}
bridge.disconnect();

let diagnostics = bridge.diagnostics().split("|");
if (diagnostics[0] !== "disconnected" || diagnostics[2] !== "0" || diagnostics[3] !== "0") {
  throw new Error("disconnect did not clear browser bridge queues");
}
if (diagnostics[5] !== "1") {
  throw new Error("disconnect did not count the queued outbound message");
}
if (bridge.poll_event() !== "state|disconnected") {
  throw new Error("disconnect state was not retained at queue limit one");
}
if (bridge.poll_event() !== "error|disconnected") {
  throw new Error("disconnect error was not retained at queue limit one");
}

if (bridge.initialize(1) !== "state|connected") {
  throw new Error("bridge did not reconnect");
}
if (bridge.poll_event() !== "state|connected") {
  throw new Error("bridge did not expose the reconnected state");
}
while (pending.length > 0) {
  pending.shift()();
}
if (bridge.poll() !== "") {
  throw new Error("stale traffic survived browser bridge reconnect");
}

bridge.enqueue("bad");
bridge.enqueue("bad");
bridge.enqueue("bad");
diagnostics = bridge.diagnostics().split("|");
if (Number(diagnostics[9]) < 1 || diagnostics[4] !== "2") {
  throw new Error("browser bridge event overflow diagnostics are incomplete");
}

console.log("transport bridge smoke: OK");
