#!/usr/bin/env python3
"""Deterministic screenshot driver for `v2/tools/browser_match_harness`.

## Why this is committed rather than rewritten each time

That harness is the only page that runs a real match with no menus and no
pause-on-blur, so it is the right place to take visual evidence about the
renderer. It was also the only one of the three browser harnesses with no
driver script, so three separate pieces of work (#429, #435, #438) each
hand-built a throwaway CDP driver for it -- and each one discovered a
DIFFERENT way for that driver to be silently wrong:

  1. #429: a same-build A/B reported 10-31% of pixels changing at every tick.
     The control -- the same build captured twice -- differed from itself by
     the same 10-31%. `cameraFollow` is a FRAME-driven accumulator while
     captures are keyed to SIM TICK, so two sessions that reach tick N after
     different numbers of rendered frames frame the shot differently and the
     whole image shifts. The simulation was identical; the entire result was
     artefact.
  2. #435: four reloads of one build produced two ALTERNATING hashes -- a
     double-buffered swap chain read by a capture that followed a single
     draw, so the capture got whichever buffer happened to be front.
  3. #438: determinism was fine; what cost the time was finding WHICH tick to
     capture. Guessing ticks and eyeballing the result was most of the
     exercise, and three poses (`aerial_bicycle`, `combat_knockback`,
     `combat_stagger`) were never reached at all across ~22,000 ticks and
     four seeds, so they could not be verified by any amount of guessing.

None of those three is predictable from the others, which is the argument for
one committed driver that has all of them built in. Every capability below
exists because one of those runs needed it.

## What makes a capture trustworthy here

THE CONTROL IS THE LOAD-BEARING PART, and it is not optional. Every capturing
subcommand runs each build TWICE, in two independent browser processes, and
compares the two runs byte for byte. Nothing is reported unless they match.
A driver that captures without a control cannot tell a real rendering change
from camera drift, which is exactly the mistake #429 made and #435 repeated
for an unrelated reason.

The guard runs in BOTH directions, because the two failures look nothing
alike:

  * CONTROL DRIFT -- same build, two sessions, different bytes. The harness
    is not deterministic under this driver; every A/B taken against it is
    meaningless. Refused.
  * FROZEN CAPTURE -- different requested ticks inside ONE run coming back
    byte-identical, or an entire A/B whose every pair is byte-identical.
    That is the freeze-race trap: the capture never advanced. During #426 it
    was caught only by checksumming the output files, because a stale
    screenshot looks perfectly plausible. Refused.

The three determinism techniques, all enforced by `_BOOTSTRAP_JS` below:

  * THE CLOCK IS TAKEN BEFORE ANY PAGE SCRIPT RUNS, via CDP
    `Page.addScriptToEvaluateOnNewDocument`. `requestAnimationFrame` and
    `performance.now` are both replaced, so frame zero onwards advances by
    exactly one 1/60 s step per frame and no wall-clock jitter ever reaches
    the page. Freezing the loop AFTER boot -- the obvious technique, and the
    one #429 started with -- is not sufficient: the drift has already
    happened by then.
  * CAPTURES GO THROUGH CDP `Page.captureScreenshot`, never
    `canvas.toDataURL()`. The harness's `WebGLRenderer` does not set
    `preserveDrawingBuffer`, so `toDataURL` returns a blank image.
  * EVERY CAPTURE REDRAWS FIRST. `__gcDriver.hold(n)` re-runs the frame
    callback n times WITHOUT advancing the virtual clock, which is a provable
    no-op for the simulation and for every renderer-side accumulator
    (`viewState.update` and `cameraFollow.update` both short-circuit on
    `dt <= 0`), so it repaints the identical frame into the current back
    buffer. That is #435's "redraw every frame" fix in the form this page
    needs.

## Finding the frame worth capturing

`--pose` answers "at which ticks does any player hold pose X" from a single
scanning pass rather than by stepping and eyeballing. The scan is recorded
IN PAGE, one entry per tick per posed slot, by the same bootstrap that owns
the clock -- so it costs one extra property read per frame and no round
trips. Any pose can then be queried from that one pass; the scan is batched,
not per-pose.

A scan run renders at `--scan-width`/`--scan-height` with the stadium and
bloom off. That is safe because the thing being scanned is SIM state: pose
selection happens in `gc-render`'s `player_pose.rs` from the match state, and
nothing in it reads the viewport. The capture that follows re-opens the page
at full fidelity and RE-VERIFIES that the pose is actually held at that tick
before shooting, so a wrong assumption here fails loudly instead of producing
a mislabelled screenshot.

`--pose` also reports whether the posed player was ON CAMERA, from the
harness's `playerX`/`playerY`/`viewX`/`viewY`/`viewZoom` diagnostics. #438
found one `keeper_tip` in ~22,000 ticks and it was off screen, which is the
same as not finding it.

## Rare poses

`search` sweeps `?seed=` x `?bot_seed=` over many short runs looking for a
pose a single match never produces, and reports the search space it covered
whether or not it found one. "Never observed in N seed-runs of M ticks" is
evidence; "I did not happen to see it" is not.

## `--self-test` proves this file, and nothing else

Per AGENTS.md §9: a harness self-test proves the CONTROLLER's own logic --
its verdict rules, its pose table, its clock arithmetic -- and never the
system it measures. `--self-test` here starts no browser and touches no GPU.
It prints that limit rather than letting a green line read as "the renderer
is fine". The established sibling is `scripts/check_fault_harness.sh
--self-test`.

## Not a CI gate

This is a developer tool for producing visual evidence on demand. It is
deliberately absent from `scripts/check.sh` and `.github/workflows/ci.yml`:
asserting screenshots in CI is AGENTS.md §9's opt-in tier 4, and pinning
baselines is explicitly out of scope for the issue this file closes (#432).

## What it reuses

Browser lifecycle is not reimplemented here. `serve_dist`, `wait_until`,
`launch` (and the `bounded_launch` it wraps), `probe_gpu`,
`resolve_binary_pair` and `build_v2_harness` come from
`scripts/browser_render_bench.py`; `quit_browser_bounded` and
`bounded_log_tail` come from `scripts/browser_determinism.py` through it.
That is the same import direction those two files already use between
themselves. `build_v2_harness` grew three defaulted parameters so this file
can point it at a different harness directory; its existing call site is
unchanged.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from browser_determinism import bounded_log_tail, quit_browser_bounded  # noqa: E402
from browser_render_bench import (  # noqa: E402
    CONNECT_TIMEOUT_SECONDS,
    GPU_MODES,
    build_v2_harness,
    launch,
    probe_gpu,
    resolve_binary_pair,
    serve_dist,
    wait_until,
)

ROOT = Path(__file__).resolve().parents[1]
V2_TS = ROOT / "v2" / "ts"
HARNESS_DIR = ROOT / "v2" / "tools" / "browser_match_harness"
HARNESS_DIST = HARNESS_DIR / "dist"
HARNESS_VITE_CONFIG = HARNESS_DIR / "vite.config.ts"
FRAME_BUFFER_TS = V2_TS / "packages" / "render" / "src" / "frame_buffer.ts"

CANVAS_SELECTOR = "#gl-canvas"

# The harness's own fixed logical size (`match_harness.ts`: "?width=/?height=
# override it"). Captures are taken at exactly this, one drawing-buffer pixel
# per image pixel, by pinning the emulated viewport to the same numbers -- see
# `_open_page`.
DEFAULT_WIDTH = 960
DEFAULT_HEIGHT = 540

DEFAULT_SEED = 1
DEFAULT_BOT_SEED = 11

# One virtual frame, in milliseconds.
#
# WHY THE EPSILON. The page computes `elapsed = (now - lastTime) / 1000` and
# then drains a fixed-timestep accumulator against `DT = 1/60`. A bare
# 1000/60 is, in double arithmetic, very slightly SHORT of 1/60 s once
# divided back down, so an occasional frame drains zero ticks and the next
# drains two -- deterministic, but it hands `viewState`/`cameraFollow` a
# 2/60 s step on those frames and breaks the one-frame-one-tick identity this
# driver's tick addressing relies on. One picosecond of slack removes it
# without ever accumulating into a spare tick: the excess after 200,000
# frames is ~2e-7 s, four orders of magnitude below one tick.
# `_simulate_accumulator` in `--self-test` pins this.
FRAME_MS = 1000.0 / 60.0 + 1e-9

# Wire-carried closed set, mirroring `PlayerPoseId` in
# `v2/ts/packages/render/src/frame_buffer.ts` (itself numbered exactly as
# `render/player_pose.lua`). Held here so `--pose`/`search` can reject a typo
# instead of silently scanning for a pose that cannot occur; `--self-test`
# re-derives the list from that file and fails if the two have drifted.
POSE_IDS = (
    "keeper_grab",
    "keeper_throw",
    "keeper_punt",
    "keeper_tip",
    "keeper_spread",
    "keeper_central",
    "keeper_stretch",
    "keeper_dive",
    "keeper_get_up",
    "keeper_set",
    "keeper_ready_low",
    "keeper_shuffle",
    "keeper_ready_tall",
    "aerial_bicycle",
    "aerial_action",
    "combat_knockback",
    "combat_stagger",
    "combat_guard",
    "combat_active",
    "combat_windup",
    "combat_aim",
    "combat_recovery",
    "soccer_windup",
    "slide",
    "tackle",
    "stumble",
    "kick_follow",
    "settle",
    "run_telegraph",
    "contain",
    "fatigue",
    "locomotion",
)

BOOT_TIMEOUT_SECONDS = 180
# Frames pumped per `Runtime.evaluate` round trip. Large enough that stepping
# thousands of ticks is not dominated by round trips, small enough that one
# call cannot outrun the webdriver command timeout on a slow GL path.
PUMP_CHUNK_FRAMES = 120
DEFAULT_HOLD_FRAMES = 2

SELF_TEST_LIMITS = """\
--self-test exercised THIS DRIVER's own logic only: its verdict rules, its
pose table, its virtual-clock arithmetic and its bootstrap invariants. It
started no browser, opened no page, touched no GPU and rendered nothing.

A green self-test is therefore NOT evidence that the renderer is correct,
that the match harness is deterministic, or that any capture is valid. The
only thing that is evidence about the renderer is a real `capture`/`ab` run
whose mandatory control came back byte-identical."""


# ---------------------------------------------------------------------------
# The page bootstrap: installed via CDP Page.addScriptToEvaluateOnNewDocument,
# so it runs BEFORE the harness module and owns the clock from frame zero.
# ---------------------------------------------------------------------------

_BOOTSTRAP_JS = """
(function () {
  "use strict";
  if (window.__gcDriver !== undefined) { return; }

  var FRAME_MS = __FRAME_MS__;
  var virtualNow = 0;
  var nextHandle = 1;
  var pending = new Map();
  var framesRun = 0;
  var poseIndex = new Map();   // pose id -> flat [tick, slot, tick, slot, ...]
  var recordedTicks = 0;
  var lastRecordedTick = -1;
  var onCamera = new Map();    // pose id -> flat [tick, slot, ...] for in-shot holds

  // 1. THE CLOCK. Replaced before the harness module reads it, so its
  //    `lastTime = performance.now()` starts at 0 and every frame it sees is
  //    exactly one step wide. Only `performance.now` is replaced: `Date.now`
  //    is left alone because nothing on the render path reads it and
  //    freezing it can deadlock unrelated library code that polls on it.
  performance.now = function () { return virtualNow; };

  // 2. THE PUMP. rAF callbacks are queued and never self-dispatch, so the
  //    page advances only when this driver says so.
  window.requestAnimationFrame = function (cb) {
    var handle = nextHandle++;
    pending.set(handle, cb);
    return handle;
  };
  window.cancelAnimationFrame = function (handle) { pending.delete(handle); };

  function push(index, id, tick, slot) {
    var rows = index.get(id);
    if (rows === undefined) { rows = []; index.set(id, rows); }
    rows.push(tick, slot);
  }

  function record() {
    var stats = window.__gcMatchHarness;
    if (stats === undefined) { return; }
    var tick = stats.tick;
    if (tick === lastRecordedTick) { return; }
    lastRecordedTick = tick;
    recordedTicks += 1;
    var poses = stats.poses || [];
    var zoom = stats.viewZoom;
    var halfW = zoom ? stats.fieldW / (2 * zoom) : 0;
    var halfH = zoom ? stats.fieldH / (2 * zoom) : 0;
    for (var i = 0; i < poses.length; i += 1) {
      var id = poses[i];
      if (id === undefined || id === null) { continue; }
      push(poseIndex, id, tick, i + 1);
      // "On camera" is deliberately the crude test: the follow view frames a
      // field.w/(2*zoom) x field.h/(2*zoom) rectangle about its focus, so a
      // player inside that rectangle is in shot. Ranking candidates, not
      // asserting anything -- the capture itself is the evidence.
      if (zoom && stats.viewX !== null && stats.viewY !== null) {
        var dx = Math.abs((stats.playerX[i] || 0) - stats.viewX);
        var dy = Math.abs((stats.playerY[i] || 0) - stats.viewY);
        if (dx <= halfW && dy <= halfH) { push(onCamera, id, tick, i + 1); }
      }
    }
  }

  function runFrame(advance) {
    if (advance) { virtualNow += FRAME_MS; }
    if (pending.size === 0) { return 0; }
    var due = [];
    pending.forEach(function (cb) { due.push(cb); });
    pending.clear();
    for (var i = 0; i < due.length; i += 1) { due[i](virtualNow); }
    if (advance) { framesRun += 1; record(); }
    return due.length;
  }

  function hits(index, id, limit) {
    var rows = index.get(id) || [];
    var out = [];
    for (var i = 0; i < rows.length && out.length < limit; i += 2) {
      out.push([rows[i], rows[i + 1]]);
    }
    return out;
  }

  window.__gcDriver = {
    version: 1,
    frameMs: FRAME_MS,

    state: function () {
      var s = window.__gcMatchHarness || {};
      return {
        booted: pending.size > 0 || framesRun > 0,
        status: s.status === undefined ? null : s.status,
        error: s.error === undefined ? null : s.error,
        tick: s.tick === undefined ? null : s.tick,
        score: s.score === undefined ? null : s.score,
        ticks_last_frame: s.ticksLastFrame === undefined ? null : s.ticksLastFrame,
        draw_calls: s.drawCalls === undefined ? null : s.drawCalls,
        frames: framesRun,
        recorded_ticks: recordedTicks,
        virtual_ms: virtualNow,
        poses: s.poses || [],
        view_zoom: s.viewZoom === undefined ? null : s.viewZoom
      };
    },

    // Advance `frames` virtual frames. One frame is one sim tick; the caller
    // checks that identity rather than assuming it.
    step: function (frames) {
      for (var i = 0; i < frames; i += 1) { runFrame(true); }
      return this.state();
    },

    // Repaint without advancing anything. `dt` reaches the page as 0, which
    // `viewState.update`/`cameraFollow.update` both short-circuit on, so the
    // frame drawn is identical to the one already there -- this exists only
    // to put fresh pixels in the back buffer for `Page.captureScreenshot`.
    hold: function (frames) {
      for (var i = 0; i < frames; i += 1) { runFrame(false); }
      return this.state();
    },

    // Every pose seen so far, from the single recording pass.
    poseSummary: function () {
      var out = {};
      poseIndex.forEach(function (rows, id) {
        var seen = onCamera.get(id) || [];
        out[id] = {
          holds: rows.length / 2,
          first_tick: rows[0],
          last_tick: rows[rows.length - 2],
          on_camera_holds: seen.length / 2,
          first_on_camera_tick: seen.length > 0 ? seen[0] : null
        };
      });
      return { recorded_ticks: recordedTicks, poses: out };
    },

    poseHits: function (id, limit, requireOnCamera) {
      return hits(requireOnCamera ? onCamera : poseIndex, id, limit);
    },

    // The DOM readout overlays the canvas and its text changes every tick,
    // so it would dominate any pixel comparison. It is not part of the
    // rendered frame -- `#stats` is a <div>, not the WebGL canvas.
    hideOverlay: function () {
      var el = document.getElementById("stats");
      if (el !== null) { el.style.visibility = "hidden"; }
      return el !== null;
    },

    canvasRect: function () {
      var el = document.querySelector("__CANVAS__");
      if (el === null) { return null; }
      var r = el.getBoundingClientRect();
      return { x: r.x, y: r.y, width: r.width, height: r.height,
               buffer_width: el.width, buffer_height: el.height };
    }
  };
})();
"""


def bootstrap_source(frame_ms: float = FRAME_MS) -> str:
    """The pre-page script, with the frame step baked in.

    Kept a function so `--self-test` can assert the invariants that make it
    work without a browser: if someone deletes the `requestAnimationFrame` or
    `performance.now` replacement, #429's camera drift comes straight back and
    every capture silently becomes artefact again.
    """
    return _BOOTSTRAP_JS.replace("__FRAME_MS__", repr(frame_ms)).replace("__CANVAS__", CANVAS_SELECTOR)


# ---------------------------------------------------------------------------
# Pure logic. Everything here is exercised by --self-test.
# ---------------------------------------------------------------------------


def digest(image: bytes) -> str:
    return hashlib.sha256(image).hexdigest()


def control_verdict(run_a: dict[int, str], run_b: dict[int, str]) -> dict[str, Any]:
    """Same build, two independent sessions, same tick set. Must match byte
    for byte at every tick or nothing downstream means anything."""
    if sorted(run_a) != sorted(run_b):
        return {
            "ok": False,
            "reason": "the two control runs captured different tick sets",
            "mismatched_ticks": sorted(set(run_a) ^ set(run_b)),
        }
    if not run_a:
        return {"ok": False, "reason": "the control captured no ticks at all", "mismatched_ticks": []}
    mismatched = [tick for tick in sorted(run_a) if run_a[tick] != run_b[tick]]
    if mismatched:
        return {
            "ok": False,
            "reason": (
                "same build, two sessions, different pixels: the harness is not "
                "deterministic under this driver, so any A/B taken against it is "
                "artefact (this is #429's and #435's failure)"
            ),
            "mismatched_ticks": mismatched,
        }
    return {"ok": True, "reason": "byte-identical across two sessions at every tick", "mismatched_ticks": []}


def frozen_capture_verdict(run: dict[int, str]) -> dict[str, Any]:
    """Distinct ticks inside ONE run must produce distinct images. Two
    different ticks with the same bytes means the capture never advanced --
    the freeze race, which looks exactly like a valid screenshot."""
    by_hash: dict[str, list[int]] = {}
    for tick in sorted(run):
        by_hash.setdefault(run[tick], []).append(tick)
    collisions = [ticks for ticks in by_hash.values() if len(ticks) > 1]
    if collisions:
        return {
            "ok": False,
            "reason": (
                "different ticks captured byte-identical images: the loop did not "
                "advance between captures (the freeze race)"
            ),
            "collisions": collisions,
        }
    return {"ok": True, "reason": "every requested tick produced distinct pixels", "collisions": []}


def ab_verdict(build_a: dict[int, str], build_b: dict[int, str]) -> dict[str, Any]:
    """Before/after across two builds. An A/B where EVERY pair is identical is
    refused: either the capture never advanced, or the two builds were the
    same tree. A pair that is identical at SOME ticks is reported, not
    refused -- a real rendering change legitimately leaves unaffected frames
    alone, and hiding that would be its own dishonesty."""
    if sorted(build_a) != sorted(build_b):
        return {
            "ok": False,
            "reason": "the two builds captured different tick sets",
            "identical_ticks": [],
            "changed_ticks": [],
        }
    if not build_a:
        return {"ok": False, "reason": "no ticks were captured", "identical_ticks": [], "changed_ticks": []}
    identical = [tick for tick in sorted(build_a) if build_a[tick] == build_b[tick]]
    changed = [tick for tick in sorted(build_a) if build_a[tick] != build_b[tick]]
    if not changed:
        return {
            "ok": False,
            "reason": (
                "every before/after pair is byte-identical: either the capture did "
                "not advance, or the two builds render this scene identically -- "
                "refusing to report it as an A/B either way"
            ),
            "identical_ticks": identical,
            "changed_ticks": [],
        }
    return {
        "ok": True,
        "reason": f"{len(changed)} of {len(build_a)} captured ticks differ between the builds",
        "identical_ticks": identical,
        "changed_ticks": changed,
    }


def pick_pose_ticks(summary: dict[str, Any], pose: str, count: int, require_on_camera: bool) -> dict[str, Any]:
    """Turn one scan pass into a verdict about a pose, without a browser."""
    poses = summary.get("poses", {})
    entry = poses.get(pose)
    scanned = int(summary.get("recorded_ticks", 0))
    if entry is None:
        return {
            "found": False,
            "reason": f"{pose} was not held by any player in {scanned} scanned ticks",
            "holds": 0,
            "on_camera_holds": 0,
        }
    holds = int(entry.get("holds", 0))
    on_camera = int(entry.get("on_camera_holds", 0))
    if require_on_camera and on_camera == 0:
        return {
            "found": False,
            "reason": (
                f"{pose} was held {holds} time(s) in {scanned} scanned ticks but never "
                f"inside the follow camera's frame"
            ),
            "holds": holds,
            "on_camera_holds": 0,
        }
    return {
        "found": True,
        "reason": f"{pose} held {holds} time(s) ({on_camera} on camera) in {scanned} scanned ticks",
        "holds": holds,
        "on_camera_holds": on_camera,
        "want": count,
    }


def summarize_search(rows: list[dict[str, Any]], pose: str) -> dict[str, Any]:
    """Aggregate a seed sweep into a statement that is honest when it fails.

    "Not found" is only worth anything alongside the space that was covered,
    which is the whole point of #438's note that three poses were "never
    observed" -- without the sweep size that sentence says nothing.
    """
    scanned_ticks = sum(int(row.get("scanned_ticks", 0)) for row in rows)
    hits = [row for row in rows if int(row.get("holds", 0)) > 0]
    on_camera = [row for row in rows if int(row.get("on_camera_holds", 0)) > 0]
    return {
        "pose": pose,
        "runs": len(rows),
        "scanned_ticks": scanned_ticks,
        "runs_with_pose": len(hits),
        "runs_with_pose_on_camera": len(on_camera),
        "found": bool(hits),
        "found_on_camera": bool(on_camera),
        "first_hit": hits[0] if hits else None,
        "first_on_camera_hit": on_camera[0] if on_camera else None,
        "seeds": sorted({int(row["seed"]) for row in rows}),
        "bot_seeds": sorted({int(row["bot_seed"]) for row in rows}),
    }


def parse_int_list(text: str, what: str) -> list[int]:
    """`1,3,5` and `1-8` and `1-8:2` (a step), mixed freely."""
    values: list[int] = []
    for chunk in text.split(","):
        chunk = chunk.strip()
        if not chunk:
            continue
        if "-" in chunk.lstrip("-"):
            span, _, step_text = chunk.partition(":")
            low_text, _, high_text = span.partition("-")
            try:
                low, high = int(low_text), int(high_text)
                step = int(step_text) if step_text else 1
            except ValueError as error:
                raise ValueError(f"bad {what} range {chunk!r}: {error}") from error
            if step <= 0 or high < low:
                raise ValueError(f"bad {what} range {chunk!r}")
            values.extend(range(low, high + 1, step))
            continue
        try:
            values.append(int(chunk))
        except ValueError as error:
            raise ValueError(f"bad {what} value {chunk!r}: {error}") from error
    if not values:
        raise ValueError(f"no {what} values in {text!r}")
    # De-duplicate while keeping the caller's order, so a sweep is reported in
    # the order it was asked for.
    seen: set[int] = set()
    ordered: list[int] = []
    for value in values:
        if value not in seen:
            seen.add(value)
            ordered.append(value)
    return ordered


# ---------------------------------------------------------------------------
# Page control
# ---------------------------------------------------------------------------


def page_extras(args: argparse.Namespace, extra: dict[str, Any] | None = None) -> dict[str, Any]:
    """Query params every session in a run must share.

    `?combat=1` in particular has to be identical between a scan and the
    capture that follows it, and between the two halves of a control -- a
    session with the combat layer on is a different match, so mixing them
    would silently compare two different simulations.
    """
    params: dict[str, Any] = {}
    if getattr(args, "combat", False):
        params["combat"] = 1
    params.update(extra or {})
    return params


def _harness_url(base_url: str, params: dict[str, Any]) -> str:
    query = "&".join(f"{key}={value}" for key, value in params.items())
    return f"{base_url}?{query}"


def driver_state(driver: Any) -> dict[str, Any]:
    value = driver.execute_script("return window.__gcDriver ? window.__gcDriver.state() : null;")
    return value if isinstance(value, dict) else {}


def install_bootstrap(driver: Any) -> None:
    """Register the pre-page script ONCE per browser process. It applies to
    every document created afterwards, so a `search` sweep can reuse the same
    browser across seeds and each navigation still gets a virgin clock."""
    driver.execute_cdp_cmd("Page.addScriptToEvaluateOnNewDocument", {"source": bootstrap_source()})
    driver.execute_cdp_cmd("Emulation.setScrollbarsHidden", {"hidden": True})


def _open_page(
    driver: Any,
    base_url: str,
    *,
    seed: int,
    bot_seed: int,
    width: int,
    height: int,
    extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Navigate, wait for the match to be live, and pin the geometry.

    The emulated viewport is set to the harness's own logical size so the
    canvas's CSS box, its drawing buffer and the captured image are all the
    same pixels -- `match_harness.ts` fits the canvas to the window and picks
    its pixel ratio from it, so any other window size means capturing an
    upscaled image whose contents depend on the machine's display.
    """
    driver.execute_cdp_cmd(
        "Emulation.setDeviceMetricsOverride",
        {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": False},
    )
    params: dict[str, Any] = {"seed": seed, "bot_seed": bot_seed, "width": width, "height": height, "ratio": 1}
    params.update(extra or {})
    driver.get(_harness_url(base_url, params))
    wait_until(lambda: driver_state(driver).get("booted"), BOOT_TIMEOUT_SECONDS, "match harness boot")
    state = driver_state(driver)
    if state.get("status") == "error":
        raise RuntimeError(f"match harness reported an error on boot: {state.get('error')}")
    driver.execute_script("window.__gcDriver.hideOverlay();")
    rect = driver.execute_script("return window.__gcDriver.canvasRect();")
    if not isinstance(rect, dict):
        raise RuntimeError(f"{CANVAS_SELECTOR} is missing from the harness page")
    if int(rect["buffer_width"]) != width or int(rect["buffer_height"]) != height:
        raise RuntimeError(
            f"drawing buffer is {rect['buffer_width']}x{rect['buffer_height']}, expected {width}x{height}: "
            f"the capture would not be one buffer pixel per image pixel"
        )
    return {"state": state, "rect": rect}


def step_to_tick(driver: Any, target: int) -> dict[str, Any]:
    """Advance until the page's own `tick` reaches `target`.

    Polls the page's tick rather than counting frames: the one-frame-one-tick
    identity is CHECKED here, not assumed, so a change to the harness's
    timestep policy fails loudly instead of quietly mis-addressing captures.
    """
    state = driver_state(driver)
    guard = 0
    while int(state.get("tick") or 0) < target:
        remaining = target - int(state.get("tick") or 0)
        state = driver.execute_script(
            "return window.__gcDriver.step(arguments[0]);", min(remaining, PUMP_CHUNK_FRAMES)
        )
        guard += 1
        if guard > (target // PUMP_CHUNK_FRAMES) + 64:
            raise RuntimeError(f"the harness stopped advancing before tick {target} (at {state.get('tick')})")
        if state.get("status") == "finished":
            break
    if int(state.get("tick") or 0) != target:
        raise RuntimeError(
            f"asked for tick {target}, the harness is at {state.get('tick')} "
            f"(status {state.get('status')!r}) -- captures would be mislabelled"
        )
    if state.get("frames") != state.get("tick"):
        raise RuntimeError(
            f"one frame is no longer one tick ({state.get('frames')} frames, {state.get('tick')} ticks): "
            f"this driver's tick addressing and FRAME_MS assume it, so refusing to capture"
        )
    return state


def capture(driver: Any, rect: dict[str, Any], hold_frames: int) -> bytes:
    """Repaint, then read the composited surface.

    `Page.captureScreenshot`, never `canvas.toDataURL()`: the harness's
    renderer does not set `preserveDrawingBuffer`, so `toDataURL` comes back
    blank. The repaint is #435's fix -- without a fresh draw the capture can
    read whichever buffer of a double-buffered chain happens to be front.
    """
    driver.execute_script("return window.__gcDriver.hold(arguments[0]);", hold_frames)
    shot = driver.execute_cdp_cmd(
        "Page.captureScreenshot",
        {
            "format": "png",
            "clip": {
                "x": float(rect["x"]),
                "y": float(rect["y"]),
                "width": float(rect["width"]),
                "height": float(rect["height"]),
                "scale": 1,
            },
            "captureBeyondViewport": False,
            "fromSurface": True,
        },
    )
    return base64.b64decode(shot["data"])


# ---------------------------------------------------------------------------
# Sessions
# ---------------------------------------------------------------------------


class BrowserSlot:
    """One browser process, torn down on the way out.

    Launch and teardown are `browser_render_bench.launch` and
    `browser_determinism.quit_browser_bounded` unchanged -- this only holds
    them together so a `with` block cannot leak a driver on an exception.
    """

    def __init__(self, args: argparse.Namespace, log: Path, label: str) -> None:
        self.args = args
        self.log = log
        self.label = label
        self.driver: Any = None

    def __enter__(self) -> Any:
        binary, driver_path = resolve_binary_pair("chrome", self.args)
        if not binary.is_file():
            raise RuntimeError(f"chrome binary not found at {binary}")
        if not driver_path.is_file():
            raise RuntimeError(f"chromedriver not found at {driver_path}")
        self.log.parent.mkdir(parents=True, exist_ok=True)
        self.driver = launch("chrome", binary, driver_path, self.log, self.args.gpu_mode, None, CONNECT_TIMEOUT_SECONDS)
        install_bootstrap(self.driver)
        return self.driver

    def __exit__(self, exc_type: Any, *_rest: Any) -> None:
        # `_FoundIt` is this file's own "stop sweeping, we have what we came
        # for" signal, not a failure, so it gets no diagnostic dump.
        if exc_type is not None and exc_type is not _FoundIt and self.log.is_file():
            # The chromedriver log is the only place a GL/EGL failure says
            # anything useful, and it is deleted with the run otherwise.
            print(f"[browser_match_harness] {self.label} webdriver log tail:\n{bounded_log_tail(self.log)}")
        if self.driver is not None:
            quit_browser_bounded(self.driver)


def capture_run(
    args: argparse.Namespace,
    base_url: str,
    ticks: list[int],
    out_dir: Path,
    label: str,
    *,
    seed: int | None = None,
) -> dict[str, Any]:
    """One full-fidelity session: boot, step to each tick in order, capture."""
    out_dir.mkdir(parents=True, exist_ok=True)
    log = Path(args.log_dir) / f"{label}-webdriver.log"
    images: dict[int, str] = {}
    files: dict[int, str] = {}
    poses_at: dict[int, list[str]] = {}
    with BrowserSlot(args, log, label) as driver:
        opened = _open_page(
            driver,
            base_url,
            seed=seed if seed is not None else args.seed,
            bot_seed=args.bot_seed,
            width=args.width,
            height=args.height,
            extra=page_extras(args),
        )
        gpu = probe_gpu(driver, CANVAS_SELECTOR)
        for tick in ticks:
            state = step_to_tick(driver, tick)
            image = capture(driver, opened["rect"], args.hold_frames)
            path = out_dir / f"tick-{tick:06d}.png"
            path.write_bytes(image)
            images[tick] = digest(image)
            files[tick] = str(path)
            poses_at[tick] = [pose for pose in (state.get("poses") or []) if pose]
        final = driver_state(driver)
    return {
        "label": label,
        "seed": seed if seed is not None else args.seed,
        "bot_seed": args.bot_seed,
        "hashes": images,
        "files": files,
        "poses_at": poses_at,
        "gpu": gpu,
        "final_state": {key: final.get(key) for key in ("tick", "score", "status", "frames", "draw_calls")},
    }


def scan_run(
    args: argparse.Namespace,
    base_url: str,
    *,
    seed: int,
    bot_seed: int,
    scan_ticks: int,
    driver: Any | None = None,
    hits_for: str | None = None,
    hits_limit: int = 64,
    hits_on_camera: bool = False,
) -> dict[str, Any]:
    """One low-cost scanning session: step `scan_ticks` and read the index.

    Renders small, with the stadium and bloom off. Pose selection is SIM
    state (`gc-render`'s `player_pose.rs` reads the match, never the
    viewport), so what this finds is exactly what a full-fidelity run would
    find at the same seed -- and the capture that follows re-verifies it.
    """

    def scan_with(active: Any) -> dict[str, Any]:
        _open_page(
            active,
            base_url,
            seed=seed,
            bot_seed=bot_seed,
            width=args.scan_width,
            height=args.scan_height,
            extra=page_extras(args, {"bloom": 0} if args.scan_no_bloom else {}),
        )
        remaining = scan_ticks
        state: dict[str, Any] = {}
        while remaining > 0:
            chunk = min(remaining, PUMP_CHUNK_FRAMES)
            state = active.execute_script("return window.__gcDriver.step(arguments[0]);", chunk)
            remaining -= chunk
            if state.get("status") in ("finished", "error"):
                break
        summary = active.execute_script("return window.__gcDriver.poseSummary();")
        # Read the hit list from THIS session rather than making the caller
        # re-scan for it: one pass answers both "does this pose occur" and
        # "at which ticks", which is the point of recording the index in
        # page instead of stepping and polling.
        hits: list[list[int]] = []
        if hits_for is not None:
            hits = active.execute_script(
                "return window.__gcDriver.poseHits(arguments[0], arguments[1], arguments[2]);",
                hits_for,
                hits_limit,
                bool(hits_on_camera),
            ) or []
        return {"summary": summary, "state": state, "hits": hits}

    if driver is not None:
        return scan_with(driver)
    log = Path(args.log_dir) / f"scan-{seed}-{bot_seed}-webdriver.log"
    with BrowserSlot(args, log, f"scan seed={seed}") as fresh:
        return scan_with(fresh)


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def emit(report: dict[str, Any], out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / "report.json"
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"[browser_match_harness] report: {path}")


def print_guard(name: str, verdict: dict[str, Any]) -> None:
    mark = "ok  " if verdict["ok"] else "FAIL"
    print(f"[browser_match_harness] {mark} {name}: {verdict['reason']}")


# ---------------------------------------------------------------------------
# Builds
# ---------------------------------------------------------------------------


def build_here(skip_wasm_build: bool) -> Path:
    build_v2_harness(
        skip_wasm_build,
        vite_config=HARNESS_VITE_CONFIG,
        dist=HARNESS_DIST,
        label="v2 live-match harness",
    )
    return HARNESS_DIST


def build_revision(revision: str, skip_wasm_build: bool, work_root: Path) -> Path:
    """Build the harness from another git revision, in its own worktree.

    The A/B this file exists to support is usually "main versus this branch",
    and a driver that could only compare two seeds would not answer it.
    Kept out of the way under `build/`, and reused if already present so a
    repeated A/B does not rebuild wasm every time.
    """
    work_root.mkdir(parents=True, exist_ok=True)
    slug = re.sub(r"[^A-Za-z0-9_.-]", "_", revision)
    tree = work_root / slug
    if not (tree / ".git").exists():
        print(f"[browser_match_harness] git worktree add --detach {tree} {revision}")
        subprocess.run(["git", "worktree", "add", "--detach", str(tree), revision], cwd=ROOT, check=True)
    ts = tree / "v2" / "ts"
    print(f"[browser_match_harness] pnpm install --frozen-lockfile ({slug})")
    subprocess.run(["pnpm", "install", "--frozen-lockfile"], cwd=ts, check=True)
    if not skip_wasm_build:
        print(f"[browser_match_harness] node packages/wasm/scripts/build_web.mjs ({slug})")
        subprocess.run(["node", str(ts / "packages" / "wasm" / "scripts" / "build_web.mjs")], cwd=ts, check=True)
    config = tree / "v2" / "tools" / "browser_match_harness" / "vite.config.ts"
    print(f"[browser_match_harness] pnpm exec vite build ({slug})")
    subprocess.run(["pnpm", "exec", "vite", "build", "--config", str(config)], cwd=ts, check=True)
    dist = tree / "v2" / "tools" / "browser_match_harness" / "dist"
    if not (dist / "index.html").is_file():
        raise RuntimeError(f"building {revision} did not produce {dist / 'index.html'}")
    return dist


def resolve_build(spec_dist: Path | None, spec_rev: str | None, args: argparse.Namespace, work_root: Path) -> Path:
    if spec_dist is not None:
        if not (spec_dist / "index.html").is_file():
            raise RuntimeError(f"{spec_dist} is not a built harness (no index.html)")
        return spec_dist
    if spec_rev is not None:
        return build_revision(spec_rev, args.skip_wasm_build, work_root)
    return build_here(args.skip_wasm_build) if not args.skip_harness_build else HARNESS_DIST


# ---------------------------------------------------------------------------
# Subcommands
# ---------------------------------------------------------------------------


def resolve_ticks(args: argparse.Namespace, base_url: str) -> tuple[list[int], dict[str, Any] | None]:
    """Either the ticks the caller named, or the ticks a pose scan found."""
    if args.pose is None:
        return parse_int_list(args.ticks, "tick"), None
    if args.pose not in POSE_IDS:
        raise RuntimeError(f"{args.pose!r} is not a pose id; see PoseId in {FRAME_BUFFER_TS}")
    print(f"[browser_match_harness] scanning {args.scan_ticks} ticks for pose {args.pose}")
    # ONE scanning session answers both halves. The index is recorded in page
    # as the scan runs, so "does this pose occur" and "at exactly which ticks"
    # come back from the same pass -- the batched scan #438 asked for, rather
    # than a second run to locate what the first one counted. The spacing
    # filter below discards most of a hold's adjacent ticks, so the hit limit
    # is sized to still satisfy `--pose-count` at the requested spacing.
    scan = scan_run(
        args,
        base_url,
        seed=args.seed,
        bot_seed=args.bot_seed,
        scan_ticks=args.scan_ticks,
        hits_for=args.pose,
        hits_limit=max(args.pose_count * max(args.pose_spacing, 1) * 4, 64),
        hits_on_camera=bool(args.pose_on_camera),
    )
    verdict = pick_pose_ticks(scan["summary"], args.pose, args.pose_count, args.pose_on_camera)
    print(f"[browser_match_harness] scan: {verdict['reason']}")
    if not verdict["found"]:
        raise RuntimeError(verdict["reason"])
    hits = scan["hits"]
    ticks: list[int] = []
    for row in hits or []:
        tick = int(row[0])
        # Adjacent ticks of one hold look the same; spread the picks so the
        # captures are actually different frames rather than four copies.
        if ticks and tick - ticks[-1] < args.pose_spacing:
            continue
        ticks.append(tick)
        if len(ticks) >= args.pose_count:
            break
    if not ticks:
        raise RuntimeError(f"no {args.pose} hits survived the spacing filter")
    print(f"[browser_match_harness] capturing {args.pose} at ticks {ticks}")
    return ticks, {"pose": args.pose, "verdict": verdict, "hits": hits}


def command_capture(args: argparse.Namespace) -> int:
    out_dir = Path(args.out)
    dist = resolve_build(args.dist, args.rev, args, Path(args.build_root))
    server, _thread, base_url = serve_dist(dist)
    try:
        ticks, pose_scan = resolve_ticks(args, base_url)
        run_a = capture_run(args, base_url, ticks, out_dir / "run-1", "run-1")
        run_b = capture_run(args, base_url, ticks, out_dir / "run-2", "run-2")
    finally:
        server.shutdown()

    control = control_verdict(run_a["hashes"], run_b["hashes"])
    frozen = frozen_capture_verdict(run_a["hashes"]) if len(ticks) > 1 else {
        "ok": True,
        "reason": "only one tick requested, so there is nothing to compare within the run",
        "collisions": [],
    }
    report = {
        "kind": "capture",
        "dist": str(dist),
        "ticks": ticks,
        "pose_scan": pose_scan,
        "runs": [run_a, run_b],
        "control": control,
        "frozen_capture": frozen,
        "verdict": "ok" if control["ok"] and frozen["ok"] else "refused",
    }
    emit(report, out_dir)
    print_guard("control (two sessions, same build)", control)
    print_guard("frozen-capture guard", frozen)
    if report["verdict"] != "ok":
        print("[browser_match_harness] REFUSED: the captures in this directory are not evidence.")
        return 1
    for tick in ticks:
        print(f"[browser_match_harness] tick {tick}: {run_a['files'][tick]}  sha256={run_a['hashes'][tick][:16]}")
    return 0


def command_ab(args: argparse.Namespace) -> int:
    out_dir = Path(args.out)
    work_root = Path(args.build_root)
    dist_a = resolve_build(args.dist_a, args.rev_a, args, work_root)
    dist_b = resolve_build(args.dist_b, args.rev_b, args, work_root)
    if dist_a == dist_b and args.seed_a == args.seed_b:
        raise RuntimeError("A and B are the same build and the same seed: that is a control, not an A/B")

    server_a, _ta, url_a = serve_dist(dist_a)
    server_b, _tb, url_b = serve_dist(dist_b)
    try:
        ticks = parse_int_list(args.ticks, "tick")
        a1 = capture_run(args, url_a, ticks, out_dir / "build-a" / "run-1", "a-run-1", seed=args.seed_a)
        a2 = capture_run(args, url_a, ticks, out_dir / "build-a" / "run-2", "a-run-2", seed=args.seed_a)
        b1 = capture_run(args, url_b, ticks, out_dir / "build-b" / "run-1", "b-run-1", seed=args.seed_b)
        b2 = capture_run(args, url_b, ticks, out_dir / "build-b" / "run-2", "b-run-2", seed=args.seed_b)
    finally:
        server_a.shutdown()
        server_b.shutdown()

    control_a = control_verdict(a1["hashes"], a2["hashes"])
    control_b = control_verdict(b1["hashes"], b2["hashes"])
    frozen_a = frozen_capture_verdict(a1["hashes"])
    frozen_b = frozen_capture_verdict(b1["hashes"])
    controls_ok = control_a["ok"] and control_b["ok"] and frozen_a["ok"] and frozen_b["ok"]

    # The A/B comparison is COMPUTED but WITHHELD when a control failed: a
    # difference measured against a build that is not deterministic under
    # this driver is not a rendering result, and printing it anyway is how
    # #429's artefact got believed for a day.
    comparison = ab_verdict(a1["hashes"], b1["hashes"])
    report: dict[str, Any] = {
        "kind": "ab",
        "build_a": {"dist": str(dist_a), "rev": args.rev_a, "seed": args.seed_a},
        "build_b": {"dist": str(dist_b), "rev": args.rev_b, "seed": args.seed_b},
        "ticks": ticks,
        "runs": {"a1": a1, "a2": a2, "b1": b1, "b2": b2},
        "control_a": control_a,
        "control_b": control_b,
        "frozen_capture_a": frozen_a,
        "frozen_capture_b": frozen_b,
        "comparison": comparison if controls_ok else None,
        "comparison_withheld": None if controls_ok else "a control failed; see control_a/control_b",
        "verdict": "ok" if controls_ok and comparison["ok"] else "refused",
    }
    emit(report, out_dir)
    print_guard("control A (two sessions, build A)", control_a)
    print_guard("control B (two sessions, build B)", control_b)
    print_guard("frozen-capture guard, build A", frozen_a)
    print_guard("frozen-capture guard, build B", frozen_b)
    if not controls_ok:
        print("[browser_match_harness] REFUSED: a control failed, so the A/B result is withheld entirely.")
        return 1
    print_guard("A/B", comparison)
    if not comparison["ok"]:
        return 1
    print(f"[browser_match_harness] changed ticks: {comparison['changed_ticks']}")
    print(f"[browser_match_harness] unchanged ticks: {comparison['identical_ticks']}")
    return 0


def command_scan(args: argparse.Namespace) -> int:
    dist = resolve_build(args.dist, args.rev, args, Path(args.build_root))
    server, _thread, base_url = serve_dist(dist)
    try:
        scan = scan_run(args, base_url, seed=args.seed, bot_seed=args.bot_seed, scan_ticks=args.scan_ticks)
    finally:
        server.shutdown()
    summary = scan["summary"]
    rows = sorted(summary["poses"].items(), key=lambda item: -int(item[1]["holds"]))
    print(f"[browser_match_harness] seed={args.seed} bot_seed={args.bot_seed} "
          f"scanned {summary['recorded_ticks']} ticks")
    print(f"{'pose':<20}{'holds':>8}{'on camera':>11}{'first tick':>12}{'first on cam':>14}")
    for pose, entry in rows:
        print(f"{pose:<20}{entry['holds']:>8}{entry['on_camera_holds']:>11}"
              f"{entry['first_tick']:>12}{str(entry['first_on_camera_tick']):>14}")
    missing = [pose for pose in POSE_IDS if pose not in summary["poses"]]
    print(f"[browser_match_harness] never seen in this run: {', '.join(missing) if missing else '(none)'}")
    if args.out is not None:
        emit({"kind": "scan", "seed": args.seed, "bot_seed": args.bot_seed, "summary": summary}, Path(args.out))
    return 0


def command_search(args: argparse.Namespace) -> int:
    if args.pose not in POSE_IDS:
        raise RuntimeError(f"{args.pose!r} is not a pose id; see PoseId in {FRAME_BUFFER_TS}")
    seeds = parse_int_list(args.seeds, "seed")
    bot_seeds = parse_int_list(args.bot_seeds, "bot seed")
    dist = resolve_build(args.dist, args.rev, args, Path(args.build_root))
    server, _thread, base_url = serve_dist(dist)
    rows: list[dict[str, Any]] = []
    # Every pose ANY run in the sweep reached. A sweep that fails to find one
    # pose is also the cheapest evidence there is about which poses this
    # harness can produce at all -- #438's "never reached in ~22,000 ticks"
    # was exactly this observation, made by hand and only for three poses.
    seen_anywhere: set[str] = set()
    started = time.monotonic()
    log = Path(args.log_dir) / "search-webdriver.log"
    found_it = False
    try:
        # One browser for the whole sweep: `Page.addScriptToEvaluateOnNewDocument`
        # persists across navigations, so each seed still gets a virgin clock
        # without paying for a process launch.
        with BrowserSlot(args, log, "search") as driver:
            for seed in seeds:
                for bot_seed in bot_seeds:
                    scan = scan_run(
                        args, base_url, seed=seed, bot_seed=bot_seed, scan_ticks=args.scan_ticks, driver=driver
                    )
                    summary = scan["summary"]
                    seen_anywhere.update(summary["poses"].keys())
                    entry = summary["poses"].get(args.pose, {})
                    row = {
                        "seed": seed,
                        "bot_seed": bot_seed,
                        "scanned_ticks": int(summary["recorded_ticks"]),
                        "holds": int(entry.get("holds", 0)),
                        "on_camera_holds": int(entry.get("on_camera_holds", 0)),
                        "first_tick": entry.get("first_tick"),
                        "first_on_camera_tick": entry.get("first_on_camera_tick"),
                    }
                    rows.append(row)
                    mark = "HIT " if row["holds"] else "    "
                    print(
                        f"[browser_match_harness] {mark}seed={seed} bot_seed={bot_seed} "
                        f"ticks={row['scanned_ticks']} holds={row['holds']} "
                        f"on_camera={row['on_camera_holds']}"
                    )
                    if row["holds"] and args.stop_on_hit and (row["on_camera_holds"] or not args.pose_on_camera):
                        found_it = True
                        raise _FoundIt()
    except _FoundIt:
        pass
    finally:
        server.shutdown()
    summary = summarize_search(rows, args.pose)
    summary["elapsed_seconds"] = round(time.monotonic() - started, 1)
    summary["scan_ticks_per_run"] = args.scan_ticks
    summary["combat_enabled"] = bool(args.combat)
    summary["stopped_early_on_hit"] = found_it
    summary["poses_seen_anywhere_in_sweep"] = sorted(seen_anywhere)
    summary["poses_never_seen_in_sweep"] = [pose for pose in POSE_IDS if pose not in seen_anywhere]
    print(json.dumps(summary, indent=2, sort_keys=True))
    if args.out is not None:
        emit({"kind": "search", "summary": summary, "rows": rows}, Path(args.out))
    if not summary["found"]:
        print(
            f"[browser_match_harness] {args.pose} was NOT reached in {summary['runs']} runs "
            f"({summary['scanned_ticks']} ticks) over seeds {args.seeds} x bot seeds {args.bot_seeds}. "
            f"That search space is the result -- report it, do not report 'not observed' alone."
        )
        return 2
    return 0


class _FoundIt(Exception):
    """Control flow only: breaks out of the nested sweep on the first hit."""


# ---------------------------------------------------------------------------
# Self-test -- this file's own logic, no browser
# ---------------------------------------------------------------------------


def _simulate_accumulator(frame_ms: float, frames: int) -> dict[int, int]:
    """`match_harness.ts`'s five-line fixed-timestep drain, in Python.

    Reproduced (not imported -- it lives in TypeScript) so the choice of
    `FRAME_MS` can be pinned without a browser. Same arithmetic, same order,
    same `MAX_TICKS_PER_FRAME` cap.
    """
    dt = 1 / 60
    virtual = 0.0
    last = 0.0
    accumulator = 0.0
    histogram: dict[int, int] = {}
    for _ in range(frames):
        virtual += frame_ms
        elapsed = min((virtual - last) / 1000, 0.25)
        last = virtual
        accumulator += elapsed
        ticks = 0
        while accumulator >= dt and ticks < 8:
            accumulator -= dt
            ticks += 1
        histogram[ticks] = histogram.get(ticks, 0) + 1
    return histogram


def _pose_ids_from_typescript() -> tuple[str, ...]:
    text = FRAME_BUFFER_TS.read_text(encoding="utf-8")
    match = re.search(r"export type PlayerPoseId =\s*(.*?);", text, re.DOTALL)
    if match is None:
        raise RuntimeError(f"could not find PlayerPoseId in {FRAME_BUFFER_TS}")
    return tuple(re.findall(r'"([a-z_]+)"', match.group(1)))


def self_test() -> int:
    problems: list[str] = []

    def check(name: str, condition: bool, detail: str = "") -> None:
        if condition:
            print(f"  ok   {name}")
        else:
            print(f"  FAIL {name}{': ' + detail if detail else ''}")
            problems.append(name)

    print("[browser_match_harness] self-test: this driver's own logic")

    # 1. The control guard, both directions.
    good = {100: "aaa", 200: "bbb"}
    check("control passes when two sessions agree", control_verdict(good, dict(good))["ok"])
    drift = control_verdict(good, {100: "aaa", 200: "zzz"})
    check("control fails on one drifting tick", not drift["ok"] and drift["mismatched_ticks"] == [200])
    check("control fails on mismatched tick sets", not control_verdict(good, {100: "aaa"})["ok"])
    check("control fails when nothing was captured", not control_verdict({}, {})["ok"])

    # 2. The freeze-race guard: distinct ticks must not collide.
    check("frozen-capture guard passes on distinct frames", frozen_capture_verdict(good)["ok"])
    frozen = frozen_capture_verdict({100: "aaa", 200: "aaa", 300: "bbb"})
    check(
        "frozen-capture guard catches a stalled loop",
        not frozen["ok"] and frozen["collisions"] == [[100, 200]],
        str(frozen),
    )

    # 3. The A/B guard.
    ab_ok = ab_verdict({100: "aaa", 200: "bbb"}, {100: "aaa", 200: "ccc"})
    check(
        "A/B reports which ticks changed and which did not",
        ab_ok["ok"] and ab_ok["changed_ticks"] == [200] and ab_ok["identical_ticks"] == [100],
        str(ab_ok),
    )
    ab_same = ab_verdict(good, dict(good))
    check("A/B refuses an all-identical pair", not ab_same["ok"], str(ab_same))
    check("A/B refuses mismatched tick sets", not ab_verdict(good, {100: "aaa"})["ok"])

    # 4. Pose-scan verdicts.
    summary = {"recorded_ticks": 3000, "poses": {"tackle": {"holds": 12, "on_camera_holds": 0}}}
    check("pose scan reports a hit", pick_pose_ticks(summary, "tackle", 2, False)["found"])
    off = pick_pose_ticks(summary, "tackle", 2, True)
    check("pose scan refuses an off-camera-only hit when asked for on camera", not off["found"], str(off))
    miss = pick_pose_ticks(summary, "combat_stagger", 1, False)
    check("pose scan names the scanned tick count when it finds nothing", not miss["found"] and "3000" in miss["reason"])

    # 5. Seed-sweep aggregation reports the space covered, hit or miss.
    rows = [
        {"seed": 1, "bot_seed": 11, "scanned_ticks": 1000, "holds": 0, "on_camera_holds": 0},
        {"seed": 2, "bot_seed": 11, "scanned_ticks": 1000, "holds": 3, "on_camera_holds": 0},
    ]
    swept = summarize_search(rows, "combat_stagger")
    check(
        "seed sweep separates 'found' from 'found on camera'",
        swept["found"] and not swept["found_on_camera"] and swept["scanned_ticks"] == 2000,
        str(swept),
    )
    empty = summarize_search([{"seed": 9, "bot_seed": 11, "scanned_ticks": 500, "holds": 0, "on_camera_holds": 0}], "x")
    check("seed sweep reports the space covered on a miss", not empty["found"] and empty["scanned_ticks"] == 500)

    # 6. Range parsing.
    check("range parsing handles lists, spans and steps", parse_int_list("1,3-6,10-14:2", "seed") == [1, 3, 4, 5, 6, 10, 12, 14])
    try:
        parse_int_list("4-1", "seed")
        check("range parsing rejects a backwards span", False)
    except ValueError:
        check("range parsing rejects a backwards span", True)

    # 7. The clock. This is the #429 fix's arithmetic, pinned.
    histogram = _simulate_accumulator(FRAME_MS, 200_000)
    check("FRAME_MS drains exactly one tick per frame over 200k frames", histogram == {1: 200_000}, str(histogram))
    naive = _simulate_accumulator(1000.0 / 60.0, 200_000)
    check("the un-nudged 1000/60 step does NOT (so the epsilon is load-bearing)", naive != {1: 200_000}, str(naive))

    # 8. Bootstrap invariants. Deleting either replacement silently restores
    #    #429's camera drift, and every capture becomes artefact again.
    source = bootstrap_source()
    check("bootstrap replaces requestAnimationFrame", "window.requestAnimationFrame = function" in source)
    check("bootstrap replaces performance.now", "performance.now = function" in source)
    check("bootstrap has no unsubstituted placeholders", "__FRAME_MS__" not in source and "__CANVAS__" not in source)
    check("bootstrap exposes the pose index", "poseSummary" in source and "poseHits" in source)

    # 9. The pose table has not drifted from the wire enum.
    if FRAME_BUFFER_TS.is_file():
        from_ts = _pose_ids_from_typescript()
        check(
            f"POSE_IDS matches PlayerPoseId in {FRAME_BUFFER_TS.name} ({len(from_ts)} ids)",
            set(from_ts) == set(POSE_IDS),
            f"only in TS: {sorted(set(from_ts) - set(POSE_IDS))}; only here: {sorted(set(POSE_IDS) - set(from_ts))}",
        )
    else:
        check("frame_buffer.ts is readable", False, f"{FRAME_BUFFER_TS} is missing")

    print()
    print(SELF_TEST_LIMITS)
    print()
    if problems:
        print(f"[browser_match_harness] self-test FAILED: {len(problems)} check(s)")
        return 1
    print("[browser_match_harness] self-test passed (see the limits above for what that does and does not mean)")
    return 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED, help="?seed= (match seed)")
    parser.add_argument("--bot-seed", type=int, default=DEFAULT_BOT_SEED, help="?bot_seed=")
    parser.add_argument("--width", type=int, default=DEFAULT_WIDTH)
    parser.add_argument("--height", type=int, default=DEFAULT_HEIGHT)
    parser.add_argument("--hold-frames", type=int, default=DEFAULT_HOLD_FRAMES,
                        help="repaints before each capture (see `capture`'s docstring)")
    parser.add_argument(
        "--combat",
        action="store_true",
        help=(
            "?combat=1 -- build the session with the combat layer on. Off by default, as the page "
            "is. NOTE this does not make combat POSES appear: gc_wasm's frame_options never hands "
            "frame::build a FrameCombatModel, so combat_stagger and its six siblings are currently "
            "unreachable in every v2 render frame (see match_harness.ts's note on `combatEnabled`)."
        ),
    )
    parser.add_argument("--gpu-mode", choices=GPU_MODES, default="hardware")
    parser.add_argument("--chrome-binary", default="/usr/bin/google-chrome")
    parser.add_argument("--chromedriver", default=str(Path.home() / ".local" / "bin" / "chromedriver"))
    parser.add_argument("--log-dir", type=Path, default=ROOT / "build" / "browser_match_harness-logs")
    parser.add_argument("--build-root", type=Path, default=ROOT / "build" / "browser_match_harness-builds")
    parser.add_argument("--skip-wasm-build", action="store_true")
    parser.add_argument("--skip-harness-build", action="store_true")


def add_scan_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--scan-ticks", type=int, default=3000)
    parser.add_argument("--scan-width", type=int, default=DEFAULT_WIDTH,
                        help="scan renders at this size; pose selection is sim state, so it is free to be small")
    parser.add_argument("--scan-height", type=int, default=DEFAULT_HEIGHT)
    parser.add_argument("--scan-no-bloom", action="store_true", default=True)
    parser.add_argument("--scan-with-bloom", dest="scan_no_bloom", action="store_false")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true",
                        help="check THIS DRIVER's own logic; starts no browser and is not evidence about the renderer")
    sub = parser.add_subparsers(dest="command")

    p_capture = sub.add_parser("capture", help="capture named ticks (or a pose) from one build")
    add_common(p_capture)
    add_scan_options(p_capture)
    p_capture.add_argument("--ticks", default="600,900,1200")
    p_capture.add_argument("--pose", default=None, help="scan for this pose instead of using --ticks")
    p_capture.add_argument("--pose-count", type=int, default=3)
    p_capture.add_argument("--pose-spacing", type=int, default=30,
                           help="minimum ticks between two captures of the same pose")
    p_capture.add_argument("--pose-on-camera", action="store_true",
                           help="only accept holds inside the follow camera's frame")
    p_capture.add_argument("--dist", type=Path, default=None, help="a prebuilt harness dist to use as-is")
    p_capture.add_argument("--rev", default=None, help="build the harness from this git revision first")
    p_capture.add_argument("--out", type=Path, default=ROOT / "build" / "browser_match_harness-capture")

    p_ab = sub.add_parser("ab", help="before/after across two builds, with a mandatory control for each")
    add_common(p_ab)
    p_ab.add_argument("--ticks", default="600,900,1200")
    p_ab.add_argument("--dist-a", type=Path, default=None)
    p_ab.add_argument("--dist-b", type=Path, default=None)
    p_ab.add_argument("--rev-a", default=None)
    p_ab.add_argument("--rev-b", default=None)
    p_ab.add_argument("--seed-a", type=int, default=DEFAULT_SEED)
    p_ab.add_argument("--seed-b", type=int, default=DEFAULT_SEED)
    p_ab.add_argument("--out", type=Path, default=ROOT / "build" / "browser_match_harness-ab")

    p_scan = sub.add_parser("scan", help="one pass: which poses occur, when, and whether on camera")
    add_common(p_scan)
    add_scan_options(p_scan)
    p_scan.add_argument("--dist", type=Path, default=None)
    p_scan.add_argument("--rev", default=None)
    p_scan.add_argument("--out", type=Path, default=None)

    p_search = sub.add_parser("search", help="sweep seeds looking for a pose a single match never produces")
    add_common(p_search)
    add_scan_options(p_search)
    p_search.add_argument("--pose", required=True)
    p_search.add_argument("--seeds", default="1-20")
    p_search.add_argument("--bot-seeds", default="11")
    p_search.add_argument("--pose-on-camera", action="store_true")
    p_search.add_argument("--stop-on-hit", action="store_true", default=True)
    p_search.add_argument("--no-stop-on-hit", dest="stop_on_hit", action="store_false")
    p_search.add_argument("--dist", type=Path, default=None)
    p_search.add_argument("--rev", default=None)
    p_search.add_argument("--out", type=Path, default=None)

    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.command is None:
        parser.print_help()
        return 2

    handlers = {
        "capture": command_capture,
        "ab": command_ab,
        "scan": command_scan,
        "search": command_search,
    }
    try:
        return handlers[args.command](args)
    except RuntimeError as error:
        print(f"[browser_match_harness] {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
