#!/usr/bin/env python3
"""Browsers-only benchmark comparing the Lua/love.js build against v2 (wasm +
three.js) on the #100 ten-player render fixture -- update and draw reported
SEPARATELY, per the task brief this script was written to satisfy.

## Why this exists, and why it does not touch either gate definition

`game/render/benchmark.lua` and `v2/ts/packages/render/src/benchmark.ts` are
independent ports that already share identical gate thresholds
(`update_p95_ms: 8`, `update_max_ms: 33`, `draw_p95_ms: 8`, `draw_max_ms: 33`,
`slow_frame_ms: 33`, `slow_frames_per_60s: 3`, `stall_frame_ms: 250`), both
sourced from `docs/online/omp0_acceptance.md` so the report and the gate
cannot drift apart -- see either file's `GATES` table. This script drives
BOTH fixtures inside real browsers and reads back what each one already
computes; it invents no new metric and re-derives no percentile itself.

## update vs draw, kept separate on purpose

A combined frame number would be actively misleading: `update` is the honest
wasm-vs-LuaJIT-under-love.js comparison (the migration's actual premise);
`draw` compares three.js (real `THREE.SkinnedMesh`, 1-2 draw calls per
character) against LÖVE's hand-rolled renderer (one rigid mesh part per
bone, no skinning at all -- `benchmark.lua`'s own header: "ten players is
several hundred draw calls"). Reporting one number for both would flatter
three.js for a reason that has nothing to do with simulation cost. This
script's output keeps them apart end to end.

## Mirrors scripts/browser_online_peers.py's structure

Same pinned-asset resolution posture, same bounded-teardown import from
`browser_determinism` (not reimplemented), same build -> serve -> launch ->
poll -> teardown -> aggregate shape as that script and
`scripts/browser_matrix.py`'s evidence campaigns -- per this task's brief
("mirror this structure; do not invent a new one"). The one deliberate
departure is `launch()`'s browser arguments: this file needs exact,
verified-working hardware-GL invocations (`--use-gl=angle --use-angle=gl-egl
--ignore-gpu-blocklist` for Chrome) that `browser_determinism.chrome_arguments`
does not produce, so this file defines its own `launch()` rather than
importing that one -- see "Software vs hardware GL" below for the full
rationale and for why Firefox does not get the same treatment.
`quit_browser_bounded`/`bounded_log_tail` (launch-argument-agnostic process
teardown) ARE imported from `browser_determinism`, unchanged.

## Live match, not an inert one

`v2/tools/browser_render_bench/web/render_bench.ts` reports `liveness`
(score, time left, final tick) precisely because an earlier session this
week ran every v2 browser match 0-0 with zero events -- a fixed-content bug
in a different layer (session slot-bot-fill), now landed. This script does
not silently trust that it stayed fixed: `run_v2_benchmark` below asserts
`final_tick` actually reached (`warmup + frames`) and that the report is not
a `status: "error"` page. It does NOT assert a goal was scored (10-15
simulated seconds is not long enough to guarantee one either build scores),
so a 0-0 result here is not on its own proof of failure -- see this script's
own report emission and the accompanying doc for how this gets stated
honestly rather than papered over.

## Software vs hardware GL

Default is now HARDWARE, not software. `launch()` sets Chrome to
`--use-gl=angle --use-angle=gl-egl --ignore-gpu-blocklist`, verified by
reading `UNMASKED_RENDERER_WEBGL` in this environment to reach the real GPU
(`ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL ES
3.2)`) with no display server needed at all -- headless=new's ANGLE/EGL path
does not touch `DISPLAY`. `--gpu-mode software` restores the previous
SwiftShader behaviour (`--use-angle=swiftshader --enable-unsafe-swiftshader`)
for comparison or for a machine without a GPU.

Firefox is a harder case, investigated and NOT solved here, so it is reported
rather than silently downgraded. `-headless` forces WebRender onto its
software rasteriser (`RenderCompositorSWGL`) regardless of
`webgl.force-enabled`/`layers.acceleration.force-enabled` -- confirmed by
launching with `MOZ_LOG=Compositor:5,WebRender:5,RenderThread:5` and reading
the resulting log for which `RenderCompositor*` class actually got
constructed (`read_firefox_compositor_backend` below). Dropping `-headless`
does reach `RenderCompositorEGL`, but only by opening a real window on
`DISPLAY=:1` -- this machine's only display, which is the operator's live
desktop (see `docs/online/browser_rigged_3d.md`'s "Not on `:1`" section: an
earlier probe fought the machine owner for the screen and they closed
windows mid-run). This script does not do that. A dedicated off-screen
display with GPU passthrough (`Xvfb` + NVIDIA PRIME render offload, the
technique that document uses) would sidestep it, but `Xvfb` is not installed
here and this script does not have sudo to add it. So: Firefox always runs
software here, and every Firefox run's record carries
`firefox_compositor_backend` (read from the MOZ_LOG evidence, not guessed)
alongside the WebGL-reported renderer string, which Firefox is independently
known to fuzz for fingerprinting resistance (`"NVIDIA GeForce GTX 980, or
similar"` on this exact RTX 2070 SUPER machine -- see the same doc). Trust
the compositor-backend read for Firefox, not the renderer string.

## Bounded launch, not a bigger timeout

A prior run of this script died mid-second-repeat with no clean error --
looked like a hang. `webdriver.Chrome(...)`/`webdriver.Firefox(...)` is one
opaque blocking call: selenium's `RemoteConnection` has no default HTTP
timeout, so if chromedriver/geckodriver's "new session" handshake stalls
(plausible now that runs contend for a real, finite, shared GPU instead of
always-available SwiftShader -- this machine already runs ~200 unrelated
Chrome processes belonging to the operator's desktop), the call blocks
forever with nothing in this script to notice, until whatever external
wrapper invoked this script times out and kills it -- which is exactly what
"a script timeout killed it" looks like from the outside, and gives no
diagnostic. `CONNECT_TIMEOUT_SECONDS` already existed as a named constant for
this before this task -- it was simply never wired to anything, so it bounded
nothing. `bounded_launch()` below wires it up: it builds the `Service`
object itself (rather than letting the all-in-one `webdriver.Chrome`
constructor own it invisibly), runs the constructor in a thread, and if the
thread has not returned within `CONNECT_TIMEOUT_SECONDS`, kills the
underlying driver process group directly (the `Service` object's `.process`
attribute is set early -- well before the handshake that can stall -- so it
is available even though the constructor itself never returned) and raises a
clear, immediate error instead of leaking the child for the rest of the run
or hanging silently. This turns an unbounded, undiagnosable stall into a
bounded, loud, per-repeat failure that the existing error-collection path
already knows how to report.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import threading
import time
import urllib.parse
from collections.abc import Callable
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from browser_determinism import bounded_log_tail, quit_browser_bounded  # noqa: E402
from web_serve import ArtifactHandler  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
V2_TS = ROOT / "v2" / "ts"
HARNESS_DIR = ROOT / "v2" / "tools" / "browser_render_bench"
HARNESS_DIST = HARNESS_DIR / "dist"
WASM_BUILD_SCRIPT = V2_TS / "packages" / "wasm" / "scripts" / "build_web.mjs"
HARNESS_VITE_CONFIG = HARNESS_DIR / "vite.config.ts"
WEB_BUILD_SCRIPT = ROOT / "scripts" / "web_build.py"

DEFAULT_FRAMES = 1800
DEFAULT_WARMUP = 180
DEFAULT_SEED = 20260803
DEFAULT_WIDTH = 960
DEFAULT_HEIGHT = 540
DEFAULT_REPEATS = 3

CONNECT_TIMEOUT_SECONDS = 60
# Generous: software (swiftshader/llvmpipe) rendering of thousands of frames
# on a shared sandbox machine can legitimately take minutes, and a timeout
# that is too tight would misreport a slow-but-correct run as a failure.
RUN_TIMEOUT_SECONDS = 900

# Verified in this environment by reading UNMASKED_RENDERER_WEBGL: reaches
# "ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL
# ES 3.2)" with no DISPLAY needed. Preferred over --use-angle=vulkan (also
# verified working) because PR #400's published figures were taken on
# "hardware ANGLE (RTX 2070 SUPER)" -- same GPU, same backend family, so
# results are directly comparable to a number already in the repo's history.
CHROME_HARDWARE_GL_ARGS = ("--use-gl=angle", "--use-angle=gl-egl", "--ignore-gpu-blocklist")
CHROME_SOFTWARE_GL_ARGS = ("--use-angle=swiftshader", "--enable-unsafe-swiftshader")

GPU_MODES = ("hardware", "software")


# --------------------------------------------------------------------------
# Build
# --------------------------------------------------------------------------


def _inherited_env() -> dict[str, str]:
    return dict(os.environ)


def build_v2_harness(
    skip_wasm_build: bool,
    vite_config: Path = HARNESS_VITE_CONFIG,
    dist: Path = HARNESS_DIST,
    label: str = "v2 render-bench harness",
) -> None:
    """`node packages/wasm/scripts/build_web.mjs` then `pnpm exec vite build`
    against this harness's own vite.config.ts -- exactly the recipe this
    task's brief specifies.

    The three defaulted parameters are this file's own harness, so every
    existing call is unchanged. They exist because the recipe is identical
    for the sibling harnesses under `v2/tools/` -- only the vite config and
    the output directory differ -- and `scripts/browser_match_harness.py`
    needs exactly this for `v2/tools/browser_match_harness`. Duplicating
    fifteen lines there would mean the wasm-build step and its
    `--skip-wasm-build` precondition could drift between the two."""
    if not skip_wasm_build:
        print("[browser_render_bench] node packages/wasm/scripts/build_web.mjs")
        subprocess.run(["node", str(WASM_BUILD_SCRIPT)], cwd=V2_TS, check=True)
    elif not (V2_TS / "packages" / "wasm" / "dist" / "pkg-web" / "gc_wasm.js").is_file():
        raise RuntimeError("--skip-wasm-build was given but dist/pkg-web/gc_wasm.js is missing")
    print(f"[browser_render_bench] pnpm exec vite build ({label})")
    subprocess.run(
        ["pnpm", "exec", "vite", "build", "--config", str(vite_config)],
        cwd=V2_TS,
        check=True,
        env={"VITE_CONFIG_NATIVE_IGNORE_WARNING": "true", **_inherited_env()},
    )
    if not (dist / "index.html").is_file():
        raise RuntimeError(f"{label} build did not produce {dist / 'index.html'}")


def build_lua_web(output_dir: Path, skip: bool) -> None:
    """`scripts/web_build.py --output DIR` -- the same love.js artifact
    `docs/online/browser_build.md` documents."""
    if skip:
        if not (output_dir / "index.html").is_file():
            raise RuntimeError(f"--skip-lua-build was given but {output_dir / 'index.html'} is missing")
        return
    print(f"[browser_render_bench] scripts/web_build.py --output {output_dir}")
    subprocess.run(
        [sys.executable, str(WEB_BUILD_SCRIPT), "--output", str(output_dir)],
        cwd=ROOT,
        check=True,
    )
    if not (output_dir / "index.html").is_file():
        raise RuntimeError(f"lua web build did not produce {output_dir / 'index.html'}")


def serve_dist(directory: Path) -> tuple[ThreadingHTTPServer, threading.Thread, str]:
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        lambda *a, **k: ArtifactHandler(*a, directory=str(directory), **k),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread, f"http://127.0.0.1:{server.server_port}/"


def wait_until(predicate: Callable[[], Any], timeout: float, description: str) -> Any:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.25)
    raise RuntimeError(f"timed out waiting for {description}")


# --------------------------------------------------------------------------
# Bounded launch -- see module docstring's "Bounded launch, not a bigger
# timeout". Builds the Service by hand (rather than letting the all-in-one
# webdriver.Chrome/Firefox constructor own it invisibly) so a stalled "new
# session" handshake can be torn down instead of hanging this whole script.
# --------------------------------------------------------------------------


def bounded_launch(build: Callable[[Any], Any], service: Any, timeout: float, description: str) -> Any:
    """`build(service)` constructs and returns the WebDriver, run in a
    daemon thread. If it has not returned within `timeout`, `service.process`
    -- set early inside `Service.start()`, well before the handshake that can
    stall -- is used to kill the underlying driver's process group directly,
    and a clear error is raised instead of leaking the child for the rest of
    the run or blocking this script indefinitely."""
    box: dict[str, Any] = {}

    def run() -> None:
        try:
            box["driver"] = build(service)
        except Exception as error:  # surfaced on the main thread below
            box["error"] = error

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    thread.join(timeout)

    if thread.is_alive():
        process = getattr(service, "process", None)
        killed = "no driver process was started yet"
        if process is not None and process.pid is not None:
            try:
                pgid = os.getpgid(process.pid)
                os.killpg(pgid, signal.SIGKILL)
                killed = f"killed process group {pgid} (pid {process.pid})"
            except ProcessLookupError:
                killed = "driver process had already exited"
        raise RuntimeError(
            f"{description}: launch did not complete within {timeout:.0f}s "
            f"(the connect/handshake phase has no other bound -- see this "
            f"file's 'Bounded launch' doc); {killed}. Likely GPU/EGL "
            f"contention on shared hardware, not a code defect in the page "
            f"under test."
        )
    if "error" in box:
        raise box["error"]
    return box["driver"]


# --------------------------------------------------------------------------
# Firefox WebRender compositor backend -- ground truth for software vs
# hardware, because the WebGL-reported renderer string is not (Firefox fuzzes
# it for fingerprinting resistance -- see module docstring). Only Firefox
# writes these lines; Chrome runs simply produce no file, and this returns
# None for them.
# --------------------------------------------------------------------------

_COMPOSITOR_CTOR_RE = re.compile(r"RenderCompositor(\w+)::RenderCompositor\1\(\)")


def read_firefox_compositor_backend(moz_log_path: Path) -> str | None:
    # Firefox's MOZ_LOG_FILE is a PREFIX, not the literal path: it always
    # appends ".moz_log" (and would append ".child-N.moz_log" per child
    # process) itself. Verified empirically in this environment -- passing
    # the prefix straight to MOZ_LOG_FILE and reading `f"{prefix}.moz_log"`
    # back is what actually has content; the literal path this function is
    # named after is never created.
    candidate = moz_log_path.with_name(moz_log_path.name + ".moz_log")
    try:
        text = candidate.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    matches = _COMPOSITOR_CTOR_RE.findall(text)
    # Last construction wins: WebRender can retry with a fallback backend
    # during startup, and the LAST one constructed is the one actually used
    # to render every subsequent frame.
    return matches[-1] if matches else None


# --------------------------------------------------------------------------
# GPU renderer/vendor probe -- queries the SAME canvas element the build
# under test already created (`#canvas` for love.js, `#gl-canvas` for v2),
# not a fresh throwaway context, so this reads exactly the backend that
# rendered the measured frames. Independent of whatever the build itself
# self-reports (Lua's benchmark.lua reports love.graphics.getRendererInfo(),
# which is not the same call and is not available to this script without
# editing Lua sources this task does not own) -- uniform methodology across
# both builds and both browsers.
# --------------------------------------------------------------------------

_GPU_PROBE_JS = """
const el = document.querySelector(arguments[0]);
if (!el) { return {error: "no element matching " + arguments[0]}; }
const gl = el.getContext("webgl2") || el.getContext("webgl") || el.getContext("experimental-webgl");
if (!gl) { return {error: "no WebGL context on " + arguments[0]}; }
const ext = gl.getExtension("WEBGL_debug_renderer_info");
if (!ext) { return {renderer: null, vendor: null, masked_renderer: gl.getParameter(gl.RENDERER)}; }
return {
  renderer: gl.getParameter(ext.UNMASKED_RENDERER_WEBGL),
  vendor: gl.getParameter(ext.UNMASKED_VENDOR_WEBGL),
  masked_renderer: gl.getParameter(gl.RENDERER),
};
"""

_SOFTWARE_RENDERER_RE = re.compile(r"swiftshader|llvmpipe|softpipe|software", re.IGNORECASE)


def probe_gpu(driver: Any, canvas_selector: str) -> dict[str, Any]:
    try:
        result = driver.execute_script(_GPU_PROBE_JS, canvas_selector)
    except Exception as error:  # page navigated away/closed underneath us
        return {"error": str(error)}
    if not isinstance(result, dict):
        return {"error": "gpu probe returned a non-dict value"}
    renderer = result.get("renderer") or result.get("masked_renderer")
    result["software_guess"] = bool(_SOFTWARE_RENDERER_RE.search(renderer or ""))
    return result


# --------------------------------------------------------------------------
# Browser launch -- the one piece deliberately NOT imported from
# browser_determinism.py (see this file's module docstring).
# --------------------------------------------------------------------------


def launch(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    log: Path,
    gpu_mode: str,
    moz_log: Path | None = None,
    connect_timeout: float = CONNECT_TIMEOUT_SECONDS,
) -> Any:
    if gpu_mode not in GPU_MODES:
        raise ValueError(f"gpu_mode must be one of {GPU_MODES}, got {gpu_mode!r}")
    log.parent.mkdir(parents=True, exist_ok=True)
    description = f"{browser_name} ({gpu_mode})"

    if browser_name == "chrome":
        from selenium import webdriver
        from selenium.webdriver.chrome.options import Options
        from selenium.webdriver.chrome.service import Service

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
        service = Service(str(driver_path), log_output=str(log), popen_kw={"start_new_session": True})
        return bounded_launch(
            lambda svc: webdriver.Chrome(service=svc, options=options), service, connect_timeout, description
        )

    from selenium import webdriver
    from selenium.webdriver.firefox.options import Options
    from selenium.webdriver.firefox.service import Service

    options = Options()
    options.binary_location = str(binary)
    options.add_argument("-headless")
    options.set_preference("extensions.autoDisableScopes", 15)
    options.set_preference("extensions.enabledScopes", 0)
    if gpu_mode == "software":
        # Same intent as Chrome's swiftshader flags, Firefox's own
        # equivalents -- forces WebRender's software rasterizer path and
        # disables the X11/EGL hardware path, matching
        # browser_determinism.py's own CI preference set.
        options.set_preference("gfx.webrender.software", True)
        options.set_preference("webgl.force-enabled", True)
        options.set_preference("gfx.x11-egl.force-disabled", True)
    else:
        # Best-effort hardware attempt: enable WebGL/acceleration and do NOT
        # force software. Measured to still land on RenderCompositorSWGL
        # under `-headless` regardless (see module docstring) -- this is
        # attempted anyway, honestly, rather than skipped, and
        # `moz_log`/`read_firefox_compositor_backend` records what actually
        # happened instead of trusting the request.
        options.set_preference("webgl.force-enabled", True)
        options.set_preference("layers.acceleration.force-enabled", True)
    env = dict(os.environ)
    if moz_log is not None:
        moz_log.parent.mkdir(parents=True, exist_ok=True)
        env["MOZ_LOG"] = "Compositor:5,WebRender:5,RenderThread:5"
        env["MOZ_LOG_FILE"] = str(moz_log)
    service = Service(str(driver_path), log_output=str(log), popen_kw={"start_new_session": True}, env=env)
    return bounded_launch(
        lambda svc: webdriver.Firefox(service=svc, options=options), service, connect_timeout, description
    )


def resolve_binary_pair(browser_name: str, args: argparse.Namespace) -> tuple[Path, Path]:
    if browser_name == "chrome":
        return Path(args.chrome_binary), Path(args.chromedriver)
    return Path(args.firefox_binary), Path(args.geckodriver)


# --------------------------------------------------------------------------
# Lua side -- reads window.__GOLISEO__.console_entries, the same mechanism
# scripts/browser_matrix.py and scripts/browser_determinism.py already use
# (the loader mirrors every console.* call there; the only channel out of
# love.js, per benchmark.lua's own header). Uniform across Chrome/Firefox,
# unlike Selenium's native get_log (Chrome-only via geckodriver here).
# --------------------------------------------------------------------------


def lua_state(driver: Any) -> dict[str, Any]:
    value = driver.execute_script(
        """
        const s = window.__GOLISEO__ || {};
        return {
          status: s.status || null,
          entries: (s.console_entries || []).map((e) => String(e.message || ""))
        };
        """
    )
    if not isinstance(value, dict):
        raise RuntimeError("lua page returned a malformed compatibility state")
    return value


def parse_pipe_marker(line: str) -> dict[str, str]:
    parts = line.split("|")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, sep, value = part.partition("=")
        if sep:
            fields[key] = value
    return fields


def run_lua_benchmark(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    log: Path,
    base_url: str,
    frames: int,
    warmup: int,
    gpu_mode: str,
    moz_log: Path | None,
) -> dict[str, Any]:
    love_args = ["--benchmark", str(frames), str(warmup), "rigged"]
    query = urllib.parse.urlencode({"arg": json.dumps(love_args, separators=(",", ":"))})
    url = f"{base_url}?{query}"
    driver = launch(browser_name, binary, driver_path, log, gpu_mode, moz_log)
    try:
        driver.set_page_load_timeout(90)
        driver.get(url)

        found: dict[str, str] = {}

        def check() -> bool:
            # `os.exit(0)` right after `benchmark.emit(...)` (main.lua's
            # `--benchmark` mode) throws an "unreachable" trap under love.js
            # -- that is love.js's own forced-exit mechanism, not a benchmark
            # failure, and it lands in this SAME console-entries array right
            # after GC_BENCH_GATE. So: classify every entry first, and return
            # as soon as the gate marker itself has been seen, BEFORE ever
            # scanning for error-shaped text -- otherwise a poll that catches
            # both in one read raises on the benign post-exit trap despite
            # already having everything this function needs.
            state = lua_state(driver)
            entries = state.get("entries", [])
            for msg in entries:
                if msg.startswith("GC_BENCH_GATE|"):
                    found["gate"] = msg
                elif msg.startswith("GC_BENCH_RESULT|"):
                    found["result"] = msg
                elif msg.startswith("GC_BENCH_ENV|"):
                    found["env"] = msg
            if "gate" in found:
                return True
            for msg in entries:
                if "error" in msg.lower() and "GC_BENCH" not in msg and "GC_BROWSER" not in msg:
                    raise RuntimeError(f"{browser_name} lua page reported an error: {msg}")
            return False

        wait_until(check, RUN_TIMEOUT_SECONDS, f"{browser_name} lua GC_BENCH_GATE marker")

        result_fields = parse_pipe_marker(found["result"])
        env_fields = parse_pipe_marker(found["env"])
        gate_fields = parse_pipe_marker(found["gate"])
        # Queries the SAME <canvas id="canvas"> love.js already rendered
        # through, independent of what benchmark.lua self-reports (see
        # `probe_gpu`'s doc).
        gpu = probe_gpu(driver, "#canvas")
        firefox_backend = read_firefox_compositor_backend(moz_log) if moz_log is not None else None
        rigged_active = result_fields.get("rigged_active") == "true"
        if not rigged_active:
            # Evidence integrity, per this task's brief: a "rigged" run that
            # silently fell back to procedural billboards is not a fast
            # rigged run, it is no measurement at all. benchmark.lua's own
            # `evaluate()` already fails its `pass` gate on this; failing the
            # repeat here too makes it impossible to miss.
            raise RuntimeError(
                f"{browser_name} lua benchmark requested the rigged renderer but "
                f"rigged_active=false (fell back to procedural) -- gate: {found.get('gate')}"
            )
        return {
            "browser": browser_name,
            "browser_version": str(driver.capabilities.get("browserVersion")),
            "gpu_mode_requested": gpu_mode,
            "result": result_fields,
            "env": env_fields,
            "gate": gate_fields,
            "rigged_active": rigged_active,
            "gpu_probe": gpu,
            "firefox_compositor_backend": firefox_backend,
        }
    finally:
        quit_browser_bounded(driver)


# --------------------------------------------------------------------------
# v2 side -- reads window.__gcRenderBench (see render_bench.ts).
# --------------------------------------------------------------------------


def run_v2_benchmark(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    log: Path,
    base_url: str,
    frames: int,
    warmup: int,
    seed: int,
    width: int,
    height: int,
    gpu_mode: str,
    moz_log: Path | None,
) -> dict[str, Any]:
    url = f"{base_url}?frames={frames}&warmup={warmup}&seed={seed}&width={width}&height={height}"
    driver = launch(browser_name, binary, driver_path, log, gpu_mode, moz_log)
    try:
        driver.set_page_load_timeout(90)
        driver.get(url)

        def check() -> dict[str, Any] | None:
            state = driver.execute_script("return window.__gcRenderBench || null;")
            if not isinstance(state, dict):
                return None
            if state.get("status") == "error":
                raise RuntimeError(f"{browser_name} v2 page reported an error: {state.get('error')}")
            if state.get("status") == "done":
                return state
            return None

        state = wait_until(check, RUN_TIMEOUT_SECONDS, f"{browser_name} v2 render-bench report")
        report = state.get("report")
        if not isinstance(report, dict):
            raise RuntimeError(f"{browser_name} v2 page finished without a report")

        liveness = report.get("liveness", {})
        expected_final_tick = warmup + frames
        # Confirms this is a LIVE, ongoing match rather than an inert one --
        # see this file's module doc. Not a proof a goal was scored (the
        # measured window may be too short), only that the sim actually ran
        # the full tick count this script asked for.
        if liveness.get("final_tick") != expected_final_tick:
            raise RuntimeError(
                f"{browser_name} v2 session only reached tick {liveness.get('final_tick')}, "
                f"expected {expected_final_tick} -- match did not run the full measured window"
            )

        result = report.get("result", {})
        # Queries the SAME <canvas id="gl-canvas"> three.js already rendered
        # through -- see `probe_gpu`'s doc. Kept alongside (not instead of)
        # `render_bench.ts`'s own self-report so a reader can cross-check the
        # two agree.
        gpu = probe_gpu(driver, "#gl-canvas")
        firefox_backend = read_firefox_compositor_backend(moz_log) if moz_log is not None else None
        rigged_active = bool(result.get("rigged_active"))
        if not rigged_active:
            raise RuntimeError(
                f"{browser_name} v2 benchmark requested the rigged renderer but "
                f"rigged_active=false (fell back to procedural) -- "
                f"failures: {report.get('failures')}"
            )
        return {
            "browser": browser_name,
            "browser_version": str(driver.capabilities.get("browserVersion")),
            "gpu_mode_requested": gpu_mode,
            "report": report,
            "rigged_active": rigged_active,
            "gpu_probe": gpu,
            "firefox_compositor_backend": firefox_backend,
        }
    finally:
        quit_browser_bounded(driver)


# --------------------------------------------------------------------------
# Aggregation across repeats
# --------------------------------------------------------------------------


def summarize_repeats(values: list[float]) -> dict[str, float | int]:
    if not values:
        return {"n": 0}
    out: dict[str, float | int] = {
        "n": len(values),
        "mean": statistics.fmean(values),
        "min": min(values),
        "max": max(values),
    }
    if len(values) > 1:
        out["stdev"] = statistics.stdev(values)
        out["coefficient_of_variation"] = out["stdev"] / out["mean"] if out["mean"] else float("inf")
    else:
        out["stdev"] = 0.0
        out["coefficient_of_variation"] = 0.0
    return out


def lua_repeat_metrics(run: dict[str, Any]) -> dict[str, float]:
    r = run["result"]
    return {
        "update_p95_ms": float(r["update_p95"]),
        "update_max_ms": float(r["update_max"]),
        "update_mean_ms": float(r["update_mean"]),
        "draw_p95_ms": float(r["draw_p95"]),
        "draw_max_ms": float(r["draw_max"]),
        "draw_mean_ms": float(r["draw_mean"]),
        "draw_calls_mean": float(r["draw_calls_mean"]),
        "draw_calls_max": float(r["draw_calls_max"]),
    }


def v2_repeat_metrics(run: dict[str, Any]) -> dict[str, float]:
    result = run["report"]["result"]
    return {
        "update_p95_ms": float(result["update"]["p95_ms"]),
        "update_max_ms": float(result["update"]["max_ms"]),
        "update_mean_ms": float(result["update"]["mean_ms"]),
        "draw_p95_ms": float(result["draw"]["p95_ms"]),
        "draw_max_ms": float(result["draw"]["max_ms"]),
        "draw_mean_ms": float(result["draw"]["mean_ms"]),
        "draw_calls_mean": float(result["draw_calls_mean"]),
        "draw_calls_max": float(result["draw_calls_max"]),
    }


METRIC_KEYS = (
    "update_p95_ms",
    "update_max_ms",
    "update_mean_ms",
    "draw_p95_ms",
    "draw_max_ms",
    "draw_mean_ms",
    "draw_calls_mean",
    "draw_calls_max",
)


# Non-numeric, per-run evidence: whether the rigged renderer actually
# engaged, and what GPU/backend actually rendered the frame. Kept separate
# from METRIC_KEYS (which summarize_repeats reduces to percentiles) because
# these are facts about the run's validity, not measurements to average --
# averaging a boolean or a GPU string would hide exactly the disagreement
# this task exists to catch.
def lua_repeat_meta(run: dict[str, Any]) -> dict[str, Any]:
    gpu = run.get("gpu_probe", {})
    return {
        "rigged_active": run.get("rigged_active"),
        "gpu_mode_requested": run.get("gpu_mode_requested"),
        "gpu_renderer": gpu.get("renderer") or gpu.get("masked_renderer"),
        "gpu_vendor": gpu.get("vendor"),
        "gpu_software_guess": gpu.get("software_guess"),
        "firefox_compositor_backend": run.get("firefox_compositor_backend"),
        "omp0_gate_pass": run.get("gate", {}).get("pass") == "true",
    }


def v2_repeat_meta(run: dict[str, Any]) -> dict[str, Any]:
    gpu = run.get("gpu_probe", {})
    report = run.get("report", {})
    return {
        "rigged_active": run.get("rigged_active"),
        "gpu_mode_requested": run.get("gpu_mode_requested"),
        "gpu_renderer": gpu.get("renderer") or gpu.get("masked_renderer"),
        "gpu_vendor": gpu.get("vendor"),
        "gpu_software_guess": gpu.get("software_guess"),
        # render_bench.ts's own self-report, kept alongside the
        # Python-side probe above so a reader can see the two agree.
        "self_reported_gl_renderer": report.get("gl_renderer_string"),
        "self_reported_software_guess": report.get("software_rendering_guess"),
        "firefox_compositor_backend": run.get("firefox_compositor_backend"),
        "omp0_gate_pass": report.get("pass"),
    }


def v2_draw_call_breakdown(run: dict[str, Any]) -> dict[str, Any] | None:
    """`render_bench.ts` computes this once, after the timed loop, from a raw
    (non-composited) re-render of the final captured frame -- see that
    file's `computeDrawCallBreakdown` doc for exactly what "raw" means here
    and why it is not part of the timed samples. `None` for the Lua build,
    which has no equivalent breakdown."""
    breakdown = run.get("report", {}).get("draw_call_breakdown")
    return breakdown if isinstance(breakdown, dict) else None


def aggregate(
    build: str,
    browser: str,
    runs: list[dict[str, Any]],
    per_run_metrics: list[dict[str, float]],
    per_run_meta: list[dict[str, Any]],
) -> dict[str, Any]:
    aggregated = {key: summarize_repeats([m[key] for m in per_run_metrics]) for key in METRIC_KEYS}
    draw_call_breakdowns = [v2_draw_call_breakdown(run) for run in runs] if build == "v2" else []
    return {
        "build": build,
        "browser": browser,
        "repeats": len(runs),
        "per_run": per_run_metrics,
        "per_run_meta": per_run_meta,
        "aggregate": aggregated,
        "draw_call_breakdown_per_run": [b for b in draw_call_breakdowns if b is not None] or None,
        "raw_runs": runs,
    }


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--frames", type=int, default=DEFAULT_FRAMES)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--width", type=int, default=DEFAULT_WIDTH)
    parser.add_argument("--height", type=int, default=DEFAULT_HEIGHT)
    parser.add_argument("--repeats", type=int, default=DEFAULT_REPEATS)
    parser.add_argument("--builds", default="lua,v2", help="comma-separated: lua,v2")
    parser.add_argument("--browsers", default="chrome,firefox", help="comma-separated: chrome,firefox")
    parser.add_argument(
        "--gpu-mode",
        choices=GPU_MODES,
        default="hardware",
        help=(
            "hardware (default): Chrome via --use-gl=angle --use-angle=gl-egl, verified to reach "
            "the real GPU in this environment. Firefox is attempted the same way but is known to "
            "still land on RenderCompositorSWGL under -headless (see module docstring) -- the "
            "per-run 'firefox_compositor_backend' field records what actually happened. "
            "software: SwiftShader/forced-software for both, e.g. for a machine with no GPU."
        ),
    )
    parser.add_argument("--skip-wasm-build", action="store_true")
    parser.add_argument("--skip-harness-build", action="store_true")
    parser.add_argument("--skip-lua-build", action="store_true")
    parser.add_argument("--lua-build-dir", type=Path, default=ROOT / "build" / "web-bench")
    parser.add_argument("--chrome-binary", default="/usr/bin/google-chrome")
    parser.add_argument("--chromedriver", default=str(Path.home() / ".local" / "bin" / "chromedriver"))
    parser.add_argument("--firefox-binary", default=str(Path.home() / ".local" / "bin" / "firefox"))
    parser.add_argument("--geckodriver", default=str(Path.home() / ".local" / "bin" / "geckodriver"))
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--log-dir", type=Path, default=ROOT / "build" / "browser_render_bench-logs")
    args = parser.parse_args()

    builds = [b.strip() for b in args.builds.split(",") if b.strip()]
    browsers = [b.strip() for b in args.browsers.split(",") if b.strip()]

    if "v2" in builds and not args.skip_harness_build:
        build_v2_harness(args.skip_wasm_build)
    if "lua" in builds:
        build_lua_web(args.lua_build_dir, args.skip_lua_build)

    servers: list[tuple[ThreadingHTTPServer, threading.Thread]] = []
    base_urls: dict[str, str] = {}
    if "v2" in builds:
        server, thread, url = serve_dist(HARNESS_DIST)
        servers.append((server, thread))
        base_urls["v2"] = url
    if "lua" in builds:
        server, thread, url = serve_dist(args.lua_build_dir)
        servers.append((server, thread))
        base_urls["lua"] = url

    results: list[dict[str, Any]] = []
    errors: list[str] = []

    try:
        for build in builds:
            for browser_name in browsers:
                binary, driver_path = resolve_binary_pair(browser_name, args)
                if not binary.is_file():
                    errors.append(f"{build}/{browser_name}: browser binary not found at {binary}")
                    continue
                if not driver_path.is_file():
                    errors.append(f"{build}/{browser_name}: driver not found at {driver_path}")
                    continue

                per_run_metrics: list[dict[str, float]] = []
                per_run_meta: list[dict[str, Any]] = []
                runs: list[dict[str, Any]] = []
                for repeat in range(1, args.repeats + 1):
                    log = args.log_dir / f"{build}-{browser_name}-{repeat}-webdriver.log"
                    moz_log = (
                        args.log_dir / f"{build}-{browser_name}-{repeat}-moz.log"
                        if browser_name == "firefox"
                        else None
                    )
                    print(f"[browser_render_bench] {build}/{browser_name} repeat {repeat}/{args.repeats}")
                    try:
                        if build == "lua":
                            run = run_lua_benchmark(
                                browser_name, binary, driver_path, log, base_urls["lua"],
                                args.frames, args.warmup, args.gpu_mode, moz_log,
                            )
                            metrics = lua_repeat_metrics(run)
                            meta = lua_repeat_meta(run)
                        else:
                            run = run_v2_benchmark(
                                browser_name, binary, driver_path, log, base_urls["v2"],
                                args.frames, args.warmup, args.seed, args.width, args.height,
                                args.gpu_mode, moz_log,
                            )
                            metrics = v2_repeat_metrics(run)
                            meta = v2_repeat_meta(run)
                    except Exception as error:
                        tail = bounded_log_tail(log)
                        errors.append(f"{build}/{browser_name} repeat {repeat}: {error}\nwebdriver log tail:\n{tail}")
                        print(f"[browser_render_bench] FAILED {build}/{browser_name} repeat {repeat}: {error}", file=sys.stderr)
                        continue
                    runs.append(run)
                    per_run_metrics.append(metrics)
                    per_run_meta.append(meta)
                    print(
                        f"[browser_render_bench]   update p95={metrics['update_p95_ms']:.3f}ms "
                        f"max={metrics['update_max_ms']:.3f}ms | draw p95={metrics['draw_p95_ms']:.3f}ms "
                        f"max={metrics['draw_max_ms']:.3f}ms | draw_calls={metrics['draw_calls_mean']:.1f} "
                        f"| rigged_active={meta['rigged_active']} | omp0_gate_pass={meta['omp0_gate_pass']} "
                        f"| gpu={meta['gpu_renderer']} (software_guess={meta['gpu_software_guess']}) "
                        f"| firefox_compositor={meta['firefox_compositor_backend']}"
                    )

                if per_run_metrics:
                    results.append(aggregate(build, browser_name, runs, per_run_metrics, per_run_meta))

        # Cross-build evidence integrity: comparing a rigged v2 draw against a
        # procedurally-fallen-back Lua draw (or vice versa) is worse than no
        # measurement at all -- see this task's brief. Each individual run
        # already asserts its OWN rigged_active is true (run_lua_benchmark /
        # run_v2_benchmark raise on a false one), so this check exists for
        # the case that matters most: both builds individually report
        # rigged_active=true but via different means -- catching a scenario
        # where the two are not actually comparable even though neither
        # single-build gate would have caught it.
        by_browser: dict[str, dict[str, dict[str, Any]]] = {}
        for entry in results:
            by_browser.setdefault(entry["browser"], {})[entry["build"]] = entry
        for browser_name, by_build in by_browser.items():
            if "lua" not in by_build or "v2" not in by_build:
                continue
            lua_rigged = {m["rigged_active"] for m in by_build["lua"]["per_run_meta"]}
            v2_rigged = {m["rigged_active"] for m in by_build["v2"]["per_run_meta"]}
            if lua_rigged != v2_rigged or lua_rigged != {True}:
                message = (
                    f"{browser_name}: lua/v2 disagree on rigged_active (lua={sorted(lua_rigged)}, "
                    f"v2={sorted(v2_rigged)}) -- draw comparison is not valid evidence for this browser"
                )
                errors.append(message)
                print(f"[browser_render_bench] FAILED integrity check: {message}", file=sys.stderr)
    finally:
        for server, thread in servers:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)

    output = {
        "config": {
            "frames": args.frames,
            "warmup": args.warmup,
            "seed": args.seed,
            "width": args.width,
            "height": args.height,
            "repeats": args.repeats,
            "builds": builds,
            "browsers": browsers,
            "gpu_mode": args.gpu_mode,
        },
        "results": results,
        "errors": errors,
    }
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(output, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"[browser_render_bench] evidence written to {args.output}")

    print(json.dumps(output, indent=2, sort_keys=True))
    return 1 if errors else 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # noqa: BLE001 -- top-level CLI failure boundary
        print(f"[browser_render_bench] FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
