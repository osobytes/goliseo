#!/usr/bin/env python3
"""Verify the render frame payload crosses the wasm boundary in real browsers (#332).

`verify_payload.js` runs the same sequence under node and fails faster; this is
the run that counts. Node is not a browser: different loader, different memory
limits, a different JIT tier schedule, and -- the one that matters most here --
a different `performance.now()` resolution policy. The payload cost being
measured is on the order of a tenth of a millisecond, which is the same order as
the coarsening browsers apply to defeat timing attacks, so the harness times a
BATCH of frames and divides rather than pretending to time one.

Deliberately a SEPARATE file from `verify_browser.py`, which owns the frozen
determinism fixture. That file asserts hashes `bfbb106aea5480f8` /
`a190b60058a64e63` on the default entry point, and #332 must not change what it
sees. Two harnesses, two entry points of the same artifact, no shared state.

FOLLOW-UP, and it is a real duplication rather than a tidy one: the page
scaffold, the temp-directory staging and the Selenium launch below are the same
shapes `verify_browser.py` uses. #344 is extracting exactly those into
`verify_common.py`, and was unmerged when this landed -- so this file could not
build on a module that did not exist yet without blocking on it. Once #344 is
in, the staging half here should be rebased onto `verify_common.py` and only
the worker (which is genuinely different: it drives the payload exports, not
the fixture) should stay.

Headless is correct here for the same reason it is there: this measures
computation with no GPU involvement. The rule is not "never headless", it is
"never headless when the GPU is the thing under test".

    python3 wasm/sim-host/verify_payload_browser.py     # after build.sh
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import platform
import shutil
import sys
import tempfile
import threading
import time
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent

# p95 update budget from docs/online/omp0_acceptance.md. That number is
# love.update's share measured inside a real LOVE loop; here it is applied to a
# standalone wasm host driven from worker JS, so it is an ANALOGOUS 60 Hz target
# rather than the same measurement. Overridable via --budget-ms so
# --require-budget can be demonstrated red on a machine that currently passes.
DEFAULT_BUDGET_MS = 8.0
# sim.fixed_clock.MAX_TICKS_PER_UPDATE.
MAX_RESIM_TICKS = 8

PAGE = """<!doctype html>
<meta charset="utf-8">
<title>goliseo render frame payload</title>
<script>
// The simulation runs in a worker, exactly as the real integration will: the
// main thread is for rendering, and a rollback burst must never block it. The
// harness therefore measures the payload where it will actually be read.
window.__RESULT__ = null;
window.__OUT__ = [];
const worker = new Worker("payload_worker.js");
worker.onmessage = (e) => {
  if (e.data && e.data.line !== undefined) window.__OUT__.push(String(e.data.line));
  if (e.data && e.data.result !== undefined) window.__RESULT__ = e.data.result;
};
worker.onerror = (e) => {
  window.__RESULT__ = { ok: false, error: "WORKER_ERROR: " + (e && e.message) };
};
</script>
"""

# The worker is the harness. It is deliberately the same sequence as
# verify_payload.js and reports a JSON result rather than text to parse.
WORKER = """
var Module = {
  arguments: ["--payload"],
  print:    function (t) { postMessage({ line: String(t) }); },
  printErr: function (t) { postMessage({ line: "ERR: " + String(t) }); },
  onAbort:  function (w) { postMessage({ result: { ok: false, error: "ABORT: " + w } }); }
};
importScripts("frame_payload.js");
importScripts("simhost.js");

// EVERYTHING BELOW IS INSIDE AN IIFE, and that is not tidiness. The Emscripten
// glue declares its own global `function run()` -- an async one. A harness
// function of the same name at worker scope is silently replaced by it the
// moment importScripts returns, so calling `run()` yields a Promise, and the
// only symptom is `DataCloneError: #<Promise> could not be cloned` from a
// postMessage several lines later. Scope the harness and the collision cannot
// happen for `run`, `abort`, `out`, `err`, or anything else the glue owns.
(function () {
var UPDATE_BUDGET_MS = __BUDGET__;
var MAX_RESIM_TICKS = __MAX_RESIM__;
var WARMUP_TICKS = 300;
var FRAME_BATCHES = 60;
var FRAMES_PER_BATCH = 10;
var BURST_BATCHES = 40;
var BURSTS_PER_BATCH = 5;

function percentile(values, quantile) {
  var sorted = values.slice().sort(function (a, b) { return a - b; });
  var rank = Math.max(1, Math.ceil(sorted.length * quantile));
  return sorted[Math.min(rank, sorted.length) - 1];
}

function mean(values) {
  return values.reduce(function (a, b) { return a + b; }, 0) / values.length;
}

function ready() {
  return typeof Module._goliseo_payload_frame === "function";
}

// Wrap EVERY payload export, discovered dynamically, exactly as
// verify_payload.js does. Hand-wrapping only the frame call would have made
// "one crossing per frame" true by construction rather than by measurement:
// a second export called inside the render loop would not have been counted.
var counts = {};
function instrument() {
  var api = {};
  Object.keys(Module).forEach(function (name) {
    if (name.indexOf("_goliseo_payload_") !== 0) { return; }
    var original = Module[name];
    counts[name] = 0;
    api[name] = function () {
      counts[name] += 1;
      return original.apply(Module, arguments);
    };
  });
  return api;
}

function measure() {
  var payload = self.GoliseoFramePayload;
  var api = instrument();
  var surface = Object.keys(counts).sort();

  // Defence in depth with a known limit: a name regex over today's surface.
  // A future `_goliseo_payload_dump` would pass it. The real guarantee is that
  // the Rust module exposes no snapshot-returning function at all.
  var leaks = surface.filter(function (name) {
    return /snapshot|restore|capture|state|tick/.test(name);
  });
  if (leaks.length > 0) {
    return { ok: false, error: "exports could leak per-tick state: " + leaks.join(", ") };
  }

  function frameCall(ticks, rollback) {
    return api._goliseo_payload_frame(ticks, rollback);
  }

  var count = api._goliseo_payload_boot();
  if (count < 0) {
    return { ok: false, error: Module.UTF8ToString(api._goliseo_payload_error()) };
  }

  var roster = payload.readRoster(
    Module.HEAPF64,
    api._goliseo_payload_roster(),
    Module.UTF8ToString(api._goliseo_payload_roster_strings())
  );

  frameCall(WARMUP_TICKS, 0);
  var framesBefore = counts._goliseo_payload_frame;
  var othersBefore = surface.reduce(function (sum, name) {
    return name === "_goliseo_payload_frame" ? sum : sum + counts[name];
  }, 0);

  // BATCHED. performance.now() in a worker is coarsened -- 0.1 ms or worse,
  // and worse still with cross-origin isolation off -- which is the same order
  // as a single payload read. Timing ten frames and dividing puts the quantum
  // an order of magnitude below the measurement instead of on top of it.
  var frameMs = [];
  var words = 0;
  var players = 0;
  for (var b = 0; b < FRAME_BATCHES; b += 1) {
    var t0 = performance.now();
    for (var i = 0; i < FRAMES_PER_BATCH; i += 1) {
      var pointer = frameCall(1, 0);
      var frame = payload.readFrame(Module.HEAPF64, pointer);
      words = frame.totalWords;
      players = frame.playerCount;
    }
    frameMs.push((performance.now() - t0) / FRAMES_PER_BATCH);
  }
  if (players !== roster.count) {
    return { ok: false, error: "frame player count " + players + " != roster " + roster.count };
  }

  var renderedFrames = FRAME_BATCHES * FRAMES_PER_BATCH;
  var crossingsPerFrame = (counts._goliseo_payload_frame - framesBefore) / renderedFrames;
  var otherPerFrame = (surface.reduce(function (sum, name) {
    return name === "_goliseo_payload_frame" ? sum : sum + counts[name];
  }, 0) - othersBefore) / renderedFrames;

  // PAYLOAD EXTRACTION, ALONE. `ticks = 0` simulates nothing, so this call is
  // exactly build + encode + copy + read: the per-frame cost this issue is
  // accountable for, with the simulation taken out of it. Measured from
  // JavaScript rather than read out of the module's own diagnostics, because a
  // renderer's cost is what a renderer can observe.
  var extractionMs = [];
  for (var b = 0; b < FRAME_BATCHES; b += 1) {
    var t0 = performance.now();
    for (var i = 0; i < FRAMES_PER_BATCH; i += 1) {
      payload.readFrame(Module.HEAPF64, frameCall(0, 0));
    }
    extractionMs.push((performance.now() - t0) / FRAMES_PER_BATCH);
  }

  var hashBefore = Module.UTF8ToString(api._goliseo_payload_hash());
  var depths = [];
  var worst = 0;
  [1, 2, 4, MAX_RESIM_TICKS].forEach(function (depth) {
    var burstMs = [];
    for (var b = 0; b < BURST_BATCHES; b += 1) {
      var t0 = performance.now();
      for (var i = 0; i < BURSTS_PER_BATCH; i += 1) {
        payload.readFrame(Module.HEAPF64, frameCall(depth, 1));
      }
      burstMs.push((performance.now() - t0) / BURSTS_PER_BATCH);
    }
    var p95 = percentile(burstMs, 0.95);
    if (depth === MAX_RESIM_TICKS) { worst = p95; }
    depths.push({ depth: depth, mean: mean(burstMs), p95: p95, max: percentile(burstMs, 1) });
  });
  var hashAfter = Module.UTF8ToString(api._goliseo_payload_hash());

  // The mismatch gate, run where the payload is actually read. Corrupting the
  // live block rather than fabricating one means the reader has to reject a
  // block that is real in every other respect -- the shape a version skew takes.
  var detection = [];
  var live = frameCall(1, 0);
  var base = live / 8;
  [[0, "magic"], [1, "layout version"], [2, "render frame version"],
   [7, "player field count"]].forEach(function (probe) {
    var original = Module.HEAPF64[base + probe[0]];
    Module.HEAPF64[base + probe[0]] = original + 1;
    var detected = false;
    try {
      payload.readFrame(Module.HEAPF64, live);
    } catch (error) {
      detected = error instanceof payload.PayloadVersionError;
    }
    Module.HEAPF64[base + probe[0]] = original;
    detection.push({ field: probe[1], detected: detected });
  });
  payload.readFrame(Module.HEAPF64, live);

  return {
    ok: true,
    rosterCount: roster.count,
    rosterIds: roster.ids,
    words: words,
    crossingsPerFrame: crossingsPerFrame,
    otherCrossingsPerFrame: otherPerFrame,
    exports: surface,
    // Sample shape, so the reader can see these are percentiles over BATCH
    // MEANS, not over individual calls. Averaging before percentiling damps
    // the tail; it is unavoidable here because a browser worker's
    // performance.now() is coarsened to the same order as one payload read,
    // but it must not be presented as the same statistic node reports.
    sampling: {
      framesPerBatch: FRAMES_PER_BATCH,
      frameBatches: FRAME_BATCHES,
      burstsPerBatch: BURSTS_PER_BATCH,
      burstBatches: BURST_BATCHES
    },
    extraction: {
      mean: mean(extractionMs),
      p95: percentile(extractionMs, 0.95),
      max: percentile(extractionMs, 1)
    },
    frame: { mean: mean(frameMs), p95: percentile(frameMs, 0.95), max: percentile(frameMs, 1) },
    depths: depths,
    worst: worst,
    hashBefore: hashBefore,
    hashAfter: hashAfter,
    detection: detection,
    budget: UPDATE_BUDGET_MS
  };
}

// The module boots asynchronously; `--payload` makes main() install the host
// and return with the runtime live, so the exports appear shortly after load.
var waited = 0;
var timer = setInterval(function () {
  if (ready()) {
    clearInterval(timer);
    var result;
    try {
      result = measure();
    } catch (error) {
      result = { ok: false, error: String(error && error.stack || error) };
    }
    postMessage({ result: result });
  } else if ((waited += 20) > 120000) {
    clearInterval(timer);
    postMessage({ result: { ok: false, error: "exports never appeared" } });
  }
}, 20);
})();
"""


def conditions() -> dict:
    """The machine conditions a measurement was taken under.

    Not decoration. The difference between a 5.1 ms p95 and a 13.1 ms p95 for
    the same artifact on this same machine was load average, so a number
    reported without its conditions is not reproducible -- only re-runnable.
    """
    cores = os.cpu_count() or 1
    try:
        load1, load5, load15 = os.getloadavg()
    except (OSError, AttributeError):  # not available on every platform
        load1 = load5 = load15 = float("nan")
    return {
        "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "platform": platform.platform(),
        "cores": cores,
        "load1": load1,
        "load5": load5,
        "load15": load15,
        # A rough flag, not a gate: more runnable work than half the cores
        # inflates every tail number in the report.
        "contended": load1 > cores * 0.5,
    }


def print_conditions(label: str, c: dict) -> None:
    print(
        f"    {label:<15} {c['timestamp']}   {c['cores']} cores   "
        f"load {c['load1']:.2f}/{c['load5']:.2f}/{c['load15']:.2f}"
        + ("   CONTENDED - tail numbers inflated" if c["contended"] else ""),
        flush=True,
    )


def launch(browser: str, headless: bool) -> Any:
    from selenium import webdriver

    if browser == "chrome":
        from selenium.webdriver.chrome.options import Options
        from selenium.webdriver.chrome.service import Service

        options = Options()
        options.binary_location = "/usr/bin/google-chrome"
        if headless:
            options.add_argument("--headless=new")
        for flag in ("--disable-dev-shm-usage", "--no-first-run", "--disable-extensions"):
            options.add_argument(flag)
        driver_path = str(Path.home() / ".local/bin/chromedriver")
        return webdriver.Chrome(options=options, service=Service(executable_path=driver_path))

    from selenium.webdriver.firefox.options import Options
    from selenium.webdriver.firefox.service import Service

    options = Options()
    options.binary_location = str(Path.home() / ".local/bin/firefox")
    if headless:
        options.add_argument("-headless")
    driver_path = str(Path.home() / ".local/bin/geckodriver")
    return webdriver.Firefox(options=options, service=Service(executable_path=driver_path))


def run(browser: str, url: str, timeout: int, headless: bool) -> dict:
    driver = launch(browser, headless)
    try:
        driver.set_page_load_timeout(120)
        driver.get(url)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = driver.execute_script("return window.__RESULT__;")
            if result is not None:
                return result
            time.sleep(1.0)
        lines = driver.execute_script("return window.__OUT__ || [];")
        return {"ok": False, "error": f"timed out; last output: {lines[-5:]}"}
    finally:
        try:
            driver.quit()
        except Exception:
            pass


def report(result: dict, require_budget: bool) -> list[str]:
    """Turn a worker result into printed lines plus a list of failures.

    The budget verdict is REPORTED, not enforced, unless `--require-budget` is
    passed. It is a measurement of the simulation's speed on the machine and
    browser it happened to run on, not a property of the payload protocol this
    harness verifies -- and the payload's own share of it is printed separately
    so the two are never confused. Pass `--require-budget` to gate on it.
    """
    failures: list[str] = []
    if not result.get("ok"):
        return [f"    ERROR: {result.get('error')}"]

    budget = result["budget"]
    print(f"    roster          {result['rosterCount']} slots, once per match")
    print(f"    frame block     {result['words']} words ({result['words'] * 8} bytes)")
    other = result.get("otherCrossingsPerFrame", 0.0)
    print(
        f"    crossings/frame {result['crossingsPerFrame']:.2f} payload + {other:.2f} other "
        f"(every _goliseo_payload_* export wrapped: {len(result.get('exports', []))} of them)"
    )
    if abs(result["crossingsPerFrame"] - 1.0) > 1e-9 or abs(other) > 1e-9:
        failures.append("more than one boundary crossing per frame")

    # SAMPLING SHAPE, stated because it is not the same statistic node reports.
    # A browser worker's performance.now() is coarsened to roughly the cost of
    # one payload read, so each sample here is the MEAN of a batch and the p95
    # is a nearest-rank pick over those batch means. Averaging before
    # percentiling damps the tail: these p95s are lower bounds on the true
    # per-call p95, and the burst rows rest on only 40 samples.
    sampling = result.get("sampling", {})
    if sampling:
        print(
            f"    sampling        p95 over BATCH MEANS, not per-call: "
            f"{sampling['frameBatches']}x{sampling['framesPerBatch']} frames, "
            f"{sampling['burstBatches']}x{sampling['burstsPerBatch']} bursts "
            f"(tail is damped; node reports per-call)"
        )

    extraction = result["extraction"]
    print(
        f"    extraction      {extraction['mean']:.4f} ms mean   "
        f"{extraction['p95']:.4f} ms p95   {extraction['max']:.4f} ms max   "
        f"(payload only, {extraction['p95'] / budget * 100:.1f}% of budget)"
    )
    frame = result["frame"]
    print(
        f"    frame call      {frame['mean']:.4f} ms mean   "
        f"{frame['p95']:.4f} ms p95   {frame['max']:.4f} ms max   (1 tick + payload)"
    )
    for row in result["depths"]:
        verdict = "WITHIN BUDGET" if row["p95"] <= budget else "OVER BUDGET"
        print(
            f"    {row['depth']}-tick burst   {row['mean']:.4f} ms batch-mean   "
            f"{row['p95']:.4f} ms p95*  {row['max']:.4f} ms max*  -> {verdict}"
        )

    print(f"    state hash      {result['hashBefore']} -> {result['hashAfter']}")
    if result["hashBefore"] != result["hashAfter"]:
        failures.append("a rollback burst did not reproduce the state it rolled back over")

    for probe in result["detection"]:
        print(f"    mismatch        corrupted {probe['field']:<22}{'detected' if probe['detected'] else 'MISSED'}")
        if not probe["detected"]:
            failures.append(f"a corrupted {probe['field']} was read as valid")

    over = result["worst"] > budget
    verdict = "OVER BUDGET" if over else "WITHIN BUDGET"
    print(
        f"    {MAX_RESIM_TICKS}-tick worst    {result['worst']:.4f} ms p95* "
        f"vs {budget:.1f} ms budget -> {verdict}"
    )
    print(
        "                    * over batch means; the budget is omp0_acceptance's love.update "
        "share,\n                      applied here to a standalone wasm host as a 60 Hz analogue."
    )
    if over and require_budget:
        failures.append(
            f"the {MAX_RESIM_TICKS}-tick burst is {result['worst']:.4f} ms p95, over the "
            f"{budget:.1f} ms budget"
        )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument("--browsers", default="chrome,firefox")
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--headed", action="store_true", help="show the browser window")
    parser.add_argument(
        "--budget-ms",
        type=float,
        default=DEFAULT_BUDGET_MS,
        help="frame budget to judge bursts against (default %(default)s)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="also emit each browser's full result, with machine conditions, as JSON",
    )
    parser.add_argument(
        "--require-budget",
        action="store_true",
        help="fail when the 8-tick burst exceeds the update budget",
    )
    args = parser.parse_args()

    dist = args.dist.resolve()
    for required in ("simhost.js", "simhost.wasm"):
        if not (dist / required).exists():
            print(f"missing {dist / required}; run wasm/sim-host/build.sh first")
            return 1

    # Served from a copy: the build runs in Docker so dist/ is root-owned, and a
    # verifier has no business writing into build output regardless.
    serve_dir = Path(tempfile.mkdtemp(prefix="goliseo-payload-verify-"))
    for artifact in dist.iterdir():
        if artifact.is_file():
            shutil.copy2(artifact, serve_dir / artifact.name)
    # The reader under test is the one the renderer will use, byte for byte --
    # not a copy of it maintained in this harness.
    shutil.copy2(ROOT / "frame_payload.js", serve_dir / "frame_payload.js")
    (serve_dir / "index.html").write_text(PAGE)
    (serve_dir / "payload_worker.js").write_text(
        WORKER.replace("__BUDGET__", json.dumps(args.budget_ms)).replace(
            "__MAX_RESIM__", json.dumps(MAX_RESIM_TICKS)
        )
    )

    server = ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=str(serve_dir))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_port}/index.html"

    failures: list[str] = []
    contended_any = False
    try:
        for browser in [b.strip() for b in args.browsers.split(",") if b.strip()]:
            print(f"==> {browser}", flush=True)
            before = conditions()
            print_conditions("conditions", before)
            try:
                result = run(browser, url, args.timeout_seconds, not args.headed)
            except Exception as error:  # noqa: BLE001 - reported, not swallowed
                failures.append(f"{browser}: {error}")
                print(f"    LAUNCH FAILED: {error}", flush=True)
                continue
            for failure in report(result, args.require_budget):
                failures.append(f"{browser}: {failure}")
            after = conditions()
            print_conditions("conditions", after)
            contended_any = contended_any or before["contended"] or after["contended"]
            if args.json:
                print(
                    "    JSON "
                    + json.dumps(
                        {
                            "browser": browser,
                            "conditions_before": before,
                            "conditions_after": after,
                            **result,
                        }
                    ),
                    flush=True,
                )
            print("", flush=True)
    finally:
        server.shutdown()

    if contended_any:
        print(
            "NOTE: this machine was contended during the run; every tail number above is "
            "inflated. Re-run on an idle machine before quoting these as capacity numbers.\n"
        )

    if failures:
        print("FAILURES:")
        for failure in failures:
            print("  " + failure)
        return 1
    print("PAYLOAD BROWSER VERIFY OK: one crossing per frame, versioned, rollback inside.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
