#!/usr/bin/env python3
"""Verify the wasm simulation host reproduces the frozen hashes in real browsers.

#327 measured the module under node. Node is not a browser -- different loader,
different memory limits, no DOM -- so "it runs in the browser" was an untested
assumption until this ran.

Headless is CORRECT here, and that is worth stating because it was wrong for the
render benchmark. This measures pure computation with no GPU involvement, so
software rasterisation is irrelevant; there is nothing to rasterise. The rule is
not "never headless", it is "never headless when the GPU is the thing under test".

    python3 wasm/sim-host/verify_browser.py            # after build.sh
"""

from __future__ import annotations

import argparse
import sys
import threading
import time
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
EXPECTED_HASH = "bfbb106aea5480f8"
EXPECTED_DIGEST = "a190b60058a64e63"

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
window.__DONE__ = false;
const worker = new Worker("worker.js");
worker.onmessage = (e) => {
  if (e.data && e.data.line !== undefined) window.__OUT__.push(String(e.data.line));
  if (e.data && e.data.done) window.__DONE__ = true;
};
worker.onerror = (e) => {
  window.__OUT__.push("WORKER_ERROR: " + (e && e.message));
  window.__DONE__ = true;
};
</script>
"""

WORKER = """
var Module = {
  print:    function (t) { postMessage({ line: String(t) }); },
  printErr: function (t) { postMessage({ line: "ERR: " + String(t) }); },
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


def run(browser: str, url: str, timeout: int, headless: bool) -> tuple[bool, list[str]]:
    driver = launch(browser, headless)
    try:
        driver.set_page_load_timeout(120)
        driver.get(url)
        deadline = time.monotonic() + timeout
        out: list[str] = []
        while time.monotonic() < deadline:
            out = driver.execute_script("return window.__OUT__ || [];")
            done = driver.execute_script("return window.__DONE__ === true;")
            if done or any("PHASE0 OK" in line or "FAILED" in line for line in out):
                break
            time.sleep(1.0)
        return True, out
    finally:
        try:
            driver.quit()
        except Exception:
            pass


def check(out: list[str]) -> tuple[bool, str]:
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument("--browsers", default="chrome,firefox")
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--headed", action="store_true", help="show the browser window")
    args = parser.parse_args()

    dist = args.dist.resolve()
    for required in ("simhost.js", "simhost.wasm"):
        if not (dist / required).exists():
            print(f"missing {dist / required}; run wasm/sim-host/build.sh first")
            return 1

    # Served from a copy rather than from dist/ itself: the build runs in Docker
    # so dist/ is root-owned, and a verifier has no business writing into build
    # output regardless.
    import shutil
    import tempfile

    serve_dir = Path(tempfile.mkdtemp(prefix="goliseo-wasm-verify-"))
    for artifact in dist.iterdir():
        if artifact.is_file():
            shutil.copy2(artifact, serve_dir / artifact.name)
    (serve_dir / "index.html").write_text(PAGE)
    (serve_dir / "worker.js").write_text(WORKER)

    server = ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(SimpleHTTPRequestHandler, directory=str(serve_dir))
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_port}/index.html"

    failures = []
    try:
        for browser in [b.strip() for b in args.browsers.split(",") if b.strip()]:
            print(f"==> {browser}", flush=True)
            try:
                _, out = run(browser, url, args.timeout_seconds, not args.headed)
            except Exception as error:  # noqa: BLE001 - reported, not swallowed
                failures.append(f"{browser}: {error}")
                print(f"    LAUNCH FAILED: {error}", flush=True)
                continue
            ok, detail = check(out)
            for line in out:
                if any(k in line for k in ("hash", "verdict", "per tick", "8-tick", "ERR", "ABORT")):
                    print("    " + line[:160], flush=True)
            print(f"    -> {'PASS' if ok else 'FAIL'}: {detail}", flush=True)
            if not ok:
                failures.append(f"{browser}: {detail}")
    finally:
        server.shutdown()

    if failures:
        print("\nFAILURES:")
        for failure in failures:
            print("  " + failure)
        return 1
    print("\nBROWSER VERIFY OK: the wasm simulation reproduces the frozen contract.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
