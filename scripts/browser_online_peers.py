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

## Two runs, two triggers, two costs

`.github/workflows/ci.yml`'s gate job runs this script as "Prove two real
browser peers agree bit for bit" on every push and pull request, at
`DEFAULT_TICKS` -- about seven and a half seconds of match. That is enough
to prove two real peers agree at all, and cheap enough to sit in front of
every merge.

`.github/workflows/scheduled.yml` runs it daily for 36,000 iterations --
thirty minutes of continuous match -- with `--sample-every`, which is the
only place long-duration evidence exists. The two differ ONLY in run
length and sampling: same script, same page, same assertions, so the
scheduled run cannot quietly diverge from the gated one.

## What makes a run fail

A run fails if any boundary hash both peers reached disagrees, if either
page logged a runtime error, if either driver ended in anything but
`active`/`completed`, if either tick loop completed fewer iterations than
it was asked for, if fewer than `--min-checkpoints` boundaries were reached
by both peers, or -- when sampling is on -- if a peer's retained
speculative-event window grew past `Omp2RollbackBudgets::memory_growth_ratio`.
`--self-test` proves each of those rejections with crafted reports and
starts no browser; per AGENTS.md §9 that is a demonstration the gate can go
red, NOT a substitute for running it.

## What this script still does not measure

Whole-instance memory. `Omp2RollbackBudgets::memory_growth_ratio` was
written against a soak's total memory growth; what `--sample-every` reads
is the engine's own exact accounting of the retained speculative event
window (`rollbackAccountingJson`), which is a real measurement of retained
history but not of the instance's footprint. `performance.memory` is the JS
heap and excludes wasm linear memory, and wasm linear memory never shrinks,
so neither is an honest stand-in. That budget is therefore left unmeasured
rather than approximated.

`gc_sim::snapshot_headroom` (#476) is where the same question is answered
NATIVELY: it measures a real rollback session's retained snapshot and
history bytes against `Omp2RollbackBudgets`'s `snapshot_bytes` and
`history_bytes`, and its `Collapsed` band exists for exactly the reason the
`retained_bytes_measured: false` flag below does -- a measurement that has
quietly stopped measuring must not read as comfortable. #476 leaves
`memory_growth_ratio` unmeasured on purpose because it is a soak quantity,
and names #472 as its owner. Exposing that band classification across the
wasm boundary so a browser soak can drive the same comparison is the shape
of the slice that would finally close it.

Also absent, and also #472's: scripted network impairment (delay, jitter,
loss) and the seed-sharded scenario matrix. Cross-engine agreement is
#473's; `--cross-engine` exists here but no workflow runs it.
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
#
# The page paces this interval against an ABSOLUTE per-iteration deadline
# rather than sleeping 50ms after each iteration's work -- see
# `online_peer.ts`'s tick loop. Sleeping after the work makes each page
# free-run at its own period, and the accumulated drift ends a run in tens
# of seconds; a long run is not possible without that fix.
DEFAULT_TICK_SLEEP_MS = 50
# The fixture match's own length, in simulated seconds. One loop iteration
# advances one 60Hz tick, so this default caps a useful run at 1,200
# iterations -- past full time the page would be ticking a finished match.
# `--duration-seconds` raises it for long runs; see `derive_duration_seconds`.
DEFAULT_DURATION_SECONDS = 20
CONNECT_TIMEOUT_SECONDS = 30
# Slack ON TOP OF the run's own nominal duration, not a total. The 90 here is
# the flat 90-second total this replaced: at the PR gate's 150 ticks x 50ms
# the nominal loop is 7.5s, so `7.5 * 2.0 + 90` makes the effective timeout
# 105s where it was 90s. That is a slightly more generous ceiling over a run
# that measures about 11s in practice, so the gate's behaviour is unchanged.
# A long run needs the nominal part to scale, or a 30-minute soak would be
# killed at 90 seconds and reported as a timeout rather than a result.
RUN_TIMEOUT_SLACK_SECONDS = 90
# The absolute-deadline pacing means a healthy loop tracks its nominal
# duration almost exactly -- measured on this repository's reference machine,
# 3,000 iterations took 150.6s against a nominal 150.0s and 36,000 took
# 1800.5s against a nominal 1800.0s, both under 0.5% over. Doubling the
# nominal is therefore not a guess at the pace but deliberate headroom for a
# runner slow enough that the loop can no longer keep its deadlines at all.
RUN_TIMEOUT_PACE_FACTOR = 2.0
TICK_HZ = 60
ERROR_MARKERS = ("GC_ONLINE_PEER|error|",)
# The only two ways this harness's tick loop is allowed to end: it ran out of
# the iterations it was asked for (`active`), or the fixture match reached
# full time (`completed`). Every other `MatchDriverStatus`
# (`confirmation_stalled`, `input_channel_failure`, `hash_mismatch`,
# `late_input`, ...) is the driver refusing to continue, which is a FAILED
# run however many boundaries agreed before it happened.
HEALTHY_FINAL_STATUSES = frozenset({"active", "completed"})
# `Omp2RollbackBudgets::memory_growth_ratio` is authored here and nowhere
# else. Read at runtime rather than restated as a constant, so a change to
# the budget cannot leave this harness silently checking the old number.
BUDGETS_SOURCE = ROOT / "rust" / "crates" / "gc-data" / "src" / "omp2_rollback_validation.rs"


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


def derive_duration_seconds(ticks: int, explicit: int | None) -> int:
    """Simulated match length for a run of `ticks` iterations.

    A run must not outlive the fixture match it is playing: one iteration
    advances at most one 60Hz tick, so `ticks / 60` simulated seconds is the
    exact floor, and the margin keeps full time beyond the last iteration
    rather than exactly on it. Runs short enough for the default 20s match
    -- the PR gate's 150 iterations, 2.5 simulated seconds, among them --
    keep that default untouched, so this returns a longer match only when a
    longer match is genuinely required. `explicit` wins when given, including
    when it is smaller: asking for a match that ends mid-run is a legitimate
    thing to test.
    """
    if explicit is not None:
        return explicit
    import math

    needed = math.ceil(ticks / TICK_HZ * 1.1) + 5
    return max(DEFAULT_DURATION_SECONDS, needed)


def derive_run_timeout_seconds(ticks: int, tick_sleep_ms: int, explicit: float | None) -> float:
    """Wall-clock budget for one peer to finish and publish its report."""
    if explicit is not None:
        return explicit
    nominal = ticks * tick_sleep_ms / 1000.0
    return nominal * RUN_TIMEOUT_PACE_FACTOR + RUN_TIMEOUT_SLACK_SECONDS


def read_memory_growth_ratio(source: Path) -> float:
    """`Omp2RollbackBudgets::memory_growth_ratio`, read from its one author.

    Fails loudly rather than falling back to a default: a silently-assumed
    ratio is exactly the "gate that cannot go red" shape AGENTS.md §9 is
    about, and a renamed field must break this harness rather than leave it
    checking a number nothing authors any more.
    """
    import re

    text = source.read_text(encoding="utf-8")
    matches = re.findall(r"memory_growth_ratio:\s*([0-9]*\.?[0-9]+)\s*,", text)
    if len(matches) != 1:
        raise RuntimeError(
            f"expected exactly one `memory_growth_ratio: <value>,` in {source}, found {len(matches)}"
        )
    return float(matches[0])


def check_report_health(report: dict[str, Any], role: str, requested_ticks: int) -> dict[str, Any]:
    """Everything the page reported that is NOT a boundary hash.

    Before this existed, `online_peer.ts` accumulated an `errors` array --
    rejected inbound frames, failed sends, star/peer errors -- published it
    in its report, and NOTHING read it. A run in which every send failed
    could still print PASS and exit 0 as long as three boundaries agreed,
    which is precisely the failure mode AGENTS.md §9 forbids ("a harness
    that prints failures and exits 0 must fail the gate anyway"). The same
    applies to a run that stopped at iteration 300 of 36,000: for a long
    run, the duration IS the property, so a short run is a failed run even
    when its checkpoints agree.
    """
    errors = report.get("errors")
    if not isinstance(errors, list):
        raise RuntimeError(f"{role} report has no errors list")
    if errors:
        raise RuntimeError(f"{role} page reported {len(errors)} runtime error(s): {errors[:5]}")

    final_status = report.get("finalStatus")
    if final_status not in HEALTHY_FINAL_STATUSES:
        raise RuntimeError(
            f"{role} driver ended in status {final_status!r} "
            f"(terminal {report.get('terminal')!r}); healthy runs end "
            f"{sorted(HEALTHY_FINAL_STATUSES)}"
        )

    if report.get("stoppedEarly"):
        raise RuntimeError(
            f"{role} tick loop stopped early after {report.get('loopIterations')} of "
            f"{requested_ticks} iterations (status {final_status!r})"
        )

    iterations = report.get("loopIterations")
    if not isinstance(iterations, int):
        raise RuntimeError(f"{role} report has no loopIterations count")
    if final_status == "active" and iterations != requested_ticks:
        raise RuntimeError(
            f"{role} completed {iterations} of the {requested_ticks} requested iterations"
        )
    # `completed` is the one legitimate way to finish short: the fixture
    # match reached full time. It is recorded rather than rejected, and
    # `--min-checkpoints` is what decides whether the run that did happen
    # still covered enough to be worth calling evidence.
    return {
        "iterations": iterations,
        "final_status": final_status,
        "ended_at_full_time": final_status == "completed",
    }


MIN_OVERLAPPING_CHECKPOINTS = 3


def compare_checkpoints(
    host_report: dict[str, Any],
    guest_report: dict[str, Any],
    min_checkpoints: int = MIN_OVERLAPPING_CHECKPOINTS,
) -> dict[str, Any]:
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
    if len(common_ticks) < min_checkpoints:
        raise RuntimeError(
            f"only {len(common_ticks)} boundary tick(s) were hashed by BOTH peers "
            f"(host hashed {len(host_by_tick)}, guest hashed {len(guest_by_tick)}) -- "
            f"need at least {min_checkpoints} for this run to prove anything; "
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


# Samples per peer below which a growth statement is not worth making.
MIN_RETENTION_SAMPLES = 6
# Non-empty byte samples a half needs before its MEDIAN means anything. Small
# on purpose: a real run gives 150-180 per half, and this floor exists only to
# stop a handful of readings being called a distribution.
MIN_BYTE_SAMPLES_PER_HALF = 3


def _mean(values: list[float]) -> float:
    return sum(values) / len(values)


def _median(values: list[int]) -> float:
    ordered = sorted(values)
    middle = len(ordered) // 2
    if len(ordered) % 2 == 1:
        return float(ordered[middle])
    return (ordered[middle - 1] + ordered[middle]) / 2.0


def check_retention(
    host_report: dict[str, Any], guest_report: dict[str, Any], ratio: float
) -> dict[str, Any]:
    """Does the retained rollback history grow with the length of the run?

    TWO quantities, with very different standing, and the difference is
    stated in the output rather than blurred.

    1. THE RETAINED WINDOW WIDTH, in ticks: `confirmed_output_tick -
       retained_floor_tick`, both from the driver's own `diagnosticsJson`.
       This is the live check. It is the driver's answer to "how much
       history am I holding behind the confirmed frontier", it is non-zero
       in a healthy run, and it must not widen as the match goes on --
       measured across a real 36,000-iteration run it sat at exactly 29
       ticks from the first sample to the last. A window that widens with
       run length IS retained history growing with run length.

    2. THE RETAINED SPECULATIVE BYTES: `rollbackAccountingJson`'s
       `retained_step_bytes`, which `gc_sim::rollback_events::accounting`
       documents as "exact logical bytes retained in the speculative event
       window". Real and exact, but a sample only observes a magnitude when
       something IS speculative at that instant, and whether it is depends
       on where the sampling instant falls relative to confirmation. Across
       three real 36,000-iteration runs: one read zero at every one of 361
       samples on both peers; one read about 291 bytes at every sample; one
       read about 292 bytes whenever it read anything, with 24 empty
       samples in its first half and 1 in its second.

       That last run is why this compares the MEDIAN OF THE NON-EMPTY
       samples in each half, not the mean of all of them. Its non-empty
       level was flat -- median 292.0 early, 292.0 late -- while the mean
       of all samples moved 253.7 -> 289.2 and "failed" a 10% ceiling.
       Nothing was growing; the empty-sample RATE had changed, and a
       statistic that moves with how often a sample lands on an empty
       window would have made this job red on a healthy match.

       WHAT THAT DELIBERATELY GIVES UP, and it is not small: growth by
       increasing FREQUENCY of non-empty windows at an unchanged magnitude
       is invisible to a median of the non-empty samples. The window-width
       check above does not backstop it either -- `retained_floor_tick`
       trails the confirmed frontier by a fixed `ROLLBACK_WINDOW_TICKS`
       capacity and sits flat whatever the occupancy. The two checks are
       independent, but NEITHER is a frequency signal, and this function
       makes no claim about that shape of growth. What is established is
       narrower than "the empty-sample rate is never a signal": on the one
       real dataset where the two disagreed, the rate was phase and the
       magnitude was flat. Occupancy IS recorded per half
       (`early_non_empty_samples`/`late_non_empty_samples`), so a later
       slice can turn it into a real check rather than rediscovering the
       gap.

       When EITHER half holds fewer than `MIN_BYTE_SAMPLES_PER_HALF`
       non-empty samples there is no median to compare, and this reports
       `retained_bytes_measured: false` rather than a verdict. That
       symmetry is deliberate; see the comment at the comparison itself for
       the false-red an asymmetric early-half fallback generates and for
       what catches the confirmation-stall signature instead.

    Neither quantity is the page's, the wasm instance's, or the browser
    process's total memory. `Omp2RollbackBudgets::memory_growth_ratio` is
    written against a whole-soak memory figure, and nothing on this
    harness's reach can produce that figure honestly today
    (`performance.memory` is the JS heap and excludes wasm linear memory;
    wasm linear memory never shrinks, so its high-water mark answers a
    different question). The budget's RATIO is borrowed as the ceiling for
    what CAN be measured exactly; the budget itself is NOT thereby
    discharged. See issue #472, and this module's docstring for how this
    relates to `gc_sim::snapshot_headroom`'s native measurement (#476).

    Both comparisons are between the run's two halves, warm-up excluded.
    The window width, which every sample observes, is compared by mean. The
    byte magnitude, which only non-empty samples observe, is compared by
    median of those -- also robust to the other end of the same quantised
    distribution, the occasional sample that catches TWO retained steps
    (533 bytes against a typical 292) and would fail a peak-to-peak ratio
    on sampling luck alone.
    """
    per_role: dict[str, Any] = {}
    measured_roles = 0
    for role, report in (("host", host_report), ("guest", guest_report)):
        samples = report.get("samples")
        if not isinstance(samples, list) or len(samples) < MIN_RETENTION_SAMPLES:
            raise RuntimeError(
                f"{role} produced {len(samples) if isinstance(samples, list) else 'no'} retention "
                f"sample(s); need at least {MIN_RETENTION_SAMPLES} to say anything about growth "
                "(lower --sample-every or raise --ticks)"
            )
        values = [int(sample["retainedStepBytes"]) for sample in samples]
        if any(value < 0 for value in values):
            raise RuntimeError(f"{role} retention sampling failed: {values}")
        # The first sample is taken before the first `advance`, so it is
        # necessarily an empty window; it says nothing and is dropped.
        body = values[1:]
        half = len(body) // 2
        early_half, late_half = body[:half], body[half:]
        early_non_empty = [value for value in early_half if value > 0]
        late_non_empty = [value for value in late_half if value > 0]

        # The driver's own retained window, in ticks. `confirmed_output_tick`
        # is -1 before the first confirmation, so the opening sample is
        # dropped here for the same reason it is dropped above.
        widths = [
            int(sample["confirmedOutputTick"]) - int(sample["retainedFloorTick"])
            for sample in samples
            if int(sample["confirmedOutputTick"]) >= 0
        ]
        if len(widths) < MIN_RETENTION_SAMPLES:
            raise RuntimeError(
                f"{role} produced only {len(widths)} usable retained-window sample(s); need at "
                f"least {MIN_RETENTION_SAMPLES}"
            )
        width_half = len(widths) // 2
        width_early = _mean(widths[:width_half])
        width_late = _mean(widths[width_half:])
        width_ceiling = width_early * (1.0 + ratio)

        # A growth statement needs a median at BOTH ends, and this is
        # symmetric on purpose. An earlier version fell back to
        # `max(early_half)` when the early half was under-sampled, so that a
        # window climbing from nothing would still fail. That is a false-red
        # generator: an early half whose sampling instants happen to catch
        # empty windows gives a baseline at or near zero, and any ordinary
        # late level then "exceeds" it. Two of the four real
        # 36,000-iteration runs behind this script read empty at essentially
        # every instant, so a run that is empty early and ordinary late is a
        # normal outcome, not a hypothetical -- and the risk GROWS as the
        # netcode improves and retained events get rarer, which is exactly
        # the wrong incentive to build into a nightly job.
        #
        # The signature that fallback existed to catch, a window climbing
        # from nothing, is the confirmation-stall signature -- and
        # `check_report_health` above catches that far more directly, by the
        # driver's own terminal status, on a real run (see its doc). Losing
        # a redundant, false-red-prone second opinion on a case that is
        # already covered by name is a good trade.
        byte_series_measured = (
            len(early_non_empty) >= MIN_BYTE_SAMPLES_PER_HALF
            and len(late_non_empty) >= MIN_BYTE_SAMPLES_PER_HALF
        )
        early_reference = _median(early_non_empty) if byte_series_measured else None
        late_median = _median(late_non_empty) if byte_series_measured else None
        ceiling = early_reference * (1.0 + ratio) if early_reference is not None else None

        per_role[role] = {
            "samples": len(values),
            "empty_samples": len(body) - len(early_non_empty) - len(late_non_empty),
            "byte_series_measured": byte_series_measured,
            "early_non_empty_samples": len(early_non_empty),
            "late_non_empty_samples": len(late_non_empty),
            "early_median_bytes": round(early_reference, 1) if early_reference is not None else None,
            "late_median_bytes": round(late_median, 1) if late_median is not None else None,
            "ceiling_bytes": round(ceiling, 1) if ceiling is not None else None,
            "peak_bytes": max(values),
            "final_bytes": values[-1],
            "window_early_mean_ticks": round(width_early, 1),
            "window_late_mean_ticks": round(width_late, 1),
            "window_ceiling_ticks": round(width_ceiling, 1),
            "window_peak_ticks": max(widths),
        }

        if width_late > width_ceiling:
            raise RuntimeError(
                f"{role} retained window widened over the run: late-half mean "
                f"{width_late:.1f} ticks exceeds the early-half mean {width_early:.1f} ticks by "
                f"more than the {ratio:.0%} memory_growth_ratio ceiling ({width_ceiling:.1f} ticks)"
            )
        if not byte_series_measured or late_median is None or ceiling is None:
            # One half or both held nothing speculative often enough to have
            # a median, so there is no byte series to bound. Recorded, and
            # never counted as a pass.
            continue
        measured_roles += 1
        if late_median > ceiling:
            raise RuntimeError(
                f"{role} retained speculative history grew beyond the budget: late-half median "
                f"{late_median:.0f} bytes exceeds the early-half median "
                f"{early_reference:.0f} bytes by more than the {ratio:.0%} memory_growth_ratio "
                f"ceiling ({ceiling:.0f} bytes)"
            )
    return {
        "memory_growth_ratio": ratio,
        # The honest headline: whether the BYTE series bounded anything at
        # all. False means the speculative window was empty every time it
        # was looked at, which is the normal outcome of a healthy paced run
        # -- see this function's doc. The retained-window width check above
        # is live either way.
        "retained_bytes_measured": measured_roles > 0,
        "roles": per_role,
    }


def format_retention(retention: dict[str, Any]) -> str:
    parts = []
    for role, data in retention["roles"].items():
        parts.append(
            f"{role} retained window: early mean {data['window_early_mean_ticks']} ticks, "
            f"late mean {data['window_late_mean_ticks']} ticks "
            f"(ceiling {data['window_ceiling_ticks']}), peak {data['window_peak_ticks']}, "
            f"over {data['samples']} samples"
        )
        if not data["byte_series_measured"]:
            parts.append(
                f"{role} speculative bytes: too few non-empty samples to state a median "
                f"({data['early_non_empty_samples']} early, {data['late_non_empty_samples']} "
                f"late, {data['empty_samples']} empty)"
            )
            continue
        parts.append(
            f"{role} speculative bytes: early median {data['early_median_bytes']}, "
            f"late median {data['late_median_bytes']} (ceiling {data['ceiling_bytes']}), "
            f"peak {data['peak_bytes']}, {data['empty_samples']} empty samples"
        )
    if not retention["retained_bytes_measured"]:
        parts.append(
            "retained-history BYTES: UNMEASURED this run -- neither peer held anything "
            "speculative often enough for a median at both ends of the run, so no byte ceiling "
            "was exercised (the retained-window width above was checked and held)"
        )
    return "; ".join(parts)


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
        duration_seconds = derive_duration_seconds(args.ticks, args.duration_seconds)
        run_timeout = derive_run_timeout_seconds(args.ticks, args.tick_sleep_ms, args.run_timeout_seconds)
        query = (
            f"ticks={args.ticks}&tick_sleep_ms={args.tick_sleep_ms}"
            f"&duration_seconds={duration_seconds}&sample_every={args.sample_every}"
        )
        print(
            f"[browser_online_peers] {args.ticks} iterations x {args.tick_sleep_ms}ms "
            f"(nominal {args.ticks * args.tick_sleep_ms / 1000.0:.1f}s), "
            f"{duration_seconds}s fixture match, sampling every {args.sample_every} iteration(s), "
            f"per-peer report timeout {run_timeout:.0f}s"
        )
        host_driver.get(f"{base_url}?role=host&{query}")
        guest_driver.get(f"{base_url}?role=guest&{query}")

        host = Peer("host", host_driver, None)
        guest = Peer("guest", guest_driver, None)

        relay_signaling(host, guest)
        release_start_barrier(host, guest)

        started = time.monotonic()
        host_report = wait_report(host, run_timeout)
        guest_report = wait_report(guest, run_timeout)
        elapsed = time.monotonic() - started
        check_no_runtime_errors(host)
        if guest_browser == "chrome":
            check_no_runtime_errors(guest)
        print(f"[browser_online_peers] both peers reported after {elapsed:.1f}s of tick loop")

        return {
            "mode": "same_engine" if host_browser == guest_browser else "cross_engine",
            "browsers": {"host": host_browser, "guest": guest_browser},
            "run": {
                "ticks": args.ticks,
                "tick_sleep_ms": args.tick_sleep_ms,
                "duration_seconds": duration_seconds,
                "sample_every": args.sample_every,
                "run_timeout_seconds": run_timeout,
                "tick_loop_seconds": round(elapsed, 1),
            },
            "host_report": host_report,
            "guest_report": guest_report,
        }
    finally:
        if host_driver is not None:
            quit_browser_bounded(host_driver)
        if guest_driver is not None:
            quit_browser_bounded(guest_driver)


def _healthy_report(role: str, ticks: int, samples: int = 8) -> dict[str, Any]:
    return {
        "role": role,
        "errors": [],
        "finalStatus": "active",
        "terminal": None,
        "stoppedEarly": False,
        "loopIterations": ticks,
        "requestedTicks": ticks,
        "checkpoints": [{"tick": tick, "hash": f"h{tick}"} for tick in range(0, ticks, 30)],
        "samples": [
            {
                "iteration": index * 30,
                "retainedStepBytes": 0 if index == 0 else 900,
                "confirmedOutputTick": index * 30 - 1,
                "retainedFloorTick": index * 30 - 30,
            }
            for index in range(samples)
        ],
    }


def _rejects(description: str, expected: str, call: Callable[[], Any]) -> None:
    """One self-test scenario: `call` MUST raise, and its message MUST name
    what went wrong. A scenario that passes, or that fails for an unrelated
    reason, fails the self-test."""
    try:
        call()
    except Exception as error:  # noqa: BLE001 -- the scenario's whole point
        if expected not in str(error):
            raise SystemExit(
                f"[self-test] {description}: rejected, but for the wrong reason.\n"
                f"  expected message containing: {expected!r}\n"
                f"  actual: {error}"
            ) from error
        print(f"[self-test] rejected as it must: {description}")
        return
    raise SystemExit(f"[self-test] {description}: WAS ACCEPTED. This gate cannot go red.")


def self_test() -> int:
    """Prove every verdict rule in this script rejects the run it is there to
    reject -- and, just as important, accepts a healthy one.

    AGENTS.md §9: "a harness self-test is not a harness run". This proves the
    RULES; only the real two-browser run below proves the netcode. The
    scheduled workflow therefore runs both, as separate steps named for what
    each actually does.
    """
    ticks = 240

    host = _healthy_report("host", ticks)
    guest = _healthy_report("guest", ticks)
    check_report_health(host, "host", ticks)
    check_report_health(guest, "guest", ticks)
    agreement = compare_checkpoints(host, guest)
    if agreement["checkpoint_count"] != len(host["checkpoints"]):
        raise SystemExit("[self-test] a healthy pair of reports was not accepted intact")
    retention = check_retention(host, guest, 0.10)
    if retention["roles"]["guest"]["late_median_bytes"] != 900:
        raise SystemExit("[self-test] a healthy retention series was misread")
    if retention["roles"]["guest"]["window_late_mean_ticks"] != 29.0:
        raise SystemExit("[self-test] a healthy retained-window series was misread")
    if not retention["retained_bytes_measured"]:
        raise SystemExit("[self-test] a non-empty byte series was not counted as measured")
    print("[self-test] accepted as it must: a healthy run")

    # The shape a real healthy run actually has: the retained window holds
    # its width, and the speculative byte window is empty every time it is
    # sampled. That must be ACCEPTED -- and must be reported as bytes
    # UNMEASURED, never as a byte ceiling that passed.
    empty_host = _healthy_report("host", ticks)
    empty_guest = _healthy_report("guest", ticks)
    for report in (empty_host, empty_guest):
        for sample in report["samples"]:
            sample["retainedStepBytes"] = 0
    empty = check_retention(empty_host, empty_guest, 0.10)
    if empty["retained_bytes_measured"]:
        raise SystemExit("[self-test] an all-empty byte series was reported as measured")
    if empty["roles"]["guest"]["byte_series_measured"]:
        raise SystemExit("[self-test] an empty speculative window was not recognised")
    if "UNMEASURED" not in format_retention(empty):
        raise SystemExit("[self-test] an unmeasured byte series was not reported as unmeasured")
    print("[self-test] accepted as it must, and reported as UNMEASURED: an empty byte series")

    # The regression a real 36,000-iteration run caught, as a COMPRESSED
    # SYNTHETIC PROXY -- 10 samples per half against that run's 180, with 4
    # empty early and 0 late against its 24 and 1. It reproduces the
    # qualitative property (a flat non-empty level under a falling
    # empty-sample rate), not the dataset: comparing means of all samples
    # called that shape 14% growth and failed a healthy match, while
    # comparing medians of the non-empty samples sees it for what it is.
    # The literal 361-sample series lives in the run artifacts, not here.
    # The two-step sample (533 against a typical 292) is included because
    # the real series had those too.
    phased = _healthy_report("guest", ticks, samples=21)
    early_shape = [0, 292, 0, 292, 0, 292, 292, 533, 0, 292]
    for index, sample in enumerate(phased["samples"]):
        if index == 0:
            continue
        position = index - 1
        sample["retainedStepBytes"] = (
            early_shape[position] if position < len(early_shape) else 292
        )
    phase = check_retention(host, phased, 0.10)
    if not phase["roles"]["guest"]["byte_series_measured"]:
        raise SystemExit("[self-test] a phased-but-flat byte series was not measured")
    if phase["roles"]["guest"]["late_median_bytes"] != 292:
        raise SystemExit("[self-test] a phased-but-flat byte series was misread")
    print(
        "[self-test] accepted as it must: a flat byte series whose empty-sample rate changed "
        "(the mean-of-all-samples statistic failed this on a real, healthy 30-minute run)"
    )

    # The false red the SYMMETRIC under-sampling rule exists to prevent, and
    # the case no real dataset here exercises: an early half that caught
    # nothing but empty windows, and an ordinary late half. An asymmetric
    # rule would take a baseline at or near zero from that early half and
    # fail every ordinary late level against it -- on a healthy run, and
    # more often the rarer retained events become. It must be ACCEPTED and
    # reported UNMEASURED.
    late_only = _healthy_report("guest", ticks, samples=21)
    for index, sample in enumerate(late_only["samples"]):
        sample["retainedStepBytes"] = 0 if index <= 10 else 292
    verdict = check_retention(host, late_only, 0.10)
    if verdict["roles"]["guest"]["byte_series_measured"]:
        raise SystemExit("[self-test] an empty early half was treated as a usable baseline")
    if verdict["roles"]["guest"]["late_non_empty_samples"] != 10:
        raise SystemExit("[self-test] per-half occupancy was not recorded")
    print(
        "[self-test] accepted as it must, and reported as UNMEASURED: an empty early half "
        "beside an ordinary late one (an asymmetric baseline would have failed this)"
    )

    widening = _healthy_report("guest", ticks, samples=9)
    for index, sample in enumerate(widening["samples"]):
        sample["retainedFloorTick"] = index * 30 - 30 - index * 10
    _rejects(
        "the driver's retained window widened as the run went on",
        "retained window widened",
        lambda: check_retention(host, widening, 0.10),
    )

    diverged = _healthy_report("guest", ticks)
    diverged["checkpoints"][3] = {"tick": 90, "hash": "WRONG"}
    _rejects(
        "one boundary hash differs between the peers",
        "diverges",
        lambda: compare_checkpoints(host, diverged),
    )

    disjoint = _healthy_report("guest", ticks)
    disjoint["checkpoints"] = [{"tick": tick + 7, "hash": "x"} for tick in range(0, ticks, 30)]
    _rejects(
        "the peers hashed no boundary in common",
        "prove anything",
        lambda: compare_checkpoints(host, disjoint),
    )

    _rejects(
        "the run reached fewer boundaries than the caller demanded",
        "prove anything",
        lambda: compare_checkpoints(host, guest, 999),
    )

    noisy = _healthy_report("host", ticks)
    noisy["errors"] = ["transport.send: channel closed"]
    _rejects(
        "the page logged runtime errors but still published a report",
        "runtime error",
        lambda: check_report_health(noisy, "host", ticks),
    )

    stalled = _healthy_report("host", ticks)
    stalled["finalStatus"] = "confirmation_stalled"
    _rejects(
        "the driver ended in a failure status",
        "healthy runs end",
        lambda: check_report_health(stalled, "host", ticks),
    )

    short = _healthy_report("host", ticks)
    short["stoppedEarly"] = True
    short["loopIterations"] = 12
    _rejects(
        "the tick loop stopped early",
        "stopped early",
        lambda: check_report_health(short, "host", ticks),
    )

    truncated = _healthy_report("host", ticks)
    truncated["loopIterations"] = ticks - 1
    _rejects(
        "the run completed fewer iterations than requested",
        "of the 240 requested iterations",
        lambda: check_report_health(truncated, "host", ticks),
    )

    growing = _healthy_report("host", ticks, samples=9)
    for index, sample in enumerate(growing["samples"]):
        sample["retainedStepBytes"] = 900 + index * 400
    _rejects(
        "retained speculative history grew past the budget ratio",
        "beyond the budget",
        lambda: check_retention(growing, guest, 0.10),
    )

    # The confirmation-stall signature -- a window climbing from nothing --
    # is NOT a byte-check rejection any more, and this scenario exists to
    # pin that handoff rather than leave it as a silent gap. The byte rule
    # reports it UNMEASURED (its early half has no median), and
    # `check_report_health` is what fails the run, by the driver's own
    # terminal status, on the real run where this actually happened.
    stalling = _healthy_report("guest", ticks, samples=9)
    for index, sample in enumerate(stalling["samples"]):
        sample["retainedStepBytes"] = 0 if index < 5 else 4900
    stalling["finalStatus"] = "confirmation_stalled"
    stalling["stoppedEarly"] = True
    stalling["loopIterations"] = 683
    if check_retention(empty_host, stalling, 0.10)["retained_bytes_measured"]:
        raise SystemExit("[self-test] a stall signature was scored off an empty early half")
    _rejects(
        "a confirmation stall (which the byte check now leaves to the health check by design)",
        "healthy runs end",
        lambda: check_report_health(stalling, "guest", ticks),
    )

    thin = _healthy_report("host", ticks, samples=2)
    _rejects(
        "too few retention samples to state a growth result",
        "say anything about growth",
        lambda: check_retention(thin, guest, 0.10),
    )

    broken = _healthy_report("host", ticks)
    broken["samples"][4]["retainedStepBytes"] = -1
    _rejects(
        "a retention sample failed to read the accounting",
        "retention sampling failed",
        lambda: check_retention(broken, guest, 0.10),
    )

    # The budget ratio is read from gc-data, not restated here. Prove both
    # that the real file still yields exactly one value, and that a source
    # which no longer authors the field is an error rather than a default.
    ratio = read_memory_growth_ratio(BUDGETS_SOURCE)
    if not 0.0 < ratio < 1.0:
        raise SystemExit(f"[self-test] implausible memory_growth_ratio read from source: {ratio}")
    print(f"[self-test] read memory_growth_ratio = {ratio} from {BUDGETS_SOURCE.name}")
    import tempfile

    with tempfile.TemporaryDirectory() as directory:
        renamed = Path(directory) / "omp2_rollback_validation.rs"
        renamed.write_text("pub struct Budgets { pub memory_headroom_ratio: f64 }", encoding="utf-8")
        _rejects(
            "the budget field was renamed out from under this harness",
            "expected exactly one",
            lambda: read_memory_growth_ratio(renamed),
        )

    # The derived long-run parameters, which are the difference between a
    # soak that runs and a soak that is killed at 90 seconds or spends its
    # second half ticking a finished match.
    soak_timeout = derive_run_timeout_seconds(36000, 50, None)
    if soak_timeout < 36000 * 0.05:
        raise SystemExit(f"[self-test] derived soak timeout {soak_timeout}s is below its own loop")
    if derive_duration_seconds(36000, None) * TICK_HZ <= 36000:
        raise SystemExit("[self-test] derived fixture match is shorter than the run itself")
    if derive_duration_seconds(150, None) != DEFAULT_DURATION_SECONDS:
        raise SystemExit("[self-test] the PR gate's 150-tick run no longer gets its 20s match")
    print("[self-test] derived run parameters hold for both the gate and the soak")

    print("[self-test] all scenarios behaved as required")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--ticks", type=int, default=DEFAULT_TICKS)
    parser.add_argument("--tick-sleep-ms", type=int, default=DEFAULT_TICK_SLEEP_MS)
    parser.add_argument(
        "--duration-seconds",
        type=int,
        default=None,
        help="Fixture match length in simulated seconds (default: long enough for --ticks).",
    )
    parser.add_argument(
        "--run-timeout-seconds",
        type=float,
        default=None,
        help="Per-peer report timeout (default: the run's nominal duration plus slack).",
    )
    parser.add_argument(
        "--sample-every",
        type=int,
        default=0,
        help="Iterations between retained-history samples; 0 disables sampling.",
    )
    parser.add_argument(
        "--min-checkpoints",
        type=int,
        default=MIN_OVERLAPPING_CHECKPOINTS,
        help="Boundaries BOTH peers must have hashed for the run to prove anything.",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Prove this script's verdict rules reject bad runs. Starts no browser.",
    )
    parser.add_argument("--skip-wasm-build", action="store_true")
    parser.add_argument("--skip-harness-build", action="store_true")
    parser.add_argument("--cross-engine", action="store_true", help="Chrome host, Firefox guest, two sessions.")
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--log-dir", type=Path, default=ROOT / "build" / "browser_online_peers-logs")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

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
        # Health first, agreement second, deliberately: a run that stopped
        # early or logged transport errors must be reported as THAT, not as
        # "N checkpoints agreed" with the rest left unsaid.
        result["health"] = {
            "host": check_report_health(result["host_report"], "host", args.ticks),
            "guest": check_report_health(result["guest_report"], "guest", args.ticks),
        }
        agreement = compare_checkpoints(
            result["host_report"], result["guest_report"], args.min_checkpoints
        )
        result["agreement"] = agreement
        if args.sample_every > 0:
            result["retention"] = check_retention(
                result["host_report"],
                result["guest_report"],
                read_memory_growth_ratio(BUDGETS_SOURCE),
            )
            print(f"[browser_online_peers] {format_retention(result['retention'])}")
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
