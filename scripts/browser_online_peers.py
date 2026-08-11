#!/usr/bin/env python3
"""Prove two REAL browser peers derive identical online-match boundary hashes.

## Why this script exists

Every OMP-3 online test that already exists (coordinator, driver, rollback,
desync, diagnostics) runs through an in-process fake transport or a single
wasm instance. None of them can prove the netcode's actual premise: that two
INDEPENDENT instances, each with its own real `RTCDataChannel` delivering
bytes on its own callback at its own arbitrary moment relative to its own
tick loop, derive byte-identical state from that traffic. This script is the
first thing in this repository that drives two such independent instances and
diffs what they each, independently, computed.

`tools/browser_online_match/web/online_peer.ts` is the page each peer
runs: it opens a real `RTCPeerConnection` (via `@gc/transport`'s
`BrowserStarTransport`, backed by a real `window.GoliseoStarTransport` --
see `ts/packages/transport/src/browser_star_bridge.ts`), then drives a
real `MatchDriverBridge` (`@gc/wasm`) for a fixed tick count, accumulating
its own `(tick, hash)` checkpoint sequence purely from whatever its own
transport delivered. This script's only job, after getting both pages
connected, is to read BOTH sequences back independently and assert they are
identical -- never comparing a peer against itself, never asserting a hash
computed once and reused (see this repository's brief for why that would
prove nothing).

## Signaling is out of band, and is not the thing under test

WebRTC offer/answer exchange has to happen somehow before there is a
connection to test. This script relays the two opaque signal strings
directly between the two Selenium-controlled tabs (`window.__gcOnlinePeer.
hostOffer` -> `guestAnswer` -> back) -- no separate relay server, no new
dependency, and precisely as invisible to the property under test as a
lobby server would be in production: once `acceptAnswer` returns, this
script never touches the connection again.

## Shares its launch/teardown plumbing with the other browser harnesses

Same pinned-asset resolution (`browser_matrix.resolve_assets`), same Chrome/
Firefox launch options and bounded teardown (imported from
`scripts/browser_launch.py`, not reimplemented) that
`scripts/browser_match_harness.py` and `scripts/browser_render_bench.py`
also use, so the three cannot drift apart on how a browser process is
started and torn down.

## Wired into CI

`.github/workflows/ci.yml`'s gate job runs this script as "Prove two real
browser peers agree bit for bit", after installing the pinned browser
evidence dependencies from `scripts/browser_matrix-requirements.txt`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import threading
import time
from collections.abc import Callable
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from browser_launch import (  # noqa: E402
    bounded_log_tail,
    launch,
    quit_browser_bounded,
)
from browser_matrix import resolve_assets  # noqa: E402
from web_serve import ArtifactHandler  # noqa: E402


ROOT = Path(__file__).resolve().parents[1]
TS = ROOT / "ts"
HARNESS_DIR = ROOT / "tools" / "browser_online_match"
HARNESS_DIST = HARNESS_DIR / "dist"
WASM_BUILD_SCRIPT = TS / "packages" / "wasm" / "scripts" / "build_web.mjs"
HARNESS_VITE_CONFIG = HARNESS_DIR / "vite.config.ts"

DEFAULT_TICKS = 150
# 50ms/tick (20Hz) rather than the real match's 60Hz: this Selenium-driven
# harness runs two independent browser processes with no shared vsync, on a
# CI/sandbox machine that may be running several other things at once (see
# `run_two_sessions`'s doc). A tighter interval measurably increased the
# chance of one peer falling behind by more than `match_driver`'s retention
# window under real scheduling jitter, producing a legitimate (not a false)
# `confirmation_stalled`/`input_channel_failure` termination -- the
# protocol correctly refusing to guess past a real delivery gap that big.
# 50ms leaves enough slack in practice; see this script's own report for the
# numbers this was tuned against.
DEFAULT_TICK_SLEEP_MS = 50
CONNECT_TIMEOUT_SECONDS = 30
RUN_TIMEOUT_SECONDS = 90
ERROR_MARKERS = ("GC_ONLINE_PEER|error|",)


def build_harness(skip_wasm_build: bool) -> None:
    """`node packages/wasm/scripts/build_web.mjs` then `pnpm exec vite build`
    -- exactly the recipe this task's brief specifies, run against this
    harness's own `vite.config.ts` rather than the app shell's."""
    if not skip_wasm_build:
        print("[browser_online_peers] node packages/wasm/scripts/build_web.mjs")
        subprocess.run(
            ["node", str(WASM_BUILD_SCRIPT)],
            cwd=TS,
            check=True,
        )
    elif not (TS / "packages" / "wasm" / "dist" / "pkg-web" / "gc_wasm.js").is_file():
        raise RuntimeError("--skip-wasm-build was given but dist/pkg-web/gc_wasm.js is missing")
    print("[browser_online_peers] pnpm exec vite build (harness)")
    subprocess.run(
        ["pnpm", "exec", "vite", "build", "--config", str(HARNESS_VITE_CONFIG)],
        cwd=TS,
        check=True,
        env={"VITE_CONFIG_NATIVE_IGNORE_WARNING": "true", **_inherited_env()},
    )
    if not (HARNESS_DIST / "index.html").is_file():
        raise RuntimeError(f"harness build did not produce {HARNESS_DIST / 'index.html'}")


def _inherited_env() -> dict[str, str]:
    import os

    return dict(os.environ)


def serve_dist(directory: Path) -> tuple[ThreadingHTTPServer, threading.Thread, str]:
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        lambda *a, **k: ArtifactHandler(*a, directory=str(directory), **k),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{server.server_port}/"
    return server, thread, base_url


def wait_until(predicate: Callable[[], Any], timeout: float, description: str) -> Any:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.15)
    raise RuntimeError(f"timed out waiting for {description}")


class Peer:
    """One browser tab (or, in cross-engine mode, one whole browser session)
    running `online_peer.ts`. `handle` is `None` in cross-engine mode -- each
    engine gets its own `WebDriver` session with exactly one tab, so there is
    nothing to switch between; it is set in same-process mode, where both
    peers share one `WebDriver` session as two tabs."""

    def __init__(self, name: str, driver: Any, handle: str | None) -> None:
        self.name = name
        self.driver = driver
        self.handle = handle

    def _focus(self) -> None:
        if self.handle is not None:
            self.driver.switch_to.window(self.handle)

    def eval(self, script: str, *args: Any) -> Any:
        self._focus()
        return self.driver.execute_script(script, *args)

    def state_field(self, field: str) -> Any:
        return self.eval(
            "return window.__gcOnlinePeer ? window.__gcOnlinePeer[arguments[0]] : null;",
            field,
        )

    def set_state_field(self, field: str, value: Any) -> None:
        self.eval("window.__gcOnlinePeer[arguments[0]] = arguments[1];", field, value)

    def console_entries(self) -> list[str]:
        if self.driver.capabilities.get("browserName") != "chrome":
            return []
        self._focus()
        return [str(entry.get("message", "")) for entry in self.driver.get_log("browser")]


def relay_signaling(host: Peer, guest: Peer) -> None:
    """Ferries the two opaque signal blobs between the tabs. See this
    module's docstring: this is explicitly the out-of-band part."""
    offer = wait_until(lambda: host.state_field("hostOffer"), CONNECT_TIMEOUT_SECONDS, "host offer")
    print(f"[browser_online_peers] relaying offer ({len(offer)} chars) host -> guest")
    guest.set_state_field("hostOfferForGuest", offer)

    answer = wait_until(lambda: guest.state_field("guestAnswer"), CONNECT_TIMEOUT_SECONDS, "guest answer")
    print(f"[browser_online_peers] relaying answer ({len(answer)} chars) guest -> host")
    host.set_state_field("guestAnswerForHost", answer)


def wait_status(peer: Peer, targets: set[str], timeout: float) -> str:
    def check() -> str | None:
        status = peer.state_field("status")
        if status == "error":
            raise RuntimeError(f"{peer.name} page reported an error: {peer.state_field('error')}")
        return status if status in targets else None

    return wait_until(check, timeout, f"{peer.name} status in {sorted(targets)}")


def release_start_barrier(host: Peer, guest: Peer) -> None:
    """Waits for both pages to finish building their `MatchDriverBridge`
    (`ready_to_run`), then pokes `startSignal` on both back-to-back. See
    `online_peer.ts`'s `PeerControlState.startSignal` doc: without this,
    whichever page's wasm/JS setup finished first ticked alone for a while
    and its unconfirmed input backlog overran the other side's transport
    queue before that side ever got a chance to consume anything -- a real
    failure this harness hit before this barrier existed."""
    wait_status(host, {"ready_to_run"}, CONNECT_TIMEOUT_SECONDS)
    wait_status(guest, {"ready_to_run"}, CONNECT_TIMEOUT_SECONDS)
    host.set_state_field("startSignal", True)
    guest.set_state_field("startSignal", True)
    print("[browser_online_peers] both peers ready; released the tick-loop start barrier")


def wait_report(peer: Peer, timeout: float) -> dict[str, Any]:
    def check() -> dict[str, Any] | None:
        status = peer.state_field("status")
        if status == "error":
            error = peer.state_field("error")
            raise RuntimeError(f"{peer.name} page reported an error: {error}")
        if status == "done":
            report = peer.state_field("report")
            if isinstance(report, dict):
                return report
        return None

    return wait_until(check, timeout, f"{peer.name} report")


def check_no_runtime_errors(peer: Peer) -> None:
    for message in peer.console_entries():
        if any(marker in message for marker in ERROR_MARKERS):
            raise RuntimeError(f"{peer.name} runtime failure: {message}")


MIN_OVERLAPPING_CHECKPOINTS = 3


def compare_checkpoints(host_report: dict[str, Any], guest_report: dict[str, Any]) -> dict[str, Any]:
    """The load-bearing assertion. Two independently-computed sequences,
    read back from two independent pages, compared here and nowhere else --
    never a peer compared against itself.

    Compared by TICK, over the overlapping range only, not by raw list
    equality. `host`/`guest` run the same fixed LOOP-ITERATION budget
    (`--ticks`), not the same fixed SIMULATION-tick budget -- host is the
    sequence authority and guest only applies confirmed batches, so the
    number of simulation ticks actually covered by N driver-loop iterations
    legitimately differs a little between the two roles, especially under
    the real scheduling jitter of a shared, contended CI/sandbox machine.
    That asymmetry is benign (harness pacing, not disagreement) as long as
    every boundary BOTH sides actually reached hashes identically -- which
    is exactly, and only, what this function asserts. A one-sided extra
    trailing checkpoint neither proves nor disproves anything and is
    dropped from the comparison, not silently treated as a match.
    """
    host_checkpoints = host_report.get("checkpoints")
    guest_checkpoints = guest_report.get("checkpoints")
    if not isinstance(host_checkpoints, list) or not isinstance(guest_checkpoints, list):
        raise RuntimeError("a report's checkpoints field is not a list")
    host_by_tick = {entry["tick"]: entry["hash"] for entry in host_checkpoints}
    guest_by_tick = {entry["tick"]: entry["hash"] for entry in guest_checkpoints}
    common_ticks = sorted(set(host_by_tick) & set(guest_by_tick))
    if len(common_ticks) < MIN_OVERLAPPING_CHECKPOINTS:
        raise RuntimeError(
            f"only {len(common_ticks)} boundary tick(s) were hashed by BOTH peers "
            f"(host hashed {sorted(host_by_tick)}, guest hashed {sorted(guest_by_tick)}) -- "
            f"need at least {MIN_OVERLAPPING_CHECKPOINTS} for this run to prove anything; "
            "raise --ticks or --tick-sleep-ms"
        )
    for tick in common_ticks:
        if host_by_tick[tick] != guest_by_tick[tick]:
            raise RuntimeError(
                f"checkpoint at tick {tick} diverges: host={host_by_tick[tick]} guest={guest_by_tick[tick]}"
            )
    return {
        "checkpoint_count": len(common_ticks),
        "host_only_ticks": sorted(set(host_by_tick) - set(guest_by_tick)),
        "guest_only_ticks": sorted(set(guest_by_tick) - set(host_by_tick)),
        "first_tick": common_ticks[0],
        "last_tick": common_ticks[-1],
        "last_hash": host_by_tick[common_ticks[-1]],
    }


def run_two_sessions(host_browser: str, guest_browser: str, args: argparse.Namespace, base_url: str) -> dict[str, Any]:
    """Two SEPARATE `WebDriver` sessions/processes, one tab each -- always
    the foreground tab in its own process, never a backgrounded second tab.

    An earlier version of this script ran both peers as two tabs in one
    Chrome session (the "same browser process is acceptable" tier the task
    brief allows). That surfaced a real methodology trap worth recording:
    opening a second tab backgrounds the first, and Chrome's background-tab
    timer throttling (`setTimeout` clamped to roughly one call/second once a
    tab has been hidden for a few seconds) is a page-visibility feature that
    applies in headless mode too. The backgrounded peer's tick loop nearly
    stalled while the foreground peer kept advancing, so its unconfirmed
    input backlog grew without bound and overflowed the OTHER peer's
    transport inbound queue -- `MatchDriverBridge` correctly reported
    `input_channel_failure`. That is a real failure mode, but it is a test
    harness pacing defect, not evidence about `net_inbox`'s tick-quantised
    delivery discipline (the property this script exists to test). Two
    always-foreground processes side-step the whole throttling class
    without needing to fight it with extra Chrome flags, and are if
    anything a MORE independent pair of instances than two tabs sharing one
    renderer process's task queue.
    """
    host_binary, host_driver_path = resolve_assets(host_browser, None, None)
    guest_binary, guest_driver_path = resolve_assets(guest_browser, None, None)
    host_log = Path(args.log_dir) / f"{host_browser}-host-webdriver.log"
    guest_log = Path(args.log_dir) / f"{guest_browser}-guest-webdriver.log"
    host_log.parent.mkdir(parents=True, exist_ok=True)
    guest_log.parent.mkdir(parents=True, exist_ok=True)

    host_driver = None
    guest_driver = None
    try:
        try:
            host_driver = launch(host_browser, host_binary, host_driver_path, host_log)
        except Exception as error:
            raise RuntimeError(
                f"{host_browser} host launch failed: {error}\nwebdriver log tail:\n{bounded_log_tail(host_log)}"
            ) from error
        try:
            guest_driver = launch(guest_browser, guest_binary, guest_driver_path, guest_log)
        except Exception as error:
            raise RuntimeError(
                f"{guest_browser} guest launch failed: {error}\nwebdriver log tail:\n{bounded_log_tail(guest_log)}"
            ) from error

        host_driver.set_page_load_timeout(60)
        guest_driver.set_page_load_timeout(60)
        host_url = f"{base_url}?role=host&ticks={args.ticks}&tick_sleep_ms={args.tick_sleep_ms}"
        guest_url = f"{base_url}?role=guest&ticks={args.ticks}&tick_sleep_ms={args.tick_sleep_ms}"
        host_driver.get(host_url)
        guest_driver.get(guest_url)

        host = Peer("host", host_driver, None)
        guest = Peer("guest", guest_driver, None)

        relay_signaling(host, guest)
        release_start_barrier(host, guest)

        host_report = wait_report(host, RUN_TIMEOUT_SECONDS)
        guest_report = wait_report(guest, RUN_TIMEOUT_SECONDS)
        check_no_runtime_errors(host)
        if guest_browser == "chrome":
            check_no_runtime_errors(guest)

        return {
            "mode": "same_engine" if host_browser == guest_browser else "cross_engine",
            "browsers": {"host": host_browser, "guest": guest_browser},
            "host_report": host_report,
            "guest_report": guest_report,
        }
    finally:
        if host_driver is not None:
            quit_browser_bounded(host_driver)
        if guest_driver is not None:
            quit_browser_bounded(guest_driver)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ticks", type=int, default=DEFAULT_TICKS)
    parser.add_argument("--tick-sleep-ms", type=int, default=DEFAULT_TICK_SLEEP_MS)
    parser.add_argument("--skip-wasm-build", action="store_true")
    parser.add_argument("--skip-harness-build", action="store_true")
    parser.add_argument("--cross-engine", action="store_true", help="Chrome host, Firefox guest, two sessions.")
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--log-dir", type=Path, default=ROOT / "build" / "browser_online_peers-logs")
    args = parser.parse_args()

    if not args.skip_harness_build:
        build_harness(args.skip_wasm_build)
    elif not (HARNESS_DIST / "index.html").is_file():
        raise SystemExit(f"--skip-harness-build was given but {HARNESS_DIST / 'index.html'} is missing")

    server, thread, base_url = serve_dist(HARNESS_DIST)
    try:
        guest_browser = "firefox" if args.cross_engine else "chrome"
        result = run_two_sessions("chrome", guest_browser, args, base_url)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)

    try:
        agreement = compare_checkpoints(result["host_report"], result["guest_report"])
        result["agreement"] = agreement
        result["pass"] = True
        print(
            f"[browser_online_peers] PASS: {agreement['checkpoint_count']} boundary checkpoints "
            f"(ticks {agreement['first_tick']}-{agreement['last_tick']}) agree bit-for-bit between "
            f"{result['browsers']['host']} host and {result['browsers']['guest']} guest, "
            f"final hash {agreement['last_hash']}"
        )
    except Exception as error:
        result["pass"] = False
        result["failure"] = str(error)
        raise
    finally:
        if args.output is not None:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
            print(f"[browser_online_peers] evidence written to {args.output}")

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 -- top-level CLI failure boundary
        print(f"[browser_online_peers] FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
