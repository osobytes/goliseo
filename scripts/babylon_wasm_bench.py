#!/usr/bin/env python3
"""Drive Babylon from the LIVE wasm simulation, in real browsers (#328).

#341 measured Babylon's skinned-character cost from a FILE of captured render
frames and said so: live wasm integration was out of its scope. #332 landed the
boundary. This joins them -- `bench/babylon/wasm_bench.js` renders the same
scene #341 measured, but every pose comes out of `_goliseo_payload_frame` across
the #332 payload, one crossing per rendered frame, zero copy.

WHAT THIS RUNNER IS ACCOUNTABLE FOR, beyond collecting numbers:

  1. IT MUST NOT PUBLISH A SOFTWARE-RASTERISER RESULT. Headless Chrome falls
     back to SwiftShader and #100 already published one false negative from
     exactly that. The refusal is `scripts/babylon_bench.py`'s and is IMPORTED
     rather than re-implemented, so there is one refusal in the repository and
     `--prove-refusal` over there demonstrates this one too.
  2. ONE CROSSING PER RENDERED FRAME. The page wraps every `_goliseo_payload_*`
     export in a counter and reports the measured rate; anything but 1.00
     payload crossings and 0.00 other crossings per frame fails the run.
  3. RENDERING MUST NOT PERTURB THE SIMULATION. Three independent checks, and
     all three have to hold:
       * the same match run WITH and WITHOUT rendering, in one page, must reach
         the same `match_snapshot.hash`;
       * the frozen 7201-tick determinism contract, run in a worker WHILE the
         main thread renders, must still print `bfbb106aea5480f8` /
         `a190b60058a64e63` -- judged by `verify_common.check`, the same rule
         #327 and #342 are judged by;
       * every runtime -- Chrome, Firefox and a node control that renders
         nothing at all -- must agree on the rendered hash.

A run in which any of that fails writes `report_incomplete.json`, deletes any
stale `report.json`, and EXITS NON-ZERO. AGENTS.md section 9: a harness that
prints failures and exits 0 must fail the gate anyway.

    python3 -B scripts/babylon_wasm_bench.py --self-test        # no browser, no wasm
    DISPLAY=:1 python3 -B scripts/babylon_wasm_bench.py         # the real run
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "wasm" / "sim-host"))

import babylon_bench  # noqa: E402  (path is set immediately above)
import verify_common  # noqa: E402

BENCH_SOURCE = ROOT / "bench" / "babylon"
PAYLOAD_READER = ROOT / "wasm" / "sim-host" / "frame_payload.js"
DEFAULT_OUTPUT = ROOT / ".bench" / "babylon_wasm"

DEFAULT_COUNTS = (10,)
DEFAULT_VARIANTS = ("authored", "merged", "merged_all")
DEFAULT_FRAMES = 600
DEFAULT_WARMUP = 300
DEFAULT_REPEATS = 3
DEFAULT_HASH_TICKS = 600

#: Files the page needs beside the module. `frame_payload.js` is the reader the
#: renderer will actually use, copied rather than duplicated: a harness with its
#: own copy of a protocol reader is a harness that can pass while the shipped
#: one is broken.
STAGED_ASSETS = (
    (BENCH_SOURCE / "scene.js", "scene.js"),
    (BENCH_SOURCE / "wasm_bench.js", "wasm_bench.js"),
    (PAYLOAD_READER, "frame_payload.js"),
)

#: The non-browser control. Same module, same tick count, no renderer anywhere
#: in the process -- so a hash that matched between "rendering" and "not
#: rendering" inside one browser, but was wrong in both, still gets caught.
NODE_CONTROL = """
// Node control for #328: advance the match `TICKS` ticks and print the hash.
// Nothing renders here and nothing can; that is the entire point of it.
"use strict";
const path = require("node:path");
process.argv = [process.argv[0], path.join(__dirname, "simhost.js"), "--payload"];
const Module = require(path.join(__dirname, "simhost.js"));
const TICKS = __TICKS__;
const poll = setInterval(() => {
  if (typeof Module._goliseo_payload_frame !== "function") { return; }
  clearInterval(poll);
  if (Module._goliseo_payload_boot() < 0) {
    console.log("GC_NODE_CONTROL|error=" + Module.UTF8ToString(Module._goliseo_payload_error()));
    process.exit(1);
  }
  for (let tick = 0; tick < TICKS; tick += 1) {
    if (!Module._goliseo_payload_frame(1, 0)) {
      console.log("GC_NODE_CONTROL|error=" + Module.UTF8ToString(Module._goliseo_payload_error()));
      process.exit(1);
    }
  }
  console.log("GC_NODE_CONTROL|ticks=" + TICKS +
    "|hash=" + Module.UTF8ToString(Module._goliseo_payload_hash()));
  process.exit(0);
}, 5);
"""


def conditions() -> dict[str, Any]:
    """The machine conditions a measurement was taken under.

    Not decoration. Other agents build on this box; an uncontended number and a
    contended one are different claims and have to be labelled as such.
    """
    cores = os.cpu_count() or 1
    try:
        load1, load5, load15 = os.getloadavg()
    except (OSError, AttributeError):  # pragma: no cover - platform dependent
        load1 = load5 = load15 = float("nan")
    return {
        "timestamp": datetime.now(UTC).isoformat(timespec="seconds"),
        "cores": cores,
        "load1": load1,
        "load5": load5,
        "load15": load15,
        "contended": load1 > cores * 0.5,
    }


def print_conditions(label: str, c: dict[str, Any]) -> None:
    print(
        f"    {label:<12} {c['timestamp']}   {c['cores']} cores   "
        f"load {c['load1']:.2f}/{c['load5']:.2f}/{c['load15']:.2f}"
        + ("   CONTENDED - tail numbers inflated" if c["contended"] else ""),
        flush=True,
    )


# --- the rules, isolated so `--self-test` can drive every one of them red -----


def crossing_failures(result: dict[str, str], label: str) -> list[str]:
    """One payload crossing per rendered frame, and nothing else.

    The page measures this by wrapping every export, so a second call made
    anywhere inside the render loop lands here as a number rather than being
    argued about from the source.
    """
    failures = []
    payload = float(result.get("crossings_per_frame", "nan"))
    other = float(result.get("other_crossings_per_frame", "nan"))
    if payload != 1.0:
        failures.append(f"{label}: {payload} payload crossings per rendered frame, want exactly 1")
    if other != 0.0:
        failures.append(f"{label}: {other} non-payload crossings per rendered frame, want 0")
    return failures


def perturbation_failures(result: dict[str, str], label: str) -> list[str]:
    """Rendering must not change what the simulation computes.

    `hash_rendered` and `hash_control` come from the same module, the same match
    and the same tick count, differing only in whether frames were drawn.
    """
    failures = []
    rendered = result.get("hash_rendered")
    control = result.get("hash_control")
    if not rendered or not control:
        failures.append(f"{label}: no rendered/control state hash was reported")
        return failures
    if rendered != control:
        failures.append(
            f"{label}: rendering PERTURBED the simulation -- {rendered} while rendering, "
            f"{control} with rendering off, over {result.get('hash_ticks')} ticks"
        )
    return failures


def agreement_failures(hashes: dict[str, str]) -> list[str]:
    """Every runtime that ran must have reached the same state hash."""
    seen = {name: value for name, value in hashes.items() if value}
    if len(set(seen.values())) <= 1:
        return []
    detail = ", ".join(f"{name}={value}" for name, value in sorted(seen.items()))
    return [f"runtimes disagreed on the rendered state hash: {detail}"]


def fixture_failures(lines: list[str], label: str) -> list[str]:
    """The frozen contract, judged by the rule every other runtime is judged by."""
    if not lines:
        return [f"{label}: the determinism worker produced no output at all"]
    ok, why = verify_common.check(lines)
    return [] if ok else [f"{label}: frozen determinism contract -- {why}"]


def exit_code(failures: list[str]) -> int:
    """A run with any failure exits non-zero. There is no partial success here."""
    return 1 if failures else 0


# --- driving a browser -------------------------------------------------------


def page_state(driver: Any) -> dict[str, Any]:
    value = driver.execute_script(
        """
        const state = window.__GC_BENCH__ || {};
        return {
          status: state.status || null,
          markers: (state.markers || []).map(String),
          errors: (state.errors || []).map(String),
          fixture: (state.fixture || []).map(String)
        };
        """
    )
    if not isinstance(value, dict):
        raise RuntimeError("page returned malformed benchmark state")
    return value


def collect_run(
    driver: Any,
    base_url: str,
    count: int,
    variant: str,
    frames: int,
    warmup: int,
    determinism: bool,
    hash_ticks: int,
    fixture_deadline_ms: int,
    timeout_seconds: int,
    label: str,
) -> tuple[dict[str, str], dict[str, str], list[str], list[str]]:
    """Drive ONE configuration in an already-open browser and read its markers."""
    url = (
        f"{base_url}/index.html?count={count}&variant={variant}"
        f"&frames={frames}&warmup={warmup}"
        f"&determinism={'1' if determinism else '0'}&hash_ticks={hash_ticks}"
        f"&fixture_deadline_ms={fixture_deadline_ms}"
    )
    driver.set_page_load_timeout(120)
    driver.get(url)
    deadline = time.monotonic() + timeout_seconds
    state: dict[str, Any] = {}
    while time.monotonic() < deadline:
        state = page_state(driver)
        if state.get("status") in {"done", "error"}:
            break
        time.sleep(0.25)
    else:
        raise RuntimeError(
            f"{label} timed out after {timeout_seconds}s (status={state.get('status')})"
        )
    if state.get("status") == "error":
        raise RuntimeError(f"{label} failed: {state.get('errors')}")
    markers = [str(m) for m in state.get("markers", [])]

    env_lines = [m for m in markers if m.startswith("GC_BENCH_ENV|")]
    result_lines = [m for m in markers if m.startswith("GC_BENCH_RESULT|")]
    if len(env_lines) != 1 or len(result_lines) != 1:
        raise RuntimeError(
            f"{label} produced {len(env_lines)} env and {len(result_lines)} result "
            "markers; exactly one of each is required"
        )
    env = babylon_bench.parse_marker(env_lines[0])
    result = babylon_bench.parse_marker(result_lines[0])
    babylon_bench.require_hardware_renderer(env, label)
    if int(result.get("characters", "0")) != count:
        raise RuntimeError(f"{label} reported {result.get('characters')} characters, want {count}")
    return env, result, markers, [str(line) for line in state.get("fixture", [])]


def summarise(
    browser: str,
    variant: str,
    count: int,
    env: dict[str, str],
    results: list[dict[str, str]],
    markers: list[str],
) -> dict[str, Any]:
    """Median across repeats, with the spread kept so nobody has to trust it."""

    def values(key: str) -> list[float]:
        return [float(r[key]) for r in results]

    draw_calls = values("draw_calls_mean")
    if max(draw_calls) != min(draw_calls):
        # Draw-call count is the quantity under test and it is deterministic: a
        # spread here means the configurations were not the same scene.
        raise RuntimeError(
            f"{browser} {variant} x{count} draw-call count varied across repeats: {draw_calls}"
        )
    verdict, renderer = babylon_bench.classify_renderer(
        env.get("gpu_unmasked_renderer", ""), env.get("gpu_renderer", "")
    )

    def stat(key: str) -> dict[str, Any]:
        v = values(key)
        return {"median": statistics.median(v), "min": min(v), "max": max(v), "runs": v}

    last = results[-1]
    return {
        "browser": browser,
        "variant": variant,
        "characters": count,
        "repeats": len(results),
        "gpu_renderer": renderer,
        "gpu_verdict": verdict,
        "source": last.get("source", "wasm"),
        "env_marker": next(m for m in markers if m.startswith("GC_BENCH_ENV|")),
        "result_marker": next(m for m in markers if m.startswith("GC_BENCH_RESULT|")),
        "draw_calls_mean": draw_calls[0],
        "draw_calls_max": int(last["draw_calls_max"]),
        "drawn_meshes": int(last.get("drawn_meshes", "0")),
        "measured_frames": int(last["measured_frames"]),
        "payload_words": int(last.get("payload_words", "0")),
        "payload_bytes": int(last.get("payload_bytes", "0")),
        "crossings_per_frame": float(last.get("crossings_per_frame", "nan")),
        "other_crossings_per_frame": float(last.get("other_crossings_per_frame", "nan")),
        "state_hash": last.get("state_hash", ""),
        "draw_p50_ms": statistics.median(values("draw_p50")),
        "draw_p95_ms": statistics.median(values("draw_p95")),
        "draw_max_ms": statistics.median(values("draw_max")),
        "frame_p50_ms": statistics.median(values("frame_p50")),
        "frame_p95_ms": statistics.median(values("frame_p95")),
        "update_p50_ms": statistics.median(values("update_p50")),
        "update_p95_ms": statistics.median(values("update_p95")),
        "spread": {k: stat(k) for k in ("draw_p50", "draw_p95", "frame_p95", "update_p95")},
    }


def run_browser(
    browser: str,
    binary: str,
    driver_path: str,
    base_url: str,
    args: argparse.Namespace,
    variants: list[str],
    counts: list[int],
    output: Path,
) -> tuple[list[dict[str, Any]], dict[str, Any], list[str]]:
    """One browser process for the whole matrix, repeats INTERLEAVED.

    Both are load-bearing and #341 records why: a fresh browser per
    configuration folds process start-up, GPU-process init and shader
    compilation into the measurement, and running one configuration to
    completion before the next lets the GPU clock ramp land entirely on whoever
    went first.
    """
    log = output / f"{browser}-webdriver.log"
    driver = babylon_bench.launch(browser, binary, driver_path, log, False)
    collected: dict[tuple[str, int], list[dict[str, str]]] = {}
    envs: dict[tuple[str, int], dict[str, str]] = {}
    latest: dict[tuple[str, int], list[str]] = {}
    failures: list[str] = []
    determinism: dict[str, Any] = {}
    try:
        for repeat in range(args.repeats):
            for variant in variants:
                for count in counts:
                    label = f"{browser} {variant} x{count} (pass {repeat + 1}/{args.repeats})"
                    print(f"==> {label}", flush=True)
                    try:
                        env, result, markers, _ = collect_run(
                            driver, base_url, count, variant, args.frames, args.warmup,
                            False, args.hash_ticks, args.fixture_deadline_ms,
                            args.timeout, label,
                        )
                    except Exception as error:  # noqa: BLE001 - reported, not swallowed
                        failures.append(f"{label}: {error}")
                        print(f"    FAILED: {error}", flush=True)
                        continue
                    failures.extend(crossing_failures(result, label))
                    key = (variant, count)
                    collected.setdefault(key, []).append(result)
                    envs[key] = env
                    latest[key] = markers
                    print(
                        f"    {float(result['draw_calls_mean']):.1f} draw calls, "
                        f"draw p95 {float(result['draw_p95']):.3f} ms, "
                        f"update p95 {float(result['update_p95']):.3f} ms, "
                        f"{result.get('crossings_per_frame')} crossings/frame",
                        flush=True,
                    )

        # The determinism run is deliberately SEPARATE from the matrix above.
        # It starts a second wasm module in a worker, which competes for a core
        # with the render loop; folding it into the timed configurations would
        # publish contended numbers as if they were quiet ones.
        label = f"{browser} determinism (render + frozen fixture)"
        print(f"==> {label}", flush=True)
        try:
            env, result, markers, fixture = collect_run(
                driver, base_url, args.determinism_count, args.determinism_variant,
                args.determinism_frames, args.warmup, True, args.hash_ticks,
                args.fixture_deadline_ms, args.determinism_timeout, label,
            )
            failures.extend(crossing_failures(result, label))
            failures.extend(perturbation_failures(result, label))
            failures.extend(fixture_failures(fixture, label))
            determinism = {
                "browser": browser,
                "characters": int(result["characters"]),
                "variant": result["variant"],
                "hash_ticks": int(result.get("hash_ticks", "0")),
                "hash_rendered": result.get("hash_rendered", ""),
                "hash_control": result.get("hash_control", ""),
                "hash_agrees": result.get("hash_agrees", ""),
                "fixture_live_at_start": result.get("fixture_live_at_start", ""),
                "measured_frames": int(result["measured_frames"]),
                "draw_p95_ms": float(result["draw_p95"]),
                "fixture_lines": fixture,
                "fixture_hashes": verify_common.observed_hashes(fixture),
                "fixture_marker": next(
                    (m for m in markers if m.startswith("GC_BENCH_FIXTURE|")), ""
                ),
                "fixture_marks": [m for m in markers if m.startswith("GC_BENCH_FIXTURE_MARK|")],
                "result_marker": next(m for m in markers if m.startswith("GC_BENCH_RESULT|")),
            }
            print(
                f"    rendered {determinism['hash_rendered']} / "
                f"control {determinism['hash_control']} over "
                f"{determinism['hash_ticks']} ticks; frozen fixture "
                f"{determinism['fixture_hashes']}",
                flush=True,
            )
        except Exception as error:  # noqa: BLE001 - reported, not swallowed
            failures.append(f"{label}: {error}")
            print(f"    FAILED: {error}", flush=True)
    finally:
        try:
            driver.quit()
        except Exception:  # pragma: no cover - teardown is best effort
            pass

    rows = [
        summarise(browser, variant, count, envs[(variant, count)], results, latest[(variant, count)])
        for (variant, count), results in sorted(collected.items())
    ]
    return rows, determinism, failures


def run_node_control(stage: Path, ticks: int, timeout: int) -> tuple[str, str | None]:
    """The same module, the same ticks, no renderer in the process at all."""
    script = stage / "node_control.js"
    script.write_text(NODE_CONTROL.replace("__TICKS__", str(int(ticks))), encoding="utf-8")
    node = babylon_bench.which("node")
    if node is None:
        return "", "node is not installed; the non-browser control did not run"
    result = subprocess.run(
        [node, str(script)], capture_output=True, text=True, timeout=timeout, cwd=str(stage)
    )
    line = next(
        (l for l in result.stdout.splitlines() if l.startswith("GC_NODE_CONTROL|")), None
    )
    if result.returncode != 0 or line is None:
        return "", f"node control failed: {result.stdout[-400:]}\n{result.stderr[-400:]}"
    fields = babylon_bench.parse_marker(line.replace("GC_NODE_CONTROL|", "GC_BENCH_NODE|", 1))
    if "error" in fields:
        return "", f"node control reported {fields['error']}"
    return fields.get("hash", ""), None


def prepare_stage(dist: Path, args: argparse.Namespace, output: Path) -> Path:
    """Assemble the served directory.

    Staging is `verify_common.stage`'s, not a private copy: it already knows how
    to copy a root-owned Docker build out of `dist/` into somewhere writable and
    to drop the frozen-fixture worker beside it, and it was parameterised for
    exactly this. The page it writes as `index.html` is this benchmark's.

    The 12 MB of pinned vendor artifacts are cached in the OUTPUT directory,
    not in the throwaway stage, so a re-run verifies hashes against a warm cache
    instead of re-downloading. `--offline` then means what it says.
    """
    page = (BENCH_SOURCE / "wasm_index.html").read_text(encoding="utf-8")
    stage = verify_common.stage(dist, page=page, worker=verify_common.WORKER)
    for source, name in STAGED_ASSETS:
        shutil.copy2(source, stage / name)
    print("==> vendor artifacts (pinned, verified)", flush=True)
    cache = output / "vendor"
    babylon_bench.ensure_vendor(cache, offline=args.offline)
    (stage / "vendor").mkdir(parents=True, exist_ok=True)
    for name in babylon_bench.VENDOR:
        shutil.copy2(cache / name, stage / "vendor" / name)
    return stage


def render_table(rows: list[dict[str, Any]]) -> str:
    header = (
        f"{'browser':8} {'variant':11} {'chars':>5} {'draw calls':>11} {'calls/char':>11} "
        f"{'draw p50 ms':>12} {'draw p95 ms':>12} {'update p95':>11} {'frame p95':>10} "
        f"{'p50 spread':>18}"
    )
    lines = [header, "-" * len(header)]
    for row in sorted(rows, key=lambda r: (r["browser"], r["variant"], r["characters"])):
        spread = row.get("spread", {}).get("draw_p50", {})
        span = (
            f"{spread.get('min', 0):.2f}-{spread.get('max', 0):.2f} ({row['repeats']}x)"
            if spread
            else ""
        )
        lines.append(
            f"{row['browser']:8} {row['variant']:11} {row['characters']:5d} "
            f"{row['draw_calls_mean']:11.1f} "
            f"{row['draw_calls_mean'] / row['characters']:11.2f} "
            f"{row['draw_p50_ms']:12.3f} {row['draw_p95_ms']:12.3f} "
            f"{row['update_p95_ms']:11.3f} {row['frame_p95_ms']:10.3f} {span:>18}"
        )
    return "\n".join(lines)


def self_test() -> None:
    """Controller logic only. Starts no browser, loads no wasm, draws nothing."""
    # 1. One crossing per rendered frame, and the ways it can be wrong.
    good = {"crossings_per_frame": "1.00", "other_crossings_per_frame": "0.00"}
    if crossing_failures(good, "t"):
        raise RuntimeError("an exactly-one-crossing result was rejected")
    for bad in (
        {"crossings_per_frame": "2.00", "other_crossings_per_frame": "0.00"},
        {"crossings_per_frame": "1.00", "other_crossings_per_frame": "0.02"},
        {"crossings_per_frame": "0.50", "other_crossings_per_frame": "0.00"},
    ):
        if not crossing_failures(bad, "t"):
            raise RuntimeError(f"crossing rate {bad} was NOT rejected")

    # 2. Rendering must not perturb the simulation -- including the silent shape
    #    where the page reported no hash at all.
    if perturbation_failures(
        {"hash_rendered": "a703074b9f33faa0", "hash_control": "a703074b9f33faa0"}, "t"
    ):
        raise RuntimeError("two agreeing hashes were rejected")
    for bad in (
        {"hash_rendered": "a703074b9f33faa0", "hash_control": "0000000000000000",
         "hash_ticks": "600"},
        {"hash_rendered": "", "hash_control": "a703074b9f33faa0"},
        {},
    ):
        if not perturbation_failures(bad, "t"):
            raise RuntimeError(f"perturbation case {bad} was NOT rejected")

    # 3. Cross-runtime agreement, including the case where one runtime is absent.
    if agreement_failures({"chrome": "abc", "firefox": "abc", "node": "abc"}):
        raise RuntimeError("agreeing runtimes were rejected")
    if agreement_failures({"chrome": "abc", "firefox": "", "node": "abc"}):
        raise RuntimeError("a runtime that did not run must not fail agreement on its own")
    if not agreement_failures({"chrome": "abc", "firefox": "def"}):
        raise RuntimeError("disagreeing runtimes were NOT rejected")

    # 4. The frozen contract, judged by verify_common's rule.
    passing = [
        "== frozen determinism contract",
        f"final hash {verify_common.EXPECTED_HASH}",
        f"sequence digest {verify_common.EXPECTED_DIGEST}",
        "verdict MATCH",
    ]
    if fixture_failures(passing, "t"):
        raise RuntimeError("a passing frozen-contract run was rejected")
    for bad in (
        [],
        ["final hash deadbeefdeadbeef", f"sequence digest {verify_common.EXPECTED_DIGEST}",
         "verdict MATCH"],
        [f"final hash {verify_common.EXPECTED_HASH}",
         f"sequence digest {verify_common.EXPECTED_DIGEST}", "verdict DIVERGED"],
    ):
        if not fixture_failures(bad, "t"):
            raise RuntimeError(f"frozen-contract case {bad!r} was NOT rejected")

    # 5. The software-rasteriser refusal, which is imported rather than copied.
    #    Asserted here too because the import is what this file relies on: a
    #    refactor over there that weakened it must go red over here as well.
    if babylon_bench.require_hardware_renderer(
        {"gpu_unmasked_renderer": "NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2"}, "t"
    ) != "NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2":
        raise RuntimeError("a real GPU string was not accepted")
    for text in ("Google SwiftShader", "llvmpipe (LLVM 15.0.7, 256 bits)", "WebKit WebGL", ""):
        try:
            babylon_bench.require_hardware_renderer(
                {"gpu_unmasked_renderer": text, "gpu_renderer": text}, "t"
            )
        except RuntimeError:
            pass
        else:
            raise RuntimeError(f"renderer {text!r} was NOT refused")

    # 6. A partial matrix must never land on the evidence-of-record path, and a
    #    run with failures must never exit 0.
    fake = Path("/nonexistent-for-self-test")
    if babylon_bench.report_path(fake, []).name != "report.json":
        raise RuntimeError("a complete run must write report.json")
    if babylon_bench.report_path(fake, ["boom"]).name != "report_incomplete.json":
        raise RuntimeError("a run with failures must NOT write report.json")
    if exit_code([]) != 0 or exit_code(["boom"]) != 1:
        raise RuntimeError("a run with failures must exit non-zero")

    # 7. Marker parsing over a real marker shape from this page.
    fields = babylon_bench.parse_marker(
        "GC_BENCH_RESULT|renderer=babylon-wasm-merged|source=wasm|characters=10"
        "|crossings_per_frame=1.00|other_crossings_per_frame=0.00"
        "|hash_rendered=a703074b9f33faa0|hash_control=a703074b9f33faa0|hash_agrees=yes"
    )
    if fields["source"] != "wasm" or fields["hash_agrees"] != "yes":
        raise RuntimeError("result marker parsing self-test failed")
    if crossing_failures(fields, "t") or perturbation_failures(fields, "t"):
        raise RuntimeError("a good result marker was rejected end to end")

    print("babylon wasm bench controller self-test OK (no browser, no wasm, nothing rendered)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="controller logic only")
    parser.add_argument("--dist", type=Path, default=ROOT / "wasm" / "sim-host" / "dist")
    parser.add_argument("--browsers", default="chrome,firefox")
    parser.add_argument("--counts", default=",".join(str(c) for c in DEFAULT_COUNTS))
    parser.add_argument("--variants", default=",".join(DEFAULT_VARIANTS))
    parser.add_argument("--frames", type=int, default=DEFAULT_FRAMES)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument("--repeats", type=int, default=DEFAULT_REPEATS)
    parser.add_argument("--hash-ticks", type=int, default=DEFAULT_HASH_TICKS)
    parser.add_argument("--determinism-count", type=int, default=10)
    parser.add_argument("--determinism-variant", default="merged")
    parser.add_argument("--determinism-frames", type=int, default=600)
    # The frozen fixture takes about 3.3 s of CPU on an idle machine and took
    # 265 s in Firefox behind an unthrottled render loop on a contended one --
    # see "the worker starvation finding" in docs/design/babylon_wasm_spike.md.
    # The deadline is generous because a starved worker must produce a slow PASS
    # or a reported FAILURE, never a browser session that times out.
    parser.add_argument("--fixture-deadline-ms", type=int, default=900_000)
    parser.add_argument("--determinism-timeout", type=int, default=1500)
    parser.add_argument("--timeout", type=int, default=420)
    parser.add_argument("--node-timeout", type=int, default=600)
    parser.add_argument("--output", default=str(DEFAULT_OUTPUT))
    parser.add_argument("--offline", action="store_true", help="never fetch; require a warm cache")
    parser.add_argument("--chrome", default="/usr/bin/google-chrome")
    parser.add_argument("--chromedriver", default=str(Path.home() / ".local/bin/chromedriver"))
    parser.add_argument("--firefox", default=str(Path.home() / ".local/bin/firefox"))
    parser.add_argument("--geckodriver", default=str(Path.home() / ".local/bin/geckodriver"))
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    if not os.environ.get("DISPLAY"):
        raise SystemExit(
            "DISPLAY is unset. This benchmark runs HEADED by design -- headless "
            "Chrome falls back to SwiftShader and #100 already published one "
            "false negative from it. Try DISPLAY=:1."
        )

    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    try:
        stage = prepare_stage(args.dist.resolve(), args, output)
    except verify_common.ArtifactsMissing as error:
        print(f"{error}")
        return 1
    digests = verify_common.artifact_digests(stage)
    print(f"==> module {verify_common.short_digest(digests)}", flush=True)

    counts = [int(c) for c in args.counts.split(",") if c.strip()]
    variants = [v.strip() for v in args.variants.split(",") if v.strip()]
    browsers = [b.strip() for b in args.browsers.split(",") if b.strip()]

    started = conditions()
    print_conditions("conditions", started)

    server, base_url = babylon_bench.serve(stage, 0)
    rows: list[dict[str, Any]] = []
    failures: list[str] = []
    determinism: list[dict[str, Any]] = []
    node_hash = ""
    try:
        print("==> node control (no renderer in the process)", flush=True)
        try:
            node_hash, node_error = run_node_control(stage, args.hash_ticks, args.node_timeout)
        except Exception as error:  # noqa: BLE001 - reported, not swallowed
            node_hash, node_error = "", f"node control raised: {error}"
        if node_error:
            failures.append(node_error)
            print(f"    FAILED: {node_error}", flush=True)
        else:
            print(f"    {args.hash_ticks} ticks -> {node_hash}", flush=True)

        for browser in browsers:
            binary = args.chrome if browser == "chrome" else args.firefox
            driver_path = args.chromedriver if browser == "chrome" else args.geckodriver
            browser_rows, browser_determinism, browser_failures = run_browser(
                browser, binary, driver_path, base_url, args, variants, counts, output
            )
            rows.extend(browser_rows)
            failures.extend(browser_failures)
            if browser_determinism:
                determinism.append(browser_determinism)
    finally:
        server.shutdown()
        server.server_close()
        # The stage is a throwaway copy of a 3 MB module plus 12 MB of vendor
        # artifacts. Other agents share this machine's /tmp; leaving one behind
        # per run is not free.
        shutil.rmtree(stage, ignore_errors=True)

    hashes = {row["browser"]: row["hash_rendered"] for row in determinism}
    if node_hash:
        hashes["node (no renderer)"] = node_hash
    failures.extend(agreement_failures(hashes))

    ended = conditions()
    print_conditions("conditions", ended)

    report = {
        "schema": 1,
        "generated_at": datetime.now(UTC).isoformat(),
        "issue": 328,
        "module": digests,
        "vendor": {name: spec["sha256"] for name, spec in babylon_bench.VENDOR.items()},
        "frames": args.frames,
        "warmup": args.warmup,
        "repeats": args.repeats,
        "hash_ticks": args.hash_ticks,
        "conditions_before": started,
        "conditions_after": ended,
        "complete": not failures,
        "rows": rows,
        "determinism": determinism,
        "node_control_hash": node_hash,
        "state_hash_agreement": hashes,
        "failures": failures,
    }
    destination = babylon_bench.report_path(output, failures)
    if failures:
        # A partial matrix must not be able to masquerade as the evidence of
        # record. It lands under a different name, and any `report.json` from an
        # earlier run is removed rather than left to be read as this one's.
        (output / "report.json").unlink(missing_ok=True)
    else:
        (output / "report_incomplete.json").unlink(missing_ok=True)
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    if rows:
        print()
        print(render_table(rows))
    if determinism or node_hash:
        print()
        print("state hash after the same tick count, per runtime:")
        for name, value in sorted(hashes.items()):
            print(f"  {name:22} {value}")
        for row in determinism:
            print(
                f"  {row['browser']:22} rendered {row['hash_rendered']} / "
                f"control {row['hash_control']} -> "
                f"{'AGREE' if row['hash_agrees'] == 'yes' else 'DISAGREE'}; "
                f"frozen contract {row['fixture_hashes']}"
            )
    if started["contended"] or ended["contended"]:
        print()
        print(
            "NOTE: this machine was contended during the run; every tail number above is "
            "inflated. Draw-call counts are exact and unaffected."
        )
    print()
    print(f"report written to {destination}")
    if failures:
        print("\nFAILURES:")
        for failure in failures:
            print(f"  {failure}")
    else:
        print("WASM BENCH OK: one crossing per frame, and rendering did not perturb the sim.")
    return exit_code(failures)


if __name__ == "__main__":
    raise SystemExit(main())
