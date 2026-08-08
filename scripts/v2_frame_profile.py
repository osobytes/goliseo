#!/usr/bin/env python3
"""Profile the v2 browser app's frame loop and attribute its frame drops.

## What this answers, and why the existing tooling could not

`scripts/browser_render_bench.py` compares Lua-vs-v2 THROUGHPUT on a fixed
render fixture and reads back percentiles each fixture computes itself. It is
a comparison harness, not a diagnostic: it cannot say *why* a particular frame
was late, and it does not drive the real app shell at all.

The match harness (`v2/tools/browser_match_harness`) reports
`simMs`/`decodeMs`/`populateMs`/`renderMs`, but as MEANS over a rolling
window -- the one statistic guaranteed to hide a drop, since a drop is a tail
event. It also deliberately assembles the scene TWICE per frame (see that
file's own note on splitting `populate` from `render`), so its absolute
numbers are not the product's.

So this script drives the PRODUCT entry (`v2/ts/packages/app/src/
browser_main.ts`) and collects three independent signals, because no single
one of them localises a drop on its own:

  1. PER-FRAME SAMPLES, from the dev-only `window.__gcFrames()` ring buffer
     that entry installs: rAF delta plus an `update`/`draw` split plus the JS
     heap size, per frame. Says WHICH frames were late and which phase owned
     them -- and, via the heap, whether a late frame was actually a GC pause
     that no phase timer would attribute to anything.
  2. A V8 CPU PROFILE, via CDP `Profiler` at a 100us sampling interval. Says
     WHICH FUNCTIONS the time went to. Aggregated here into self-time by
     function, which is what actually names a hotspot.
  3. A DEVTOOLS TRACE, via CDP `Tracing` with the `devtools.timeline` and
     `disabled-by-default-v8.gc` categories. Says what the ENGINE was doing
     outside our JS -- major/minor GC, raster, GPU work. This is the signal
     that distinguishes "our code is slow" from "our code allocated so much
     that the collector stalled the frame".

Signals 1 and 3 are deliberately redundant on GC (a heap drop in 1, an
explicit `MajorGC` event in 3). AGENTS.md §9's "never trust one signal"
applies to diagnosis as much as to gates: a GC hypothesis supported by both a
heap trace and the engine's own GC events is worth acting on, one supported by
neither is a guess.

## Mirrors scripts/browser_render_bench.py

Same pinned-asset resolution, same hardware-GL Chrome invocation
(`--use-gl=angle --use-angle=gl-egl --ignore-gpu-blocklist`, verified in this
environment to reach the real GPU with no DISPLAY), same bounded launch and
teardown imported from `browser_determinism` rather than reimplemented. The
one real departure is that this script needs CDP, so it uses
`execute_cdp_cmd` -- unavailable for Firefox, which is why this script is
Chrome-only and says so rather than silently degrading.

## Driving the app to a match

The product shell boots to the title screen, so a profile taken at boot would
profile menus. This clicks through via `window.__gcClickWidget` (the same
dev-only hook `browser_main.ts` installs for exactly this purpose), which
drives real `app.event` clicks computed from the LIVE layout rather than
guessed pixels: play -> next (squad) -> next (formation) -> kickoff. Then it
waits until `__gcFrames()` reports frames with `inMatch` true before starting
the profiler, so no menu frame lands in the sample.
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from collections import defaultdict
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "scripts"))

CHROME_HARDWARE_GL_ARGS = ("--use-gl=angle", "--use-angle=gl-egl", "--ignore-gpu-blocklist")
CHROME_SOFTWARE_GL_ARGS = ("--use-angle=swiftshader", "--enable-unsafe-swiftshader")

DEFAULT_URL = "http://localhost:5176"
SAMPLING_INTERVAL_US = 100
TRACE_CATEGORIES = (
    "devtools.timeline",
    "disabled-by-default-v8.gc",
    "v8",
    "blink.user_timing",
)

# A 60Hz budget. Anything past this missed a vsync.
FRAME_BUDGET_MS = 1000.0 / 60.0


def log(message: str) -> None:
    print(message, flush=True)


def find_chrome() -> Path:
    for candidate in ("/usr/bin/google-chrome", "/usr/bin/chromium", "/usr/bin/chromium-browser"):
        path = Path(candidate)
        if path.exists():
            return path
    raise SystemExit("no chrome binary found")


def find_chromedriver() -> Path:
    for candidate in (Path.home() / ".local/bin/chromedriver", Path("/usr/bin/chromedriver")):
        if candidate.exists():
            return candidate
    raise SystemExit("no chromedriver found")


def launch(binary: Path, driver_path: Path, log_path: Path, gpu_mode: str, window: str) -> Any:
    from selenium import webdriver
    from selenium.webdriver.chrome.options import Options
    from selenium.webdriver.chrome.service import Service

    log_path.parent.mkdir(parents=True, exist_ok=True)
    options = Options()
    options.binary_location = str(binary)
    options.add_argument("--headless=new")
    for argument in CHROME_HARDWARE_GL_ARGS if gpu_mode == "hardware" else CHROME_SOFTWARE_GL_ARGS:
        options.add_argument(argument)
    options.add_argument("--no-sandbox")
    options.add_argument("--disable-dev-shm-usage")
    options.add_argument("--disable-extensions")
    options.add_argument("--no-default-browser-check")
    options.add_argument("--no-first-run")
    options.add_argument(f"--window-size={window}")
    # `performance.memory` is what the per-frame heap column reads; without
    # this flag Chrome quantises it hard enough to hide a GC sawtooth.
    options.add_argument("--enable-precise-memory-info")
    # `Tracing.tracingComplete` carries the stream handle the trace is read
    # from, and it arrives as an EVENT. `execute_cdp_cmd` is request/response
    # only, so the event has to be picked up out of the performance log --
    # which requires asking for that log up front, here.
    options.set_capability("goog:loggingPrefs", {"performance": "ALL", "browser": "ALL"})
    service = Service(str(driver_path), log_output=str(log_path), popen_kw={"start_new_session": True})
    return webdriver.Chrome(service=service, options=options)


def click_into_match(driver: Any, timeout: float = 60.0) -> None:
    """Walk title -> squad -> formation -> tactic -> match via `__gcClickWidget`."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        ready = driver.execute_script("return typeof window.__gcClickWidget === 'function';")
        if ready:
            break
        time.sleep(0.5)
    else:
        raise SystemExit("app never installed __gcClickWidget (did it fail to boot?)")

    steps = ("play", "next", "next", "kickoff")
    for widget in steps:
        clicked = driver.execute_script("return window.__gcClickWidget(arguments[0]);", widget)
        log(f"    click {widget!r}: {'ok' if clicked else 'NOT FOUND'}")
        time.sleep(0.4)

    # Do not trust the clicks -- confirm the app is actually rendering match
    # frames (`inMatch`) before anything is profiled, so no menu frame can
    # pollute the sample.
    deadline = time.time() + timeout
    while time.time() < deadline:
        in_match = driver.execute_script(
            "const f = window.__gcFrames ? window.__gcFrames() : [];"
            "return f.length > 5 && f[f.length - 1].inMatch === true;"
        )
        if in_match:
            return
        time.sleep(0.5)
    raise SystemExit("never reached a match frame after clicking through")


GL_INSTRUMENTATION_JS = r"""
// Count the GL calls that shader compilation is made of, and TIME them.
//
// `WebGLRenderer.info.programs.length` is a LIVE count, so it cannot see churn:
// a frame that compiles one program and releases another leaves the length
// unchanged, and the profile still pays for the compile. Counting the
// underlying `linkProgram`/`compileShader`/`getProgramInfoLog` calls is what
// makes recompilation visible, and wrapping them in a timer is what shows that
// `getProgramInfoLog` is not bookkeeping but a synchronous GPU stall -- it
// blocks until the driver has finished compiling.
window.__glStats = { link: 0, compile: 0, infoLog: 0, linkMs: 0, compileMs: 0, infoLogMs: 0, events: [] };
for (const proto of [window.WebGL2RenderingContext && WebGL2RenderingContext.prototype,
                     window.WebGLRenderingContext && WebGLRenderingContext.prototype]) {
  if (!proto) continue;
  for (const [method, countKey, msKey] of [["linkProgram", "link", "linkMs"],
                                           ["compileShader", "compile", "compileMs"],
                                           ["getProgramInfoLog", "infoLog", "infoLogMs"]]) {
    const original = proto[method];
    if (typeof original !== "function" || original.__gcWrapped) continue;
    const wrapped = function (...args) {
      const t0 = performance.now();
      const result = original.apply(this, args);
      const dt = performance.now() - t0;
      window.__glStats[countKey] += 1;
      window.__glStats[msKey] += dt;
      if (dt > 1) window.__glStats.events.push({ m: method, t: Math.round(t0), ms: Math.round(dt * 10) / 10 });
      return result;
    };
    wrapped.__gcWrapped = true;
    proto[method] = wrapped;
  }
}
return true;
"""


def percentile(values: list[float], q: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round(q / 100.0 * (len(ordered) - 1)))))
    return ordered[index]


def summarise_frames(samples: list[dict[str, Any]]) -> dict[str, Any]:
    match = [s for s in samples if s.get("inMatch")]
    # The first match frames include one-time construction (the `Stadium` is
    # built lazily on the first draw -- browser_main.ts), which is a real cost
    # but not a "frame drop"; reported separately rather than blended in.
    warmup, steady = match[:10], match[10:]
    deltas = [s["delta"] for s in steady]
    updates = [s["update"] for s in steady]
    draws = [s["draw"] for s in steady]

    late = [s for s in steady if s["delta"] > FRAME_BUDGET_MS * 1.5]
    # A GC pause shows up as a late frame whose heap SHRANK against the frame
    # before it.
    gc_suspects = []
    for prev, cur in zip(steady, steady[1:]):
        if cur["delta"] > FRAME_BUDGET_MS * 1.5 and cur["heap"] < prev["heap"]:
            gc_suspects.append({"t": cur["t"], "delta": cur["delta"], "freedMB": (prev["heap"] - cur["heap"]) / 1e6})

    worst = sorted(steady, key=lambda s: -s["draw"])[:15]

    # SHADER COMPILES MID-MATCH. `THREE.WebGLRenderer` compiles a program the
    # first time a material/geometry/light combination is rendered, and
    # `getProgramInfoLog` (which three.js calls unless `debug.checkShaderErrors`
    # is off) forces a synchronous GPU round-trip. So a program count that keeps
    # CLIMBING after warmup is not a curiosity, it is a per-frame stall
    # generator: the count is the number of compiles, and every one landed on
    # some frame.
    #
    # Correlating the two directly -- mean draw time on frames where the count
    # rose, against frames where it did not -- is what turns "the profiler says
    # shader compilation is expensive" into "shader compilation is what is
    # making frames late". A profile alone cannot distinguish a big one-time
    # warmup cost from a recurring one.
    compiles = []
    for prev, cur in zip(steady, steady[1:]):
        grew = cur.get("programs", 0) - prev.get("programs", 0)
        if grew > 0:
            compiles.append({"t": round(cur["t"], 1), "newPrograms": grew, "drawMs": round(cur["draw"], 2)})
    compile_draws = [c["drawMs"] for c in compiles]
    quiet_draws = [
        cur["draw"] for prev, cur in zip(steady, steady[1:]) if cur.get("programs", 0) == prev.get("programs", 0)
    ]

    return {
        "programs": {
            "atStart": steady[0].get("programs", 0) if steady else 0,
            "atEnd": steady[-1].get("programs", 0) if steady else 0,
            "compileFrames": len(compiles),
            "newProgramsAfterWarmup": sum(c["newPrograms"] for c in compiles),
            "meanDrawOnCompileFramesMs": round(statistics.mean(compile_draws), 2) if compile_draws else None,
            "meanDrawOnQuietFramesMs": round(statistics.mean(quiet_draws), 2) if quiet_draws else None,
            "maxDrawOnCompileFrameMs": round(max(compile_draws), 2) if compile_draws else None,
            "firstCompiles": compiles[:12],
        },
        "leaks": {
            "geometriesStart": steady[0].get("geometries", 0) if steady else 0,
            "geometriesEnd": steady[-1].get("geometries", 0) if steady else 0,
            "texturesStart": steady[0].get("textures", 0) if steady else 0,
            "texturesEnd": steady[-1].get("textures", 0) if steady else 0,
        },
        "frames": len(steady),
        "warmupWorstMs": round(max((s["delta"] for s in warmup), default=0.0), 2),
        "fpsMean": round(1000.0 / statistics.mean(deltas), 1) if deltas else 0.0,
        "delta": {
            "meanMs": round(statistics.mean(deltas), 2) if deltas else 0,
            "p50Ms": round(percentile(deltas, 50), 2),
            "p95Ms": round(percentile(deltas, 95), 2),
            "p99Ms": round(percentile(deltas, 99), 2),
            "maxMs": round(max(deltas), 2) if deltas else 0,
        },
        "update": {
            "meanMs": round(statistics.mean(updates), 2) if updates else 0,
            "p95Ms": round(percentile(updates, 95), 2),
            "maxMs": round(max(updates), 2) if updates else 0,
        },
        "draw": {
            "meanMs": round(statistics.mean(draws), 2) if draws else 0,
            "p95Ms": round(percentile(draws, 95), 2),
            "maxMs": round(max(draws), 2) if draws else 0,
        },
        "lateFrames": len(late),
        "lateFramesPerMinute": round(len(late) / max(sum(deltas) / 60000.0, 1e-9), 1) if deltas else 0,
        "gcSuspectLateFrames": len(gc_suspects),
        "heapMinMB": round(min((s["heap"] for s in steady), default=0) / 1e6, 1),
        "heapMaxMB": round(max((s["heap"] for s in steady), default=0) / 1e6, 1),
        "worstFrames": [
            {
                "t": round(s["t"], 1),
                "deltaMs": round(s["delta"], 2),
                "updateMs": round(s["update"], 2),
                "drawMs": round(s["draw"], 2),
                "unaccountedMs": round(s["delta"] - s["update"] - s["draw"], 2),
                "heapMB": round(s["heap"] / 1e6, 1),
            }
            for s in worst
        ],
    }


def summarise_cpu_profile(profile: dict[str, Any], top: int = 30) -> dict[str, Any]:
    """Aggregate a .cpuprofile into self-time by function.

    A CPU profile's `samples` are node ids at `timeDeltas` microsecond gaps, so
    self time per node is the sum of the deltas attributed to it. Aggregating
    by (functionName, url:line) rather than by node id is what collapses the
    same function reached down many call paths into one honest total.
    """
    nodes = {node["id"]: node for node in profile.get("nodes", [])}
    samples = profile.get("samples", []) or []
    deltas = profile.get("timeDeltas", []) or []

    self_us: dict[tuple[str, str], float] = defaultdict(float)
    hits: dict[tuple[str, str], int] = defaultdict(int)
    total_us = 0.0
    for i, node_id in enumerate(samples):
        delta = deltas[i] if i < len(deltas) else 0
        if delta < 0:
            delta = 0
        node = nodes.get(node_id)
        if node is None:
            continue
        frame = node.get("callFrame", {})
        name = frame.get("functionName") or "(anonymous)"
        url = frame.get("url") or ""
        # Strip the dev-server origin so entries read as source paths.
        url = url.split("localhost:5176/", 1)[-1] if url else ""
        line = frame.get("lineNumber", -1)
        key = (name, f"{url}:{line + 1}" if url else "(native)")
        self_us[key] += delta
        hits[key] += 1
        total_us += delta

    ranked = sorted(self_us.items(), key=lambda kv: -kv[1])[:top]
    return {
        "totalSampledMs": round(total_us / 1000.0, 1),
        "topSelfTime": [
            {
                "function": name,
                "at": where,
                "selfMs": round(us / 1000.0, 1),
                "selfPct": round(100.0 * us / total_us, 2) if total_us else 0,
                "samples": hits[(name, where)],
            }
            for (name, where), us in ranked
        ],
    }


def summarise_trace(events: list[dict[str, Any]]) -> dict[str, Any]:
    """Pull GC and the coarse engine phases out of a devtools trace."""
    by_name: dict[str, list[float]] = defaultdict(list)
    for event in events:
        name = event.get("name", "")
        dur = event.get("dur")
        if dur is None:
            continue
        by_name[name].append(dur / 1000.0)

    interesting = {}
    for name, durations in by_name.items():
        if not durations:
            continue
        lowered = name.lower()
        if not any(
            key in lowered
            for key in ("gc", "majorgc", "minorgc", "rasterize", "gpu", "commit", "paint", "layerize", "functioncall", "evaluatescript")
        ):
            continue
        interesting[name] = {
            "count": len(durations),
            "totalMs": round(sum(durations), 1),
            "maxMs": round(max(durations), 2),
            "meanMs": round(statistics.mean(durations), 3),
        }
    ranked = dict(sorted(interesting.items(), key=lambda kv: -kv[1]["totalMs"])[:25])
    return {"events": ranked}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", default=DEFAULT_URL, help="dev-server URL of the v2 app (default %(default)s)")
    parser.add_argument("--seconds", type=float, default=25.0, help="profile duration (default %(default)s)")
    parser.add_argument("--gpu-mode", choices=("hardware", "software"), default="hardware")
    parser.add_argument("--window", default="1600x900", help="browser window size (default %(default)s)")
    parser.add_argument("--out", type=Path, default=None, help="output directory for raw artifacts")
    parser.add_argument("--no-trace", action="store_true", help="skip the devtools trace (CPU profile only)")
    args = parser.parse_args()

    out = args.out or (REPO_ROOT / "v2-frame-profile")
    out.mkdir(parents=True, exist_ok=True)

    log(f"==> chrome ({args.gpu_mode}) -> {args.url}")
    driver = launch(find_chrome(), find_chromedriver(), out / "chromedriver.log", args.gpu_mode, args.window)
    report: dict[str, Any] = {"url": args.url, "gpuMode": args.gpu_mode, "seconds": args.seconds, "window": args.window}
    try:
        renderer = None
        driver.get(args.url)
        try:
            renderer = driver.execute_script(
                "const c=document.createElement('canvas');const g=c.getContext('webgl2')||c.getContext('webgl');"
                "if(!g)return null;const e=g.getExtension('WEBGL_debug_renderer_info');"
                "return e?g.getParameter(e.UNMASKED_RENDERER_WEBGL):'(no debug_renderer_info)';"
            )
        except Exception:
            renderer = None
        log(f"    GL renderer: {renderer}")
        report["glRenderer"] = renderer

        log("==> clicking through to a live match")
        click_into_match(driver)

        # Discard everything sampled while clicking through menus, so the
        # window below is match frames only.
        driver.execute_script("window.__gcFrames().length = 0;")
        # Installed AFTER warmup on purpose: the boot-time compiles are real
        # but expected, and counting them would drown out the mid-match ones
        # that are the actual defect.
        driver.execute_script(GL_INSTRUMENTATION_JS)

        log(f"==> profiling {args.seconds}s")
        driver.execute_cdp_cmd("Profiler.enable", {})
        driver.execute_cdp_cmd("Profiler.setSamplingInterval", {"interval": SAMPLING_INTERVAL_US})
        if not args.no_trace:
            driver.execute_cdp_cmd(
                "Tracing.start",
                {
                    "categories": ",".join(TRACE_CATEGORIES),
                    "transferMode": "ReturnAsStream",
                    "options": "sampling-frequency=10000",
                },
            )
        driver.execute_cdp_cmd("Profiler.start", {})
        time.sleep(args.seconds)
        profile = driver.execute_cdp_cmd("Profiler.stop", {})["profile"]

        trace_events: list[dict[str, Any]] = []
        if not args.no_trace:
            driver.execute_cdp_cmd("Tracing.end", {})
            # `Tracing.end` is asynchronous: the stream handle arrives on a
            # `Tracing.tracingComplete` event. Selenium's `execute_cdp_cmd` has
            # no event pump, so this polls the performance log for it instead
            # of assuming a fixed sleep is enough.
            time.sleep(3.0)
            trace_events = collect_trace(driver)

        frames = driver.execute_script("return window.__gcFrames();")
        long_tasks = driver.execute_script("return window.__gcLongTasks ? window.__gcLongTasks() : [];")

        (out / "profile.cpuprofile").write_text(json.dumps(profile))
        (out / "frames.json").write_text(json.dumps(frames))
        if trace_events:
            (out / "trace.json").write_text(json.dumps(trace_events))

        report["frameLoop"] = summarise_frames(frames)
        report["cpuProfile"] = summarise_cpu_profile(profile)
        gl_stats = driver.execute_script("return window.__glStats || null;")
        if gl_stats:
            report["shaderCompiles"] = {
                "linkProgramCalls": gl_stats["link"],
                "compileShaderCalls": gl_stats["compile"],
                "getProgramInfoLogCalls": gl_stats["infoLog"],
                "linkTotalMs": round(gl_stats["linkMs"], 1),
                "compileTotalMs": round(gl_stats["compileMs"], 1),
                # This is the number that matters: `getProgramInfoLog` blocks
                # until the driver has finished the compile it is asking about,
                # so its total IS the stall the player feels.
                "getProgramInfoLogTotalMs": round(gl_stats["infoLogMs"], 1),
                "meanInfoLogMs": round(gl_stats["infoLogMs"] / gl_stats["infoLog"], 1) if gl_stats["infoLog"] else None,
                "slowCalls": sorted(gl_stats["events"], key=lambda e: -e["ms"])[:15],
            }
        report["longTasks"] = {
            "count": len(long_tasks),
            "maxMs": round(max((t["duration"] for t in long_tasks), default=0.0), 1),
            "totalMs": round(sum(t["duration"] for t in long_tasks), 1),
        }
        if trace_events:
            report["trace"] = summarise_trace(trace_events)
    finally:
        try:
            driver.quit()
        except Exception:
            subprocess.run(["pkill", "-f", "chromedriver"], check=False)

    (out / "report.json").write_text(json.dumps(report, indent=2))
    log(json.dumps(report, indent=2))
    log(f"\n==> artifacts in {out}")
    return 0


def collect_trace(driver: Any) -> list[dict[str, Any]]:
    """Read trace events back out of the CDP stream opened by `Tracing.end`.

    Kept separate and defensive: the trace is the third, corroborating signal
    (see this module's docstring), so a failure to retrieve it must degrade the
    report rather than lose the CPU profile and frame samples that already
    succeeded.
    """
    try:
        handle = None
        for entry in driver.get_log("performance"):
            message = json.loads(entry["message"])["message"]
            if message.get("method") == "Tracing.tracingComplete":
                handle = message.get("params", {}).get("stream")
        if handle is None:
            log("    (no trace stream handle -- continuing without the trace)")
            return []
        chunks: list[str] = []
        while True:
            piece = driver.execute_cdp_cmd("IO.read", {"handle": handle, "size": 5_000_000})
            chunks.append(piece.get("data", ""))
            if piece.get("eof"):
                break
        driver.execute_cdp_cmd("IO.close", {"handle": handle})
        payload = json.loads("".join(chunks))
        return payload["traceEvents"] if isinstance(payload, dict) else payload
    except Exception as cause:  # noqa: BLE001 -- corroborating signal only
        log(f"    (trace collection failed: {cause} -- continuing)")
        return []


if __name__ == "__main__":
    raise SystemExit(main())
