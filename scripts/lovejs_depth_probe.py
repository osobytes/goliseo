#!/usr/bin/env python3
"""Measure what love.js actually gives LÖVE, in real browsers on real GPUs (#360).

#328's issue text asserts "love.js is WebGL 1 and cannot supply a depth
attachment at all", and every downstream decision rests on it -- #100's browser
leg, PR #350's skipped part sorting, and §6.2 of the render migration decision.
That sentence had never been measured. This is the measurement.

WHAT IT RUNS. Four phases per browser, each a separate page load of the same
`scripts/web_build.py` artifact:

  survey   `love . --gl-probe` -- what the runtime says it supports, creating
           nothing.
  canvas   `love . --gl-probe canvas FORMAT READABLE` -- one canvas per page
           load. One per load because a format love.js will not take does NOT
           raise a catchable Lua error: it aborts the runtime, so the second
           attempt in a shared process would never run and its absence would be
           misread as a refusal.
  shader   `love . --gl-probe shader STEP` -- a ladder from a trivial LÖVE
           shader up to the real rig3d one, so "the shader does not compile"
           can name the construct rather than the file.
  bench    `love . --benchmark N W rigged|procedural` -- the existing #100
           fixture, which reports `rigged_active` and therefore proves a rigged
           run did not silently fall back.

WHAT IT REFUSES. A result whose GPU cannot be proven real. Headless Chrome falls
back to SwiftShader and already produced one false negative in #100, and Firefox
answers `WEBGL_debug_renderer_info` with a GPU it invented. The classifier is
`native_shell_bench.gpu_verdict` -- imported, not reimplemented -- which
combines the renderer string with the drivers the browser process tree actually
mapped and the GPU device nodes it holds open.

`--self-test` exercises the parsing, the refusal and the verdict rules without
starting a browser. `--prove-refusal` starts a REAL Chrome forced onto
SwiftShader and asserts this runner declines to publish it.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

from native_shell_bench import (  # noqa: E402
    gpu_verdict,
    process_tree_gpu_devices,
    process_tree_maps,
    require_hardware,
)

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / ".bench" / "lovejs-depth"
BROWSERS = ("chrome", "firefox")

MARKER_PREFIX = "GC_GLPROBE"
BENCH_PREFIX = "GC_BENCH_"

#: Ladder rungs, in the order they must be attempted. Mirrors
#: `gl_probe.SHADER_LADDER` plus the real shader; the controller asserts the
#: two agree rather than trusting the copy.
SHADER_STEPS = (
    "baseline",
    "one_custom_attribute",
    "four_custom_attributes",
    "uniform_array_constant_index",
    "uniform_array_dynamic_index",
    "bone_array_dynamic_index",
    "rig3d",
)

#: Mirrors `gl_probe.DEPTH_ATTEMPTS`.
CANVAS_ATTEMPTS = (
    ("depth24stencil8", False),
    ("depth24stencil8", True),
    ("depth32fstencil8", False),
    ("depth32f", False),
    ("depth24", False),
    ("depth24", True),
    ("depth16", False),
    ("depth16", True),
    ("stencil8", False),
)


class ArtifactHandler(SimpleHTTPRequestHandler):
    """Serves the browser artifact with the headers the runtime expects."""

    def end_headers(self) -> None:
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, *args: Any) -> None:  # noqa: A003
        return


def loadavg() -> str:
    try:
        return Path("/proc/loadavg").read_text(encoding="utf-8").split(" 0/")[0].strip()
    except OSError:
        return "unavailable"


def parse_marker(line: str) -> dict[str, str]:
    """`GC_GLPROBE|kind|k=v|k=v` -> dict, kind under `_kind`, values decoded.

    Values are percent-encoded by `gl_probe.encode` precisely so a LÖVE error
    string cannot split a marker. Decoding here is what makes the error text
    readable again.
    """
    parts = line.split("|")
    if len(parts) < 2 or parts[0] != MARKER_PREFIX:
        raise RuntimeError(f"not a {MARKER_PREFIX} marker: {line[:120]}")
    fields: dict[str, str] = {"_kind": parts[1]}
    for part in parts[2:]:
        if "=" not in part:
            raise RuntimeError(f"malformed marker field {part!r} in {parts[1]}")
        key, value = part.split("=", 1)
        if key in fields:
            raise RuntimeError(f"duplicate marker field {key!r} in {parts[1]}")
        fields[key] = urllib.parse.unquote(value)
    return fields


def markers_of(messages: list[str]) -> list[dict[str, str]]:
    return [parse_marker(m) for m in messages if m.startswith(MARKER_PREFIX + "|")]


def marker_kind(markers: list[dict[str, str]], kind: str) -> dict[str, str] | None:
    for marker in markers:
        if marker["_kind"] == kind:
            return marker
    return None


def truthy(value: str | None) -> bool:
    return value == "true"


def phase_completed(messages: list[str]) -> bool:
    """A probe phase counts as complete only if it printed its terminator.

    love.js can abort mid-call, taking every marker that would have followed
    with it. Without this rule a truncated run is indistinguishable from a run
    that found nothing, and the truncation would be published as a finding.
    """
    return any(m.startswith(f"{MARKER_PREFIX}|end") for m in markers_terminators(messages))


def markers_terminators(messages: list[str]) -> list[str]:
    return [m for m in messages if m.startswith(MARKER_PREFIX + "|")]


def bench_fields(line: str) -> dict[str, str]:
    parts = line.split("|")
    fields: dict[str, str] = {"_kind": parts[0]}
    for part in parts[1:]:
        if "=" in part:
            key, value = part.split("=", 1)
            fields[key] = value
    return fields


# --------------------------------------------------------------------------
# Verdict rules -- the part the self-test sabotages
# --------------------------------------------------------------------------


def browser_verdict(phases: dict[str, Any]) -> dict[str, Any]:
    """Can this browser render rigged 3D under LÖVE? Every clause is required.

    Deliberately conjunctive and deliberately redundant. `bloom.has_depth` and
    the benchmark's `rigged_active` are two independent observations of the same
    fact taken through two different code paths, and a disagreement between them
    means the measurement is wrong, not that one of them is good enough.
    """
    reasons: list[str] = []
    survey = phases.get("survey") or {}
    if not survey.get("complete"):
        reasons.append("the capability survey did not reach its terminator")
    bloom = survey.get("bloom") or {}
    shader = survey.get("shader") or {}
    rigged = survey.get("rigged") or {}
    if not truthy(bloom.get("has_depth")):
        reasons.append("bloom.hasDepth() was false")
    if not truthy(shader.get("linked")):
        reasons.append("the rig3d shader did not link")
    if not truthy(rigged.get("available")):
        reasons.append("player_renderer_3d.available() was false")

    bench = phases.get("bench_rigged") or {}
    if not bench.get("complete"):
        reasons.append("the rigged benchmark did not finish")
    elif not truthy(str(bench.get("rigged_active", "false"))):
        reasons.append("the rigged benchmark silently fell back to procedural")

    return {"capable": not reasons, "reasons": reasons}


def overall_verdict(browsers: dict[str, Any]) -> dict[str, Any]:
    capable = sorted(name for name, row in browsers.items() if row["verdict"]["capable"])
    incapable = sorted(name for name, row in browsers.items() if not row["verdict"]["capable"])
    if not browsers:
        answer = "unmeasured"
    elif capable and not incapable:
        answer = "yes"
    elif capable:
        answer = "partial"
    else:
        answer = "no"
    return {"answer": answer, "capable": capable, "incapable": incapable}


def failures_in(browsers: dict[str, Any]) -> list[str]:
    """Every reason a run must not be published as a clean pass.

    A runner that prints failures and exits 0 is the #279 defect. This is the
    single place that decides the exit code, and it counts an incomplete matrix
    as a failure -- a browser that never ran cannot be evidence of anything.
    """
    problems: list[str] = []
    for name in BROWSERS:
        if name not in browsers:
            problems.append(f"{name}: not measured")
    for name, row in sorted(browsers.items()):
        if row.get("gpu", {}).get("verdict") != "hardware":
            problems.append(f"{name}: GPU not proven to be hardware")
    return problems


# --------------------------------------------------------------------------
# Browser driving
# --------------------------------------------------------------------------


def chrome_options(binary: str, force_software: bool) -> Any:
    from selenium.webdriver.chrome.options import Options

    options = Options()
    options.binary_location = binary
    # HEADED on purpose. `--headless=new` is what put SwiftShader in #100.
    for argument in (
        "--disable-dev-shm-usage",
        "--disable-extensions",
        "--no-default-browser-check",
        "--no-first-run",
        "--disable-features=CalculateNativeWinOcclusion",
    ):
        options.add_argument(argument)
    if force_software:
        options.add_argument("--use-gl=angle")
        options.add_argument("--use-angle=swiftshader")
        options.add_argument("--disable-gpu")
    else:
        options.add_argument("--ignore-gpu-blocklist")
        options.add_argument("--enable-gpu-rasterization")
    # LÖVE's error path calls `alert()`. Left unhandled it blocks every
    # subsequent WebDriver command, so a probe that found something would
    # report a WebDriver error instead of the finding.
    options.set_capability("unhandledPromptBehavior", "ignore")
    return options


def firefox_options(binary: str) -> Any:
    from selenium.webdriver.firefox.options import Options

    options = Options()
    options.binary_location = binary
    options.set_preference("webgl.force-enabled", True)
    options.set_preference("webgl.disabled", False)
    options.set_preference("webgl.enable-debug-renderer-info", True)
    options.set_preference("privacy.resistFingerprinting", False)
    options.set_preference("gfx.webrender.software", False)
    options.set_preference("dom.min_background_timeout_value", 0)
    options.set_preference("extensions.autoDisableScopes", 15)
    options.set_preference("extensions.enabledScopes", 0)
    options.set_capability("unhandledPromptBehavior", "ignore")
    return options


def launch(browser: str, binary: str, driver: str, log: Path, force_software: bool) -> Any:
    from selenium import webdriver

    log.parent.mkdir(parents=True, exist_ok=True)
    if browser == "chrome":
        from selenium.webdriver.chrome.service import Service

        return webdriver.Chrome(
            service=Service(driver, log_output=str(log), popen_kw={"start_new_session": True}),
            options=chrome_options(binary, force_software),
        )
    from selenium.webdriver.firefox.service import Service

    return webdriver.Firefox(
        service=Service(driver, log_output=str(log), popen_kw={"start_new_session": True}),
        options=firefox_options(binary),
    )


GL_SCRIPT = r"""
const out = {};
const canvas = document.getElementById("canvas");
let gl = null;
const M = window.Module || {};
if (M.GLctx) { gl = M.GLctx; out.source = "Module.GLctx"; }
if (!gl && canvas) {
  gl = canvas.getContext("webgl2");
  if (gl) { out.source = "canvas.getContext(webgl2)"; }
  else {
    gl = canvas.getContext("webgl") || canvas.getContext("experimental-webgl");
    if (gl) { out.source = "canvas.getContext(webgl)"; }
  }
}
if (!gl) { return { source: null }; }
out.webgl2 = (typeof WebGL2RenderingContext !== "undefined")
  && (gl instanceof WebGL2RenderingContext);
out.version = gl.getParameter(gl.VERSION);
out.shading_language = gl.getParameter(gl.SHADING_LANGUAGE_VERSION);
out.gpu_vendor = gl.getParameter(gl.VENDOR);
out.gpu_renderer = gl.getParameter(gl.RENDERER);
const dbg = gl.getExtension("WEBGL_debug_renderer_info");
if (dbg) {
  out.gpu_unmasked_vendor = gl.getParameter(dbg.UNMASKED_VENDOR_WEBGL);
  out.gpu_unmasked_renderer = gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL);
}
out.extensions = gl.getSupportedExtensions();
out.max_vertex_uniform_vectors = gl.getParameter(gl.MAX_VERTEX_UNIFORM_VECTORS);
out.max_fragment_uniform_vectors = gl.getParameter(gl.MAX_FRAGMENT_UNIFORM_VECTORS);
out.max_vertex_attribs = gl.getParameter(gl.MAX_VERTEX_ATTRIBS);
out.context_attributes = gl.getContextAttributes();
return out;
"""

CONSOLE_SCRIPT = (
    "const s = window.__GOLISEO__ || {};"
    "return { status: s.status || null,"
    " entries: (s.console_entries || []).map((e) => String(e.message || '')) };"
)


def script_with_alerts(driver: Any, script: str, tries: int = 4) -> Any:
    """Run a script, dismissing LÖVE's `alert()` if it is in the way."""
    from selenium.common.exceptions import UnexpectedAlertPresentException

    seen: list[str] = []
    for _ in range(tries):
        try:
            return driver.execute_script(script), seen
        except UnexpectedAlertPresentException:
            try:
                alert = driver.switch_to.alert
                seen.append(alert.text)
                alert.accept()
            except Exception:  # noqa: BLE001 - the alert can vanish underneath us
                pass
            time.sleep(0.3)
    return None, seen


def run_page(
    browser: str,
    binary: str,
    driver_path: str,
    base_url: str,
    love_args: list[str],
    log: Path,
    timeout: float,
    done: Any,
    force_software: bool = False,
) -> dict[str, Any]:
    """One page load. `done(messages)` decides when the phase has finished."""
    query = urllib.parse.urlencode({"arg": json.dumps(love_args, separators=(",", ":"))})
    url = f"{base_url}?{query}" if love_args else base_url
    driver = launch(browser, binary, driver_path, log, force_software)
    service_pid = driver.service.process.pid
    record: dict[str, Any] = {
        "browser": browser,
        "args": love_args,
        "alerts": [],
        "messages": [],
        "loadavg_start": loadavg(),
    }
    try:
        record["browser_version"] = str(driver.capabilities.get("browserVersion"))
        driver.set_page_load_timeout(120)
        try:
            driver.get(url)
        except Exception as error:  # noqa: BLE001 - an alert can interrupt navigation
            record["navigation_note"] = str(error)[:200]
        deadline = time.monotonic() + timeout
        messages: list[str] = []
        while time.monotonic() < deadline:
            state, alerts = script_with_alerts(driver, CONSOLE_SCRIPT)
            record["alerts"].extend(alerts)
            if isinstance(state, dict):
                messages = [str(entry) for entry in state.get("entries") or []]
                record["loader_status"] = state.get("status")
                if done(messages):
                    break
            time.sleep(0.4)
        record["messages"] = messages
        # GPU facts last: the context has to exist, and after the phase has run
        # it definitely does.
        gl, alerts = script_with_alerts(driver, GL_SCRIPT)
        record["alerts"].extend(alerts)
        record["gl"] = gl if isinstance(gl, dict) else {}
        # Driver evidence while the browser is still alive -- once it exits,
        # /proc has nothing left to read.
        record["mapped_drivers_raw"] = process_tree_maps(service_pid)
        record["gpu_devices"] = process_tree_gpu_devices(service_pid)
    finally:
        try:
            driver.quit()
        except Exception:  # noqa: BLE001 - a dead browser is not a measurement failure
            pass
    record["loadavg_end"] = loadavg()
    return record


def gpu_from(record: dict[str, Any]) -> dict[str, Any]:
    gl = record.get("gl") or {}
    verdict = gpu_verdict(
        {
            "gpu_unmasked_renderer": gl.get("gpu_unmasked_renderer", ""),
            "gpu_renderer": gl.get("gpu_renderer", ""),
        },
        record.get("mapped_drivers_raw", ""),
    )
    verdict["gpu_devices"] = record.get("gpu_devices", [])
    return verdict


# --------------------------------------------------------------------------
# Phases
# --------------------------------------------------------------------------


def until_probe_end(messages: list[str]) -> bool:
    return any(m.startswith(f"{MARKER_PREFIX}|end") for m in messages)


def until_bench_gate(messages: list[str]) -> bool:
    return any(m.startswith("GC_BENCH_GATE") for m in messages)


def summarise_survey(record: dict[str, Any]) -> dict[str, Any]:
    markers = markers_of(record["messages"])
    summary: dict[str, Any] = {
        "complete": until_probe_end(record["messages"]),
        "markers": [m for m in record["messages"] if m.startswith(MARKER_PREFIX + "|")],
    }
    for kind in ("env", "supported", "canvas_formats", "limits", "shader", "bloom", "rigged"):
        found = marker_kind(markers, kind)
        if found is not None:
            summary[kind] = {k: v for k, v in found.items() if k != "_kind"}
    if not summary["complete"]:
        # Where it stopped is the finding. Naming the last marker turns "the
        # survey failed" into "the survey died at the shader step".
        summary["truncated_after"] = markers[-1]["_kind"] if markers else "<nothing>"
        summary["tail"] = record["messages"][-4:]
    return summary


def summarise_single(record: dict[str, Any], kind: str) -> dict[str, Any]:
    markers = markers_of(record["messages"])
    found = marker_kind(markers, kind)
    complete = until_probe_end(record["messages"])
    result: dict[str, Any] = {"complete": complete}
    # The announcement marker is printed BEFORE the thing that might abort, so
    # it survives when the result marker does not. Without it a row that killed
    # the runtime would not even say which format or rung it was.
    for announcement in ("attempting", "compiling"):
        announced = marker_kind(markers, announcement)
        if announced:
            result.update({k: v for k, v in announced.items() if k != "_kind"})
    if found:
        result.update({k: v for k, v in found.items() if k != "_kind"})
    if not complete:
        # The abort case. `ok` is false, and the reason is that asking killed
        # the process -- which is a strictly worse failure than a refusal and
        # must not be recorded as the same thing.
        result["ok"] = "false"
        result["aborted"] = True
        result["error"] = "the runtime aborted; no terminator marker was printed"
        result["tail"] = record["messages"][-3:]
    return result


def summarise_bench(record: dict[str, Any]) -> dict[str, Any]:
    lines = [m for m in record["messages"] if m.startswith(BENCH_PREFIX)]
    result: dict[str, Any] = {"complete": until_bench_gate(record["messages"])}
    for line in lines:
        fields = bench_fields(line)
        if fields["_kind"] == "GC_BENCH_RESULT":
            result.update(
                {
                    "renderer": fields.get("renderer"),
                    "rigged_active": fields.get("rigged_active"),
                    "measured_frames": fields.get("measured_frames"),
                    "draw_p50_ms": fields.get("draw_p50"),
                    "draw_p95_ms": fields.get("draw_p95"),
                    "draw_max_ms": fields.get("draw_max"),
                    "update_p95_ms": fields.get("update_p95"),
                    "frame_p95_ms": fields.get("frame_p95"),
                    "draw_calls_mean": fields.get("draw_calls_mean"),
                    "draw_calls_max": fields.get("draw_calls_max"),
                    "state_hash": fields.get("state_hash"),
                }
            )
        elif fields["_kind"] == "GC_BENCH_GATE":
            result["gate_pass"] = fields.get("pass")
            result["gate_failures"] = fields.get("failures", "")
    if not result["complete"]:
        result["tail"] = record["messages"][-3:]
    return result


def measure_browser(
    browser: str,
    binary: str,
    driver_path: str,
    base_url: str,
    output: Path,
    args: argparse.Namespace,
) -> dict[str, Any]:
    logs = output / f"{browser}-logs"
    phases: dict[str, Any] = {}
    records: list[dict[str, Any]] = []

    def page(label: str, love_args: list[str], done: Any, timeout: float) -> dict[str, Any]:
        print(f"    {browser}: {label}", flush=True)
        try:
            record = run_page(
                browser,
                binary,
                driver_path,
                base_url,
                love_args,
                logs / f"{label}.log",
                timeout,
                done,
            )
        except Exception as error:  # noqa: BLE001
            # A page that kills the browser outright is a RESULT, not a lost
            # run. Firefox's session dies partway through the shader ladder
            # because LÖVE keeps aborting the runtime under it; letting that
            # abort the whole leg would have thrown away the very evidence the
            # leg exists to collect. Each page owns its own browser process, so
            # the next one starts clean.
            print(f"      {label}: browser session died ({type(error).__name__})", flush=True)
            record = {
                "browser": browser,
                "args": love_args,
                "messages": [],
                "alerts": [],
                "gl": {},
                "session_died": True,
                "session_error": str(error)[:400],
                "loadavg_end": loadavg(),
            }
        record["label"] = label
        records.append(record)
        return record

    survey = page("survey", ["--gl-probe"], until_probe_end, args.timeout_seconds)
    phases["survey"] = summarise_survey(survey)

    canvas: list[dict[str, Any]] = []
    for fmt, readable in CANVAS_ATTEMPTS:
        label = f"canvas-{fmt}-{'readable' if readable else 'attachment'}"
        record = page(
            label,
            ["--gl-probe", "canvas", fmt, "true" if readable else "false"],
            until_probe_end,
            args.timeout_seconds,
        )
        canvas.append(summarise_single(record, "attempt"))
    phases["canvas"] = canvas

    shaders: list[dict[str, Any]] = []
    for step in SHADER_STEPS:
        record = page(
            f"shader-{step}", ["--gl-probe", "shader", step], until_probe_end, args.timeout_seconds
        )
        shaders.append(summarise_single(record, "shader_step"))
    phases["shader_ladder"] = shaders

    for renderer in ("rigged", "procedural"):
        record = page(
            f"bench-{renderer}",
            ["--benchmark", str(args.frames), str(args.warmup), renderer],
            until_bench_gate,
            args.bench_timeout_seconds,
        )
        phases[f"bench_{renderer}"] = summarise_bench(record)

    # Any page that got a context can supply the GPU evidence -- the survey
    # usually does, but if its session died the run is not thereby allowed to
    # skip the hardware proof, it just reads it from another page.
    gl_record = next((r for r in records if (r.get("gl") or {}).get("gpu_renderer")), survey)
    gpu = gpu_from(gl_record)
    require_hardware(gpu, f"{browser} love.js depth probe")

    gl = gl_record.get("gl") or {}
    return {
        "browser": browser,
        "browser_version": survey.get("browser_version") or gl_record.get("browser_version"),
        "sessions_died": [r["label"] for r in records if r.get("session_died")],
        "webgl": {
            "version": gl.get("version"),
            "webgl2": gl.get("webgl2"),
            "shading_language": gl.get("shading_language"),
            "context_source": gl.get("source"),
            "context_attributes": gl.get("context_attributes"),
            "max_vertex_uniform_vectors": gl.get("max_vertex_uniform_vectors"),
            "max_fragment_uniform_vectors": gl.get("max_fragment_uniform_vectors"),
            "max_vertex_attribs": gl.get("max_vertex_attribs"),
            "extensions": gl.get("extensions"),
        },
        "gpu": gpu,
        "phases": phases,
        "verdict": browser_verdict(phases),
        "loadavg": {"start": survey.get("loadavg_start"), "end": records[-1].get("loadavg_end")},
    }


# --------------------------------------------------------------------------
# Self-test
# --------------------------------------------------------------------------


def self_test() -> None:
    """Controller logic only. Starts no browser and loads no artifact."""
    # Marker parsing, including the percent-decoding that makes an error string
    # survive a pipe-delimited format.
    line = (
        "GC_GLPROBE|attempt|format=depth24stencil8|readable=false|ok=false"
        "|error=Error%3A%20The%20depth24stencil8%20canvas%20format%20is%20not%20supported%7Cx"
    )
    fields = parse_marker(line)
    if fields["_kind"] != "attempt" or fields["ok"] != "false":
        raise RuntimeError("marker parsing self-test failed")
    if "|" not in fields["error"] or "not supported" not in fields["error"]:
        raise RuntimeError("percent-decoding self-test failed: a `|` did not survive the round trip")

    for bad in ("GC_BENCH|x", "GC_GLPROBE|attempt|noequals", "GC_GLPROBE|a|k=1|k=2"):
        try:
            parse_marker(bad)
        except RuntimeError:
            continue
        raise RuntimeError(f"malformed marker was accepted: {bad}")

    # The terminator rule. A truncated phase must not read as a complete one.
    if until_probe_end(["GC_GLPROBE|env|love=11.5.0"]):
        raise RuntimeError("a phase with no terminator was treated as complete")
    if not until_probe_end(["GC_GLPROBE|env|love=11.5.0", "GC_GLPROBE|end|mode=survey"]):
        raise RuntimeError("a terminated phase was not treated as complete")

    aborted = summarise_single(
        {"messages": ["GC_GLPROBE|attempting|format=depth24stencil8"]}, "attempt"
    )
    if aborted.get("ok") != "false" or not aborted.get("aborted"):
        raise RuntimeError("an aborted canvas attempt was not classified as a failure")

    # A page whose browser session died leaves no messages at all. That must
    # read as incomplete, never as a clean negative -- it is the same trap as
    # the terminator rule, one level up.
    # An aborted rung must still name itself, from the marker printed before the
    # call that killed the runtime.
    named = summarise_single(
        {"messages": ["GC_GLPROBE|compiling|step=rig3d"]}, "shader_step"
    )
    if named.get("step") != "rig3d" or named.get("complete"):
        raise RuntimeError("an aborted shader rung lost the step name it announced")

    dead = summarise_single({"messages": []}, "shader_step")
    if dead.get("complete") or dead.get("ok") != "false":
        raise RuntimeError("a dead browser session was not classified as incomplete")
    if summarise_bench({"messages": []})["complete"]:
        raise RuntimeError("a benchmark that never ran was treated as complete")

    # The verdict rule, and every single clause of it. Sabotage one at a time;
    # each must flip the answer on its own, or that clause is decorative.
    def phases(**overrides: Any) -> dict[str, Any]:
        base = {
            "survey": {
                "complete": True,
                "bloom": {"has_depth": "true", "depth_format": "depth16"},
                "shader": {"linked": "true"},
                "rigged": {"available": "true"},
            },
            "bench_rigged": {"complete": True, "rigged_active": "true"},
        }
        base.update(overrides)
        return base

    if not browser_verdict(phases())["capable"]:
        raise RuntimeError("a fully passing browser was not judged capable")

    sabotage = [
        ("survey", {"complete": False}),
        (
            "survey",
            {
                "complete": True,
                "bloom": {"has_depth": "false"},
                "shader": {"linked": "true"},
                "rigged": {"available": "true"},
            },
        ),
        (
            "survey",
            {
                "complete": True,
                "bloom": {"has_depth": "true"},
                "shader": {"linked": "false"},
                "rigged": {"available": "true"},
            },
        ),
        (
            "survey",
            {
                "complete": True,
                "bloom": {"has_depth": "true"},
                "shader": {"linked": "true"},
                "rigged": {"available": "false"},
            },
        ),
        ("bench_rigged", {"complete": False}),
        ("bench_rigged", {"complete": True, "rigged_active": "false"}),
    ]
    for key, value in sabotage:
        if browser_verdict(phases(**{key: value}))["capable"]:
            raise RuntimeError(f"sabotaging {key}={value} did not flip the verdict")

    # The cross-browser rollup.
    yes = {"a": {"verdict": {"capable": True}}, "b": {"verdict": {"capable": True}}}
    mixed = {"a": {"verdict": {"capable": True}}, "b": {"verdict": {"capable": False}}}
    no = {"a": {"verdict": {"capable": False}}}
    if overall_verdict(yes)["answer"] != "yes":
        raise RuntimeError("two capable browsers did not roll up to yes")
    if overall_verdict(mixed)["answer"] != "partial":
        raise RuntimeError("one capable and one incapable browser did not roll up to partial")
    if overall_verdict(no)["answer"] != "no":
        raise RuntimeError("no capable browser did not roll up to no")
    if overall_verdict({})["answer"] != "unmeasured":
        raise RuntimeError("an empty matrix did not roll up to unmeasured")

    # The publish rule. A partial matrix and an unproven GPU are both refusals,
    # and either alone must be enough.
    hardware = {"verdict": "hardware"}
    complete = {
        "chrome": {"gpu": hardware, "verdict": {"capable": True}},
        "firefox": {"gpu": hardware, "verdict": {"capable": False}},
    }
    if failures_in(complete):
        raise RuntimeError(f"a complete hardware matrix was refused: {failures_in(complete)}")
    if not failures_in({"chrome": {"gpu": hardware, "verdict": {"capable": True}}}):
        raise RuntimeError("a one-browser matrix was accepted as complete")
    unproven = dict(complete)
    unproven["firefox"] = {"gpu": {"verdict": "unknown"}, "verdict": {"capable": False}}
    if not failures_in(unproven):
        raise RuntimeError("an unproven GPU was accepted")

    # The inherited refusal still refuses. This runner adds no GPU logic of its
    # own on purpose -- if `native_shell_bench` ever stops refusing SwiftShader,
    # this runner inherits that hole silently, so assert it here too.
    software = gpu_verdict(
        {"gpu_unmasked_renderer": "Google SwiftShader", "gpu_renderer": "Google SwiftShader"},
        "",
    )
    try:
        require_hardware(software, "self-test")
    except RuntimeError:
        pass
    else:
        raise RuntimeError("the inherited classifier accepted SwiftShader")

    # The ladder the controller drives must match the ladder the game defines.
    # Two hand-maintained lists that silently disagree would quietly stop
    # testing a rung.
    ladder = ROOT / "game" / "render" / "gl_probe.lua"
    source = ladder.read_text(encoding="utf-8")
    for step in SHADER_STEPS:
        if step == "rig3d":
            continue
        if f'name = "{step}"' not in source:
            raise RuntimeError(f"shader ladder step {step!r} is not defined in {ladder}")
    for fmt, _ in CANVAS_ATTEMPTS:
        if f'format = "{fmt}"' not in source:
            raise RuntimeError(f"canvas format {fmt!r} is not defined in {ladder}")

    # Report placement is hermetic: a failing run must land on a name nothing
    # downstream reads as evidence.
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        if report_path(root, []).name != "report.json":
            raise RuntimeError("a clean run did not land on report.json")
        if report_path(root, ["boom"]).name != "report_incomplete.json":
            raise RuntimeError("a failing run landed on report.json")

    print("love.js depth probe controller self-test OK (no browser was started)")


def report_path(output: Path, failures: list[str]) -> Path:
    return output / ("report.json" if not failures else "report_incomplete.json")


def prove_refusal(args: argparse.Namespace) -> int:
    """Start a REAL Chrome forced onto SwiftShader; the runner must refuse it."""
    artifact = args.artifact.resolve()
    server, base_url = serve(artifact, args.port)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    try:
        record = run_page(
            "chrome",
            args.chrome,
            args.chromedriver,
            base_url,
            ["--gl-probe"],
            output / "prove-refusal.log",
            args.timeout_seconds,
            until_probe_end,
            force_software=True,
        )
        gpu = gpu_from(record)
        try:
            require_hardware(gpu, "prove-refusal")
        except RuntimeError as error:
            print(f"REFUSED as required: {error}")
            return 0
        print(
            "FAIL: Chrome forced onto SwiftShader produced a PUBLISHABLE result. "
            f"GPU verdict was {gpu}"
        )
        return 1
    finally:
        server.shutdown()


def serve(directory: Path, port: int) -> tuple[ThreadingHTTPServer, str]:
    server = ThreadingHTTPServer(
        ("127.0.0.1", port),
        lambda *a, **k: ArtifactHandler(*a, directory=str(directory), **k),
    )
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}/"


def build_artifact(output: Path) -> Path:
    artifact = output / "web"
    subprocess.run(
        [sys.executable, "-B", str(ROOT / "scripts" / "web_build.py"), "--output", str(artifact)],
        check=True,
        cwd=ROOT,
    )
    return artifact


def resolve_assets(browser: str, binary: str | None, driver: str | None) -> tuple[str, str]:
    from browser_matrix import resolve_assets as resolve

    resolved_binary, resolved_driver = resolve(
        browser,
        Path(binary) if binary else None,
        Path(driver) if driver else None,
    )
    return str(resolved_binary), str(resolved_driver)


def render_summary(report: dict[str, Any]) -> str:
    lines = ["", "#360 love.js depth probe", ""]
    lines.append("| browser | WebGL | bloom.hasDepth | depth format | rig3d shader | rigged |")
    lines.append("| --- | --- | --- | --- | --- | --- |")
    for name, row in sorted(report["browsers"].items()):
        survey = row["phases"]["survey"]
        bloom = survey.get("bloom", {})
        shader = survey.get("shader", {})
        rigged = survey.get("rigged", {})
        lines.append(
            f"| {name} {row['browser_version']} | {row['webgl']['version']} | "
            f"{bloom.get('has_depth', '-')} | {bloom.get('depth_format') or '-'} | "
            f"{shader.get('linked', '-')} | {rigged.get('available', '-')} |"
        )
    lines.append("")
    lines.append(f"verdict: {report['verdict']['answer']}")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact", type=Path, help="an existing scripts/web_build.py artifact")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--browsers", nargs="*", default=list(BROWSERS))
    parser.add_argument("--frames", type=int, default=600)
    parser.add_argument("--warmup", type=int, default=300)
    parser.add_argument("--timeout-seconds", type=float, default=90)
    parser.add_argument("--bench-timeout-seconds", type=float, default=600)
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--chrome")
    parser.add_argument("--chromedriver")
    parser.add_argument("--firefox")
    parser.add_argument("--geckodriver")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--prove-refusal", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)

    if args.prove_refusal:
        if not args.artifact:
            args.artifact = build_artifact(output)
        args.chrome, args.chromedriver = resolve_assets("chrome", args.chrome, args.chromedriver)
        return prove_refusal(args)

    artifact = args.artifact.resolve() if args.artifact else build_artifact(output)
    manifest = json.loads((artifact / "manifest.json").read_text(encoding="utf-8"))
    server, base_url = serve(artifact, args.port)
    browsers: dict[str, Any] = {}
    errors: list[str] = []
    try:
        for browser in args.browsers:
            binary, driver_path = resolve_assets(
                browser,
                args.chrome if browser == "chrome" else args.firefox,
                args.chromedriver if browser == "chrome" else args.geckodriver,
            )
            print(f"  {browser}: {binary}", flush=True)
            try:
                browsers[browser] = measure_browser(
                    browser, binary, driver_path, base_url, output, args
                )
            except Exception as error:  # noqa: BLE001 - one browser must not lose the other
                errors.append(f"{browser}: {error}")
                print(f"  {browser} FAILED: {error}", flush=True)
    finally:
        server.shutdown()

    failures = failures_in(browsers) + errors
    report = {
        "schema": "lovejs-depth-probe-1",
        "artifact_manifest": {
            "source_revision": manifest.get("source_revision"),
            "source_dirty": manifest.get("source_dirty"),
            "runtime": manifest.get("runtime"),
        },
        "loadavg": loadavg(),
        "browsers": browsers,
        "verdict": overall_verdict(browsers),
        "failures": failures,
    }
    destination = report_path(output, failures)
    destination.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(render_summary(report))
    print(f"wrote {destination}")
    if failures:
        # #279's rule, restated in code: printing the failures is not enough.
        for failure in failures:
            print(f"FAILURE: {failure}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
