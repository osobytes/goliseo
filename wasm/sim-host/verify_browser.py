#!/usr/bin/env python3
"""Verify the wasm simulation host reproduces the frozen hashes in real browsers.

#327 measured the module under node. Node is not a browser -- different loader,
different memory limits, no DOM -- so "it runs in the browser" was an untested
assumption until this ran.

Headless is CORRECT here, and that is worth stating because it was wrong for the
render benchmark. This measures pure computation with no GPU involvement, so
software rasterisation is irrelevant; there is nothing to rasterise. The rule is
not "never headless", it is "never headless when the GPU is the thing under test".

It also reports cold start -- the time before the first tick can be taken -- per
browser, because that is the number #334 set out to move and node, reading the
artifacts off local disk, cannot measure it honestly.

The page, the worker and the verdict rule live in `verify_common.py`, shared
with `verify_webview.py` so the desktop-webview comparison in #342 runs the
identical experiment rather than a lookalike.

    python3 wasm/sim-host/verify_browser.py            # after build.sh
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import verify_common
from verify_common import ROOT, check, stage

# Re-exported for callers and readers that expect these names here, where they
# lived before the shared module existed.
EXPECTED_HASH = verify_common.EXPECTED_HASH
EXPECTED_DIGEST = verify_common.EXPECTED_DIGEST
PAGE = verify_common.PAGE
WORKER = verify_common.WORKER

__all__ = ["EXPECTED_DIGEST", "EXPECTED_HASH", "PAGE", "ROOT", "WORKER", "check", "main", "run"]


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


def run(browser: str, url: str, timeout: int, headless: bool) -> tuple[list[str], list[dict], str]:
    """Run the fixture and return its output, its cold-start marks and the engine.

    The engine string is read from the live session rather than from a version
    flag, because #342 compares runtimes: a result is only comparable if the
    build that produced it is recorded beside it.
    """
    driver = launch(browser, headless)
    try:
        capabilities = driver.capabilities or {}
        engine = "{} {}".format(
            capabilities.get("browserName", browser),
            capabilities.get("browserVersion", "unknown"),
        )
        out, marks = verify_common.poll_webdriver(driver, url, timeout)
        return out, marks, engine
    finally:
        try:
            driver.quit()
        except Exception:
            pass


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    parser.add_argument("--browsers", default="chrome,firefox")
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--headed", action="store_true", help="show the browser window")
    args = parser.parse_args()

    try:
        serve_dir = stage(args.dist)
    except verify_common.ArtifactsMissing as error:
        print(error)
        return 1

    server, url = verify_common.serve(serve_dir)

    failures = []
    try:
        for browser in [b.strip() for b in args.browsers.split(",") if b.strip()]:
            print(f"==> {browser}", flush=True)
            try:
                out, marks, engine = run(browser, url, args.timeout_seconds, not args.headed)
            except Exception as error:  # noqa: BLE001 - reported, not swallowed
                failures.append(f"{browser}: {error}")
                print(f"    LAUNCH FAILED: {error}", flush=True)
                continue
            ok, detail = check(out)
            print(f"    engine: {engine}", flush=True)
            verify_common.print_run(out, marks)
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
