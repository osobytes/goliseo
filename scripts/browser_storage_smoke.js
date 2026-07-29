#!/usr/bin/env node
"use strict";

const fs = require("fs");
const vm = require("vm");

const source = fs.readFileSync(require.resolve("./browser_storage_host.js"), "utf8");
const window = {};
vm.runInNewContext(source, { window, Boolean, String });

const scheduled = [];
const events = [];
const savedSettings = [
  "version=1",
  "master_volume=0.42",
  "sfx_volume=0.64",
  "crowd_volume=0.31",
  "muted=true",
  "fullscreen=true",
  "screen_shake=false",
  "bloom=false",
  ""
].join("\n");

function drainScheduled() {
  while (scheduled.length > 0) {
    scheduled.shift()();
  }
}

function makeFilesystem(remote, syncError) {
  return {
    local: null,
    close(stream) {
      stream.fd = null;
    },
    syncfs(populate, callback) {
      if (syncError) {
        callback(new Error(syncError));
      } else if (populate) {
        this.local = remote.contents;
        callback(null);
      } else {
        remote.contents = this.local;
        callback(null);
      }
    }
  };
}

function attach(filesystem, options = {}) {
  const host = window.GoliseoBrowserStorage.create({
    fs: filesystem,
    persistent_root: "/home/web_user/love",
    force_unavailable: options.forceUnavailable,
    schedule(callback) {
      scheduled.push(callback);
    },
    on_event(name, fields, level) {
      events.push({ name, fields, level });
    }
  });
  host.attach();
  return host;
}

function populate(filesystem) {
  let callbackError = "missing";
  filesystem.syncfs(true, (error) => {
    callbackError = error;
  });
  if (callbackError !== null) {
    throw new Error("populate did not complete as a recoverable operation");
  }
}

const remote = { contents: "version=1\nmuted=false\n" };
const firstFilesystem = makeFilesystem(remote);
const firstHost = attach(firstFilesystem);
populate(firstFilesystem);
if (firstFilesystem.local !== remote.contents || firstHost.snapshot().populate_count !== 1) {
  throw new Error("populate did not restore the browser-backed settings");
}

firstFilesystem.local = savedSettings;
firstFilesystem.close({
  fd: 5,
  flags: 1,
  path: "/home/web_user/love/goliseo/settings.txt"
});
drainScheduled();
if (remote.contents !== savedSettings || firstHost.snapshot().flush_count !== 1) {
  throw new Error("a closed settings write was not flushed to browser storage");
}

const reloadedFilesystem = makeFilesystem(remote);
const reloadedHost = attach(reloadedFilesystem);
populate(reloadedFilesystem);
if (
  reloadedFilesystem.local !== savedSettings ||
  reloadedHost.snapshot().populate_count !== 1
) {
  throw new Error("settings did not round-trip through a clean host reload");
}

const unavailableFilesystem = makeFilesystem({ contents: null }, "IndexedDB denied");
const unavailableHost = attach(unavailableFilesystem);
populate(unavailableFilesystem);
if (
  unavailableHost.snapshot().state !== "unavailable" ||
  !unavailableHost.snapshot().last_error.recoverable
) {
  throw new Error("storage failure was not exposed as recoverable");
}
unavailableFilesystem.local = savedSettings;
unavailableFilesystem.close({
  fd: 6,
  flags: 1,
  path: "/home/web_user/love/goliseo/settings.txt"
});
drainScheduled();
if (unavailableHost.snapshot().skipped_flush_count !== 1) {
  throw new Error("unavailable storage did not retain its in-memory fallback");
}
if (!events.some((event) => event.name === "storage_error" && event.level === "warn")) {
  throw new Error("recoverable storage error telemetry is missing");
}

console.log("browser storage smoke: OK");
