#!/usr/bin/env python3
"""Shared page, worker and verdict logic for the wasm sim-host verifiers.

Two runners drive the same experiment through different plumbing:

    verify_browser.py   Chrome and Firefox, over WebDriver (#327).
    verify_webview.py   desktop webviews -- WebKitGTK over PyGObject,
                        WebView2 over msedgedriver (#342).

The *evidence* has to be identical or the comparison is worthless: same page,
same worker, same fixture, same verdict rule. So all of it lives here and
neither runner is allowed a private copy. If the page a webview loads differed
from the page Chrome loads by so much as a flag, a hash difference could not be
attributed to the JavaScript engine, which is the only thing #342 is measuring.
"""

from __future__ import annotations

import shutil
import tempfile
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXPECTED_HASH = "bfbb106aea5480f8"
EXPECTED_DIGEST = "a190b60058a64e63"

#: Artifacts a staged directory must contain before anything is worth running.
REQUIRED_ARTIFACTS = ("simhost.js", "simhost.wasm")

PAGE = """<!doctype html>
<meta charset="utf-8">
<title>goliseo wasm sim host</title>
<script>
// The module runs the full 7201-tick determinism fixture synchronously. On the
// main thread that blocks the renderer for minutes and every browser automation
// call times out -- which is not a harness quirk, it is exactly why the real
// integration has to put the simulation in a worker and leave the main thread
// to rendering. So the verification runs it the way it will actually be used.
window.__OUT__ = [];
window.__MARKS__ = [];
window.__DONE__ = false;
const worker = new Worker("worker.js");
worker.onmessage = (e) => {
  if (e.data && e.data.line !== undefined) window.__OUT__.push(String(e.data.line));
  if (e.data && e.data.mark !== undefined) window.__MARKS__.push(e.data);
  if (e.data && e.data.done) window.__DONE__ = true;
};
worker.onerror = (e) => {
  window.__OUT__.push("WORKER_ERROR: " + (e && e.message));
  window.__DONE__ = true;
};
</script>
"""

WORKER = """
// Cold start, measured where it actually costs something: over HTTP, in a
// browser, with the fetch and the wasm compile in the clock. Node reads the
// artifacts off local disk, so its numbers understate this. T0 is taken before
// importScripts, so both marks include fetching and compiling the module.
//
//   runtime ready  the module is instantiated and main() is about to run.
//                  Under the old preload build this also covered downloading
//                  and mounting the 3.2 MB .data package.
//   sim ready      all 13 sim/, core/ and data/ modules are loaded, so the
//                  first tick can be taken.
var __T0__ = performance.now();
var __SEEN_SIM_READY__ = false;
function __mark__(name) { postMessage({ mark: name, ms: performance.now() - __T0__ }); }
var Module = {
  print:    function (t) {
    var line = String(t);
    if (!__SEEN_SIM_READY__ && line.indexOf("== frozen determinism contract") === 0) {
      __SEEN_SIM_READY__ = true;
      __mark__("sim ready");
    }
    postMessage({ line: line });
  },
  printErr: function (t) { postMessage({ line: "ERR: " + String(t) }); },
  onRuntimeInitialized: function () { __mark__("runtime ready"); },
  onExit:   function ()  { postMessage({ done: true }); },
  onAbort:  function (w) { postMessage({ line: "ABORT: " + String(w) }); postMessage({ done: true }); }
};
try {
  importScripts("simhost.js");
} catch (err) {
  postMessage({ line: "IMPORT_ERROR: " + err });
  postMessage({ done: true });
}
"""

#: The one JavaScript expression a runner evaluates to sample the page. Every
#: runner uses this exact string so the sampling itself cannot differ between
#: engines.
POLL_SCRIPT = (
    "JSON.stringify({"
    "out: window.__OUT__ || [],"
    "marks: window.__MARKS__ || [],"
    "done: window.__DONE__ === true"
    "})"
)


class ArtifactsMissing(RuntimeError):
    """A dist directory does not hold a built module."""


def stage(dist: Path) -> Path:
    """Copy a built dist into a fresh servable directory with page and worker.

    Served from a copy rather than from dist/ itself: the build runs in Docker
    so dist/ is root-owned, and a verifier has no business writing into build
    output regardless.
    """
    dist = dist.resolve()
    missing = [name for name in REQUIRED_ARTIFACTS if not (dist / name).exists()]
    if missing:
        raise ArtifactsMissing(
            f"missing {', '.join(str(dist / name) for name in missing)}; "
            "run wasm/sim-host/build.sh first"
        )

    serve_dir = Path(tempfile.mkdtemp(prefix="goliseo-wasm-verify-"))
    for artifact in dist.iterdir():
        if artifact.is_file():
            shutil.copy2(artifact, serve_dir / artifact.name)
    (serve_dir / "index.html").write_text(PAGE)
    (serve_dir / "worker.js").write_text(WORKER)
    return serve_dir


def serve(serve_dir: Path) -> tuple[ThreadingHTTPServer, str]:
    """Serve a staged directory on loopback and return the server and page URL.

    Over HTTP on purpose. Cold start is part of what these runners report, and a
    `file://` load skips the fetch that a shipped build pays for -- and would
    also put the worker on a null origin, which not every engine allows.
    """
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=str(serve_dir))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}/index.html"


def poll_webdriver(driver, url: str, timeout: int) -> tuple[list[str], list[dict]]:
    """Drive one WebDriver session through the fixture and return what it printed.

    Shared by the Chrome/Firefox runner and the WebView2 runner. WebView2 is
    driven by msedgedriver, so the only difference between them is which session
    was opened -- keeping the loop in one place is what makes that true.
    """
    import json
    import time

    driver.set_page_load_timeout(120)
    driver.get(url)
    deadline = time.monotonic() + timeout
    out: list[str] = []
    marks: list[dict] = []
    while time.monotonic() < deadline:
        sample = json.loads(driver.execute_script(f"return {POLL_SCRIPT};"))
        out = [str(line) for line in sample.get("out", [])]
        marks = list(sample.get("marks", []))
        if is_finished(out, bool(sample.get("done"))):
            break
        time.sleep(1.0)
    return out, marks


def is_finished(out: list[str], done: bool) -> bool:
    """Whether a run has produced everything it is ever going to produce."""
    return done or any("PHASE0 OK" in line or "FAILED" in line for line in out)


def check(out: list[str]) -> tuple[bool, str]:
    """Decide the verdict for one runtime's captured output.

    Every clause here is a way a run can be wrong, including the quiet ones: an
    empty output list means the module never printed, which must not read as
    "no divergence found".
    """
    text = "\n".join(out)
    if EXPECTED_HASH not in text:
        return False, f"frozen hash {EXPECTED_HASH} not found in output"
    if EXPECTED_DIGEST not in text:
        return False, f"sequence digest {EXPECTED_DIGEST} not found in output"
    if "DIVERGED" in text or "FAILED" in text:
        return False, "probe reported divergence"
    if "verdict          MATCH" not in text and "MATCH" not in text:
        return False, "no MATCH verdict"
    return True, "hashes match the frozen contract"


def observed_hashes(out: list[str]) -> dict[str, str]:
    """Pull the hashes a run actually printed, for reporting rather than judging.

    `check` decides pass or fail; this exists so a report can state the value a
    runtime produced instead of only whether it was the expected one. A run that
    printed nothing yields an empty mapping and the report says so.
    """
    found: dict[str, str] = {}
    for line in out:
        for label, key in (
            ("final hash", "final_hash"),
            ("sequence digest", "sequence_digest"),
        ):
            if line.startswith(label) and key not in found:
                found[key] = line[len(label) :].strip()
    return found


def print_run(out: list[str], marks: list[dict], indent: str = "    ") -> None:
    """Print the cold-start marks and the lines worth reading from one run."""
    for mark in marks:
        print(f"{indent}cold start: {mark['mark']:<14}{mark['ms']:.1f} ms", flush=True)
    for line in out:
        if any(k in line for k in ("hash", "verdict", "per tick", "8-tick", "ERR", "ABORT")):
            print(indent + line[:160], flush=True)
