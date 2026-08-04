#!/usr/bin/env python3
"""Build a deterministic browser artifact for the GOLISEO LÖVE project."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath


ROOT = Path(__file__).resolve().parents[1]
GAME_PACKAGE_NAME = "goliseo.love"
RUNTIME_REPOSITORY = "https://github.com/2dengine/love.js"
RUNTIME_COMMIT = "495c5eb7eb55b54aaadfc21405c58f50a6d819c4"
RUNTIME_ARCHIVE_SHA256 = "89b56e7953935d6cb06c454d0ee0c0d8903e433b9a94d1d6d501fb8b516f5ff6"
RUNTIME_ARCHIVE_URL = f"{RUNTIME_REPOSITORY}/archive/{RUNTIME_COMMIT}.tar.gz"

PACKAGE_ROOT_FILES = ("conf.lua", "main.lua")
PACKAGE_ROOT_DIRECTORIES = ("core", "data", "game", "render", "sim")
RUNTIME_FILES = {
    ".htaccess": ".htaccess",
    "11.5/love.js": "11.5/love.js",
    "11.5/love.wasm": "11.5/love.wasm",
    "lua/normalize1.lua": "lua/normalize1.lua",
    "lua/normalize2.lua": "lua/normalize2.lua",
    "player.js": "third_party/lovejs-player.js",
    "style.css": "style.css",
}

BROWSER_STYLE_OVERRIDE = r'''
/* GOLISEO preserves its 960x540 canvas aspect ratio in windowed mode. */
html,
body {
  width: 100%;
  height: 100%;
}

#canvas:not(:fullscreen) {
  width: min(100vw, 177.7777777778vh) !important;
  height: min(100vh, 56.25vw) !important;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  object-fit: contain !important;
}
'''

BROWSER_LOADER = r'''/* GOLISEO browser bootstrap. */
(function () {
  "use strict";

  /*__GOLISEO_BROWSER_STORAGE__*/

  /*
   * OMP-0 transport host. Lua calls this bounded queue API through the
   * pinned runtime's love.js.eval hook. Delivery is scheduled as a microtask
   * (or a timer fallback), so no browser/network operation runs in the LÖVE
   * update call. The WebRTC proof host is embedded separately below and uses
   * the same envelope shape.
   */
  window.GoliseoTransportBridge = window.GoliseoTransportBridge || (function () {
    var VERSION = 1;
    var DEFAULT_QUEUE_LIMIT = 64;
    var MAX_QUEUE_LIMIT = 256;
    var MAX_PAYLOAD_BYTES = 1024;
    var MESSAGE_TYPES = { input: true, event: true, state: true };
    var state = "new";
    var queueLimit = DEFAULT_QUEUE_LIMIT;
    var eventLimit = Math.max(2, queueLimit);
    var outbound = [];
    var inbound = [];
    var events = [];
    var deliveryScheduled = false;
    var droppedOutbound = 0;
    var droppedInbound = 0;
    var malformed = 0;
    var unsupportedVersion = 0;
    var overflow = 0;
    var sent = 0;
    var received = 0;
    var lastError = "";

    function byteLength(value) {
      if (window.TextEncoder) {
        return new window.TextEncoder().encode(value).length;
      }
      return unescape(encodeURIComponent(value)).length;
    }

    function encodeField(value) {
      return encodeURIComponent(String(value));
    }

    function decodeField(value) {
      try {
        return decodeURIComponent(value);
      } catch (error) {
        return null;
      }
    }

    function recordState(nextState) {
      state = nextState;
      pushEvent("state|" + nextState);
    }

    function pushEvent(event) {
      if (events.length >= eventLimit) {
        events.shift();
        overflow += 1;
        lastError = "event_queue_full";
      }
      events.push(event);
    }

    function recordError(code, detail) {
      if (code === "malformed" || code === "payload_too_large") {
        malformed += 1;
      } else if (code === "unsupported_version") {
        unsupportedVersion += 1;
      } else if (code === "overflow") {
        overflow += 1;
      }
      lastError = code;
      pushEvent("error|" + encodeField(code));
      return "error|" + code + "|" + (detail || code);
    }

    function invalid(code, detail) {
      return { error: code, detail: detail || code };
    }

    function parseWire(wire) {
      if (typeof wire !== "string") {
        return invalid("malformed", "wire is not a string");
      }
      var fields = wire.split("|");
      if (fields.length !== 5) {
        return invalid("malformed", "wire has invalid fields");
      }
      if (!/^(0|[1-9][0-9]*)$/.test(fields[0])) {
        return invalid("malformed", "wire version is invalid");
      }
      var version = Number(fields[0]);
      if (version !== VERSION) {
        return invalid("unsupported_version", "wire version is unsupported");
      }
      var type = decodeField(fields[1]);
      if (!type || !MESSAGE_TYPES[type]) {
        return invalid("malformed", "wire type is invalid");
      }
      if (!/^(0|[1-9][0-9]*)$/.test(fields[2])) {
        return invalid("malformed", "wire sequence is invalid");
      }
      var tick = null;
      if (fields[3] !== "") {
        if (!/^(0|[1-9][0-9]*)$/.test(fields[3])) {
          return invalid("malformed", "wire tick is invalid");
        }
        tick = Number(fields[3]);
      }
      if (type === "input" && tick === null) {
        return invalid("malformed", "input wire is missing its tick");
      }
      var payload = decodeField(fields[4]);
      if (payload === null) {
        return invalid("malformed", "wire payload escape is invalid");
      }
      if (byteLength(payload) > MAX_PAYLOAD_BYTES) {
        return invalid("payload_too_large", "wire payload exceeds the byte limit");
      }
      return { version: version, type: type, seq: Number(fields[2]), tick: tick, payload: payload };
    }

    function deliverOne() {
      deliveryScheduled = false;
      if (state !== "connected" || outbound.length === 0 || inbound.length >= queueLimit) {
        return;
      }
      inbound.push(outbound.shift());
      if (outbound.length > 0) {
        scheduleDelivery();
      }
    }

    function scheduleDelivery() {
      if (deliveryScheduled || state !== "connected" || outbound.length === 0) {
        return;
      }
      deliveryScheduled = true;
      if (window.queueMicrotask) {
        window.queueMicrotask(deliverOne);
      } else {
        window.setTimeout(deliverOne, 0);
      }
    }

    return {
      initialize: function (requestedLimit) {
        var limit = requestedLimit === undefined ? DEFAULT_QUEUE_LIMIT : Number(requestedLimit);
        if (!/^[0-9]+$/.test(String(limit)) || limit < 1 || limit > MAX_QUEUE_LIMIT) {
          return recordError("malformed", "queue limit is invalid");
        }
        queueLimit = limit;
        eventLimit = Math.max(2, queueLimit);
        if (state === "connected") {
          return "state|connected";
        }
        outbound = [];
        inbound = [];
        recordState("connected");
        return "state|connected";
      },

      shutdown: function () {
        droppedOutbound += outbound.length;
        droppedInbound += inbound.length;
        outbound = [];
        inbound = [];
        recordState("closed");
        return "state|closed";
      },

      enqueue: function (wire) {
        if (state !== "connected") {
          return recordError("not_connected", "transport is not connected");
        }
        var parsed = parseWire(wire);
        if (parsed.error) {
          return recordError(parsed.error, parsed.detail);
        }
        if (outbound.length >= queueLimit) {
          droppedOutbound += 1;
          return recordError("overflow", "outbound queue is full");
        }
        outbound.push(wire);
        sent += 1;
        scheduleDelivery();
        return "ok";
      },

      poll: function () {
        if (state !== "connected" || inbound.length === 0) {
          return "";
        }
        var wire = inbound.shift();
        received += 1;
        scheduleDelivery();
        return wire;
      },

      poll_event: function () {
        return events.length === 0 ? "" : events.shift();
      },

      disconnect: function () {
        droppedOutbound += outbound.length;
        droppedInbound += inbound.length;
        outbound = [];
        inbound = [];
        recordState("disconnected");
        recordError("disconnected", "transport disconnected");
        return "state|disconnected";
      },

      diagnostics: function () {
        return [
          state,
          queueLimit,
          outbound.length,
          inbound.length,
          events.length,
          droppedOutbound,
          droppedInbound,
          malformed,
          unsupportedVersion,
          overflow,
          sent,
          received,
          lastError
        ].join("|");
      }
    };
  })();

  /*__GOLISEO_WEBRTC_PROOF__*/

  /*__GOLISEO_STAR_TRANSPORT__*/

  var script = document.currentScript;
  var canvas = document.getElementById("canvas");
  var spinner = document.getElementById("spinner");
  var page_query = new URL(window.location.href).searchParams;
  var script_query = new URL(script.src).searchParams;
  var version = "11.5";
  var uri = script_query.get("g") || "goliseo.love";
  var args = [];
  var browser_compat = window.__GOLISEO__ = {
    artifact: "goliseo-web",
    build_id: "__GOLISEO_BUILD_ID__",
    console_entries: [],
    events: [],
    storage: { state: "pending" },
    status: "loading",
    started_at_ms: performance.now()
  };

  ["debug", "info", "log", "warn", "error"].forEach(function (level) {
    var original = console[level].bind(console);
    console[level] = function () {
      var values = Array.prototype.slice.call(arguments);
      browser_compat.console_entries.push({
        at_ms: performance.now() - browser_compat.started_at_ms,
        level: level,
        message: values.map(function (value) {
          if (typeof value === "string") {
            return value;
          }
          try {
            return JSON.stringify(value);
          } catch (_error) {
            return String(value);
          }
        }).join(" ")
      });
      original.apply(console, values);
    };
  });

  function mark(name, detail, level) {
    var event = { name: name, at_ms: performance.now() - browser_compat.started_at_ms };
    if (detail) {
      event.detail = detail;
    }
    browser_compat.events.push(event);
    console[level || "info"]("GC_BROWSER|" + name + "|at_ms=" + event.at_ms.toFixed(3) +
      (detail ? "|detail=" + detail : ""));
  }

  function storage_detail(fields) {
    return Object.keys(fields).sort().map(function (key) {
      return key + "=" + encodeURIComponent(String(fields[key]));
    }).join("|");
  }

  mark("loader_start");

  if (page_query.get("arg")) {
    try {
      args = JSON.parse(page_query.get("arg"));
      if (!Array.isArray(args)) {
        args = [String(args)];
      }
    } catch (error) {
      console.warn(error);
      args = [];
    }
  }

  canvas.oncontextmenu = function (event) {
    event.preventDefault();
  };

  function fail(error) {
    browser_compat.status = "failed";
    mark("error", String(error));
    console.error(error);
    canvas.style.display = "none";
    spinner.className = "error";
  }

  function fetch_binary(path) {
    return fetch(path, { credentials: "same-origin" }).then(function (response) {
      if (!response.ok) {
        throw new Error("Could not fetch " + path + " (HTTP " + response.status + ")");
      }
      return response.arrayBuffer().then(function (buffer) {
        return new Uint8Array(buffer);
      });
    });
  }

  function start() {
    spinner.className = "loading";
    var paths = [uri, "lua/normalize1.lua", "lua/normalize2.lua", version + "/love.wasm"];

    Promise.all(paths.map(fetch_binary))
      .then(function (files) {
        mark("assets_loaded", "count=" + files.length);
        var cache = {};
        for (var i = 0; i < paths.length; i++) {
          cache[paths[i]] = files[i];
        }
        if (cache[uri][0] !== 80 || cache[uri][1] !== 75) {
          throw new Error("The fetched resource is not a valid love package");
        }

        var Module = window.Module || {};
        window.Module = Module;
        Module.INITIAL_MEMORY = Math.min(
          4 * cache[uri].length + 2e7,
          (navigator.deviceMemory || 1) * 1e9
        );
        Module.canvas = canvas;
        Module.warn = window.onerror;
        Module.args = [uri.substring(uri.lastIndexOf("/") + 1)].concat(args);
        Module.cache = cache;
        Module.prerun = function () {
          var storage_host = window.GoliseoBrowserStorage.create({
            fs: Module.FS,
            persistent_root: Module.env.HOME + "/love",
            force_unavailable: page_query.get("storage") === "unavailable",
            on_event: function (name, fields, level) {
              mark(name, storage_detail(fields), level);
            },
            on_state: function (state) {
              browser_compat.storage = state;
            },
            schedule: window.setTimeout.bind(window)
          });
          storage_host.attach();
          Module.FS.mkdirTree("/usr/local/share/lua/5.1");
          for (var path in cache) {
            var filename = path.split("/").pop();
            var directory = path === uri ? "/" : "/usr/local/share/lua/5.1";
            var target = path === uri ? Module.args[0] : filename;
            Module.FS.createDataFile(directory, target, cache[path], true, true, true);
          }
        };
        Module.postrun = function () {
          browser_compat.status = "running";
          mark("runtime_postrun");
          canvas.style.display = "block";
          canvas.focus();
          spinner.className = "";
          requestAnimationFrame(function () {
            mark("first_frame");
          });
        };

        var runtime = document.createElement("script");
        runtime.src = version + "/love.js";
        runtime.async = true;
        runtime.onload = function () {
          mark("runtime_script_loaded");
          try {
            window.Love(Module);
          } catch (error) {
            fail(error);
          }
        };
        runtime.onerror = function () {
          fail(new Error("Could not load the LÖVE " + version + " runtime"));
        };
        document.body.appendChild(runtime);
      })
      .catch(fail);
  }

  window.onerror = function (message) {
    mark("window_error", String(message));
    fail(message);
  };
  window.onunhandledrejection = function (event) {
    mark("unhandled_rejection", String(event.reason || "Unhandled browser promise rejection"));
    fail(event.reason || "Unhandled browser promise rejection");
  };
  window.onload = window.focus.bind(window);
  start();
})();
'''

INDEX_HTML = """<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0, user-scalable=no">
    <title>GOLISEO</title>
    <link rel="icon" href="data:,">
    <link rel="stylesheet" href="style.css">
  </head>
  <body>
    <canvas id="canvas" aria-label="GOLISEO game"></canvas>
    <div id="spinner" class="pending" aria-live="polite"></div>
    <script src="player.js?g=goliseo.love&amp;v=11.5"></script>
  </body>
</html>
"""

WEBRTC_PROOF_HTML = """<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>GOLISEO WebRTC proof</title>
    <style>
      :root { color-scheme: dark; font-family: system-ui, sans-serif; }
      body { background: #070b18; color: #f4f7ff; margin: 0 auto; max-width: 900px; padding: 24px; }
      h1 { margin-top: 0; }
      .summary { display: flex; flex-wrap: wrap; gap: 16px; margin-bottom: 16px; }
      .summary span { background: #151d36; border-radius: 6px; padding: 8px 12px; }
      label { display: block; font-weight: 700; margin-top: 16px; }
      textarea { box-sizing: border-box; min-height: 120px; width: 100%; }
      button { margin: 12px 8px 0 0; padding: 9px 14px; }
      pre { background: #11182c; border-radius: 6px; overflow: auto; padding: 16px; }
      #status { color: #86e1ff; font-weight: 700; }
    </style>
  </head>
  <body>
    <h1>GOLISEO WebRTC proof</h1>
    <div class="summary">
      <span>Role: <strong id="role-value"></strong></span>
      <span>Profile: <strong id="profile-value"></strong></span>
      <span>Duration: <strong id="duration-value"></strong> ms</span>
      <span>Status: <strong id="status" data-state="loading">loading</strong></span>
    </div>
    <p>
      Open one host tab and one guest tab with the same profile. Copy the offer
      and answer between the signal fields, then start traffic in both tabs.
    </p>
    <label for="signal-input">Remote signal</label>
    <textarea id="signal-input" data-testid="signal-input"></textarea>
    <button id="create-offer" data-testid="create-offer">Create offer</button>
    <button id="accept-offer" data-testid="accept-offer">Accept offer</button>
    <button id="accept-answer" data-testid="accept-answer">Accept answer</button>
    <label for="signal-output">Local signal</label>
    <textarea id="signal-output" data-testid="signal-output" readonly></textarea>
    <div>
      <button id="start-traffic" data-testid="start-traffic" disabled>Start 60 Hz traffic</button>
      <button id="stop-traffic" data-testid="stop-traffic" disabled>Stop traffic</button>
    </div>
    <h2>Diagnostics</h2>
    <pre id="diagnostics" data-testid="diagnostics"></pre>
    <script src="webrtc-proof.js"></script>
  </body>
</html>
"""

WEBRTC_PROOF_SUITE_HTML = """<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>GOLISEO WebRTC proof suite</title>
    <style>
      :root { color-scheme: dark; font-family: system-ui, sans-serif; }
      body { background: #070b18; color: #f4f7ff; margin: 0 auto; max-width: 1200px; padding: 24px; }
      h1 { margin-top: 0; }
      button { margin: 8px 0 16px; padding: 10px 16px; }
      .pair { display: grid; gap: 12px; grid-template-columns: 1fr 1fr; margin-bottom: 20px; }
      iframe { background: #070b18; border: 1px solid #445078; height: 460px; width: 100%; }
      .mismatch iframe { height: 260px; }
      pre { background: #11182c; border-radius: 6px; max-height: 620px; overflow: auto; padding: 16px; }
      #suite-status { color: #86e1ff; font-weight: 700; }
    </style>
  </head>
  <body>
    <h1>GOLISEO WebRTC proof suite</h1>
    <p>
      Runs baseline and 100 ms RTT/1% input-loss profiles concurrently in four
      separate browsing contexts, plus a build-mismatch handshake.
    </p>
    <p>Status: <span id="suite-status" data-state="loading">loading</span></p>
    <button id="run-suite" data-testid="run-suite">Connect and run all profiles</button>
    <h2>Baseline profile</h2>
    <div class="pair">
      <iframe id="baseline-host" title="Baseline host"></iframe>
      <iframe id="baseline-guest" title="Baseline guest"></iframe>
    </div>
    <h2>Shaped profile</h2>
    <div class="pair">
      <iframe id="shaped-host" title="Shaped host"></iframe>
      <iframe id="shaped-guest" title="Shaped guest"></iframe>
    </div>
    <h2>Mismatch proof</h2>
    <div class="pair mismatch">
      <iframe id="mismatch-host" title="Mismatch host"></iframe>
      <iframe id="mismatch-guest" title="Mismatch guest"></iframe>
    </div>
    <h2>Suite diagnostics</h2>
    <pre id="suite-diagnostics" data-testid="suite-diagnostics"></pre>
    <script src="webrtc-proof-suite.js"></script>
  </body>
</html>
"""


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_revision() -> str:
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else "unknown"


def source_dirty() -> bool:
    result = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    return result.returncode != 0 or bool(result.stdout.strip())


def package_paths() -> list[tuple[Path, str]]:
    allowed = [*PACKAGE_ROOT_FILES, *PACKAGE_ROOT_DIRECTORIES]
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--", *allowed],
        cwd=ROOT,
        check=False,
        capture_output=True,
    )
    if result.returncode != 0:
        raise RuntimeError("cannot enumerate tracked browser-package files")
    names = [name.decode("utf-8") for name in result.stdout.split(b"\0") if name]
    for required in PACKAGE_ROOT_FILES:
        if required not in names:
            raise RuntimeError(f"missing tracked package entrypoint: {required}")

    files = []
    for name in names:
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise RuntimeError(f"unsafe or missing tracked package file: {name}")
        files.append((path, name))
    return sorted(files, key=lambda item: item[1])


def write_game_package(destination: Path) -> str:
    with zipfile.ZipFile(
        destination,
        mode="w",
        compression=zipfile.ZIP_DEFLATED,
        compresslevel=9,
    ) as archive:
        for source, name in package_paths():
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            archive.writestr(info, source.read_bytes())

    return sha256(destination)


def download_runtime(destination: Path) -> Path:
    archive_path = destination / "love.js.tar.gz"
    request = urllib.request.Request(
        RUNTIME_ARCHIVE_URL,
        headers={"User-Agent": "goliseo-web-build/1"},
    )
    with urllib.request.urlopen(request) as response, archive_path.open("wb") as stream:
        shutil.copyfileobj(response, stream)

    actual_hash = sha256(archive_path)
    if actual_hash != RUNTIME_ARCHIVE_SHA256:
        raise RuntimeError(
            "runtime archive checksum mismatch: "
            f"expected {RUNTIME_ARCHIVE_SHA256}, got {actual_hash}"
        )

    extracted = destination / "runtime"
    extracted.mkdir()
    with tarfile.open(archive_path, mode="r:gz") as archive:
        members = archive.getmembers()
        for member in members:
            member_path = PurePosixPath(member.name)
            if (
                member_path.is_absolute()
                or ".." in member_path.parts
                or member.issym()
                or member.islnk()
            ):
                raise RuntimeError(f"unsafe runtime archive member: {member.name}")
        archive.extractall(extracted)

    roots = [path for path in extracted.iterdir() if path.is_dir()]
    if len(roots) != 1:
        raise RuntimeError("runtime archive has an unexpected top-level layout")
    return roots[0]


def copy_runtime(runtime_root: Path, output: Path) -> None:
    for source_name, destination_name in RUNTIME_FILES.items():
        source = runtime_root / source_name
        if not source.is_file():
            raise RuntimeError(f"missing runtime file: {source_name}")
        destination = output / destination_name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)

    license_path = output / "third_party" / "lovejs.LICENSE.txt"
    license_path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(runtime_root / "license.txt", license_path)


def write_browser_style(output: Path) -> None:
    path = output / "style.css"
    upstream = path.read_text(encoding="utf-8").rstrip()
    path.write_text(
        upstream + "\n\n" + BROWSER_STYLE_OVERRIDE.strip() + "\n",
        encoding="utf-8",
    )


def write_browser_loader(output: Path) -> None:
    storage_host = (ROOT / "scripts" / "browser_storage_host.js").read_text(
        encoding="utf-8"
    )
    proof_host = (ROOT / "scripts" / "webrtc_proof_host.js").read_text(encoding="utf-8")
    star_host = (ROOT / "scripts" / "webrtc_star_host.js").read_text(encoding="utf-8")
    loader = BROWSER_LOADER.replace(
        "  /*__GOLISEO_BROWSER_STORAGE__*/",
        storage_host,
    )
    loader = loader.replace("  /*__GOLISEO_WEBRTC_PROOF__*/", proof_host)
    loader = loader.replace("  /*__GOLISEO_STAR_TRANSPORT__*/", star_host)
    loader = loader.replace("__GOLISEO_BUILD_ID__", source_revision())
    (output / "player.js").write_text(loader, encoding="utf-8")


def write_webrtc_proof_runner(output: Path) -> None:
    build_id = json.dumps(source_revision())
    proof_host = (ROOT / "scripts" / "webrtc_proof_host.js").read_text(encoding="utf-8")
    runner = (ROOT / "scripts" / "webrtc_proof_runner.js").read_text(encoding="utf-8")
    source = (
        f"window.__GOLISEO__ = {{ build_id: {build_id} }};\n"
        f"{proof_host}\n"
        f"{runner}\n"
    )
    (output / "webrtc-proof.js").write_text(source, encoding="utf-8")
    shutil.copyfile(
        ROOT / "scripts" / "webrtc_proof_suite.js",
        output / "webrtc-proof-suite.js",
    )


def write_manifest(output: Path, package_hash: str) -> None:
    files = {}
    for path in sorted(output.rglob("*")):
        if path.is_file() and path.name != "manifest.json":
            files[path.relative_to(output).as_posix()] = sha256(path)

    manifest = {
        "artifact": "goliseo-web",
        "game_package": {
            "path": GAME_PACKAGE_NAME,
            "sha256": package_hash,
        },
        "source_revision": source_revision(),
        "source_dirty": source_dirty(),
        "runtime": {
            "repository": RUNTIME_REPOSITORY,
            "commit": RUNTIME_COMMIT,
            "archive_sha256": RUNTIME_ARCHIVE_SHA256,
            "license": "mixed upstream licenses; full notices included",
        },
        "files": files,
    }
    (output / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def build(output: Path) -> None:
    output = output.resolve()
    if output == ROOT or output == output.parent:
        raise RuntimeError(f"refusing unsafe output directory: {output}")
    if output.is_symlink():
        raise RuntimeError(f"refusing symlink output directory: {output}")

    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent, prefix=f".{output.name}.") as temp:
        staging = Path(temp) / output.name
        staging.mkdir()
        package_hash = write_game_package(staging / GAME_PACKAGE_NAME)
        runtime_root = download_runtime(Path(temp))
        (staging / "index.html").write_text(INDEX_HTML, encoding="utf-8")
        (staging / "webrtc-proof.html").write_text(
            WEBRTC_PROOF_HTML,
            encoding="utf-8",
        )
        (staging / "webrtc-proof-suite.html").write_text(
            WEBRTC_PROOF_SUITE_HTML,
            encoding="utf-8",
        )
        copy_runtime(runtime_root, staging)
        write_browser_style(staging)
        write_browser_loader(staging)
        write_webrtc_proof_runner(staging)
        write_manifest(staging, package_hash)

        if output.exists():
            shutil.rmtree(output)
        staging.rename(output)

    print(f"built {output}")
    print(f"game package: {output / GAME_PACKAGE_NAME} ({package_hash})")
    print(f"runtime commit: {RUNTIME_COMMIT}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "build" / "web",
        help="artifact directory (default: build/web)",
    )
    args = parser.parse_args()
    build(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
