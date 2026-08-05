#!/usr/bin/env python3
"""Measure Babylon's skinned-character cost in real Chrome and Firefox (#341).

This runs the harness in `bench/babylon/` against a captured `RenderFrame`
payload and folds the `GC_BENCH_*` markers it emits into one report.

The refusal below is the reason this file exists rather than a shell one-liner.
Headless Chrome silently falls back to SwiftShader, a software rasteriser, and
#100 already published one false negative from exactly that. So:

  * the browser must be HEADED, on a real display;
  * the GPU renderer string is captured verbatim and carried into the report;
  * a run whose renderer is a software rasteriser -- or whose renderer cannot be
    identified at all -- is a hard failure, not a warning, and never reaches the
    report. There is no override flag, because an override flag is how the false
    negative gets published the second time.

`--self-test` exercises the parsing and the refusal without starting a browser;
`--prove-refusal` starts a real Chrome forced onto SwiftShader and asserts the
runner rejects it. The two are not interchangeable (AGENTS.md section 9).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import threading
import time
import urllib.request
from datetime import UTC, datetime
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
BENCH_SOURCE = ROOT / "bench" / "babylon"
DEFAULT_OUTPUT = ROOT / ".bench" / "babylon"
DEFAULT_COUNTS = (10, 20, 40)
DEFAULT_VARIANTS = ("authored", "merged")
DEFAULT_FRAMES = 600
# Long enough for the NVIDIA clock ramp to finish. At 300 warm-up frames the
# first configuration of a session measured the driver waking up, not the scene.
DEFAULT_WARMUP = 300
DEFAULT_REPEATS = 3

# Pinned third-party artifacts. Fetched on demand and verified, never committed:
# THIRD_PARTY.md records that this repository tracks no third-party binary, and
# a 3.6 MB character pack is not the place to start.
VENDOR = {
    "babylon.js": {
        "url": "https://cdn.jsdelivr.net/npm/babylonjs@9.19.1/babylon.js",
        "sha256": "d722288208ed611fa2ee6c19848908edfc7e01de1c0f644dc8f9022094405ae0",
        "bytes": 8185508,
    },
    "babylonjs.loaders.min.js": {
        "url": "https://cdn.jsdelivr.net/npm/babylonjs-loaders@9.19.1/babylonjs.loaders.min.js",
        "sha256": "99d1bd29cca1a97d639829191f544b0d957c2594f00a2cd574fbe56030a327ca",
        "bytes": 523398,
    },
    # KayKit Adventurers 1.0, Knight. CC0 1.0 (Kay Lousberg). 41 joints, six
    # skinned meshes, 76 clips, one shared material -- see THIRD_PARTY.md.
    "character.glb": {
        "url": (
            "https://raw.githubusercontent.com/KayKit-Game-Assets/"
            "KayKit-Character-Pack-Adventures-1.0/"
            "672074b73ba276876a19e8816ecdc5241817ab47/"
            "addons/kaykit_character_pack_adventures/Characters/gltf/Knight.glb"
        ),
        "sha256": "60428e3abc09ba83e595d256e3af8c5c976b46cdae599f0802fc82b4a3445168",
        "bytes": 3659532,
    },
}

# Substrings that identify a software rasteriser. Deliberately narrow: "mesa" on
# its own is NOT here, because Mesa is the ordinary hardware driver stack for AMD
# and Intel and blocking it would refuse honest results.
SOFTWARE_RENDERER_PATTERNS = (
    "swiftshader",
    "llvmpipe",
    "softpipe",
    "swrast",
    "lavapipe",
    "software rasterizer",
    "software rasteriser",
    "microsoft basic render",
    "mesa offscreen",
    "d3d11 warp",
    "google, vulkan 1.3.0 (swiftshader",
)
# Strings a browser substitutes when it declines to name the GPU. These are not
# software -- they are UNPROVEN, which for publishing purposes is just as bad.
MASKED_RENDERER_VALUES = (
    "?",
    "",
    "webkit webgl",
    "mozilla",
    "webkit",
    "mozilla -- mozilla",
)


class BenchHandler(SimpleHTTPRequestHandler):
    """Static files with no caching, so a rebuilt payload is never stale.

    The cross-origin isolation headers are not security theatre here: without
    them browsers clamp `performance.now()` to 100 microseconds, which is a
    fifth of the difference this benchmark is trying to resolve between two
    configurations. Isolated, the clamp drops to 5 microseconds.
    """

    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store")
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        super().end_headers()

    def log_message(self, *_args: Any) -> None:  # pragma: no cover - quiet server
        return


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ensure_vendor(vendor_dir: Path, offline: bool = False) -> dict[str, str]:
    """Fetch and verify the pinned Babylon bundles and the CC0 character."""
    vendor_dir.mkdir(parents=True, exist_ok=True)
    provenance: dict[str, str] = {}
    for name, spec in VENDOR.items():
        target = vendor_dir / name
        if target.exists() and sha256_of(target) == spec["sha256"]:
            provenance[name] = spec["sha256"]
            continue
        if offline:
            raise RuntimeError(f"vendor artifact {name} is missing and --offline was given")
        print(f"    fetching {name} ({spec['bytes']} bytes)", flush=True)
        request = urllib.request.Request(spec["url"], headers={"User-Agent": "goliseo-bench"})
        with urllib.request.urlopen(request, timeout=180) as response:
            target.write_bytes(response.read())
        actual = sha256_of(target)
        if actual != spec["sha256"]:
            target.unlink(missing_ok=True)
            raise RuntimeError(
                f"{name} hash mismatch: expected {spec['sha256']}, got {actual}. "
                "The pin moved or the download was tampered with; do not proceed."
            )
        provenance[name] = actual
    return provenance


def ensure_capture(payload: Path, frames: int, warmup: int) -> str:
    """Produce the captured render-frame payload if it is not already there."""
    if payload.exists():
        return "reused"
    love = which("love")
    if love is None:
        raise RuntimeError(
            f"{payload} does not exist and LOVE is not installed to capture it. "
            f"Run: love . --capture-frames {frames} {warmup} {payload}"
        )
    payload.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        [love, ".", "--capture-frames", str(frames), str(warmup), str(payload)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        timeout=600,
    )
    marker = next(
        (line for line in result.stdout.splitlines() if line.startswith("GC_CAPTURE|")), None
    )
    if result.returncode != 0 or marker is None or marker.startswith("GC_CAPTURE|error"):
        raise RuntimeError(f"capture failed: {result.stdout}\n{result.stderr}")
    print(f"    {marker}", flush=True)
    return marker


def which(name: str) -> str | None:
    for directory in os.environ.get("PATH", "").split(os.pathsep):
        candidate = Path(directory) / name
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def parse_marker(line: str) -> dict[str, str]:
    """`GC_BENCH_KIND|k=v|k=v` -> dict, with the kind under `_kind`."""
    parts = line.split("|")
    if not parts or not parts[0].startswith("GC_BENCH_"):
        raise RuntimeError(f"not a GC_BENCH marker: {line[:120]}")
    fields: dict[str, str] = {"_kind": parts[0]}
    for part in parts[1:]:
        if "=" not in part:
            raise RuntimeError(f"malformed marker field {part!r} in {parts[0]}")
        key, value = part.split("=", 1)
        fields[key] = value
    return fields


def classify_renderer(*candidates: str) -> tuple[str, str]:
    """Return (verdict, the string the verdict was made on).

    `hardware` needs a positive identification. `unknown` is not a pass: a
    browser that will not name its GPU cannot be evidence that the GPU was real.
    """
    for candidate in candidates:
        text = (candidate or "").strip()
        lowered = text.lower()
        if lowered in MASKED_RENDERER_VALUES:
            continue
        for pattern in SOFTWARE_RENDERER_PATTERNS:
            if pattern in lowered:
                return "software", text
        return "hardware", text
    return "unknown", " / ".join(c for c in candidates if c) or "<none reported>"


def require_hardware_renderer(env: dict[str, str], context: str) -> str:
    verdict, text = classify_renderer(
        env.get("gpu_unmasked_renderer", ""), env.get("gpu_renderer", "")
    )
    if verdict == "software":
        raise RuntimeError(
            f"{context}: refusing to publish a software-rasteriser result. "
            f"GPU renderer reported as {text!r}. A software rasteriser measures "
            "the CPU, not the GPU, and #100 already published one false negative "
            "from exactly this. Run headed on real hardware."
        )
    if verdict != "hardware":
        raise RuntimeError(
            f"{context}: the browser did not identify its GPU ({text!r}), so this "
            "run cannot be evidence that it ran on hardware. Refusing to publish."
        )
    return text


def chrome_options(binary: str, force_software: bool) -> Any:
    from selenium.webdriver.chrome.options import Options

    options = Options()
    options.binary_location = binary
    # HEADED on purpose. --headless=new is what put SwiftShader in #100.
    for argument in (
        "--disable-dev-shm-usage",
        "--disable-extensions",
        "--no-default-browser-check",
        "--no-first-run",
        "--disable-features=CalculateNativeWinOcclusion",
        "--autoplay-policy=no-user-gesture-required",
    ):
        options.add_argument(argument)
    if force_software:
        # The demonstration path only: prove the refusal can fire.
        options.add_argument("--use-gl=angle")
        options.add_argument("--use-angle=swiftshader")
        options.add_argument("--disable-gpu")
    else:
        options.add_argument("--ignore-gpu-blocklist")
        options.add_argument("--enable-gpu-rasterization")
    return options


def firefox_options(binary: str) -> Any:
    from selenium.webdriver.firefox.options import Options

    options = Options()
    options.binary_location = binary
    options.set_preference("webgl.force-enabled", True)
    options.set_preference("webgl.disabled", False)
    # Without this the renderer string is masked, and a masked string is refused.
    options.set_preference("webgl.enable-debug-renderer-info", True)
    options.set_preference("privacy.resistFingerprinting", False)
    options.set_preference("gfx.webrender.software", False)
    options.set_preference("dom.min_background_timeout_value", 0)
    options.set_preference("extensions.autoDisableScopes", 15)
    options.set_preference("extensions.enabledScopes", 0)
    return options


def launch(browser: str, binary: str, driver: str, log: Path, force_software: bool) -> Any:
    from selenium import webdriver

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


def bench_state(driver: Any) -> dict[str, Any]:
    value = driver.execute_script(
        """
        const state = window.__GC_BENCH__ || {};
        return {
          status: state.status || null,
          markers: (state.markers || []).map(String),
          errors: (state.errors || []).map(String)
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
    timeout_seconds: int,
    label: str,
) -> tuple[dict[str, str], dict[str, str], list[str]]:
    """Drive ONE configuration in an already-open browser and read its markers."""
    url = f"{base_url}/index.html?count={count}&variant={variant}&frames={frames}&warmup={warmup}"
    driver.set_page_load_timeout(120)
    driver.get(url)
    deadline = time.monotonic() + timeout_seconds
    state: dict[str, Any] = {}
    while time.monotonic() < deadline:
        state = bench_state(driver)
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
    env = parse_marker(env_lines[0])
    result = parse_marker(result_lines[0])
    require_hardware_renderer(env, label)
    if int(result.get("characters", "0")) != count:
        raise RuntimeError(f"{label} reported {result.get('characters')} characters, want {count}")
    return env, result, markers


def summarise_repeats(
    browser: str,
    variant: str,
    count: int,
    env: dict[str, str],
    results: list[dict[str, str]],
    markers: list[str],
) -> dict[str, Any]:
    """Median across repeats, with the spread kept so nobody has to trust it.

    The median, not the minimum: a best-of run reports the machine on its best
    behaviour rather than the machine.
    """

    def values(key: str) -> list[float]:
        return [float(r[key]) for r in results]

    def stat(key: str) -> dict[str, float]:
        v = values(key)
        return {
            "median": statistics.median(v),
            "min": min(v),
            "max": max(v),
            "runs": v,
        }

    draw_calls = values("draw_calls_mean")
    if max(draw_calls) != min(draw_calls):
        # Draw-call count is the quantity under test and it is deterministic:
        # a spread here means the configurations were not the same scene.
        raise RuntimeError(
            f"{browser} {variant} x{count} draw-call count varied across repeats: {draw_calls}"
        )
    verdict, renderer = classify_renderer(
        env.get("gpu_unmasked_renderer", ""), env.get("gpu_renderer", "")
    )
    return {
        "browser": browser,
        "variant": variant,
        "characters": count,
        "repeats": len(results),
        "gpu_renderer": renderer,
        "gpu_verdict": verdict,
        "env_marker": next(m for m in markers if m.startswith("GC_BENCH_ENV|")),
        "result_marker": next(m for m in markers if m.startswith("GC_BENCH_RESULT|")),
        "sample_markers": [m for m in markers if m.startswith("GC_BENCH_SAMPLES|")],
        "draw_calls_mean": draw_calls[0],
        "draw_calls_max": int(results[-1]["draw_calls_max"]),
        "drawn_meshes": int(results[-1].get("drawn_meshes", "0")),
        "measured_frames": int(results[-1]["measured_frames"]),
        "draw_p50_ms": statistics.median(values("draw_p50")),
        "draw_p95_ms": statistics.median(values("draw_p95")),
        "draw_max_ms": statistics.median(values("draw_max")),
        "frame_p50_ms": statistics.median(values("frame_p50")),
        "frame_p95_ms": statistics.median(values("frame_p95")),
        "update_p95_ms": statistics.median(values("update_p95")),
        "spread": {key: stat(key) for key in ("draw_p50", "draw_p95", "frame_p95")},
    }


def run_browser(
    browser: str,
    binary: str,
    driver_path: str,
    base_url: str,
    variants: list[str],
    counts: list[int],
    frames: int,
    warmup: int,
    repeats: int,
    output: Path,
    timeout_seconds: int,
    force_software: bool = False,
) -> tuple[list[dict[str, Any]], list[str]]:
    """One browser process for the whole matrix, repeats INTERLEAVED.

    Both of those are load-bearing, and the first version of this file had
    neither. A fresh browser per configuration folds process start-up, GPU
    process init and shader compilation into the measurement, and running one
    configuration to completion before starting the next lets the GPU's clock
    ramp land entirely on whichever configuration went first. The result was a
    matrix in which the variant with a THIRD of the draw calls measured slower.
    One session, interleaved passes, and the ordering came right.
    """
    log = output / f"{browser}-webdriver.log"
    driver = launch(browser, binary, driver_path, log, force_software)
    collected: dict[tuple[str, int], list[dict[str, str]]] = {}
    envs: dict[tuple[str, int], dict[str, str]] = {}
    latest: dict[tuple[str, int], list[str]] = {}
    failures: list[str] = []
    try:
        for repeat in range(repeats):
            for variant in variants:
                for count in counts:
                    label = f"{browser} {variant} x{count} (pass {repeat + 1}/{repeats})"
                    print(f"==> {label}", flush=True)
                    try:
                        env, result, markers = collect_run(
                            driver, base_url, count, variant, frames, warmup,
                            timeout_seconds, label,
                        )
                    except Exception as error:  # noqa: BLE001 - reported, not swallowed
                        failures.append(f"{label}: {error}")
                        print(f"    FAILED: {error}", flush=True)
                        continue
                    key = (variant, count)
                    collected.setdefault(key, []).append(result)
                    envs[key] = env
                    latest[key] = markers
                    print(
                        f"    {float(result['draw_calls_mean']):.1f} draw calls, "
                        f"draw p50 {float(result['draw_p50']):.3f} ms, "
                        f"draw p95 {float(result['draw_p95']):.3f} ms",
                        flush=True,
                    )
    finally:
        try:
            driver.quit()
        except Exception:  # pragma: no cover - teardown is best effort
            pass

    rows = []
    for (variant, count), results in sorted(collected.items()):
        rows.append(
            summarise_repeats(browser, variant, count, envs[(variant, count)], results,
                              latest[(variant, count)])
        )
    return rows, failures


def serve(directory: Path, port: int) -> tuple[ThreadingHTTPServer, str]:
    handler = partial(BenchHandler, directory=str(directory))
    server = ThreadingHTTPServer(("127.0.0.1", port), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, f"http://127.0.0.1:{server.server_address[1]}"


def marginal_cost(rows: list[dict[str, Any]], key: str) -> list[tuple[int, int, float]]:
    """Per-character deltas between consecutive counts: the curve's SHAPE."""
    ordered = sorted(rows, key=lambda r: r["characters"])
    out = []
    for previous, current in zip(ordered, ordered[1:], strict=False):
        span = current["characters"] - previous["characters"]
        if span > 0:
            out.append(
                (previous["characters"], current["characters"], (current[key] - previous[key]) / span)
            )
    return out


def render_report(rows: list[dict[str, Any]]) -> str:
    lines = []
    header = (
        f"{'browser':8} {'variant':9} {'chars':>5} {'draw calls':>11} {'calls/char':>11} "
        f"{'draw p50 ms':>12} {'draw p95 ms':>12} {'frame p95 ms':>13} {'p50 spread':>18}"
    )
    lines.append(header)
    lines.append("-" * len(header))
    for row in sorted(rows, key=lambda r: (r["browser"], r["variant"], r["characters"])):
        spread = row.get("spread", {}).get("draw_p50", {})
        span = (
            f"{spread.get('min', 0):.2f}-{spread.get('max', 0):.2f} ({row['repeats']}x)"
            if spread
            else ""
        )
        lines.append(
            f"{row['browser']:8} {row['variant']:9} {row['characters']:5d} "
            f"{row['draw_calls_mean']:11.1f} "
            f"{row['draw_calls_mean'] / row['characters']:11.2f} "
            f"{row['draw_p50_ms']:12.3f} {row['draw_p95_ms']:12.3f} "
            f"{row['frame_p95_ms']:13.3f} {span:>18}"
        )
    lines.append("")
    lines.append("marginal cost per added character (the SHAPE of the curve):")
    groups: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for row in rows:
        groups.setdefault((row["browser"], row["variant"]), []).append(row)
    for (browser, variant), group in sorted(groups.items()):
        calls = marginal_cost(group, "draw_calls_mean")
        draws = marginal_cost(group, "draw_p50_ms")
        for (low, high, per_call), (_, _, per_draw) in zip(calls, draws, strict=True):
            lines.append(
                f"  {browser:8} {variant:9} {low:>3}->{high:<3} "
                f"{per_call:6.2f} draw calls/char   {per_draw * 1000:8.1f} us draw p50/char"
            )
    return "\n".join(lines)


def self_test() -> None:
    """Controller logic only: parsing and refusal. Starts no browser."""
    good = (
        "GC_BENCH_ENV|runtime=babylon|babylon=9.19.1|api=webgl2"
        "|gpu_renderer=ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2)"
        "|gpu_unmasked_renderer=NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2"
        "|gpu_vendor=Google Inc.|gpu_unmasked_vendor=NVIDIA Corporation"
    )
    fields = parse_marker(good)
    if fields["_kind"] != "GC_BENCH_ENV" or "RTX 2070" not in fields["gpu_unmasked_renderer"]:
        raise RuntimeError("env marker parsing self-test failed")
    if require_hardware_renderer(fields, "self-test") != "NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2":
        raise RuntimeError("a real GPU string was not accepted")

    # Every shape of software rasteriser we know how to be lied to by.
    software_strings = (
        "Google SwiftShader",
        "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero)), SwiftShader driver)",
        "llvmpipe (LLVM 15.0.7, 256 bits)",
        "Mesa OffScreen",
        "softpipe",
        "SwiftShader Device (LLVM 10.0.0)",
    )
    for text in software_strings:
        env = {"gpu_unmasked_renderer": text, "gpu_renderer": text}
        try:
            require_hardware_renderer(env, "self-test")
        except RuntimeError as error:
            if "software-rasteriser" not in str(error):
                raise RuntimeError(f"wrong refusal for {text!r}: {error}") from error
        else:
            raise RuntimeError(f"software rasteriser {text!r} was NOT refused")

    # A masked renderer is unproven, and unproven must not publish either.
    for text in ("WebKit WebGL", "Mozilla", "?", ""):
        env = {"gpu_unmasked_renderer": text, "gpu_renderer": text}
        try:
            require_hardware_renderer(env, "self-test")
        except RuntimeError as error:
            if "did not identify its GPU" not in str(error):
                raise RuntimeError(f"wrong refusal for masked {text!r}: {error}") from error
        else:
            raise RuntimeError(f"masked renderer {text!r} was NOT refused")

    # A masked primary must still pass when the unmasked extension answered.
    mixed = {"gpu_renderer": "WebKit WebGL", "gpu_unmasked_renderer": "AMD Radeon RX 6800 (RADV)"}
    if require_hardware_renderer(mixed, "self-test") != "AMD Radeon RX 6800 (RADV)":
        raise RuntimeError("unmasked hardware string was not preferred over the masked one")

    # Mesa is the ordinary hardware stack on AMD/Intel and must NOT be refused.
    for text in ("AMD Radeon Graphics (radeonsi, navi22, LLVM 17.0.6, DRM 3.54)", "Mesa Intel(R) UHD Graphics"):
        env = {"gpu_unmasked_renderer": text, "gpu_renderer": text}
        if require_hardware_renderer(env, "self-test") != text:
            raise RuntimeError(f"hardware Mesa string {text!r} was wrongly refused")

    for bad in ("not a marker", "GC_BENCH_ENV|novalue"):
        try:
            parse_marker(bad)
        except RuntimeError:
            pass
        else:
            raise RuntimeError(f"malformed marker {bad!r} was accepted")

    rows = [
        {"characters": 10, "draw_calls_mean": 100.0, "draw_p95_ms": 1.0},
        {"characters": 20, "draw_calls_mean": 190.0, "draw_p95_ms": 1.8},
        {"characters": 40, "draw_calls_mean": 370.0, "draw_p95_ms": 3.4},
    ]
    marginals = marginal_cost(rows, "draw_calls_mean")
    if [round(v, 3) for _, _, v in marginals] != [9.0, 9.0]:
        raise RuntimeError(f"marginal cost arithmetic self-test failed: {marginals}")

    print("babylon bench controller self-test OK (no browser was started)")


def prove_refusal(args: argparse.Namespace) -> int:
    """Start a REAL Chrome forced onto SwiftShader; the runner must refuse it.

    This is the demonstration AGENTS.md section 9 asks for: proof the gate can
    go red, taken against a browser rather than against a fixture string.
    """
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    stage = prepare_stage(output, args)
    server, base_url = serve(stage, 0)
    try:
        rows, failures = run_browser(
            "chrome", args.chrome, args.chromedriver, base_url,
            ["authored"], [10], 60, 30, 1, output, args.timeout,
            force_software=True,
        )
        if rows:
            print("FAIL: Chrome forced onto SwiftShader produced a PUBLISHED result")
            print(f"  {rows[0]['result_marker']}")
            return 1
        if not any("software-rasteriser" in failure for failure in failures):
            print("FAIL: Chrome-on-SwiftShader failed for the wrong reason:")
            for failure in failures:
                print(f"  {failure}")
            return 1
        print("refusal proven against a real browser:")
        for failure in failures:
            print(f"  {failure}")
        return 0
    finally:
        server.shutdown()
        server.server_close()


def prepare_stage(output: Path, args: argparse.Namespace) -> Path:
    """Assemble the served directory: harness + vendor + captured payload."""
    stage = output / "site"
    stage.mkdir(parents=True, exist_ok=True)
    for name in ("index.html", "bench.js"):
        (stage / name).write_bytes((BENCH_SOURCE / name).read_bytes())
    print("==> vendor artifacts (pinned, verified)", flush=True)
    ensure_vendor(stage / "vendor", offline=args.offline)
    print("==> captured render frames", flush=True)
    ensure_capture(stage / "render_frames.json", args.capture_frames, args.capture_warmup)
    return stage


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="controller logic only")
    parser.add_argument(
        "--prove-refusal",
        action="store_true",
        help="start a real Chrome on SwiftShader and assert the runner refuses it",
    )
    parser.add_argument("--browsers", default="chrome,firefox")
    parser.add_argument("--counts", default=",".join(str(c) for c in DEFAULT_COUNTS))
    parser.add_argument("--variants", default=",".join(DEFAULT_VARIANTS))
    parser.add_argument("--frames", type=int, default=DEFAULT_FRAMES)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument("--capture-frames", type=int, default=1800)
    parser.add_argument("--capture-warmup", type=int, default=300)
    parser.add_argument("--repeats", type=int, default=DEFAULT_REPEATS)
    parser.add_argument("--timeout", type=int, default=420)
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
    if args.prove_refusal:
        return prove_refusal(args)

    if not os.environ.get("DISPLAY"):
        raise SystemExit(
            "DISPLAY is unset. This benchmark runs HEADED by design -- headless "
            "Chrome falls back to SwiftShader and #100 already published one "
            "false negative from it. Try DISPLAY=:1."
        )

    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    stage = prepare_stage(output, args)

    counts = [int(c) for c in args.counts.split(",") if c.strip()]
    variants = [v.strip() for v in args.variants.split(",") if v.strip()]
    browsers = [b.strip() for b in args.browsers.split(",") if b.strip()]

    server, base_url = serve(stage, 0)
    rows: list[dict[str, Any]] = []
    failures: list[str] = []
    try:
        for browser in browsers:
            binary = args.chrome if browser == "chrome" else args.firefox
            driver_path = args.chromedriver if browser == "chrome" else args.geckodriver
            browser_rows, browser_failures = run_browser(
                browser,
                binary,
                driver_path,
                base_url,
                variants,
                counts,
                args.frames,
                args.warmup,
                args.repeats,
                output,
                args.timeout,
            )
            rows.extend(browser_rows)
            failures.extend(browser_failures)
    finally:
        server.shutdown()
        server.server_close()

    report = {
        "schema": 1,
        "generated_at": datetime.now(UTC).isoformat(),
        "issue": 341,
        "vendor": {name: spec["sha256"] for name, spec in VENDOR.items()},
        "frames": args.frames,
        "warmup": args.warmup,
        "repeats": args.repeats,
        "rows": rows,
        "failures": failures,
    }
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if rows:
        print()
        print(render_report(rows))
    print()
    print(f"report written to {output / 'report.json'}")
    if failures:
        print("\nFAILURES:")
        for failure in failures:
            print(f"  {failure}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
