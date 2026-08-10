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
nothing in it reads the viewport.

That argument is checked, not trusted. `pose_hold_verdict` compares the pose
the scan asked for against the poses BOTH full-fidelity capturing sessions
report at each captured tick, and refuses the whole capture on any
disagreement -- so if pose selection ever stops being viewport-independent,
the failure is a named refusal rather than a plausible screenshot filed
under the wrong pose. The guard has its own red-path coverage in
`--self-test`.

`--pose` also reports whether the posed player was ON CAMERA, from the
harness's `playerX`/`playerY`/`viewX`/`viewY`/`viewZoom` diagnostics. #438
found one `keeper_tip` in ~22,000 ticks and it was off screen, which is the
same as not finding it.

## Rare poses

`search` sweeps `?seed=` x `?bot_seed=` over many short runs looking for a
pose a single match never produces, and reports the search space it covered
whether or not it found one. "Never observed in N seed-runs of M ticks" is
evidence; "I did not happen to see it" is not.

## Counting an outcome, not just locating a frame

`count` answers a different question from everything above: not "where is a
frame worth looking at" but "over a whole session, how often did the renderer
actually do the thing". It exists because #449's fix was justified by a
measurement -- save frames that reached their lean, before and after -- taken
by hand in a scratch worktree that no longer exists, so nobody could re-derive
it and nothing would notice if the fix regressed. The invariant test that
shipped with the fix pins the PROPERTY (a keeper's drawn facing never tracks
its own dive direction); a regression that keeps the property and collapses
the count passes every gate.

What it tallies, per session: save frames and how many reached a lean; save
EPISODES -- one contiguous hold by one slot -- classified always-leaning,
never-leaning, or POPPED (leaning changed mid-episode, which is what #449
actually looked like on screen); and the same treatment for `keeper_get_up`
and `keeper_tip`. Output is a `report.json` a later gate could assert on,
plus a table for a human.

IT COUNTS THE SIGNAL A RENDERED FRAME USES, which is the whole point and the
part that is easy to get subtly wrong. Not a proxy re-derived from simulation
state: the numbers come out of `gc_render::frame::build`'s own output block,
through `rig3d/action_pose.ts`'s own `lateralSign` formula, because that
function returning zero is precisely what makes `save()` skip a dive's entire
overlay. `lateral_sign` here mirrors it and `--self-test` reads the formula
and its dead band back out of `action_pose.ts` to catch a drift.

HOW THE FRAME IS READ, and why it is not read from the page. `match_harness.ts`
republishes pose ids and world positions for a driver, but not `facing` or
`dive_dir` -- the two `lateralSign` needs. Adding them to that page would make
the instrument exist only at revisions carrying the edit, and a count's whole
purpose is to compare a fix against the build BEFORE it. So the driver adopts
the page's own `WebAssembly.Instance` as it is instantiated and reads the same
block the page has just decoded (bootstrap section 3). One instrument, every
revision, including builds made before this mode existed.

The bargain that buys: at every counted tick, for every slot, the pose code
and world position the driver read by field index are compared against what
the page decoded through `frame_buffer.ts` for the same slot. A moved column
is a named refusal (`page_cross_check_verdict`), not a plausible number. The
two columns that cross-check cannot reach are checked structurally instead --
`facing` and `dive_dir` are unit vectors or zero, and no neighbouring column
is that by accident over thousands of frames.

Reading another revision's frames is the primary use, so the mirrors are
checked against THAT revision, not this one: `mirrored_source_verdict` hashes
both `frame_buffer.ts` and `action_pose.ts` at the counted build against this
tree's. `--self-test`'s drift checks can only ever read ROOT, so without it a
`--rev` build whose own `lateralSign` differed would be measured with this
tree's formula and nothing would say so.

Three sessions, all mandatory. Two counting sessions in independent browser
processes must produce the IDENTICAL stream of counted frames -- same control
discipline as the pixel subcommands, and a count needs it more, since an
unstable count is reported as a number with no image for anyone to disbelieve.
The third runs a short prefix at full size with bloom on and must count the
same stream, which is what earns the small, fast viewport the other two use.

AND EVERY SESSION MUST COVER ITS WHOLE TICK BUDGET, which the control cannot
check and must not be mistaken for checking. The simulation is deterministic,
so two sessions that stop early stop at the SAME tick -- the control then
compares two identically truncated streams and says they agree, which reads as
reassurance while the count is a fraction of the run. A session can stop early
by erroring, by the match ending, or by the page quietly failing to re-arm its
`requestAnimationFrame`, after which every step is a successful no-op. All
three are refused by `session_completeness_verdict`, and the report carries
the ticks reached beside the ticks asked for so the two cannot be read apart.

On a failed guard the report withholds `counts` entirely and carries a
top-level `verdict`, the way `command_ab` withholds its `comparison`: a
downstream reader should not have to iterate ten verdicts to learn whether a
number means anything, and a JSON file outlives the terminal that printed the
refusal.

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

`count` is out for a second, independent reason (#454's own scope): its
figures are a function of one seed, one bot seed and one tick budget, and
pinning them would need a stability argument nobody has made. Making the
measurement reproducible is the deliverable; asserting on it is a later
decision, and the report.json exists so that decision does not need new
instrumentation.

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
import tempfile
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
ACTION_POSE_TS = V2_TS / "packages" / "render" / "src" / "rig3d" / "action_pose.ts"

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

# Where `count` reads a render frame from, inside the flat f64 block
# `gc_render::frame::build` writes.
#
# `magic` is `frame_buffer.ts`'s `MAGIC` ("GOLF"), and the field numbers are
# its own `column(words, playersAt, N, count)` arguments -- the same
# structure-of-arrays indices `frame_buffer.rs` and `render/frame_buffer.lua`
# write. Nothing about the block's SHAPE is duplicated here: the header
# carries its own sizes and the bootstrap reads them from it. `--self-test`
# re-derives every number below from `frame_buffer.ts` and fails if they have
# drifted, and the bootstrap additionally cross-checks `pose_id`/`x`/`y`
# against the page's own decode of the same block at every counted tick.
FRAME_FIELDS = {
    "magic": 0x474F4C46,
    "x": 0,
    "y": 1,
    "facing_x": 2,
    "facing_y": 3,
    "pose_id": 5,
    "dive": 11,
    "dive_dir_x": 12,
    "dive_dir_y": 13,
}

# The poses whose drawn root transform is decided by `lateralSign`, split into
# the families #449 reported against.
#
# `save` is exactly the key set of `rig3d/action_pose.ts`'s `SAVES`, because
# that table is what `save()` looks a pose id up in; `keeper_tip` is a member
# of it and is ALSO reported on its own, since it reaches `lateralSign` by a
# `dive_dir` that `gc_render::frame::build` synthesises rather than one the
# simulation ran a dive along. `keeper_get_up` reads the same sign from
# `tip()` and is its own family.
SAVE_POSES = ("keeper_spread", "keeper_central", "keeper_stretch", "keeper_tip", "keeper_dive")
GET_UP_POSES = ("keeper_get_up",)
TIP_POSES = ("keeper_tip",)
LEAN_POSES = SAVE_POSES + GET_UP_POSES

# `rig3d/action_pose.ts`'s own dead-band on the cross product. Mirrored, not
# approximated: a lean is skipped at exactly this threshold and not one ulp
# either side. `--self-test` reads it back out of that file.
LATERAL_SIGN_EPSILON = 1e-6

# Rows a counting session may record before it gives up. One row is one
# watched player-frame; a 24,000-tick session records low thousands. The cap
# exists so a mis-armed watch (every pose id, say) cannot exhaust the page's
# memory silently -- hitting it sets `truncated`, which refuses the count.
DEFAULT_ROW_LIMIT = 400_000

# `count`'s session shape. The tick budget is #449's own: it is the session
# whose figures this mode exists to make re-derivable, and a shorter one
# simply sees fewer saves. Deliberately NOT a pinned expectation anywhere --
# see "Not a CI gate" in this file's header.
DEFAULT_COUNT_TICKS = 24_000
DEFAULT_COUNT_WIDTH = 320
DEFAULT_COUNT_HEIGHT = 180
DEFAULT_FIDELITY_TICKS = 1_200

BOOT_TIMEOUT_SECONDS = 180
# Frames pumped per `Runtime.evaluate` round trip. Large enough that stepping
# thousands of ticks is not dominated by round trips, small enough that one
# call cannot outrun the webdriver command timeout on a slow GL path.
PUMP_CHUNK_FRAMES = 120
DEFAULT_HOLD_FRAMES = 2
# How long one `execute_script` may take before selenium gives up. See
# `BrowserSlot.__enter__` for why the 30 s default is not enough.
SCRIPT_TIMEOUT_SECONDS = 600

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
  var FRAME = __FRAME_FIELDS__;
  var virtualNow = 0;
  var nextHandle = 1;
  var pending = new Map();
  var framesRun = 0;
  var poseIndex = new Map();   // pose id -> flat [tick, slot, tick, slot, ...]
  var recordedTicks = 0;
  var lastRecordedTick = -1;
  var onCamera = new Map();    // pose id -> flat [tick, slot, ...] for in-shot holds
  var wasm = null;             // the module's own raw exports, see 3 below
  var wasmNote = "no WebAssembly instance exporting render_frame_ptr was instantiated";
  var watch = null;            // pose code -> 1, or null when not counting
  var rows = [];               // one row per watched player-frame
  var rowLimit = 0;
  var rowsTruncated = false;
  var framesRead = 0;
  var crossFailures = 0;       // raw read vs the page's own frame_buffer.decode
  var unposedDives = 0;        // a dive the wire named no pose for
  var crossExamples = [];
  var readError = null;
  var layout = null;

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

  // 3. THE RENDER FRAME'S OWN WORDS, for `count`.
  //
  //    `gc_render::frame::build` writes one flat f64 block into wasm linear
  //    memory and `match_harness.ts` decodes it through `frame_buffer.decode`
  //    once per frame. Counting how a keeper is DRAWN needs two per-player
  //    fields that page does not republish -- `facing` and `dive_dir`, the
  //    two `rig3d/action_pose.ts`'s `lateralSign` is computed from -- so they
  //    are read here, out of the same block the page has just decoded.
  //
  //    WHY NOT PUBLISH THEM FROM THE PAGE INSTEAD, which would be less
  //    machinery. Because the instrument would then only exist at revisions
  //    that carry the edit, and a count's whole purpose is to compare a fix
  //    against the build BEFORE it. Reading the block from the driver keeps
  //    one instrument for every revision, including ones built months ago.
  //
  //    WHAT MAKES IT SAFE. The block is self-describing: word 0 is the magic,
  //    words 4/5/6/7 are the header size, scalar count, player count and
  //    per-player field count, so nothing about the block's SHAPE is assumed
  //    here. What is assumed is which field index means what, and that is
  //    checked rather than trusted -- `readFrame` compares the pose code and
  //    world position it reads against the ones the page itself decoded
  //    through `frame_buffer.ts` for the same tick, every tick, every slot.
  //    A field-index drift shows up as a named cross-check failure instead of
  //    a plausible-looking count.
  function adopt(instance) {
    if (wasm !== null || !instance || !instance.exports) { return; }
    var e = instance.exports;
    if (typeof e.render_frame_ptr !== "function") { return; }
    if (typeof e.render_frame_len !== "function" || !e.memory) { return; }
    wasm = e;
    wasmNote = "adopted the WebAssembly instance exporting render_frame_ptr/render_frame_len/memory";
  }

  function interceptInstantiation(name) {
    var original = WebAssembly[name];
    if (typeof original !== "function") { return; }
    WebAssembly[name] = function () {
      var out = original.apply(WebAssembly, arguments);
      return Promise.resolve(out).then(function (result) {
        // `instantiate(bytes|Response)` resolves to {instance, module};
        // `instantiate(Module)` resolves to the Instance itself.
        adopt(result && result.instance ? result.instance : result);
        return result;
      });
    };
  }
  interceptInstantiation("instantiateStreaming");
  interceptInstantiation("instantiate");

  function readFrame(tick, stats) {
    var ptr = wasm.render_frame_ptr();
    var len = wasm.render_frame_len();
    if (!(len > 0)) {
      readError = readError || ("render_frame_len() returned " + len + " at tick " + tick);
      return;
    }
    // Never cached across ticks: `memory.buffer` is replaced wholesale when
    // wasm memory grows, which detaches any view held over it.
    var words = new Float64Array(wasm.memory.buffer, ptr, len);
    if (words[0] !== FRAME.magic) {
      readError = readError || ("frame block magic is " + words[0] + ", expected " + FRAME.magic);
      return;
    }
    var headerWords = words[4];
    var scalarWords = words[5];
    var count = words[6];
    if (layout === null) {
      layout = {
        magic: words[0],
        layout_version: words[1],
        render_frame_version: words[2],
        total_words: words[3],
        header_words: headerWords,
        scalar_words: scalarWords,
        player_count: count,
        player_fields: words[7],
        words: len
      };
    }
    var at0 = headerWords + scalarWords;
    var poses = stats.poses || [];
    var px = stats.playerX || [];
    var py = stats.playerY || [];
    for (var i = 0; i < count; i += 1) {
      var code = words[at0 + FRAME.pose_id * count + i];
      var rawX = words[at0 + FRAME.x * count + i];
      var rawY = words[at0 + FRAME.y * count + i];
      var pagePose = (poses[i] === undefined || poses[i] === null) ? null : poses[i];
      if ((code !== 0) !== (pagePose !== null) || rawX !== px[i] || rawY !== py[i]) {
        crossFailures += 1;
        if (crossExamples.length < 16) {
          crossExamples.push({
            tick: tick, slot: i + 1, code: code, page_pose: pagePose,
            raw_x: rawX, page_x: px[i] === undefined ? null : px[i],
            raw_y: rawY, page_y: py[i] === undefined ? null : py[i]
          });
        }
      }
      // `save()` also fires for a frame with NO pose id when `dive > 0`,
      // falling back to `SAVES.keeper_dive`. A count keyed to pose ids cannot
      // see that, so it is counted here as the thing the count would miss --
      // see `unposed_dive_verdict`.
      if (code === 0 && words[at0 + FRAME.dive * count + i] > 0) { unposedDives += 1; }
      if (watch[code] !== 1) { continue; }
      if (rows.length >= rowLimit) { rowsTruncated = true; break; }
      rows.push([
        tick, i + 1, code,
        words[at0 + FRAME.facing_x * count + i],
        words[at0 + FRAME.facing_y * count + i],
        words[at0 + FRAME.dive_dir_x * count + i],
        words[at0 + FRAME.dive_dir_y * count + i],
        pagePose
      ]);
    }
    framesRead += 1;
  }

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
    // Same one-entry-per-tick discipline as the pose index above, and after
    // it, so a counted frame is a frame the pose scan also saw.
    if (watch !== null && wasm !== null) { readFrame(tick, stats); }
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

    // Arm the render-frame reader for `count`. Must be called BEFORE the
    // first `step`: nothing is recorded retroactively.
    leanWatch: function (codes, limit) {
      watch = {};
      for (var i = 0; i < codes.length; i += 1) { watch[codes[i]] = 1; }
      rows = [];
      rowLimit = limit;
      rowsTruncated = false;
      framesRead = 0;
      crossFailures = 0;
      crossExamples = [];
      unposedDives = 0;
      readError = null;
      layout = null;
      return { armed: codes.length, wasm_available: wasm !== null, wasm_note: wasmNote };
    },

    leanRows: function () {
      return {
        rows: rows,
        truncated: rowsTruncated,
        frames_read: framesRead,
        cross_check_failures: crossFailures,
        cross_check_examples: crossExamples,
        unposed_dive_frames: unposedDives,
        read_error: readError,
        layout: layout,
        wasm_available: wasm !== null,
        wasm_note: wasmNote,
        recorded_ticks: recordedTicks
      };
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
    return (
        _BOOTSTRAP_JS.replace("__FRAME_MS__", repr(frame_ms))
        .replace("__FRAME_FIELDS__", json.dumps(FRAME_FIELDS, sort_keys=True))
        .replace("__CANVAS__", CANVAS_SELECTOR)
    )


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


def pose_hold_verdict(pose: str | None, runs: dict[str, dict[int, list[str]]]) -> dict[str, Any]:
    """The scan found these ticks; the capture says what was actually held.

    THIS IS THE RE-VERIFICATION THE SCAN'S CHEAPNESS IS BOUGHT WITH. `--pose`
    locates ticks in a small, bloom-less session on the argument that pose
    selection is simulation state -- `player_pose::select` takes no viewport
    -- so the tick a scan finds is the tick a full-fidelity run will hold.
    That argument is true today and load-bearing, and nothing else would
    notice it stopping being true: a scan/capture disagreement produces a
    perfectly plausible screenshot filed under the wrong pose.

    So the ticks are checked against the FULL-FIDELITY runs' own reported
    poses -- read from `__gcMatchHarness.poses` at the captured tick, in
    every capturing session, not just the first -- and a disagreement
    refuses the whole capture. Checking only the run whose files are
    reported would leave the second session's evidence unexamined while
    still calling it a control.
    """
    if pose is None:
        return {"ok": True, "reason": "no pose was requested, so there is nothing to re-verify", "missing": []}
    missing: list[dict[str, Any]] = []
    checked = 0
    for label in sorted(runs):
        held = runs[label]
        for tick in sorted(held):
            checked += 1
            if pose not in held[tick]:
                missing.append({"run": label, "tick": tick, "held": sorted(set(held[tick]))})
    if not checked:
        return {"ok": False, "reason": f"{pose} was requested but no tick was captured", "missing": []}
    if missing:
        first = missing[0]
        return {
            "ok": False,
            "reason": (
                f"the scan said {pose} is held at every captured tick and the capture disagrees "
                f"({len(missing)} of {checked} tick/run pairs) -- e.g. {first['run']} at tick "
                f"{first['tick']} held {first['held']}. The scan runs at a smaller viewport, so "
                f"either pose selection has stopped being viewport-independent or the two sessions "
                f"are not the same match; refusing to file these frames under {pose}"
            ),
            "missing": missing,
        }
    return {
        "ok": True,
        "reason": f"{pose} confirmed held at all {checked} captured tick/run pairs by the full-fidelity runs",
        "missing": [],
    }


def renderer_state_verdict(diagnostics: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """Refuse a capture taken while `@gc/render`'s `effects` layer holds state.

    `effects.burst` spawns spark particles from bare `Math.random()` --
    deliberately, and correctly: it is juice, never fed back into the
    simulation (`effects.ts`'s own comment). `stadium_prng.ts` uses a seeded
    PRNG precisely because ITS output must be reproducible; `effects` makes
    the opposite trade.

    That makes it the one known thing on this page that would break the
    control if it were ever live. It is not live today -- `match_harness.ts`
    never calls `effects.update`/`consume`/`apply_event_diff`/
    `confirm_event`/`consume_combat`, so the particle and trail arrays stay
    empty and `pitch.ts`'s draw calls emit nothing. But `pitch.ts` already
    imports `effects`, so wiring match or combat events into it here is a
    plausible future edit, and the failure it would cause is the worst kind:
    the control would start failing near any shot, pass, tackle or
    reception, and neither the bootstrap nor the self-test would say why.

    So the capture reads `effects.diagnostics()` and refuses with a NAMED
    CAUSE if anything is there. This does not fix `effects.ts` and must not
    -- an unseeded particle system is right for the product. It makes the
    next person's mystery into a sentence.
    """
    offenders: list[dict[str, Any]] = []
    for label in sorted(diagnostics):
        entry = diagnostics[label] or {}
        if entry.get("unavailable"):
            # Older harness builds do not publish the handle. Not a failure:
            # this guard is a diagnosis aid, not a determinism requirement.
            continue
        particles = int(entry.get("particle_count") or 0)
        trail = int(entry.get("trail_count") or 0)
        if particles or trail:
            offenders.append({"run": label, "particle_count": particles, "trail_count": trail})
    if offenders:
        return {
            "ok": False,
            "reason": (
                "`@gc/render`'s `effects` layer held state during a capture "
                f"({offenders}). `effects.burst` spawns particles from unseeded "
                "`Math.random()`, so two sessions cannot agree and the control below is "
                "not meaningful. Something now drives `effects` from this page; that is "
                "the cause, not the clock and not the swap chain"
            ),
            "offenders": offenders,
        }
    return {
        "ok": True,
        "reason": "the unseeded `effects` particle layer was empty in every capturing session",
        "offenders": [],
    }


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


# ---------------------------------------------------------------------------
# Counting: what a rendered frame's keeper actually does
# ---------------------------------------------------------------------------


def lateral_sign(dive_dir_x: float, dive_dir_y: float, facing_x: float, facing_y: float) -> int:
    """`rig3d/action_pose.ts`'s `lateralSign`, character for character.

    THIS IS THE WHOLE MEASUREMENT, so it is a mirror and not an approximation:
    same 2D cross product, same operand order, same dead band. It decides
    whether a save's roll and travel happen at all -- `save()` returns `null`
    on a zero sign and the ENTIRE overlay is skipped -- and whether a
    `keeper_get_up` keeps the side it landed on.

    Reproduced here rather than imported because it lives in TypeScript inside
    a bundled ES module, with no runtime handle on the page. `--self-test`
    reads the formula and the epsilon back out of `action_pose.ts` and fails
    if either has moved, which is the same anti-drift treatment `POSE_IDS`
    gets.

    The `facing ? facing.x : 1` fallback in the TypeScript is deliberately not
    reproduced: `pitch.ts`'s `playerOptions` always constructs `facing` from
    the frame's own `facing_x`/`facing_y`, so the undefined branch is
    unreachable from a rendered frame. A zero facing vector still yields a
    zero sign here, exactly as it does there.
    """
    along_left = dive_dir_x * facing_y - dive_dir_y * facing_x
    if abs(along_left) < LATERAL_SIGN_EPSILON:
        return 0
    return 1 if along_left > 0 else -1


def lean_records(raw_rows: list[list[Any]]) -> list[dict[str, Any]]:
    """One recorded player-frame per row, with its lean resolved.

    A row is `[tick, slot, pose_code, facing_x, facing_y, dive_dir_x,
    dive_dir_y, page_pose]` as the bootstrap pushed it: the first seven read
    straight out of `frame::build`'s block, the last one the pose id the PAGE
    decoded for the same slot on the same tick through `frame_buffer.ts`.
    `pose_code_verdict` is what makes the two agree; this only resolves them.
    """
    records: list[dict[str, Any]] = []
    for row in raw_rows:
        tick, slot, code, facing_x, facing_y, dive_x, dive_y, page_pose = row
        index = int(code) - 1
        pose = POSE_IDS[index] if 0 <= index < len(POSE_IDS) else None
        sign = lateral_sign(float(dive_x), float(dive_y), float(facing_x), float(facing_y))
        records.append(
            {
                "tick": int(tick),
                "slot": int(slot),
                "pose": pose,
                "page_pose": page_pose,
                "facing": (float(facing_x), float(facing_y)),
                "dive_dir": (float(dive_x), float(dive_y)),
                "sign": sign,
                "leaned": sign != 0,
            }
        )
    return records


def count_family(records: list[dict[str, Any]], poses: tuple[str, ...]) -> dict[str, Any]:
    """Frames, leans and EPISODES for one pose family.

    An episode is one contiguous hold: the same roster slot, the family's
    poses, consecutive ticks. It is the unit that matters for #449, because
    the defect's visible symptom was not a lost lean but a lean that came and
    went WITHIN one save -- a hard state-meaning switch, drawn as a pop.
    Frame counts alone cannot distinguish "half the saves never leaned" from
    "every save leaned for half its length", and those look nothing alike on
    screen.

    A gap of even one tick opens a new episode. That is the conservative
    direction: it can only split one real save into two, never merge two
    saves into one, so `popped` is never inflated by the bookkeeping.
    """
    subset = [record for record in records if record["pose"] in poses]
    by_slot: dict[int, list[dict[str, Any]]] = {}
    for record in subset:
        by_slot.setdefault(record["slot"], []).append(record)

    episodes: list[list[dict[str, Any]]] = []
    for slot in sorted(by_slot):
        run: list[dict[str, Any]] = []
        for record in sorted(by_slot[slot], key=lambda item: item["tick"]):
            if run and record["tick"] != run[-1]["tick"] + 1:
                episodes.append(run)
                run = []
            run.append(record)
        if run:
            episodes.append(run)

    always = never = popped = 0
    for episode in episodes:
        leaning = sum(1 for record in episode if record["leaned"])
        if leaning == len(episode):
            always += 1
        elif leaning == 0:
            never += 1
        else:
            popped += 1

    leaned = sum(1 for record in subset if record["leaned"])
    return {
        "poses": list(poses),
        "frames": len(subset),
        "leaned": leaned,
        "not_leaned": len(subset) - leaned,
        "episodes": {
            "total": len(episodes),
            "always_leaning": always,
            "never_leaning": never,
            "popped_mid_episode": popped,
        },
        "longest_episode_ticks": max((len(episode) for episode in episodes), default=0),
    }


def tally(records: list[dict[str, Any]]) -> dict[str, Any]:
    """The whole count: per family, and per pose so a family is traversable.

    `save` is the five ids `action_pose.ts`'s `SAVES` table holds, which is
    the set `save()` will pose at all -- with one deliberate exclusion:
    `save()` also fires for a frame carrying NO pose id at all when
    `opts.dive > 0`, falling back to `SAVES.keeper_dive`. That branch is not
    counted, because it is not a pose the wire names and a count keyed to a
    pose id cannot honestly attribute it -- and it is not silently ignored
    either: `unposed_dive_verdict` counts exactly those frames and refuses the
    whole session if any occurred, so "the count saw every save" is a checked
    claim rather than an assumption. `tip` and `save_excluding_tip` are both
    reported because `keeper_tip` is a member of `SAVES` AND reaches
    `lateralSign` through a `dive_dir` the frame builder synthesises, so
    "save frames" is ambiguous between the two readings and a reader should
    not have to guess which one a number came from.
    """
    return {
        "families": {
            "save": count_family(records, SAVE_POSES),
            "save_excluding_tip": count_family(records, tuple(p for p in SAVE_POSES if p not in TIP_POSES)),
            "tip": count_family(records, TIP_POSES),
            "get_up": count_family(records, GET_UP_POSES),
        },
        "by_pose": {pose: count_family(records, (pose,)) for pose in LEAN_POSES},
    }


def records_digest(records: list[dict[str, Any]]) -> str:
    """A run's whole counted stream, not just its totals.

    Two sessions that disagree about WHICH frames leaned but happen to agree
    on how many is a determinism failure the totals would hide, so the control
    hashes the stream."""
    payload = [
        [record["tick"], record["slot"], record["pose"], record["sign"]] for record in records
    ]
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode("utf-8")).hexdigest()


def frame_read_verdict(runs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """The render frame was actually read, and the block was the right one.

    The reader adopts the page's own `WebAssembly.Instance` as it is
    instantiated. If wasm-bindgen ever stops going through
    `WebAssembly.instantiate`/`instantiateStreaming`, nothing is adopted and
    the count is simply empty -- which would otherwise read as "no keeper ever
    dived". It is a named refusal instead.
    """
    problems: list[dict[str, Any]] = []
    for label in sorted(runs):
        raw = runs[label]
        if not raw.get("wasm_available"):
            problems.append({"run": label, "problem": raw.get("wasm_note", "no wasm instance was adopted")})
        elif raw.get("read_error"):
            problems.append({"run": label, "problem": raw["read_error"]})
        elif raw.get("truncated"):
            problems.append({"run": label, "problem": "the row limit was hit; the count is a prefix, not a count"})
        elif not raw.get("frames_read"):
            problems.append({"run": label, "problem": "no frame was read at all"})
    if problems:
        return {
            "ok": False,
            "reason": f"the render frame could not be read in {len(problems)} run(s): {problems[0]['problem']}",
            "problems": problems,
        }
    return {
        "ok": True,
        "reason": "every run read `frame::build`'s own block, with its header sizes taken from the block",
        "problems": [],
    }


def session_completeness_verdict(runs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """Every session counted the whole budget it was asked for.

    THE FAILURE THIS EXISTS FOR IS A CONTROL THAT REASSURES YOU ABOUT A
    TRUNCATED RUN. A session can stop early three ways and none of them used
    to reach a verdict:

      * `match_harness.ts`'s `main().catch` sets `status = "error"`, and the
        stepping loop broke out of the pump on it exactly as it does for a
        legitimate `finished`;
      * the match itself ends (`?duration=`) before the tick budget does;
      * the page stops re-registering its `requestAnimationFrame` -- `loop`
        re-arms on its LAST line, so anything returning early leaves `pending`
        empty, after which every `step()` is a successful no-op and the tick
        silently stops advancing while the driver pumps out the rest of the
        budget.

    The simulation is deterministic, so two control sessions on one seed stop
    at the SAME tick. `count_control_verdict` then compares two identically
    truncated streams and reports ok -- "two independent sessions counted an
    identical stream of N frames" -- which reads as reassurance while N is a
    fraction of the run, and the report carries only the budget that was
    REQUESTED. A number about to justify changes in #450 and #451 cannot be
    allowed to fail that way.

    So the budget is checked three ways per session -- the page's own final
    tick, the bootstrap's recorded-tick count, and the frames the reader
    actually read -- and all three must equal what was asked for. That triple
    is also the one-frame-one-tick identity this driver's tick addressing
    rests on, checked rather than assumed.
    """
    problems: list[dict[str, Any]] = []
    for label in sorted(runs):
        run = runs[label]
        requested = int(run["requested_ticks"])
        final_tick = run.get("final_tick")
        if run.get("status") == "error":
            problems.append(
                {"run": label, "problem": f"the page reported an error at tick {final_tick}: {run.get('error')}"}
            )
            continue
        if final_tick != requested:
            ended = " (the match ended)" if run.get("status") == "finished" else ""
            problems.append({"run": label, "problem": f"stopped at tick {final_tick} of {requested} requested{ended}"})
            continue
        for name, value in (("recorded ticks", run.get("recorded_ticks")), ("frames read", run.get("frames_read"))):
            if value != requested:
                problems.append({"run": label, "problem": f"{name} is {value}, not the {requested} ticks stepped"})
    if problems:
        return {
            "ok": False,
            "reason": (
                f"a counting session did not cover its tick budget ({problems[0]['run']}: "
                f"{problems[0]['problem']}). Both control sessions stop at the same tick on a "
                f"deterministic seed, so the control CANNOT catch this -- refusing rather than "
                f"reporting a fraction of a session as a session"
            ),
            "problems": problems,
        }
    return {
        "ok": True,
        "reason": "every session reached its full tick budget, with one recorded tick and one frame read per tick",
        "problems": [],
    }


def unposed_dive_verdict(runs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """No save happened that a pose-id-keyed count could not see.

    `action_pose.ts`'s `save()` has a branch this count is blind to by
    construction: a frame carrying NO pose id but `dive > 0` falls back to
    `SAVES.keeper_dive` and is drawn as a save anyway. `count` keys everything
    to pose ids, so such a frame would be missing from the total AND from the
    denominator -- the worst shape of undercount, because the ratio would
    still look fine.

    It does not happen today: `player_pose::select` names a pose for a diving
    keeper. This is that fact checked once per session instead of assumed
    forever, and it costs one comparison per slot per tick.
    """
    offenders = [
        {"run": label, "frames": int(runs[label].get("unposed_dive_frames", 0))}
        for label in sorted(runs)
        if int(runs[label].get("unposed_dive_frames", 0))
    ]
    if offenders:
        return {
            "ok": False,
            "reason": (
                f"{offenders[0]['frames']} frame(s) in {offenders[0]['run']} carried a live dive with no "
                f"pose id, which `action_pose.ts`'s `save()` still draws as a save: this count is keyed "
                f"to pose ids and cannot see them, so its totals are an undercount of unknown size"
            ),
            "offenders": offenders,
        }
    return {
        "ok": True,
        "reason": "no frame carried a dive without a pose id, so every drawn save is one this count could see",
        "offenders": [],
    }


def page_cross_check_verdict(runs: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """The driver's raw read agrees with the page's own `frame_buffer.decode`.

    THIS IS WHAT THE RAW READ IS BOUGHT WITH, and it is the same bargain
    `pose_hold_verdict` strikes for the scan. The driver reads the block by
    field index; the page reads the same block through `frame_buffer.ts`. At
    every counted tick, for every slot, the pose code and the world position
    the driver read must resolve to exactly what the page decoded. If the
    per-player field layout ever moves, this fails loudly rather than
    producing a count of the wrong column.
    """
    offenders: list[dict[str, Any]] = []
    for label in sorted(runs):
        failures = int(runs[label].get("cross_check_failures", 0))
        if failures:
            offenders.append(
                {
                    "run": label,
                    "failures": failures,
                    "examples": runs[label].get("cross_check_examples", [])[:4],
                }
            )
    if offenders:
        return {
            "ok": False,
            "reason": (
                f"the driver's field-index read of `frame::build`'s block disagrees with the page's "
                f"own `frame_buffer.decode` of it ({offenders[0]['failures']} slot-ticks in "
                f"{offenders[0]['run']}): the per-player layout has moved, so every column this "
                f"count reads is suspect"
            ),
            "offenders": offenders,
        }
    return {
        "ok": True,
        "reason": "pose ids and world positions agree with the page's own decode at every counted slot-tick",
        "offenders": [],
    }


def pose_code_verdict(runs: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    """`POSE_IDS[code - 1]` is the pose the page named for that same slot-tick.

    The wire carries a pose as an integer. Turning it back into a name is a
    table, and a table that has silently rotated would file every save frame
    under the wrong pose while the totals stayed plausible. The page decoded
    the same word through `frame_buffer.ts`'s `poseIdFromCode`; this is the
    two agreeing on live data, on top of `--self-test`'s static check that the
    two tables are the same tables.
    """
    mismatches: list[dict[str, Any]] = []
    checked = 0
    for label in sorted(runs):
        for record in runs[label]:
            checked += 1
            if record["pose"] != record["page_pose"]:
                if len(mismatches) < 8:
                    mismatches.append(
                        {"run": label, "tick": record["tick"], "driver": record["pose"], "page": record["page_pose"]}
                    )
    if not checked:
        return {"ok": False, "reason": "no watched frame was recorded, so nothing was cross-checked", "mismatches": []}
    if mismatches:
        return {
            "ok": False,
            "reason": (
                f"the driver's pose-code table disagrees with the page's at {len(mismatches)}+ of "
                f"{checked} recorded frames -- e.g. tick {mismatches[0]['tick']}: driver "
                f"{mismatches[0]['driver']}, page {mismatches[0]['page']}"
            ),
            "mismatches": mismatches,
        }
    return {
        "ok": True,
        "reason": f"all {checked} recorded frames resolve to the pose the page decoded for them",
        "mismatches": [],
    }


def vector_shape_verdict(runs: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    """`facing` and `dive_dir` are unit vectors (or zero), as the sim writes them.

    The two columns `lateralSign` is computed from are the two the page does
    NOT republish, so `page_cross_check_verdict` cannot reach them. This is
    the structural check that stands in for it: `MatchPlayer::facing` and
    `dive_dir` are normalised (or left at zero), and no neighbouring column in
    the block -- `speed`, a pose code, a pose priority, a dive amount -- is a
    unit vector's component by coincidence over thousands of frames. Reading
    the wrong pair fails this almost immediately.
    """
    offenders: list[dict[str, Any]] = []
    checked = 0
    for label in sorted(runs):
        for record in runs[label]:
            checked += 1
            for name in ("facing", "dive_dir"):
                x, y = record[name]
                length = (x * x + y * y) ** 0.5
                if abs(length) > 1e-6 and abs(length - 1.0) > 1e-6:
                    if len(offenders) < 8:
                        offenders.append(
                            {"run": label, "tick": record["tick"], "field": name, "value": [x, y], "length": length}
                        )
    if not checked:
        return {"ok": False, "reason": "no watched frame was recorded, so nothing was checked", "offenders": []}
    if offenders:
        first = offenders[0]
        return {
            "ok": False,
            "reason": (
                f"a recorded {first['field']} is neither unit nor zero (|{first['value']}| = "
                f"{first['length']:.6f} at tick {first['tick']}): the columns being read are not "
                f"the columns this count claims to read"
            ),
            "offenders": offenders,
        }
    return {
        "ok": True,
        "reason": f"every recorded facing and dive_dir over {checked} frames is unit-length or zero",
        "offenders": [],
    }


def count_control_verdict(run_a: list[dict[str, Any]], run_b: list[dict[str, Any]]) -> dict[str, Any]:
    """Same build, two independent browser processes, same counted stream.

    The pixel subcommands run this guard because a capture that is not
    reproducible is not evidence. A COUNT has the same exposure and a worse
    failure mode: an unstable count would be reported as a number, with no
    image for anyone to disbelieve. So it is mandatory here too, and it
    compares the whole stream (which tick, which slot, which sign) rather than
    the totals.
    """
    digest_a, digest_b = records_digest(run_a), records_digest(run_b)
    if not run_a and not run_b:
        return {"ok": False, "reason": "neither control session recorded a single watched frame", "digests": []}
    if digest_a != digest_b:
        totals_a = tally(run_a)["families"]["save"]
        totals_b = tally(run_b)["families"]["save"]
        return {
            "ok": False,
            "reason": (
                "same build, two sessions, different counted frames: this measurement is not "
                f"deterministic, so no figure from it means anything (save frames "
                f"{totals_a['frames']} vs {totals_b['frames']}, rolled {totals_a['leaned']} vs "
                f"{totals_b['leaned']})"
            ),
            "digests": [digest_a, digest_b],
        }
    return {
        "ok": True,
        "reason": f"two independent sessions counted an identical stream of {len(run_a)} frames ({digest_a[:12]})",
        "digests": [digest_a, digest_b],
    }


def fidelity_verdict(small: list[dict[str, Any]], full: list[dict[str, Any]], ticks: int) -> dict[str, Any]:
    """A count is a property of the simulation, not of the viewport it drew at.

    A counting session renders small, with bloom off, because 24,000 frames of
    full-fidelity SwiftShader is an hour that buys nothing: what is counted is
    `frame::build`'s output, and `frame::build` takes a `MatchState` and no
    viewport. That is the same argument `scan_run` makes -- and, like it, the
    argument is checked instead of trusted. A third session runs the first
    `ticks` ticks at the harness's full size with bloom and the stadium on,
    and its counted stream must be exactly the small session's prefix.

    Skipped, not assumed, when `--fidelity-ticks 0` asks for it: the verdict
    then says so rather than reporting an unrun check as passed.
    """
    if ticks <= 0:
        return {
            "ok": True,
            "reason": "SKIPPED by --fidelity-ticks 0: nothing checked that the small viewport is irrelevant",
            "compared": 0,
        }
    prefix = [record for record in small if record["tick"] <= ticks]
    trimmed = [record for record in full if record["tick"] <= ticks]
    if records_digest(prefix) != records_digest(trimmed):
        return {
            "ok": False,
            "reason": (
                f"the full-fidelity session counted a different stream over its first {ticks} ticks "
                f"({len(trimmed)} frames) than the small one did ({len(prefix)}): what this counts "
                f"is not viewport-independent after all, and the whole session's figures are void"
            ),
            "compared": len(prefix),
        }
    return {
        "ok": True,
        "reason": (
            f"a full-size, bloom-on session counted the identical stream over its first {ticks} "
            f"ticks ({len(prefix)} frames), so the small viewport is not what is being measured"
        ),
        "compared": len(prefix),
    }


def mirrored_source_verdict(dist: Path, *, tree_root: Path = ROOT) -> dict[str, Any]:
    """The build being counted mirrors the SAME TypeScript this driver does.

    This driver reproduces two TypeScript files in Python, and `--self-test`
    keeps both mirrors honest by reading the originals back out of THIS TREE:

      * `frame_buffer.ts` -- the per-player column indices `FRAME_FIELDS`
        reads a frame block by, and the wire order `POSE_IDS` turns a pose
        code back into a name with;
      * `action_pose.ts` -- `lateralSign`'s cross product and its 1e-6 dead
        band, which `lateral_sign` mirrors, and the `SAVES` table that
        `SAVE_POSES` mirrors.

    THE GAP THIS CLOSES IS THAT `--self-test` ONLY EVER READS THIS TREE. A
    count's primary use is `--rev`: build another revision and compare. If
    that revision's `action_pose.ts` carried a different `lateralSign` --
    a different operand order, a different epsilon, a different `SAVES` set --
    the Python mirror would silently apply THIS tree's formula to THAT
    revision's frames, and nothing would notice. `--self-test` is looking at
    the wrong tree; the live page cross-check cannot reach `facing`/`dive_dir`
    at all (which is why `vector_shape_verdict` exists as a structural
    stand-in); and `vector_shape_verdict` only proves the columns are unit
    vectors, not that the formula consuming them still means the same thing.

    So both files are hashed at the counted build and compared with this
    tree's. It is a blunt instrument on purpose -- ANY difference refuses,
    including a comment-only one -- because the alternative is deciding which
    differences are safe, and the whole point is that this driver cannot see
    inside the revision it is counting.

    Reports `unavailable` rather than failing when the dist did not come from
    a source tree: a bare `--dist` is a reason this cannot run, not a reason
    to refuse.
    """
    mirrored = {path.name: path.relative_to(tree_root) for path in (FRAME_BUFFER_TS, ACTION_POSE_TS)}
    for parent in [dist] + list(dist.parents):
        if not all((parent / relative).is_file() for relative in mirrored.values()):
            continue
        if parent == tree_root:
            return {"ok": True, "reason": "the counted build is this tree", "sha256": {}}
        digests: dict[str, dict[str, str]] = {}
        drifted: list[str] = []
        for name, relative in mirrored.items():
            theirs = hashlib.sha256((parent / relative).read_bytes()).hexdigest()
            ours = hashlib.sha256((tree_root / relative).read_bytes()).hexdigest()
            digests[name] = {"build": theirs, "tree": ours}
            if theirs != ours:
                drifted.append(name)
        if drifted:
            return {
                "ok": False,
                "reason": (
                    f"{', '.join(drifted)} differs between {parent} and this tree, so what this "
                    f"driver mirrors in Python -- the frame's column indices, the pose-code order, "
                    f"`lateralSign`'s formula and dead band, the SAVES table -- is checked against "
                    f"the wrong revision. --self-test only ever reads this tree, so nothing else "
                    f"would notice"
                ),
                "sha256": digests,
                "drifted": drifted,
            }
        return {
            "ok": True,
            "reason": f"the counted build's {' and '.join(sorted(mirrored))} are byte-identical to this tree's",
            "sha256": digests,
            "drifted": [],
        }
    return {
        "ok": True,
        "reason": "unavailable: this dist has no source tree beside it, so the mirrored sources were not compared",
        "sha256": {},
        "drifted": [],
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
        # WebDriver's default script timeout is 30 s, and `__gcDriver.step` is
        # a SYNCHRONOUS call that runs a whole chunk of frames inside it. On
        # the software GL path a chunk of `PUMP_CHUNK_FRAMES` can pass 30 s --
        # `scan --scan-ticks 3000 --gpu-mode software` hits it today -- and
        # the failure is a bare `TimeoutException` from selenium's internals
        # that says nothing about frames. Raised here rather than by shrinking
        # the chunk, because the chunk size is a round-trip/latency tradeoff
        # and this is a hang budget.
        self.driver.set_script_timeout(SCRIPT_TIMEOUT_SECONDS)
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


def read_effects_diagnostics(driver: Any) -> dict[str, Any]:
    """`effects.diagnostics()` through the page's `__gcScene` handle.

    Returns `{"unavailable": ...}` rather than raising when the handle is not
    there: an older harness build is a reason this guard cannot run, not a
    reason to fail a capture. `renderer_state_verdict` treats it that way.
    """
    try:
        value = driver.execute_script(
            """
            const scene = window.__gcScene;
            if (!scene || !scene.effects || typeof scene.effects.diagnostics !== "function") {
              return {unavailable: "window.__gcScene.effects is not exposed by this harness build"};
            }
            const d = scene.effects.diagnostics();
            return {particle_count: d.particle_count, trail_count: d.trail_count};
            """
        )
    except Exception as error:  # page navigated away/closed underneath us
        return {"unavailable": str(error)}
    return value if isinstance(value, dict) else {"unavailable": "effects.diagnostics() returned a non-dict"}


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
        effects_state = read_effects_diagnostics(driver)
    return {
        "label": label,
        "seed": seed if seed is not None else args.seed,
        "bot_seed": args.bot_seed,
        "hashes": images,
        "files": files,
        "poses_at": poses_at,
        "effects": effects_state,
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
    find at the same seed. `pose_hold_verdict` checks that rather than
    assuming it -- see its docstring.
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


def count_run(
    args: argparse.Namespace,
    base_url: str,
    *,
    label: str,
    ticks: int,
    width: int,
    height: int,
    full_fidelity: bool,
) -> dict[str, Any]:
    """One counting session: arm the frame reader, step, read the rows back.

    Nothing is captured and nothing is drawn that anyone looks at. The session
    still renders -- the page has one loop and it draws -- but small, with
    bloom and the stadium off, because what is counted comes out of
    `gc_render::frame::build`, which takes a `MatchState` and none of those.
    `fidelity_verdict` is the check on that argument, not this docstring.
    """
    log = Path(args.log_dir) / f"{label}-webdriver.log"
    codes = [POSE_IDS.index(pose) + 1 for pose in LEAN_POSES]
    with BrowserSlot(args, log, label) as driver:
        # A counting session draws a small, bloom-less, stadium-less frame
        # nobody looks at, because 24,000 frames of full-fidelity SwiftShader
        # buys nothing a count can use: `frame::build` takes a `MatchState`
        # and no viewport, no bloom flag and no stadium. That argument is what
        # `fidelity_verdict` checks -- the full-fidelity session below turns
        # every one of these levers back on.
        extra: dict[str, Any] = {} if full_fidelity else {"bloom": 0, "stadium": 0}
        _open_page(
            driver,
            base_url,
            seed=args.seed,
            bot_seed=args.bot_seed,
            width=width,
            height=height,
            extra=page_extras(args, extra),
        )
        gpu = probe_gpu(driver, CANVAS_SELECTOR)
        # ARMED BEFORE THE FIRST STEP. `record()` writes one entry per tick as
        # the tick happens and never backfills, so a watch installed late
        # silently counts a shorter session than the one reported.
        armed = driver.execute_script(
            "return window.__gcDriver.leanWatch(arguments[0], arguments[1]);", codes, DEFAULT_ROW_LIMIT
        )
        if not armed.get("wasm_available"):
            raise RuntimeError(
                f"{label}: the render frame reader has no WebAssembly instance ({armed.get('wasm_note')}). "
                f"The page's wasm module did not go through WebAssembly.instantiate/instantiateStreaming, "
                f"so nothing can be counted -- see the bootstrap's section 3."
            )
        remaining = ticks
        state: dict[str, Any] = {}
        started = time.monotonic()
        last_report = started
        while remaining > 0:
            chunk = min(remaining, PUMP_CHUNK_FRAMES)
            state = driver.execute_script("return window.__gcDriver.step(arguments[0]);", chunk)
            remaining -= chunk
            # A 24,000-tick software-GL session is minutes long. Without this
            # it is indistinguishable from a hang, which is how you end up
            # killing a run that was fine.
            if time.monotonic() - last_report > 30:
                last_report = time.monotonic()
                print(
                    f"[browser_match_harness] {label}: tick {state.get('tick')}/{ticks}, "
                    f"{round(time.monotonic() - started)}s elapsed",
                    flush=True,
                )
            # NOT BROKEN OUT OF LIKE A LEGITIMATE END. `_open_page` already
            # refuses a boot error; treating a mid-run one as an ordinary
            # end-of-session hands a truncated stream to the report, and a
            # deterministic seed makes both control sessions die at the same
            # tick, so the control would call the pair identical and fine.
            # `session_completeness_verdict` is the backstop that catches
            # every other way a session can stop short; this is the loud
            # failure at the point it happens.
            if state.get("status") == "error":
                raise RuntimeError(
                    f"{label}: the match harness reported an error at tick {state.get('tick')} of "
                    f"{ticks} requested: {state.get('error')}"
                )
            if state.get("status") == "finished":
                break
        raw = driver.execute_script("return window.__gcDriver.leanRows();")
        effects_state = read_effects_diagnostics(driver)
        final = driver_state(driver)
    elapsed = round(time.monotonic() - started, 1)
    print(
        f"[browser_match_harness] {label}: {final.get('tick')} of {ticks} ticks, "
        f"{raw.get('frames_read')} frames read, {len(raw.get('rows') or [])} watched frames, {elapsed}s"
    )
    return {
        "label": label,
        "raw": raw,
        "records": lean_records(raw.get("rows") or []),
        "effects": effects_state,
        "gpu": gpu,
        "viewport": {"width": width, "height": height, "bloom": full_fidelity, "stadium": full_fidelity},
        "elapsed_seconds": elapsed,
        # What `session_completeness_verdict` reads. The tick budget is carried
        # next to what was actually reached, in the same dict, so a reader of
        # the report cannot see one without the other.
        "budget": {
            "requested_ticks": ticks,
            "final_tick": final.get("tick"),
            "recorded_ticks": final.get("recorded_ticks"),
            "frames_read": raw.get("frames_read"),
            "status": final.get("status"),
            "error": final.get("error"),
        },
        "final_state": {key: final.get(key) for key in ("tick", "score", "status", "frames", "recorded_ticks")},
    }


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
    # Both capturing sessions, not just the one whose files get reported.
    pose_held = pose_hold_verdict(
        pose_scan["pose"] if pose_scan is not None else None,
        {run_a["label"]: run_a["poses_at"], run_b["label"]: run_b["poses_at"]},
    )
    renderer_state = renderer_state_verdict({run_a["label"]: run_a["effects"], run_b["label"]: run_b["effects"]})
    ok = control["ok"] and frozen["ok"] and pose_held["ok"] and renderer_state["ok"]
    report = {
        "kind": "capture",
        "dist": str(dist),
        "ticks": ticks,
        "pose_scan": pose_scan,
        "runs": [run_a, run_b],
        "control": control,
        "frozen_capture": frozen,
        "pose_held": pose_held,
        "renderer_state": renderer_state,
        "verdict": "ok" if ok else "refused",
    }
    emit(report, out_dir)
    print_guard("unseeded-renderer-state guard", renderer_state)
    print_guard("control (two sessions, same build)", control)
    print_guard("frozen-capture guard", frozen)
    print_guard("pose re-verification (scan vs capture)", pose_held)
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
    # Checked across all four sessions: if the unseeded `effects` layer is
    # live it is the cause of whatever the controls are about to say, and
    # naming it here is the whole point of the guard.
    renderer_state = renderer_state_verdict({run["label"]: run["effects"] for run in (a1, a2, b1, b2)})
    controls_ok = (
        control_a["ok"] and control_b["ok"] and frozen_a["ok"] and frozen_b["ok"] and renderer_state["ok"]
    )

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
        "renderer_state": renderer_state,
        "comparison": comparison if controls_ok else None,
        "comparison_withheld": None if controls_ok else "a control failed; see control_a/control_b",
        "verdict": "ok" if controls_ok and comparison["ok"] else "refused",
    }
    emit(report, out_dir)
    print_guard("unseeded-renderer-state guard", renderer_state)
    print_guard("control A (two sessions, build A)", control_a)
    print_guard("control B (two sessions, build B)", control_b)
    print_guard("frozen-capture guard, build A", frozen_a)
    print_guard("frozen-capture guard, build B", frozen_b)
    if not controls_ok:
        print("[browser_match_harness] REFUSED: a control guard failed, so the A/B result is withheld entirely.")
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


def _count_table(counts: dict[str, Any]) -> list[str]:
    lines = [f"{'family / pose':<22}{'frames':>8}{'leaned':>8}{'flat':>7}{'episodes':>10}{'always':>8}{'never':>7}{'popped':>8}"]
    rows: list[tuple[str, dict[str, Any]]] = [(f"[{name}]", entry) for name, entry in counts["families"].items()]
    rows += list(counts["by_pose"].items())
    for name, entry in rows:
        episodes = entry["episodes"]
        lines.append(
            f"{name:<22}{entry['frames']:>8}{entry['leaned']:>8}{entry['not_leaned']:>7}"
            f"{episodes['total']:>10}{episodes['always_leaning']:>8}"
            f"{episodes['never_leaning']:>7}{episodes['popped_mid_episode']:>8}"
        )
    return lines


def command_count(args: argparse.Namespace) -> int:
    dist = resolve_build(args.dist, args.rev, args, Path(args.build_root))
    mirrored = mirrored_source_verdict(dist)
    server, _thread, base_url = serve_dist(dist)
    try:
        run_a = count_run(
            args, base_url, label="count-1", ticks=args.count_ticks,
            width=args.count_width, height=args.count_height, full_fidelity=False,
        )
        run_b = count_run(
            args, base_url, label="count-2", ticks=args.count_ticks,
            width=args.count_width, height=args.count_height, full_fidelity=False,
        )
        run_full: dict[str, Any] | None = None
        if args.fidelity_ticks > 0:
            run_full = count_run(
                args, base_url, label="count-fidelity", ticks=args.fidelity_ticks,
                width=args.width, height=args.height, full_fidelity=True,
            )
    finally:
        server.shutdown()

    runs = {run["label"]: run for run in [run_a, run_b] + ([run_full] if run_full else [])}
    raw_by_label = {label: run["raw"] for label, run in runs.items()}
    records_by_label = {label: run["records"] for label, run in runs.items()}

    guards = {
        "frame read": frame_read_verdict(raw_by_label),
        "page cross-check": page_cross_check_verdict(raw_by_label),
        "unposed dives": unposed_dive_verdict(raw_by_label),
        "pose code table": pose_code_verdict(records_by_label),
        "vector shape": vector_shape_verdict(records_by_label),
        "control": count_control_verdict(run_a["records"], run_b["records"]),
        "viewport independence": fidelity_verdict(
            run_a["records"], run_full["records"] if run_full else [], args.fidelity_ticks
        ),
        "renderer state": renderer_state_verdict({label: run["effects"] for label, run in runs.items()}),
        "session completeness": session_completeness_verdict({label: run["budget"] for label, run in runs.items()}),
        "mirrored sources": mirrored,
    }
    failed = [name for name, verdict in guards.items() if not verdict["ok"]]
    counts = tally(run_a["records"])

    for name, verdict in guards.items():
        print_guard(name, verdict)
    print()
    for line in _count_table(counts):
        print(line)
    if failed:
        print("\n[browser_match_harness] the table above is NOT a result: see the failed guard(s).")

    report = {
        "kind": "count",
        "session": {
            "seed": args.seed,
            "bot_seed": args.bot_seed,
            "combat_enabled": bool(args.combat),
            "requested_ticks": args.count_ticks,
            "teams": "nebula/orion",
            "dist": str(dist),
            "rev": args.rev,
            "count_viewport": [args.count_width, args.count_height],
            "fidelity_ticks": args.fidelity_ticks,
            "gpu_mode": args.gpu_mode,
        },
        "guards": {name: verdict for name, verdict in guards.items()},
        # WITHHELD ON A FAILED GUARD, the same way `command_ab` withholds its
        # `comparison`. A downstream reader -- #450 and #451 are the named
        # ones -- should not have to iterate nine verdicts to learn whether
        # `counts` means anything, and a JSON file outlives the terminal that
        # printed the refusal.
        "verdict": "refused" if failed else "ok",
        "failed_guards": failed,
        "counts": None if failed else counts,
        "counts_withheld": f"guard(s) failed: {', '.join(failed)}" if failed else None,
        "runs": {
            label: {
                "final_state": run["final_state"],
                # The tick budget carried NEXT TO the ticks actually reached.
                # Council review of PR #456 found the report surfaced only the
                # requested count, so a truncated session read as a whole one.
                "budget": run["budget"],
                "viewport": run["viewport"],
                "elapsed_seconds": run["elapsed_seconds"],
                "watched_frames": len(run["records"]),
                "frames_read": run["raw"].get("frames_read"),
                "layout": run["raw"].get("layout"),
                "gpu": run["gpu"],
                "digest": records_digest(run["records"]),
            }
            for label, run in runs.items()
        },
    }
    emit(report, Path(args.out))
    # THE COUNTED FRAMES THEMSELVES, next to the totals.
    #
    # A total is only ever an answer to the question that was asked. #449
    # reported "save frames"; whether that reading included `keeper_tip`, and
    # where one episode ended and the next began, are exactly the questions a
    # later reader has about a number they did not take -- and re-running a
    # 24,000-tick session to re-slice it is half an hour. Every counted frame
    # is written out so any grouping can be re-derived from the same
    # measurement, and so #450/#451 can ask their own questions of it.
    stream = Path(args.out) / "records.json"
    stream.write_text(
        json.dumps(
            {
                "session": report["session"],
                # Carried here too: this file is the one a later analysis
                # opens, often without the report beside it.
                "verdict": report["verdict"],
                "failed_guards": failed,
                "digest": records_digest(run_a["records"]),
                "columns": ["tick", "slot", "pose", "facing_x", "facing_y", "dive_dir_x", "dive_dir_y", "lateral_sign"],
                "rows": [
                    [
                        record["tick"], record["slot"], record["pose"],
                        record["facing"][0], record["facing"][1],
                        record["dive_dir"][0], record["dive_dir"][1],
                        record["sign"],
                    ]
                    for record in run_a["records"]
                ],
            },
            indent=1,
        )
        + "\n",
        encoding="utf-8",
    )
    print(f"[browser_match_harness] counted frames: {stream}")
    if failed:
        print(f"[browser_match_harness] REFUSED: {', '.join(failed)} -- the counts above are not evidence")
        return 1
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


def _pose_codes_from_typescript() -> dict[int, str]:
    """`poseIdFromCode`'s switch, as {wire code: pose id}.

    `count` turns a wire code back into a name by `POSE_IDS[code - 1]`, which
    only works while that tuple is in wire order -- and the existing check
    above compares the two as SETS, so a rotation would pass it."""
    text = FRAME_BUFFER_TS.read_text(encoding="utf-8")
    match = re.search(r"function poseIdFromCode\(code: number\).*?\n}", text, re.DOTALL)
    if match is None:
        raise RuntimeError(f"could not find poseIdFromCode in {FRAME_BUFFER_TS}")
    pairs = re.findall(r'case (\d+):\s*\n\s*return "([a-z_]+)";', match.group(0))
    return {int(code): name for code, name in pairs}


def _player_fields_from_typescript() -> dict[str, int]:
    """`decode`'s own `column(words, playersAt, N, count)` arguments.

    The bootstrap reads the block's shape out of its header but has to be told
    which field index means what. This is where those numbers come from, so
    `--self-test` reads them back from the same file rather than trusting the
    constant."""
    text = FRAME_BUFFER_TS.read_text(encoding="utf-8")
    pairs = re.findall(r"(\w+): column\(words, playersAt, (\d+), count\)", text)
    return {name: int(index) for name, index in pairs}


def _lateral_sign_from_typescript() -> dict[str, Any]:
    """`lateralSign`'s cross product and dead band, out of `action_pose.ts`."""
    text = ACTION_POSE_TS.read_text(encoding="utf-8")
    match = re.search(r"function lateralSign\(.*?\n}", text, re.DOTALL)
    if match is None:
        raise RuntimeError(f"could not find lateralSign in {ACTION_POSE_TS}")
    body = match.group(0)
    epsilon = re.search(r"Math\.abs\(alongLeft\) < ([0-9.e-]+)", body)
    formula = re.search(r"const alongLeft = ([^;]+);", body)
    return {
        "epsilon": float(epsilon.group(1)) if epsilon else None,
        "formula": " ".join(formula.group(1).split()) if formula else None,
        "positive_is": 1 if "alongLeft > 0 ? 1 : -1" in " ".join(body.split()) else None,
    }


def _save_poses_from_typescript() -> tuple[str, ...]:
    """The `SAVES` table's keys -- the pose ids `save()` will pose at all."""
    text = ACTION_POSE_TS.read_text(encoding="utf-8")
    match = re.search(r"const SAVES: Readonly<Record<string, SaveSpec>> = \{(.*?)\n\};", text, re.DOTALL)
    if match is None:
        raise RuntimeError(f"could not find SAVES in {ACTION_POSE_TS}")
    return tuple(re.findall(r"^\s*(\w+): \{", match.group(1), re.MULTILINE))


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

    # 3b. Pose RE-VERIFICATION. The scan's cheapness -- a small, bloom-less
    #     viewport -- is bought with this check, so it gets the same
    #     treatment as the control rather than living in a docstring.
    held_ok = {"run-1": {50: ["locomotion", "aerial_action"]}, "run-2": {50: ["aerial_action"]}}
    check("pose re-verification passes when both runs hold the pose", pose_hold_verdict("aerial_action", held_ok)["ok"])
    check("pose re-verification is a no-op when no pose was requested", pose_hold_verdict(None, {"run-1": {}})["ok"])
    drifted = pose_hold_verdict("aerial_action", {"run-1": {50: ["aerial_action"], 116: ["locomotion", "tackle"]}})
    check(
        "pose re-verification catches a scan/capture disagreement",
        not drifted["ok"] and drifted["missing"] == [{"run": "run-1", "tick": 116, "held": ["locomotion", "tackle"]}],
        str(drifted),
    )
    second_only = pose_hold_verdict("tackle", {"run-1": {50: ["tackle"]}, "run-2": {50: ["locomotion"]}})
    check(
        "pose re-verification checks the SECOND capturing run too",
        not second_only["ok"] and second_only["missing"][0]["run"] == "run-2",
        str(second_only),
    )
    check(
        "pose re-verification refuses when a pose was requested but nothing was captured",
        not pose_hold_verdict("tackle", {"run-1": {}})["ok"],
    )

    # 3c. The unseeded-particle guard: a named cause instead of a mystery.
    check(
        "renderer-state guard passes on an empty effects layer",
        renderer_state_verdict({"run-1": {"particle_count": 0, "trail_count": 0}})["ok"],
    )
    live = renderer_state_verdict(
        {"run-1": {"particle_count": 0, "trail_count": 0}, "run-2": {"particle_count": 7, "trail_count": 0}}
    )
    check(
        "renderer-state guard catches live unseeded particles and names the run",
        not live["ok"] and live["offenders"] == [{"run": "run-2", "particle_count": 7, "trail_count": 0}],
        str(live),
    )
    check(
        "renderer-state guard also catches a live ball trail",
        not renderer_state_verdict({"run-1": {"particle_count": 0, "trail_count": 3}})["ok"],
    )
    check(
        "renderer-state guard tolerates a harness build that does not expose the handle",
        renderer_state_verdict({"run-1": {"unavailable": "not exposed"}})["ok"],
    )

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
    check("bootstrap exposes the render-frame reader", "leanWatch" in source and "leanRows" in source)
    check(
        "bootstrap intercepts both wasm instantiation entry points",
        'interceptInstantiation("instantiateStreaming")' in source and 'interceptInstantiation("instantiate")' in source,
    )

    # 8b. `count`'s own logic: the sign, the episodes, and the guards.
    check("lateralSign is zero when dive and facing are parallel", lateral_sign(0.0, 1.0, 0.0, 1.0) == 0)
    check("lateralSign is zero when they are antiparallel", lateral_sign(0.0, -1.0, 0.0, 1.0) == 0)
    check("lateralSign is +1 diving to the character's own left", lateral_sign(0.0, 1.0, -1.0, 0.0) == 1)
    check("lateralSign is -1 diving to their right", lateral_sign(0.0, 1.0, 1.0, 0.0) == -1)
    check("lateralSign is zero with no dive direction at all", lateral_sign(0.0, 0.0, 1.0, 0.0) == 0)
    check(
        "lateralSign's dead band is exclusive, as the TypeScript's `<` is",
        lateral_sign(LATERAL_SIGN_EPSILON, 0.0, 0.0, 1.0) == 1
        and lateral_sign(LATERAL_SIGN_EPSILON / 2, 0.0, 0.0, 1.0) == 0,
    )

    def _row(tick: int, slot: int, pose: str, leaned: bool) -> list[Any]:
        code = POSE_IDS.index(pose) + 1
        # Facing across the dive when it leans, along it when it does not --
        # the exact degeneracy #449 is about.
        return [tick, slot, code, 1.0, 0.0, 0.0, 1.0, pose] if leaned else [tick, slot, code, 0.0, 1.0, 0.0, 1.0, pose]

    episode_rows = (
        [_row(t, 1, "keeper_dive", True) for t in range(10, 15)]           # one always-leaning save
        + [_row(t, 1, "keeper_dive", False) for t in range(40, 44)]        # one that never leans
        + [_row(t, 2, "keeper_stretch", t < 72) for t in range(70, 75)]    # one that pops mid-save
        + [_row(t, 2, "keeper_get_up", True) for t in range(75, 78)]       # a separate family
    )
    counted = tally(lean_records(episode_rows))
    save = counted["families"]["save"]
    check(
        "episodes split by slot and by tick gap, and classify three ways",
        save["frames"] == 14
        and save["leaned"] == 7
        and save["episodes"] == {"total": 3, "always_leaning": 1, "never_leaning": 1, "popped_mid_episode": 1},
        str(save),
    )
    check(
        "get-up is counted as its own family, not folded into saves",
        counted["families"]["get_up"]["frames"] == 3 and counted["families"]["get_up"]["episodes"]["total"] == 1,
        str(counted["families"]["get_up"]),
    )
    check(
        "a save family spanning two pose ids is one episode, not two",
        count_family(
            lean_records(
                [_row(t, 1, "keeper_dive", True) for t in range(10, 13)]
                + [_row(t, 1, "keeper_stretch", True) for t in range(13, 16)]
            ),
            SAVE_POSES,
        )["episodes"]["total"]
        == 1,
    )
    check(
        "tip is reported both inside and outside the save family",
        counted["families"]["save"]["frames"]
        == counted["families"]["save_excluding_tip"]["frames"] + counted["families"]["tip"]["frames"],
    )

    leaning_records = lean_records([_row(10, 1, "keeper_dive", True)])
    check("count control passes on two identical streams", count_control_verdict(leaning_records, list(leaning_records))["ok"])
    check(
        "count control refuses two sessions that counted different frames",
        not count_control_verdict(leaning_records, lean_records([_row(11, 1, "keeper_dive", True)]))["ok"],
    )
    check("count control refuses two empty sessions", not count_control_verdict([], [])["ok"])
    check(
        "count control catches a stream that differs only in WHICH frames leaned",
        not count_control_verdict(leaning_records, lean_records([_row(10, 1, "keeper_dive", False)]))["ok"],
    )

    check("frame-read guard passes on a healthy run", frame_read_verdict({"a": {"wasm_available": True, "frames_read": 9}})["ok"])
    check(
        "frame-read guard refuses when no wasm instance was adopted",
        not frame_read_verdict({"a": {"wasm_available": False, "wasm_note": "none"}})["ok"],
    )
    check(
        "frame-read guard refuses a truncated recording rather than reporting a prefix",
        not frame_read_verdict({"a": {"wasm_available": True, "frames_read": 9, "truncated": True}})["ok"],
    )
    check(
        "page cross-check refuses a layout that has moved",
        not page_cross_check_verdict({"a": {"cross_check_failures": 3, "cross_check_examples": []}})["ok"],
    )
    check("page cross-check passes when nothing disagreed", page_cross_check_verdict({"a": {"cross_check_failures": 0}})["ok"])
    check("unposed-dive guard passes when the wire named a pose for every dive", unposed_dive_verdict({"a": {"unposed_dive_frames": 0}})["ok"])
    check(
        "unposed-dive guard refuses a session with a save the count cannot see",
        not unposed_dive_verdict({"a": {"unposed_dive_frames": 4}})["ok"],
    )

    # 8c. The truncation guard. A control CANNOT catch a short run -- both
    #     sessions stop at the same tick on a deterministic seed and the
    #     control calls the pair identical -- so this is the only thing
    #     between a fraction of a session and a confidently reported count.
    whole = {"requested_ticks": 24000, "final_tick": 24000, "recorded_ticks": 24000,
             "frames_read": 24000, "status": "running", "error": None}
    check("completeness guard passes on a session that covered its budget",
          session_completeness_verdict({"a": whole})["ok"])
    short = session_completeness_verdict({"a": {**whole, "final_tick": 9312}})
    check(
        "completeness guard catches a session that stopped short, and names the tick",
        not short["ok"] and "9312" in short["problems"][0]["problem"],
        str(short),
    )
    errored = session_completeness_verdict({"a": {**whole, "status": "error", "error": "boom", "final_tick": 400}})
    check(
        "completeness guard catches a page error and reports its message",
        not errored["ok"] and "boom" in errored["problems"][0]["problem"],
        str(errored),
    )
    ended = session_completeness_verdict({"a": {**whole, "status": "finished", "final_tick": 7200}})
    check(
        "completeness guard distinguishes a match that ended from a run that died",
        not ended["ok"] and "the match ended" in ended["problems"][0]["problem"],
        str(ended),
    )
    frozen_ticks = session_completeness_verdict({"a": {**whole, "recorded_ticks": 5000}})
    check(
        "completeness guard catches a frozen recorder even when the tick reached the budget",
        not frozen_ticks["ok"] and "recorded ticks" in frozen_ticks["problems"][0]["problem"],
        str(frozen_ticks),
    )
    check(
        "completeness guard catches a reader that read fewer frames than were stepped",
        not session_completeness_verdict({"a": {**whole, "frames_read": 5000}})["ok"],
    )
    check(
        "completeness guard checks EVERY session, not just the first",
        not session_completeness_verdict({"a": whole, "b": {**whole, "final_tick": 12}})["ok"],
    )

    # 8d. The mirrored-source guard, which had NO coverage at all -- not even
    #     positive-path -- until council review of PR #456 found the claim
    #     that it did. It is the only guard that can see a `--rev` build whose
    #     own TypeScript no longer matches what this driver mirrors in Python,
    #     and the checks in section 10 below cannot see that themselves
    #     because they only ever read ROOT.
    with tempfile.TemporaryDirectory() as scratch:
        build = Path(scratch) / "build"
        for source in (FRAME_BUFFER_TS, ACTION_POSE_TS):
            copy = build / source.relative_to(ROOT)
            copy.parent.mkdir(parents=True, exist_ok=True)
            copy.write_bytes(source.read_bytes())
        dist = build / "v2" / "tools" / "browser_match_harness" / "dist"
        dist.mkdir(parents=True, exist_ok=True)
        same = mirrored_source_verdict(dist, tree_root=ROOT)
        check("mirrored-source guard passes when the counted build matches this tree", same["ok"], str(same))
        check(
            "mirrored-source guard hashes BOTH mirrored files, not just the frame layout",
            set(same.get("sha256", {})) == {FRAME_BUFFER_TS.name, ACTION_POSE_TS.name},
            str(sorted(same.get("sha256", {}))),
        )
        # The drift this driver was blind to: another revision's own
        # `lateralSign`, silently measured with this revision's formula.
        pose_copy = build / ACTION_POSE_TS.relative_to(ROOT)
        pose_copy.write_bytes(pose_copy.read_bytes().replace(b"1e-6", b"1e-3", 1))
        drifted_pose = mirrored_source_verdict(dist, tree_root=ROOT)
        check(
            "mirrored-source guard catches a drifted action_pose.ts (the mirror-drift gap)",
            not drifted_pose["ok"] and drifted_pose["drifted"] == [ACTION_POSE_TS.name],
            str(drifted_pose.get("drifted")),
        )
        pose_copy.write_bytes(ACTION_POSE_TS.read_bytes())
        buffer_copy = build / FRAME_BUFFER_TS.relative_to(ROOT)
        buffer_copy.write_bytes(buffer_copy.read_bytes().replace(b"playersAt, 12", b"playersAt, 15", 1))
        drifted_buffer = mirrored_source_verdict(dist, tree_root=ROOT)
        check(
            "mirrored-source guard catches a moved per-player column",
            not drifted_buffer["ok"] and drifted_buffer["drifted"] == [FRAME_BUFFER_TS.name],
            str(drifted_buffer.get("drifted")),
        )
        bare = mirrored_source_verdict(Path(scratch) / "nothing-here", tree_root=ROOT)
        check(
            "mirrored-source guard reports unavailable for a bare --dist rather than refusing",
            bare["ok"] and "unavailable" in bare["reason"],
            str(bare),
        )
    own = mirrored_source_verdict(HARNESS_DIST, tree_root=ROOT)
    check(
        "mirrored-source guard recognises this tree's own dist as this tree",
        own["ok"] and "this tree" in own["reason"],
        str(own),
    )

    check(
        "pose-code guard catches a driver/page disagreement",
        not pose_code_verdict({"a": lean_records([[10, 1, POSE_IDS.index("keeper_dive") + 1, 1.0, 0.0, 0.0, 1.0, "keeper_tip"]])})["ok"],
    )
    check("pose-code guard refuses an empty recording", not pose_code_verdict({"a": []})["ok"])
    check("vector-shape guard passes on unit and zero vectors", vector_shape_verdict({"a": leaning_records})["ok"])
    check(
        "vector-shape guard catches a non-unit column",
        not vector_shape_verdict({"a": lean_records([[10, 1, 8, 3.7, 0.0, 0.0, 1.0, "keeper_dive"]])})["ok"],
    )
    check(
        "viewport-independence guard compares the small run's prefix",
        fidelity_verdict(leaning_records, list(leaning_records), 100)["ok"],
    )
    check(
        "viewport-independence guard catches a viewport-dependent count",
        not fidelity_verdict(leaning_records, lean_records([_row(10, 1, "keeper_dive", False)]), 100)["ok"],
    )
    skipped = fidelity_verdict(leaning_records, [], 0)
    check("viewport-independence guard says SKIPPED rather than passing silently", skipped["ok"] and "SKIPPED" in skipped["reason"])

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

    # 10. `count` reads the wire by index and by code. Both tables come from
    #     TypeScript, so both are read back out of it here -- the same
    #     anti-drift treatment POSE_IDS gets above, and the only thing
    #     standing between a moved column and a plausible wrong number that
    #     does not need a browser to catch.
    if FRAME_BUFFER_TS.is_file():
        codes = _pose_codes_from_typescript()
        expected = {index + 1: pose for index, pose in enumerate(POSE_IDS)}
        check(
            f"POSE_IDS is in WIRE ORDER, not merely the same set ({len(codes)} codes)",
            codes == expected,
            f"differs at: {sorted(code for code in set(codes) | set(expected) if codes.get(code) != expected.get(code))}",
        )
        fields = _player_fields_from_typescript()
        drifted = {
            name: (index, fields.get(name))
            for name, index in FRAME_FIELDS.items()
            if name != "magic" and fields.get(name) != index
        }
        check(
            f"FRAME_FIELDS matches decode()'s own column indices in {FRAME_BUFFER_TS.name}",
            not drifted,
            f"here vs frame_buffer.ts: {drifted}",
        )
        magic = re.search(r"export const MAGIC = (0x[0-9a-fA-F]+);", FRAME_BUFFER_TS.read_text(encoding="utf-8"))
        check(
            "FRAME_FIELDS['magic'] matches frame_buffer.ts's MAGIC",
            magic is not None and int(magic.group(1), 16) == FRAME_FIELDS["magic"],
        )

    if ACTION_POSE_TS.is_file():
        formula = _lateral_sign_from_typescript()
        check(
            "the epsilon mirrors action_pose.ts's own dead band",
            formula["epsilon"] == LATERAL_SIGN_EPSILON,
            f"action_pose.ts: {formula['epsilon']}, here: {LATERAL_SIGN_EPSILON}",
        )
        check(
            "lateralSign's cross product has not been re-ordered under this mirror",
            formula["formula"] == "diveDir.x * fy - diveDir.y * fx",
            f"action_pose.ts: {formula['formula']!r}",
        )
        check("a positive cross product still means the character's LEFT", formula["positive_is"] == 1)
        from_ts = _save_poses_from_typescript()
        check(
            f"SAVE_POSES is exactly action_pose.ts's SAVES table ({len(from_ts)} ids)",
            set(from_ts) == set(SAVE_POSES),
            f"only in TS: {sorted(set(from_ts) - set(SAVE_POSES))}; only here: {sorted(set(SAVE_POSES) - set(from_ts))}",
        )
        # The other half of what `count` counts. `tip()` reaches `lateralSign`
        # for exactly one pose id; if that ever stops being true, the get-up
        # family here is measuring something that no longer exists.
        tip_body = re.search(r"function tip\(.*?\n}\n", ACTION_POSE_TS.read_text(encoding="utf-8"), re.DOTALL)
        check(
            "keeper_get_up still reaches lateralSign in tip(), so it is still worth counting",
            tip_body is not None
            and 'poseId === "keeper_get_up"' in tip_body.group(0)
            and "lateralSign(" in tip_body.group(0),
        )
    else:
        check("action_pose.ts is readable", False, f"{ACTION_POSE_TS} is missing")

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
            "is. REQUIRED for any of the seven combat_* poses: without it the session builds no "
            "CombatMatchState at all, so gc_wasm's frame_options has nothing to hand frame::build "
            "and combat_stagger and its six siblings cannot occur. Before #441 they could not "
            "occur even WITH it (see match_harness.ts's note on `combatEnabled`)."
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

    p_count = sub.add_parser(
        "count",
        help="tally how many keeper save/get-up/tip frames actually reach their lean",
    )
    add_common(p_count)
    p_count.add_argument("--count-ticks", type=int, default=DEFAULT_COUNT_TICKS,
                         help="ticks to step in each counting session")
    p_count.add_argument("--count-width", type=int, default=DEFAULT_COUNT_WIDTH,
                         help="counting sessions render at this size; `fidelity_verdict` is what makes that safe")
    p_count.add_argument("--count-height", type=int, default=DEFAULT_COUNT_HEIGHT)
    p_count.add_argument("--fidelity-ticks", type=int, default=DEFAULT_FIDELITY_TICKS,
                         help="ticks of a third, full-size, bloom-on session that must count the same stream; "
                              "0 skips the check and says so in the report")
    p_count.add_argument("--dist", type=Path, default=None, help="a prebuilt harness dist to use as-is")
    p_count.add_argument("--rev", default=None, help="build the harness from this git revision first")
    p_count.add_argument("--out", type=Path, default=ROOT / "build" / "browser_match_harness-count")
    # This mode needs no GPU: it draws nothing anyone looks at. Defaulting it
    # to software keeps a long counting session off a machine's one display
    # adapter, which other work may be using.
    p_count.set_defaults(gpu_mode="software")

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
        "count": command_count,
    }
    try:
        return handlers[args.command](args)
    except RuntimeError as error:
        print(f"[browser_match_harness] {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
