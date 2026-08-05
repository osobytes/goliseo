#!/usr/bin/env python3
"""Measure installer size, cold start and frame cost per native shell (#329).

#329 has to choose between Babylon Native, Electron and Tauri, and its third
acceptance criterion is a per-route table of installer size, cold start and
frame cost. This runner produces the Electron and Tauri rows on Linux.

It reuses the #341 benchmark page verbatim -- same scene, same captured
`RenderFrame` payload, same markers -- so a shell row and a browser row are the
same measurement taken through a different window. `scripts/babylon_bench.py`
owns the page, the pinned vendor artifacts and the capture; this file imports
them rather than copying them, because two drifting definitions of "the bench"
would make the comparison meaningless.

What is different from `babylon_bench.py`:

  * There is no WebDriver. A shell is an application, not a driver session, so
    the page reports its own markers over HTTP (`bench/native_shell/reporter.js`,
    injected at serve time). Both shells report through the identical channel;
    otherwise the comparison would partly be a comparison of channels.
  * Cold start is measured, because for a native route it is a product
    property. The clock starts when this process spawns the shell and stops at
    two page-side marks: `dom_ready` and `scene_ready`.
  * The GPU refusal has a second path. WebKitGTK -- which is the engine Tauri
    ships on Linux, so it is half of what this file exists to measure -- masks
    `WEBGL_debug_renderer_info` and reports its renderer as "WebKit WebGL".
    `babylon_bench.py` correctly refuses that as unproven. Refusing to measure
    Tauri at all would be a worse answer than proving hardware another way, so
    this runner also inspects the shell process tree's mapped libraries: a
    process with `libGLX_nvidia` mapped is on the GPU no matter what string the
    engine is willing to print, and a process with only `swrast_dri` mapped is
    not, no matter what string it prints. Neither path can be overridden by a
    flag.

`--self-test` exercises the parsing, the refusal and the report-naming rules
and starts no shell and no browser. It is not evidence that any shell runs;
per AGENTS.md section 9 those are different claims and this file keeps them
separate.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
import urllib.request
from datetime import UTC, datetime
from functools import partial
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from babylon_bench import (  # noqa: E402  (path must be set first)
    BenchHandler,
    classify_renderer,
    ensure_capture,
    ensure_vendor,
    parse_marker,
    sha256_of,
)

BENCH_SOURCE = ROOT / "bench" / "babylon"
SHELL_SOURCE = ROOT / "bench" / "native_shell"
DEFAULT_OUTPUT = ROOT / ".bench" / "native_shell"

DEFAULT_FRAMES = 600
DEFAULT_WARMUP = 300
DEFAULT_CHARACTERS = 10
DEFAULT_VARIANT = "merged"
DEFAULT_COLD_STARTS = 5
DEFAULT_FRAME_REPEATS = 3

# Mapped shared objects that prove the process is rendering on real hardware.
# These are drivers, not strings a renderer chose to print about itself, which
# is what makes them usable where the engine masks its renderer.
HARDWARE_DRIVER_PATTERNS = (
    "libglx_nvidia",
    "libegl_nvidia",
    "libnvidia-glcore",
    "libnvidia-eglcore",
    "nvidia_drv",
    "radeonsi_dri",
    "amdgpu_dri",
    "iris_dri",
    "i965_dri",
    "crocus_dri",
    "nouveau_dri",
    "panfrost_dri",
    "zink_dri",
)
SOFTWARE_DRIVER_PATTERNS = (
    "swrast_dri",
    "libswrast",
    "llvmpipe",
    "softpipe",
    "libvulkan_lvp",
    "swiftshader",
)

# Renderer strings that are not masked -- they name a specific GPU -- but name
# the WRONG one. WebKit answers `WEBGL_debug_renderer_info` with "Apple GPU" /
# vendor "Apple Inc." on every platform including Linux on an NVIDIA card, as
# fingerprinting resistance. `babylon_bench.py`'s masked list cannot catch this
# because the string looks like a positive identification, so it would sail
# through as hardware evidence and put a fabricated GPU name in a table. These
# strings are demoted to "unproven", which sends the verdict to the process map.
SPOOFED_RENDERER_VALUES = (
    "apple gpu",
    "apple inc.",
    "apple gpu (apple inc.)",
)


def classify_renderer_strict(*candidates: str) -> tuple[str, str]:
    """`babylon_bench.classify_renderer`, plus the WebKit spoof demoted."""
    for candidate in candidates:
        if (candidate or "").strip().lower() in SPOOFED_RENDERER_VALUES:
            return "spoofed", candidate.strip()
    return classify_renderer(*candidates)


class ShellBenchHandler(BenchHandler):
    """The #341 static server, plus a mark endpoint and reporter injection.

    Injection happens here rather than in `bench/babylon/index.html` so the page
    #341 measured stays byte-identical when a browser fetches it. A shell asks
    for `?report=1`; nothing else does.
    """

    marks: list[dict[str, Any]] = []
    reporter_source: bytes = b""

    def do_GET(self) -> None:  # noqa: N802  (http.server API)
        path = self.path.split("?", 1)[0]
        if path == "/gc_shell_reporter.js":
            self.send_response(200)
            self.send_header("Content-Type", "application/javascript")
            self.send_header("Content-Length", str(len(self.reporter_source)))
            self.end_headers()
            self.wfile.write(self.reporter_source)
            return
        if path in ("/", "/index.html") and "report=1" in self.path:
            html = (Path(self.directory) / "index.html").read_bytes()
            injected = html.replace(
                b"</body>",
                b'    <script src="/gc_shell_reporter.js"></script>\n    </body>',
            )
            if injected == html:
                raise RuntimeError("index.html has no </body> to inject the reporter before")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(injected)))
            self.end_headers()
            self.wfile.write(injected)
            return
        super().do_GET()

    def do_POST(self) -> None:  # noqa: N802  (http.server API)
        if self.path != "/gc_shell_mark":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            body = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self.send_error(400, f"unparseable mark: {error}")
            return
        body["received_wall_ms"] = time.time() * 1000.0
        type(self).marks.append(body)
        self.send_response(204)
        self.send_header("Content-Length", "0")
        self.end_headers()


def classify_process_drivers(maps_text: str) -> tuple[str, list[str]]:
    """Judge hardware vs software from the shared objects a process mapped.

    Returns ("hardware" | "software" | "unknown", the driver names seen).

    A hardware driver being mapped wins over a software one also being mapped:
    Mesa's loader routinely probes and maps `swrast_dri` before settling on the
    real driver, so treating its presence as disqualifying would refuse honest
    hardware runs. The reverse -- only software drivers mapped -- is decisive.
    """
    lowered = maps_text.lower()
    hardware = sorted({p for p in HARDWARE_DRIVER_PATTERNS if p in lowered})
    software = sorted({p for p in SOFTWARE_DRIVER_PATTERNS if p in lowered})
    if hardware:
        return "hardware", hardware + software
    if software:
        return "software", software
    return "unknown", []


def process_tree_maps(pid: int, proc_root: Path = Path("/proc")) -> str:
    """Concatenate /proc/<pid>/maps for a process and its descendants.

    Both shells are multi-process: Electron's GPU work happens in a child, and
    WebKitGTK's happens in `WebKitWebProcess`. Reading only the parent would
    find no driver at all and report `unknown` for a run that was on the GPU.
    """
    seen: set[int] = set()
    pending = [pid]
    chunks: list[str] = []
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        try:
            chunks.append((proc_root / str(current) / "maps").read_text(errors="replace"))
        except OSError:
            continue
        # A process tree measured while it is running is a moving target: a
        # thread or a child can exit between the listing and the read. Every
        # step here tolerates that, because a disappearing child must degrade
        # the evidence, never crash the run that is collecting it.
        try:
            tasks = list((proc_root / str(current) / "task").iterdir())
        except OSError:
            continue
        for task in tasks:
            try:
                kids = (task / "children").read_text().split()
            except OSError:
                continue
            pending.extend(int(k) for k in kids)
    return "\n".join(chunks)


def gpu_verdict(env: dict[str, str], maps_text: str) -> dict[str, Any]:
    """Combine the engine's own claim with what the process actually mapped.

    The engine string is preferred when it names hardware, because it is the
    most direct statement about what drew the frame. When the engine declines
    to name its GPU -- WebKitGTK always does -- the mapped drivers decide. A
    software rasteriser found by either route is a refusal.
    """
    string_verdict, string_text = classify_renderer_strict(
        env.get("gpu_unmasked_renderer", ""), env.get("gpu_renderer", "")
    )
    maps_verdict, drivers = classify_process_drivers(maps_text)
    if string_verdict == "software" or maps_verdict == "software":
        verdict = "software"
    elif string_verdict == "hardware":
        verdict = "hardware"
    elif maps_verdict == "hardware":
        verdict = "hardware"
    else:
        verdict = "unknown"
    return {
        "verdict": verdict,
        "engine_string": string_text,
        "engine_string_verdict": string_verdict,
        "mapped_drivers": drivers,
        "mapped_driver_verdict": maps_verdict,
        "evidence": (
            string_text
            if string_verdict == "hardware"
            else ("mapped drivers: " + ", ".join(drivers) if drivers else "<none>")
        ),
        "engine_string_rejected_as": (
            f"{string_verdict}: the engine reported {string_text!r}"
            if string_verdict in ("spoofed", "unknown")
            else ""
        ),
    }


def require_hardware(verdict: dict[str, Any], context: str) -> None:
    if verdict["verdict"] == "software":
        raise RuntimeError(
            f"{context}: refusing to publish a software-rasteriser result "
            f"({verdict['evidence']}). #100 already published one false negative "
            "from exactly this."
        )
    if verdict["verdict"] != "hardware":
        raise RuntimeError(
            f"{context}: neither the engine nor the process map proved this ran "
            f"on a GPU ({verdict['evidence']}). Unproven does not publish."
        )


# --------------------------------------------------------------------------
# Shell routes
# --------------------------------------------------------------------------


class ShellRoute:
    """One packaged native shell: how to build it, size it and launch it."""

    def __init__(self, name: str, directory: Path):
        self.name = name
        self.directory = directory

    def build(self, timeout: int) -> dict[str, Any]:  # pragma: no cover - subclass
        raise NotImplementedError

    def artifacts(self) -> dict[str, int]:  # pragma: no cover - subclass
        raise NotImplementedError

    def command(self, url: str) -> list[str]:  # pragma: no cover - subclass
        raise NotImplementedError


def run_step(command: list[str], cwd: Path, timeout: int, label: str) -> None:
    print(f"    $ {' '.join(command)}", flush=True)
    result = subprocess.run(command, cwd=cwd, timeout=timeout, capture_output=True, text=True)
    if result.returncode != 0:
        tail = (result.stdout + result.stderr).strip().splitlines()[-30:]
        raise RuntimeError(f"{label} failed ({result.returncode}):\n" + "\n".join(tail))


class ElectronRoute(ShellRoute):
    def build(self, timeout: int) -> dict[str, Any]:
        if not (self.directory / "node_modules").is_dir():
            run_step(["npm", "install"], self.directory, timeout, "npm install (electron)")
        run_step(
            ["npx", "--no-install", "electron-builder", "--linux", "appimage"],
            self.directory,
            timeout,
            "electron-builder",
        )
        return {"built": True}

    def _appimage(self) -> Path:
        candidates = sorted((self.directory / "dist").glob("*.AppImage"))
        if not candidates:
            raise RuntimeError(f"no AppImage in {self.directory / 'dist'}; run with --build")
        return candidates[-1]

    def artifacts(self) -> dict[str, int]:
        appimage = self._appimage()
        unpacked = self.directory / "dist" / "linux-unpacked"
        sizes = {"installer_bytes": appimage.stat().st_size}
        if unpacked.is_dir():
            sizes["unpacked_bytes"] = directory_size(unpacked)
        return sizes

    def command(self, url: str) -> list[str]:
        return [str(self._appimage()), url]


def write_solid_png(size: int, path: Path) -> None:
    """Write a solid-colour RGBA PNG with nothing but the standard library.

    Both bundlers refuse to package without an icon, and nothing ever displays
    this one. Generating it here keeps THIRD_PARTY.md's "no tracked
    third-party binary, no tracked image asset" audit true: there is no PNG in
    the repository, only the four lines that produce one.
    """
    import struct
    import zlib

    rows = b"".join(b"\x00" + bytes([13, 24, 33, 255]) * size for _ in range(size))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


class TauriRoute(ShellRoute):
    ICONS = {"32x32.png": 32, "128x128.png": 128, "icon.png": 512}

    def ensure_icons(self) -> None:
        for name, size in self.ICONS.items():
            target = self.directory / "icons" / name
            if not target.is_file():
                write_solid_png(size, target)

    def build(self, timeout: int) -> dict[str, Any]:
        self.ensure_icons()
        run_step(
            ["npx", "--yes", "@tauri-apps/cli@2", "build", "--no-bundle"],
            self.directory,
            timeout,
            "tauri build (binary)",
        )
        run_step(
            ["npx", "--yes", "@tauri-apps/cli@2", "build"],
            self.directory,
            timeout,
            "tauri build (bundle)",
        )
        return {"built": True}

    def _bundle(self, kind: str, suffix: str) -> Path | None:
        found = sorted((self.directory / "target" / "release" / "bundle" / kind).glob(suffix))
        return found[-1] if found else None

    def artifacts(self) -> dict[str, int]:
        sizes: dict[str, int] = {}
        appimage = self._bundle("appimage", "*.AppImage")
        deb = self._bundle("deb", "*.deb")
        if appimage:
            sizes["installer_bytes"] = appimage.stat().st_size
        if deb:
            sizes["deb_bytes"] = deb.stat().st_size
        binary = self.directory / "target" / "release" / "goliseo-native-shell"
        if binary.is_file():
            sizes["binary_bytes"] = binary.stat().st_size
        if not sizes:
            raise RuntimeError(f"no Tauri bundle under {self.directory}; run with --build")
        return sizes

    def command(self, url: str) -> list[str]:
        appimage = self._bundle("appimage", "*.AppImage")
        if appimage:
            return [str(appimage), url]
        binary = self.directory / "target" / "release" / "goliseo-native-shell"
        if not binary.is_file():
            raise RuntimeError(f"no Tauri binary at {binary}; run with --build")
        return [str(binary), url]


ROUTES: dict[str, type[ShellRoute]] = {"electron": ElectronRoute, "tauri": TauriRoute}


def directory_size(path: Path) -> int:
    total = 0
    for entry in path.rglob("*"):
        if entry.is_file() and not entry.is_symlink():
            total += entry.stat().st_size
    return total


# --------------------------------------------------------------------------
# Measurement
# --------------------------------------------------------------------------


def loadavg() -> str:
    return Path("/proc/loadavg").read_text().split(" 0/")[0].strip()


def launch_shell(route: ShellRoute, url: str, log: Path) -> subprocess.Popen[bytes]:
    environment = dict(os.environ)
    # AppImages need FUSE; extract-and-run is the documented fallback and costs
    # start-up time, which is why it is recorded rather than hidden.
    environment.setdefault("APPIMAGE_EXTRACT_AND_RUN", "1")
    handle = log.open("wb")
    return subprocess.Popen(
        route.command(url),
        stdout=handle,
        stderr=subprocess.STDOUT,
        env=environment,
        start_new_session=True,
    )


def stop(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(os.getpgid(process.pid), 15)
    except OSError:
        process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(os.getpgid(process.pid), 9)
        except OSError:
            process.kill()


def run_once(
    route: ShellRoute,
    base_url: str,
    query: str,
    timeout: int,
    log: Path,
    capture_maps: bool,
) -> dict[str, Any]:
    """One shell launch. Returns the marks, markers, timings and GPU evidence."""
    ShellBenchHandler.marks = []
    url = f"{base_url}/index.html?report=1&{query}"
    spawn_wall_ms = time.time() * 1000.0
    process = launch_shell(route, url, log)
    maps_text = ""
    deadline = time.time() + timeout
    try:
        while time.time() < deadline:
            if capture_maps and not maps_text:
                text = process_tree_maps(process.pid)
                if any(p in text.lower() for p in HARDWARE_DRIVER_PATTERNS + SOFTWARE_DRIVER_PATTERNS):
                    maps_text = text
            if any(m.get("name") == "finished" for m in ShellBenchHandler.marks):
                break
            if process.poll() is not None and not ShellBenchHandler.marks:
                raise RuntimeError(
                    f"{route.name} exited {process.returncode} before reporting anything; "
                    f"see {log}"
                )
            time.sleep(0.05)
        else:
            raise RuntimeError(f"{route.name} did not finish within {timeout}s; see {log}")
    finally:
        if capture_maps and not maps_text:
            maps_text = process_tree_maps(process.pid)
        stop(process)

    marks = list(ShellBenchHandler.marks)
    timings: dict[str, float] = {}
    for name in ("reporter_ready", "dom_ready", "scene_ready", "finished"):
        entry = next((m for m in marks if m.get("name") == name), None)
        if entry:
            timings[f"{name}_ms"] = round(entry["wall_ms"] - spawn_wall_ms, 1)
    markers = [m["line"] for m in marks if m.get("kind") == "marker"]
    return {
        "url": url,
        "timings": timings,
        "markers": markers,
        "maps_text": maps_text,
        "log": str(log),
        # Per launch, not per session. Other agents build in sibling worktrees on
        # this box and the load average moved between 6 and 38 across one run;
        # a reader deciding how much of a number to believe needs the load that
        # was present for *that* launch, not an average over the session.
        "loadavg": loadavg(),
    }


def summarise_run(markers: list[str]) -> tuple[dict[str, str], dict[str, str]]:
    env: dict[str, str] = {}
    result: dict[str, str] = {}
    for line in markers:
        fields = parse_marker(line)
        if fields["_kind"] == "GC_BENCH_ENV":
            env = fields
        elif fields["_kind"] == "GC_BENCH_RESULT":
            result = fields
        elif fields["_kind"] == "GC_BENCH_ERROR":
            raise RuntimeError(f"the bench page reported an error: {line}")
    if not env:
        raise RuntimeError("no GC_BENCH_ENV marker was reported")
    return env, result


def cold_start_run(
    route: ShellRoute, base_url: str, args: argparse.Namespace, output: Path, index: int
) -> dict[str, float]:
    log = output / f"{route.name}_coldstart_{index}.log"
    run = run_once(
        route,
        base_url,
        f"count=1&frames=1&warmup=0&variant={args.variant}&shadows=off",
        args.timeout,
        log,
        capture_maps=False,
    )
    print(
        f"    {route.name} cold start {index}: {run['timings']} load={run['loadavg']}", flush=True
    )
    return {**run["timings"], "loadavg": run["loadavg"]}


def frame_run(
    route: ShellRoute, base_url: str, args: argparse.Namespace, output: Path, index: int
) -> tuple[dict[str, str], dict[str, str], str, str]:
    log = output / f"{route.name}_frames_{index}.log"
    run = run_once(
        route,
        base_url,
        (
            f"count={args.characters}&frames={args.frames}&warmup={args.warmup}"
            f"&variant={args.variant}"
        ),
        args.frame_timeout,
        log,
        capture_maps=True,
    )
    env_pass, result_pass = summarise_run(run["markers"])
    result_pass["_loadavg"] = run["loadavg"]
    print(
        f"    {route.name} pass {index}: draw_calls={result_pass.get('draw_calls_mean')} "
        f"draw_p50={result_pass.get('draw_p50')} frame_p95={result_pass.get('frame_p95')} "
        f"load={run['loadavg']}",
        flush=True,
    )
    return env_pass, result_pass, run["maps_text"], run["log"]


def build_row(
    route_name: str,
    sizes: dict[str, int],
    cold: list[dict[str, float]],
    passes: list[tuple[dict[str, str], dict[str, str], str, str]],
    args: argparse.Namespace,
) -> dict[str, Any]:
    if not passes:
        raise RuntimeError(
            f"{route_name}: no frame-cost pass completed, so there is no row to build"
        )
    env = passes[0][0]
    maps_text = next((m for _, _, m, _ in passes if m), "")
    verdict = gpu_verdict(env, maps_text)
    require_hardware(verdict, f"{route_name} frame cost")

    calls = {float(r.get("draw_calls_mean", 0.0)) for _, r, _, _ in passes}
    if len(calls) != 1:
        raise RuntimeError(
            f"{route_name}: draw-call count is not deterministic across passes ({calls}); "
            "the scene changed between runs and the timings are not comparable"
        )

    def across(key: str) -> dict[str, float]:
        values = [float(r.get(key, 0.0)) for _, r, _, _ in passes]
        return {
            "median": round(statistics.median(values), 3),
            "min": round(min(values), 3),
            "max": round(max(values), 3),
        }

    result = passes[0][1]

    def cold_stat(key: str) -> dict[str, float] | None:
        values = [run[key] for run in cold if key in run]
        if not values:
            return None
        return {
            "runs": len(values),
            "min": round(min(values), 1),
            "median": round(statistics.median(values), 1),
            "max": round(max(values), 1),
        }

    return {
        "route": route_name,
        "sizes": sizes,
        "cold_start": {
            "dom_ready_ms": cold_stat("dom_ready_ms"),
            "scene_ready_ms": cold_stat("scene_ready_ms"),
            "raw": cold,
        },
        "gpu": verdict,
        "env": env,
        "frame": {
            "characters": int(result.get("characters", args.characters)),
            "variant": result.get("variant", args.variant),
            "measured_frames": int(result.get("measured_frames", 0)),
            "passes": len(passes),
            "draw_calls_mean": calls.pop(),
            "draw_p50_ms": across("draw_p50"),
            "draw_p95_ms": across("draw_p95"),
            "frame_p50_ms": across("frame_p50"),
            "frame_p95_ms": across("frame_p95"),
            "state_hash": result.get("state_hash", ""),
            "loadavg_per_pass": [r.get("_loadavg", "") for _, r, _, _ in passes],
        },
        "user_agent": env.get("user_agent", ""),
        "loadavg": loadavg(),
        "logs": {"frames": [log for _, _, _, log in passes]},
    }


def report_path(output: Path, failures: list[str]) -> Path:
    """A partial matrix must not be able to land on the evidence-of-record path.

    Same rule and same reason as `babylon_bench.py`: a reader who forgets to
    check the exit code must not be able to pick up half a table.
    """
    return output / ("report.json" if not failures else "report_incomplete.json")


def prepare_stage(output: Path, args: argparse.Namespace) -> Path:
    stage = output / "site"
    stage.mkdir(parents=True, exist_ok=True)
    for name in ("index.html", "bench.js"):
        (stage / name).write_bytes((BENCH_SOURCE / name).read_bytes())
    print("==> vendor artifacts (pinned, verified)", flush=True)
    ensure_vendor(stage / "vendor", offline=args.offline)
    print("==> captured render frames", flush=True)
    ensure_capture(stage / "render_frames.json", args.capture_frames, args.capture_warmup)
    return stage


def serve(directory: Path, reporter: bytes, port: int) -> tuple[ThreadingHTTPServer, str]:
    ShellBenchHandler.reporter_source = reporter
    handler = partial(ShellBenchHandler, directory=str(directory))
    server = ThreadingHTTPServer(("127.0.0.1", port), handler)
    import threading

    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_address[1]}"


def self_test() -> None:
    """Controller logic only. Starts no shell, no browser, and no server."""
    # Driver classification: the reason this file can measure WebKitGTK at all.
    hardware_maps = (
        "7f0e00000000-7f0e00100000 r-xp /usr/lib/x86_64-linux-gnu/libGLX_nvidia.so.0\n"
        "7f0e00200000-7f0e00300000 r-xp /usr/lib/x86_64-linux-gnu/dri/swrast_dri.so\n"
    )
    verdict, drivers = classify_process_drivers(hardware_maps)
    if verdict != "hardware" or "libglx_nvidia" not in drivers:
        raise RuntimeError(f"a mapped NVIDIA driver was not recognised: {verdict} {drivers}")

    software_maps = "7f00-7f01 r-xp /usr/lib/x86_64-linux-gnu/dri/swrast_dri.so\n"
    verdict, drivers = classify_process_drivers(software_maps)
    if verdict != "software":
        raise RuntimeError(f"a software-only process map was not refused: {verdict} {drivers}")

    verdict, drivers = classify_process_drivers("7f00-7f01 r-xp /usr/lib/libc.so.6\n")
    if verdict != "unknown" or drivers:
        raise RuntimeError(f"an empty process map must be unknown, got {verdict} {drivers}")

    # Reading a process tree that is exiting underneath the reader must degrade
    # the evidence, not crash the run. The first version of this raised
    # FileNotFoundError from a lazily-evaluated /proc listing and lost a
    # complete Tauri measurement to it.
    if process_tree_maps(0x7FFFFFFF) != "":
        raise RuntimeError("reading a nonexistent process tree must return no evidence")
    own = process_tree_maps(os.getpid())
    if "r-xp" not in own and "r--p" not in own:
        raise RuntimeError("reading this process's own map returned nothing usable")

    # The exact shape that lost a complete Tauri measurement: a process whose
    # `maps` is readable and whose `task` directory has gone. `Path.iterdir()`
    # is lazy, so the original `except OSError` around the call never saw the
    # FileNotFoundError -- it surfaced from the `for` that consumed it. Built
    # against a fake /proc under `mktemp -d` rather than by racing a real
    # process, so it is deterministic and cannot disturb sibling worktrees.
    import tempfile

    fake_proc = Path(tempfile.mkdtemp(prefix="gc329-proc-"))
    try:
        (fake_proc / "4242").mkdir()
        (fake_proc / "4242" / "maps").write_text(
            "7f00-7f01 r-xp /usr/lib/x86_64-linux-gnu/libGLX_nvidia.so.0\n"
        )
        vanished = process_tree_maps(4242, proc_root=fake_proc)
        if "libGLX_nvidia" not in vanished:
            raise RuntimeError("a process whose task dir vanished lost its map evidence entirely")
        if classify_process_drivers(vanished)[0] != "hardware":
            raise RuntimeError("evidence survived the vanishing task dir but was misclassified")
    finally:
        shutil.rmtree(fake_proc, ignore_errors=True)

    # The combined verdict: a masked engine string must not block a run whose
    # process map proves hardware, and must not rescue one that proves software.
    masked = {"gpu_renderer": "WebKit WebGL", "gpu_unmasked_renderer": "WebKit WebGL"}
    combined = gpu_verdict(masked, hardware_maps)
    if combined["verdict"] != "hardware":
        raise RuntimeError(f"masked engine + real driver must pass, got {combined}")
    require_hardware(combined, "self-test")

    combined = gpu_verdict(masked, software_maps)
    if combined["verdict"] != "software":
        raise RuntimeError(f"masked engine + software driver must fail, got {combined}")
    for bad, needle in (
        (gpu_verdict(masked, software_maps), "software-rasteriser"),
        (gpu_verdict(masked, ""), "did not"),
    ):
        try:
            require_hardware(bad, "self-test")
        except RuntimeError as error:
            if needle not in str(error) and "Unproven" not in str(error):
                raise RuntimeError(f"wrong refusal for {bad}: {error}") from error
        else:
            raise RuntimeError(f"{bad} was NOT refused")

    # A software rasteriser named by the engine must be refused even when the
    # process happens to have a hardware driver mapped: the string is a direct
    # statement about what drew, and the map is only circumstantial.
    swiftshader = {
        "gpu_renderer": "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device), SwiftShader driver)",
        "gpu_unmasked_renderer": "",
    }
    if gpu_verdict(swiftshader, hardware_maps)["verdict"] != "software":
        raise RuntimeError("SwiftShader named by the engine was not refused")

    # WebKit's "Apple GPU" spoof. This is the one that would have published a
    # fabricated GPU name: it is a positive-looking identification, so the
    # inherited classifier accepts it, and on this Linux/NVIDIA box it is false.
    spoofed = {"gpu_renderer": "Apple GPU", "gpu_unmasked_renderer": "Apple GPU"}
    if classify_renderer(spoofed["gpu_renderer"])[0] != "hardware":
        raise RuntimeError(
            "the inherited classifier no longer accepts 'Apple GPU'; the strict "
            "wrapper below may have become dead code and should be re-checked"
        )
    if classify_renderer_strict(spoofed["gpu_renderer"])[0] != "spoofed":
        raise RuntimeError("the WebKit 'Apple GPU' spoof was not demoted")
    combined = gpu_verdict(spoofed, hardware_maps)
    if combined["verdict"] != "hardware":
        raise RuntimeError(f"spoofed string + real driver must pass on the map, got {combined}")
    if "Apple" in combined["evidence"]:
        raise RuntimeError(f"a spoofed GPU name reached the evidence column: {combined}")
    if "libglx_nvidia" not in combined["evidence"]:
        raise RuntimeError(f"the mapped driver did not become the evidence: {combined}")
    if gpu_verdict(spoofed, software_maps)["verdict"] != "software":
        raise RuntimeError("a spoofed string must not rescue a software-only process map")
    if gpu_verdict(spoofed, "")["verdict"] != "unknown":
        raise RuntimeError("a spoofed string with no driver evidence must stay unproven")

    # A real GPU string must still be preferred and reported verbatim.
    real = {
        "gpu_renderer": "ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2)",
        "gpu_unmasked_renderer": "NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2",
    }
    combined = gpu_verdict(real, "")
    if combined["verdict"] != "hardware" or "RTX 2070" not in combined["evidence"]:
        raise RuntimeError(f"a real GPU string was not accepted: {combined}")

    # Marker summarising, including the failure that matters most: a page that
    # errors must not be summarised into a row.
    env_line = (
        "GC_BENCH_ENV|runtime=babylon|babylon=9.19.1|api=webgl2|gpu_renderer=WebKit WebGL"
        "|gpu_unmasked_renderer=WebKit WebGL|user_agent=probe"
    )
    result_line = (
        "GC_BENCH_RESULT|renderer=babylon-merged|variant=merged|characters=10"
        "|measured_frames=600|draw_p50=1.35|draw_p95=2.29|frame_p50=4.1|frame_p95=4.61"
        "|draw_calls_mean=87.0|state_hash=deadbeefdeadbeef"
    )
    env, result = summarise_run([env_line, result_line])
    if env["gpu_renderer"] != "WebKit WebGL" or result["draw_calls_mean"] != "87.0":
        raise RuntimeError("marker summarising self-test failed")
    try:
        summarise_run([env_line, "GC_BENCH_ERROR|where=window|message=boom"])
    except RuntimeError as error:
        if "reported an error" not in str(error):
            raise RuntimeError(f"wrong error for a failed page: {error}") from error
    else:
        raise RuntimeError("a page that reported GC_BENCH_ERROR was summarised anyway")
    try:
        summarise_run([result_line])
    except RuntimeError as error:
        if "GC_BENCH_ENV" not in str(error):
            raise RuntimeError(f"wrong error for a missing env marker: {error}") from error
    else:
        raise RuntimeError("a run with no environment marker was accepted")

    # Reporter injection: the shell rows are worthless if the reporter silently
    # fails to land, so the injector refuses a page it cannot inject into.
    reporter_tag = b'<script src="/gc_shell_reporter.js"></script>'
    page = (BENCH_SOURCE / "index.html").read_bytes()
    if b"</body>" not in page:
        raise RuntimeError("bench/babylon/index.html no longer has a </body> to inject before")
    if reporter_tag in page:
        raise RuntimeError("the bench page must not ship the reporter; it is injected at serve")
    if not (SHELL_SOURCE / "reporter.js").is_file():
        raise RuntimeError("bench/native_shell/reporter.js is missing")

    # The generated bundler icons, checked hermetically because other agents are
    # building in sibling worktrees and a self-test that writes into the source
    # tree would race them.
    import tempfile

    scratch = Path(tempfile.mkdtemp(prefix="gc329-selftest-"))
    try:
        for size in (32, 512):
            target = scratch / f"{size}.png"
            write_solid_png(size, target)
            head = target.read_bytes()[:24]
            if head[:8] != b"\x89PNG\r\n\x1a\n":
                raise RuntimeError(f"generated icon {target} is not a PNG")
            width = int.from_bytes(head[16:20], "big")
            if width != size:
                raise RuntimeError(f"generated icon {target} is {width}px, expected {size}px")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)

    # A route that produced no completed pass must not become a row of zeroes.
    class _Args:
        characters = 10
        variant = "merged"

    try:
        build_row("tauri", {"installer_bytes": 1}, [], [], _Args())  # type: ignore[arg-type]
    except RuntimeError as error:
        if "no frame-cost pass" not in str(error):
            raise RuntimeError(f"wrong error for a route with no passes: {error}") from error
    else:
        raise RuntimeError("a route with no completed pass was turned into a row anyway")

    # Draw calls are deterministic for a fixed scene; a route whose passes
    # disagree measured two different scenes and must not be averaged.
    drifting = [
        ({"gpu_unmasked_renderer": "NVIDIA GeForce RTX 2070 SUPER"}, {"draw_calls_mean": "87.0"}, "", "a"),
        ({"gpu_unmasked_renderer": "NVIDIA GeForce RTX 2070 SUPER"}, {"draw_calls_mean": "88.0"}, "", "b"),
    ]
    try:
        build_row("electron", {}, [], drifting, _Args())  # type: ignore[arg-type]
    except RuntimeError as error:
        if "not deterministic" not in str(error):
            raise RuntimeError(f"wrong error for drifting draw calls: {error}") from error
    else:
        raise RuntimeError("passes with different draw-call counts were averaged anyway")

    if report_path(Path("/nonexistent"), []).name != "report.json":
        raise RuntimeError("a complete run must write report.json")
    if report_path(Path("/nonexistent"), ["tauri: timed out"]).name != "report_incomplete.json":
        raise RuntimeError("a run with failures must NOT write report.json")

    print("native shell bench controller self-test OK (no shell was started)")


def render_report(rows: list[dict[str, Any]]) -> str:
    lines = [
        "| route | installer | cold start dom_ready | cold start scene_ready | "
        "draw calls | draw p50 | draw p95 | frame p95 | GPU evidence |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    for row in rows:
        installer = row["sizes"].get("installer_bytes", 0) / 1_048_576
        cold_dom = row["cold_start"]["dom_ready_ms"]
        cold_scene = row["cold_start"]["scene_ready_ms"]
        frame = row["frame"]
        lines.append(
            f"| {row['route']} | {installer:.1f} MiB "
            f"| {cold_dom['median'] if cold_dom else 'NOT MEASURED'} ms "
            f"| {cold_scene['median'] if cold_scene else 'NOT MEASURED'} ms "
            f"| {frame['draw_calls_mean']:.1f} | {frame['draw_p50_ms']['median']:.2f} ms "
            f"| {frame['draw_p95_ms']['median']:.2f} ms "
            f"| {frame['frame_p95_ms']['median']:.2f} ms "
            f"| {row['gpu']['evidence']} |"
        )
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="controller logic only")
    parser.add_argument("--routes", default="electron,tauri")
    parser.add_argument("--build", action="store_true", help="build the shells before measuring")
    parser.add_argument("--characters", type=int, default=DEFAULT_CHARACTERS)
    parser.add_argument("--variant", default=DEFAULT_VARIANT)
    parser.add_argument("--frames", type=int, default=DEFAULT_FRAMES)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument("--cold-starts", type=int, default=DEFAULT_COLD_STARTS)
    parser.add_argument("--frame-repeats", type=int, default=DEFAULT_FRAME_REPEATS)
    parser.add_argument("--capture-frames", type=int, default=1800)
    parser.add_argument("--capture-warmup", type=int, default=300)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--frame-timeout", type=int, default=600)
    parser.add_argument("--build-timeout", type=int, default=1800)
    parser.add_argument("--output", default=str(DEFAULT_OUTPUT))
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    if not os.environ.get("DISPLAY"):
        raise SystemExit(
            "DISPLAY is unset. These shells are desktop applications and this "
            "measurement runs headed on real hardware by design. Try DISPLAY=:1."
        )

    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    stage = prepare_stage(output, args)
    reporter = (SHELL_SOURCE / "reporter.js").read_bytes()

    routes = [name.strip() for name in args.routes.split(",") if name.strip()]
    for name in routes:
        if name not in ROUTES:
            raise SystemExit(f"unknown route {name!r}; known: {', '.join(sorted(ROUTES))}")

    server, base_url = serve(stage, reporter, 0)
    rows: list[dict[str, Any]] = []
    failures: list[str] = []
    started = datetime.now(UTC).isoformat()

    live: dict[str, ShellRoute] = {}
    sizes: dict[str, dict[str, int]] = {}
    cold: dict[str, list[dict[str, float]]] = {}
    passes: dict[str, list[tuple[dict[str, str], dict[str, str], str, str]]] = {}

    def drop(name: str, error: Exception) -> None:
        failures.append(f"{name}: {error}")
        print(f"    !! {name}: {error}", flush=True)
        live.pop(name, None)

    try:
        for name in routes:
            route = ROUTES[name](name, SHELL_SOURCE / name)
            try:
                if args.build:
                    print(f"==> {name}: build", flush=True)
                    route.build(args.build_timeout)
                print(f"==> {name}: installer artifacts", flush=True)
                sizes[name] = route.artifacts()
                for key, value in sizes[name].items():
                    print(f"    {key} = {value} ({value / 1_048_576:.1f} MiB)", flush=True)
                live[name] = route
                cold[name] = []
                passes[name] = []
            except Exception as error:  # noqa: BLE001 - every route failure is reportable
                drop(name, error)

        # Interleaved, one pass of every route at a time, not one route to
        # completion and then the next. #341 learned this the expensive way: run
        # sequentially, and browser start-up, GPU-process init and the NVIDIA
        # clock ramp all land on whichever configuration went first, and a
        # drifting machine load lands entirely on whichever went last. The first
        # sequential version of this file measured Electron 6.4x faster than
        # Tauri in one session and 1.4x faster in the next, on the same binaries,
        # because a C++ build started halfway through. Interleaving puts the
        # drift into every route equally, where the median across passes can see
        # it.
        print(f"==> cold start, {args.cold_starts} interleaved passes", flush=True)
        for index in range(args.cold_starts):
            for name, route in list(live.items()):
                try:
                    cold[name].append(cold_start_run(route, base_url, args, output, index))
                except Exception as error:  # noqa: BLE001
                    drop(name, error)

        print(
            f"==> frame cost ({args.characters} characters, {args.variant}), "
            f"{args.frame_repeats} interleaved passes",
            flush=True,
        )
        for index in range(args.frame_repeats):
            for name, route in list(live.items()):
                try:
                    passes[name].append(frame_run(route, base_url, args, output, index))
                except Exception as error:  # noqa: BLE001
                    drop(name, error)

        for name in list(live):
            try:
                rows.append(build_row(name, sizes[name], cold[name], passes[name], args))
            except Exception as error:  # noqa: BLE001
                drop(name, error)
    finally:
        server.shutdown()

    report = {
        "issue": 329,
        "started": started,
        "finished": datetime.now(UTC).isoformat(),
        "host": {
            "uname": subprocess.run(["uname", "-sr"], capture_output=True, text=True).stdout.strip(),
            "display": os.environ.get("DISPLAY", ""),
            "loadavg": loadavg(),
        },
        "config": {
            "characters": args.characters,
            "variant": args.variant,
            "frames": args.frames,
            "warmup": args.warmup,
            "cold_starts": args.cold_starts,
            "frame_repeats": args.frame_repeats,
        },
        "site_sha256": {
            name: sha256_of(stage / name) for name in ("index.html", "bench.js") if (stage / name).is_file()
        },
        "rows": rows,
        "failures": failures,
    }
    destination = report_path(output, failures)
    stale = output / "report.json"
    if failures and stale.is_file():
        stale.unlink()
    destination.write_text(json.dumps(report, indent=2) + "\n")
    print()
    print(render_report(rows))
    print()
    print(f"wrote {destination}")
    if failures:
        for failure in failures:
            print(f"FAILED {failure}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
