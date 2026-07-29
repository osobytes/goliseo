#!/usr/bin/env python3
"""Collect one OMP-0 stable-browser evidence packet with Selenium."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import os
import platform
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
CANVAS_WIDTH = 960
CANVAS_HEIGHT = 540
GEOMETRY_TOLERANCE = 0.5
DEFAULT_VIEWPORTS = [(960, 540), (1280, 720), (1920, 1080)]
LETTERBOX_PROBES = {
    "tall": (800, 540),
    "wide": (1280, 540),
}
POINTER_TARGET = {
    "route": "credits",
    "rect": {"height": 42, "width": 260, "x": 350, "y": 382},
    "x": 480,
    "y": 403,
}
EXPECTED_FLOW = ["title", "squad", "formation", "tactic", "match", "result"]
SOFTWARE_RENDERERS = (
    "lavapipe",
    "llvmpipe",
    "microsoft basic render",
    "software",
    "softpipe",
    "swiftshader",
    "warp",
)
MARKER_PATTERN = re.compile(r"(GC_(?:BROWSER|METRICS)\|.*?)(?:\"$|$)")
HARD_RUNTIME_PATTERN = re.compile(
    r"(?:\bfatal\b|causing a crash|\buncaught\b|unhandled (?:exception|rejection)|"
    r"webassembly.*(?:trap|error)|wasm.*trap)",
    re.IGNORECASE,
)
HEAP_CROSSCHECK_TOLERANCE_PERCENT = 5
FIREFOX_FLOW_WAIT_CHUNK_SECONDS = 90
AUDIO_PROBE_SECONDS = 5
AUDIO_PROBE_INTERVAL_SECONDS = 0.5
AUDIO_PROBE_MAX_OBSERVATIONS = int(AUDIO_PROBE_SECONDS / AUDIO_PROBE_INTERVAL_SECONDS) + 1
AUDIO_PROBE_WAIT_SLACK_SECONDS = 0.75
WINDOWS_GPU_MAX_ADAPTERS = 8
WINDOWS_GPU_MAX_FIELD_LENGTH = 512
FIREFOX_HEAP_CHECKPOINTS = (("t0", 0), ("t5", 300), ("t10", 600))
FIREFOX_HEAP_PROMPT_LEAD_SECONDS = 15
FIREFOX_HEAP_TIMING_TOLERANCE_SECONDS = 15


def utc_now() -> str:
    return datetime.now(UTC).isoformat()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_gzip_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(path, "wt", encoding="utf-8", compresslevel=9) as stream:
        json.dump(value, stream, separators=(",", ":"), sort_keys=True)
        stream.write("\n")


def file_evidence(path: Path, not_before_ns: int | None = None) -> dict[str, Any]:
    if not path.is_file():
        raise RuntimeError(f"required evidence file does not exist: {path}")
    stat = path.stat()
    if stat.st_size <= 0:
        raise RuntimeError(f"required evidence file is empty: {path}")
    if not_before_ns is not None and stat.st_mtime_ns < not_before_ns:
        raise RuntimeError(f"evidence file predates its checkpoint prompt: {path}")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "modified_at": datetime.fromtimestamp(stat.st_mtime, UTC).isoformat(),
        "name": path.name,
        "path": str(path.resolve()),
        "sha256": digest.hexdigest(),
        "size_bytes": stat.st_size,
    }


def firefox_heap_snapshot_timing(creation_times_us: list[int]) -> dict[str, Any]:
    if (
        len(creation_times_us) != 3
        or any(
            isinstance(value, bool) or not isinstance(value, int)
            for value in creation_times_us
        )
    ):
        return {"offsets_seconds": [], "pass": False}
    offsets = [
        round((created - creation_times_us[0]) / 1_000_000, 6)
        for created in creation_times_us
    ]
    return {
        "offsets_seconds": offsets,
        "pass": (
            creation_times_us[0] < creation_times_us[1] < creation_times_us[2]
            and abs(offsets[1] - 300) <= FIREFOX_HEAP_TIMING_TOLERANCE_SECONDS
            and abs(offsets[2] - 600) <= FIREFOX_HEAP_TIMING_TOLERANCE_SECONDS
        ),
    }


def parse_viewport(value: str) -> tuple[int, int]:
    match = re.fullmatch(r"([1-9][0-9]*)x([1-9][0-9]*)", value.lower())
    if not match:
        raise argparse.ArgumentTypeError("viewport must look like 960x540")
    return int(match.group(1)), int(match.group(2))


def expected_canvas_rect(width: int, height: int) -> dict[str, float]:
    scale = min(width / CANVAS_WIDTH, height / CANVAS_HEIGHT)
    canvas_width = CANVAS_WIDTH * scale
    canvas_height = CANVAS_HEIGHT * scale
    return {
        "height": canvas_height,
        "width": canvas_width,
        "x": (width - canvas_width) / 2,
        "y": (height - canvas_height) / 2,
    }


def rect_matches(
    actual: dict[str, Any],
    expected: dict[str, float],
    tolerance: float = GEOMETRY_TOLERANCE,
) -> bool:
    try:
        return all(
            abs(float(actual.get(key, 0)) - expected[key]) <= tolerance
            for key in ("height", "width", "x", "y")
        )
    except (TypeError, ValueError):
        return False


def logical_to_client(rect: dict[str, Any], x: float, y: float) -> dict[str, float]:
    return {
        "x": float(rect["x"]) + x * float(rect["width"]) / CANVAS_WIDTH,
        "y": float(rect["y"]) + y * float(rect["height"]) / CANVAS_HEIGHT,
    }


def point_in_rect(rect: dict[str, Any], point: dict[str, float]) -> bool:
    return (
        float(rect["x"]) <= point["x"] <= float(rect["x"]) + float(rect["width"])
        and float(rect["y"]) <= point["y"] <= float(rect["y"]) + float(rect["height"])
    )


def pointer_offset_control(
    rect: dict[str, Any],
    logical_x: float,
    logical_y: float,
) -> dict[str, Any]:
    client = logical_to_client(rect, logical_x, logical_y)
    logical_if_offset_omitted = {
        "x": client["x"] * CANVAS_WIDTH / float(rect["width"]),
        "y": client["y"] * CANVAS_HEIGHT / float(rect["height"]),
    }
    correct_logical = {"x": logical_x, "y": logical_y}
    correct_hits_target = point_in_rect(POINTER_TARGET["rect"], correct_logical)
    omitted_offset_hits_target = point_in_rect(
        POINTER_TARGET["rect"],
        logical_if_offset_omitted,
    )
    return {
        "correct_hits_target": correct_hits_target,
        "expected_target_rect": POINTER_TARGET["rect"],
        "logical_if_offset_omitted": logical_if_offset_omitted,
        "omitted_offset_hits_target": omitted_offset_hits_target,
        "pass": correct_hits_target and not omitted_offset_hits_target,
    }


def marker_message(message: str) -> str | None:
    match = MARKER_PATTERN.search(message)
    return match.group(1) if match else None


def parse_marker(message: str) -> dict[str, str] | None:
    message = marker_message(message) or message
    prefix, separator, payload = message.partition("|")
    if prefix not in {"GC_BROWSER", "GC_METRICS"} or not separator:
        return None
    parts = payload.split("|")
    record = {"source": prefix, "kind": parts[0]}
    for part in parts[1:]:
        key, equals, value = part.partition("=")
        if equals:
            record["event_kind" if key == "kind" else key] = value
    return record


def records(entries: list[dict[str, Any]]) -> list[dict[str, str]]:
    parsed = []
    for entry in entries:
        message = entry.get("message")
        if isinstance(message, str):
            record = parse_marker(message)
            if record:
                parsed.append(record)
    return parsed


def wait_until(predicate: Any, timeout: float, description: str) -> Any:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise RuntimeError(f"timed out waiting for {description}")


def fetch_json(url: str) -> dict[str, Any]:
    request = urllib.request.Request(url, headers={"User-Agent": "goliseo-evidence/1"})
    with urllib.request.urlopen(request, timeout=30) as response:
        value = json.load(response)
    if not isinstance(value, dict):
        raise RuntimeError(f"expected an object from {url}")
    return value


def fetch_bytes(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "goliseo-evidence/1"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def with_query(url: str, values: dict[str, str]) -> str:
    parsed = urllib.parse.urlsplit(url)
    query = dict(urllib.parse.parse_qsl(parsed.query, keep_blank_values=True))
    query.update(values)
    return urllib.parse.urlunsplit(
        (
            parsed.scheme,
            parsed.netloc,
            parsed.path,
            urllib.parse.urlencode(query),
            parsed.fragment,
        )
    )


def validate_manifest(base_url: str, manifest: dict[str, Any]) -> None:
    files = manifest.get("files")
    if not isinstance(files, dict) or not files:
        raise RuntimeError("artifact manifest has no file hashes")
    for name, expected in sorted(files.items()):
        if (
            not isinstance(name, str)
            or not isinstance(expected, str)
            or name.startswith("/")
            or ".." in Path(name).parts
        ):
            raise RuntimeError(f"artifact manifest has an unsafe file entry: {name!r}")
        actual = hashlib.sha256(
            fetch_bytes(urllib.parse.urljoin(base_url, urllib.parse.quote(name)))
        ).hexdigest()
        if actual != expected:
            raise RuntimeError(f"served artifact hash mismatch for {name}")
    package = manifest.get("game_package", {})
    package_path = package.get("path")
    if not isinstance(package_path, str) or files.get(package_path) != package.get("sha256"):
        raise RuntimeError("game-package hash is inconsistent with the artifact manifest")


def command_output(command: list[str]) -> str | None:
    executable = shutil.which(command[0])
    if not executable:
        return None
    result = subprocess.run(
        [executable, *command[1:]],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    output = result.stdout.strip()
    return output if result.returncode == 0 and output else None


def executable_metadata(path: Path) -> dict[str, Any]:
    resolved = path.resolve()
    version = None
    try:
        result = subprocess.run(
            [str(resolved), "--version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
        output = (result.stdout or result.stderr).strip()
        if output:
            version = output
    except (OSError, subprocess.SubprocessError):
        pass
    digest = hashlib.sha256()
    with resolved.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "path": str(resolved),
        "sha256": digest.hexdigest(),
        "size_bytes": resolved.stat().st_size,
        "version_output": version,
    }


def selenium_metadata() -> dict[str, Any]:
    try:
        import selenium
        from selenium.webdriver.common.selenium_manager import SeleniumManager
    except ImportError as error:
        raise RuntimeError("Selenium is required: python3 -m pip install selenium") from error
    return {
        "cache_path": os.environ.get("SE_CACHE_PATH"),
        "manager": executable_metadata(SeleniumManager._get_binary()),
        "python": executable_metadata(Path(sys.executable)),
        "selenium_version": selenium.__version__,
    }


def parse_windows_gpu_output(output: str | None) -> dict[str, Any]:
    result: dict[str, Any] = {
        "adapters": [],
        "available": False,
        "truncated": False,
    }
    if not output:
        return result
    try:
        rows = json.loads(output)
    except json.JSONDecodeError:
        return result
    if isinstance(rows, dict):
        rows = [rows]
    if not isinstance(rows, list):
        return result
    adapters = []
    fields = {
        "AdapterCompatibility": "adapter_compatibility",
        "DriverVersion": "driver_version",
        "Name": "name",
        "PNPDeviceID": "pnp_device_id",
        "VideoProcessor": "video_processor",
    }
    for row in rows[:WINDOWS_GPU_MAX_ADAPTERS]:
        if not isinstance(row, dict):
            continue
        adapter = {}
        for source, target in fields.items():
            value = row.get(source)
            if value is not None:
                adapter[target] = str(value)[:WINDOWS_GPU_MAX_FIELD_LENGTH]
        if adapter:
            adapters.append(adapter)
    result["adapters"] = adapters
    result["available"] = bool(adapters)
    result["truncated"] = len(rows) > WINDOWS_GPU_MAX_ADAPTERS
    return result


def windows_gpu_metadata() -> dict[str, Any]:
    script = (
        "Get-CimInstance Win32_VideoController | "
        "Select-Object Name,DriverVersion,AdapterCompatibility,PNPDeviceID,VideoProcessor | "
        "ConvertTo-Json -Compress"
    )
    return parse_windows_gpu_output(command_output(["powershell", "-NoProfile", "-Command", script]))


def os_metadata() -> dict[str, Any]:
    value: dict[str, Any] = {
        "platform": platform.platform(),
        "release": platform.release(),
        "system": platform.system(),
        "version": platform.version(),
    }
    release_file = Path("/etc/os-release")
    if release_file.is_file():
        fields = {}
        for line in release_file.read_text(encoding="utf-8").splitlines():
            key, separator, raw = line.partition("=")
            if separator:
                fields[key] = raw.strip('"')
        value["os_release"] = fields
    value["gpu"] = command_output(
        [
            "nvidia-smi",
            "--query-gpu=name,driver_version",
            "--format=csv,noheader",
        ]
    )
    value["windows_video_controllers"] = (
        windows_gpu_metadata()
        if os.name == "nt"
        else {"adapters": [], "available": False, "truncated": False}
    )
    return value


def linux_processes() -> dict[int, dict[str, int]]:
    processes: dict[int, dict[str, int]] = {}
    for status in Path("/proc").glob("[0-9]*/status"):
        try:
            fields: dict[str, str] = {}
            for line in status.read_text(encoding="utf-8", errors="replace").splitlines():
                key, separator, value = line.partition(":")
                if separator:
                    fields[key] = value.strip()
            pid = int(status.parent.name)
            ppid = int(fields.get("PPid", "0"))
            rss_match = re.match(r"([0-9]+)", fields.get("VmRSS", "0"))
            processes[pid] = {"ppid": ppid, "rss_kib": int(rss_match.group(1)) if rss_match else 0}
        except (FileNotFoundError, PermissionError, ValueError):
            continue
    return processes


def windows_processes() -> dict[int, dict[str, int]]:
    script = (
        "Get-CimInstance Win32_Process | "
        "Select-Object ProcessId,ParentProcessId,WorkingSetSize | ConvertTo-Json -Compress"
    )
    result = subprocess.run(
        ["powershell", "-NoProfile", "-Command", script],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    rows = json.loads(result.stdout)
    if isinstance(rows, dict):
        rows = [rows]
    return {
        int(row["ProcessId"]): {
            "ppid": int(row["ParentProcessId"]),
            "rss_kib": int(row.get("WorkingSetSize") or 0) // 1024,
        }
        for row in rows
    }


def process_tree_rss(root_pid: int) -> dict[str, Any]:
    try:
        processes = windows_processes() if os.name == "nt" else linux_processes()
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError):
        return {"available": False, "pids": [], "rss_kib": None}
    descendants = []
    pending = [root_pid]
    while pending:
        parent = pending.pop()
        children = [pid for pid, info in processes.items() if info["ppid"] == parent]
        descendants.extend(children)
        pending.extend(children)
    return {
        "available": bool(descendants),
        "pids": sorted(descendants),
        "rss_kib": sum(processes[pid]["rss_kib"] for pid in descendants),
    }


def resolve_assets(browser_name: str, binary: Path | None, driver: Path | None) -> tuple[Path, Path]:
    try:
        from selenium.webdriver.common.selenium_manager import SeleniumManager
    except ImportError as error:
        raise RuntimeError("Selenium is required: python3 -m pip install --user selenium") from error

    if binary and not binary.is_file():
        raise RuntimeError(f"browser binary does not exist: {binary}")
    if driver and not driver.is_file():
        raise RuntimeError(f"driver binary does not exist: {driver}")
    if binary and driver:
        return binary.resolve(), driver.resolve()

    args = [
        "--browser",
        browser_name,
        "--browser-version",
        "stable",
        "--skip-driver-in-path",
    ]
    if binary:
        args.extend(["--browser-path", str(binary.resolve())])
    result = SeleniumManager().binary_paths(args)
    resolved_binary = binary or Path(result["browser_path"])
    resolved_driver = driver or Path(result["driver_path"])
    if not resolved_binary.is_file() or not resolved_driver.is_file():
        raise RuntimeError("Selenium Manager did not return usable browser and driver paths")
    return resolved_binary.resolve(), resolved_driver.resolve()


def launch_browser(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    driver_log: Path,
    trace_enabled: bool,
    browser_console_enabled: bool = False,
) -> Any:
    try:
        from selenium import webdriver
    except ImportError as error:
        raise RuntimeError("Selenium is required: python3 -m pip install --user selenium") from error

    driver_log.parent.mkdir(parents=True, exist_ok=True)
    if browser_name == "chrome":
        from selenium.webdriver.chrome.options import Options
        from selenium.webdriver.chrome.service import Service

        options = Options()
        options.binary_location = str(binary)
        options.add_argument("--disable-extensions")
        options.add_argument("--disable-default-apps")
        options.add_argument("--no-default-browser-check")
        options.add_argument("--no-first-run")
        log_types = {"browser": "ALL"}
        if trace_enabled:
            options.add_experimental_option(
                "perfLoggingPrefs",
                {
                    "enableNetwork": False,
                    "enablePage": False,
                    "traceCategories": (
                        "devtools.timeline,"
                        "disabled-by-default-devtools.timeline.frame,"
                        "v8.execute"
                    ),
                },
            )
            log_types["performance"] = "ALL"
        options.set_capability("goog:loggingPrefs", log_types)
        service = Service(str(driver_path), log_output=str(driver_log))
        browser = webdriver.Chrome(service=service, options=options)
        browser.execute_cdp_cmd("Performance.enable", {})
        return browser

    from selenium.webdriver.firefox.options import Options
    from selenium.webdriver.firefox.service import Service

    options = Options()
    options.binary_location = str(binary)
    options.set_preference("extensions.autoDisableScopes", 15)
    options.set_preference("extensions.enabledScopes", 0)
    options.set_preference("devtools.console.stdout.content", True)
    if browser_console_enabled:
        options.set_preference("devtools.chrome.enabled", True)
    service = Service(str(driver_path), log_output=str(driver_log))
    return webdriver.Firefox(service=service, options=options)


def profile_directory(capabilities: dict[str, Any]) -> str | None:
    chrome = capabilities.get("chrome")
    if isinstance(chrome, dict):
        value = chrome.get("userDataDir")
        return str(value) if value else None
    value = capabilities.get("moz:profile")
    return str(value) if value else None


def quit_browser_bounded(driver: Any, timeout_seconds: float = 15) -> dict[str, Any]:
    service_process = getattr(getattr(driver, "service", None), "process", None)
    service_pid = getattr(service_process, "pid", None)
    quit_error: list[Exception] = []

    def quit_driver() -> None:
        try:
            driver.quit()
        except Exception as error:
            quit_error.append(error)

    thread = threading.Thread(target=quit_driver, daemon=True)
    thread.start()
    thread.join(timeout_seconds)
    timed_out = thread.is_alive()
    forced_cleanup = False
    cleanup_error = None
    if timed_out or (service_process and service_process.poll() is None):
        forced_cleanup = True
        try:
            if os.name == "nt" and service_pid:
                subprocess.run(
                    ["taskkill", "/PID", str(service_pid), "/T", "/F"],
                    check=True,
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
            elif service_process:
                service_process.terminate()
                try:
                    service_process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    service_process.kill()
                    service_process.wait(timeout=5)
        except Exception as error:
            cleanup_error = str(error)
    thread.join(5)
    service_stopped = not service_process or service_process.poll() is not None
    return {
        "error": str(quit_error[0]) if quit_error else cleanup_error,
        "forced_cleanup": forced_cleanup,
        "pass": (
            not timed_out
            and not quit_error
            and cleanup_error is None
            and not forced_cleanup
            and service_stopped
        ),
        "service_pid": service_pid,
        "service_stopped": service_stopped,
        "timed_out": timed_out,
        "timeout_seconds": timeout_seconds,
    }


def set_viewport(driver: Any, width: int, height: int) -> dict[str, Any]:
    for _attempt in range(5):
        dimensions = driver.execute_script(
            "return {"
            "innerWidth: window.innerWidth, innerHeight: window.innerHeight, "
            "outerWidth: window.outerWidth, outerHeight: window.outerHeight"
            "};"
        )
        width_delta = width - int(dimensions["innerWidth"])
        height_delta = height - int(dimensions["innerHeight"])
        if width_delta == 0 and height_delta == 0:
            return dimensions
        rect = driver.get_window_rect()
        driver.set_window_rect(
            width=max(320, int(rect["width"]) + width_delta),
            height=max(240, int(rect["height"]) + height_delta),
        )
    dimensions = driver.execute_script(
        "return {innerWidth: window.innerWidth, innerHeight: window.innerHeight};"
    )
    if dimensions != {"innerWidth": width, "innerHeight": height}:
        raise RuntimeError(f"could not establish exact {width}x{height} viewport: {dimensions}")
    return dimensions


PAGE_STATE = """
const compat = window.__GOLISEO__ || {};
const canvas = document.getElementById("canvas");
let renderer = null;
let vendor = null;
if (canvas) {
  const gl = canvas.getContext("webgl") || canvas.getContext("experimental-webgl");
  if (gl) {
    const debug = gl.getExtension("WEBGL_debug_renderer_info");
    renderer = debug ? gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    vendor = debug ? gl.getParameter(debug.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR);
  }
}
return {
  build_id: compat.build_id || null,
  canvas: canvas ? {
    height: canvas.height,
    rect: {
      height: canvas.getBoundingClientRect().height,
      width: canvas.getBoundingClientRect().width,
      x: canvas.getBoundingClientRect().x,
      y: canvas.getBoundingClientRect().y
    },
    width: canvas.width
  } : null,
  console_entries: compat.console_entries || [],
  events: compat.events || [],
  fullscreen: Boolean(document.fullscreenElement),
  gamepads: Array.from(navigator.getGamepads ? navigator.getGamepads() : [])
    .filter(Boolean)
    .map((pad) => ({connected: pad.connected, id: pad.id, index: pad.index, mapping: pad.mapping})),
  js_heap_bytes: performance.memory ? performance.memory.usedJSHeapSize : null,
  renderer,
  storage: compat.storage || null,
  status: compat.status || "missing",
  user_activation: navigator.userActivation ? {
    active: navigator.userActivation.isActive,
    has_been_active: navigator.userActivation.hasBeenActive
  } : null,
  vendor,
  viewport: {
    device_pixel_ratio: window.devicePixelRatio,
    height: window.innerHeight,
    width: window.innerWidth
  }
};
"""

FIREFOX_FLOW_WAIT = """
const marker = arguments[0];
const timeoutMs = arguments[1];
const done = arguments[arguments.length - 1];
const deadline = performance.now() + timeoutMs;
let cursor = 0;
function inspect() {
  const compat = window.__GOLISEO__ || {};
  const status = compat.status || "missing";
  const entries = compat.console_entries || [];
  while (cursor < entries.length) {
    const message = String(entries[cursor].message || "");
    cursor += 1;
    if (message.includes("GC_BROWSER|error|")) {
      done({error: message, matched: false, status});
      return;
    }
    if (message.includes(marker)) {
      done({error: null, matched: true, status});
      return;
    }
  }
  if (status !== "running" || performance.now() >= deadline) {
    done({error: null, matched: false, status});
    return;
  }
  window.setTimeout(inspect, 100);
}
inspect();
"""


def page_state(driver: Any) -> dict[str, Any]:
    value = driver.execute_script(PAGE_STATE)
    if not isinstance(value, dict):
        raise RuntimeError("browser did not return page state")
    return value


def wait_for_firefox_flow_marker(driver: Any, marker: str, timeout: float) -> bool:
    """Wait in Firefox without repeatedly synchronizing WebDriver and the renderer."""
    if timeout <= 0:
        return False
    driver.set_script_timeout(timeout + 5)
    value = driver.execute_async_script(FIREFOX_FLOW_WAIT, marker, timeout * 1000)
    if not isinstance(value, dict):
        raise RuntimeError("Firefox did not return flow-wait state")
    if value.get("error"):
        raise RuntimeError(f"browser runtime reported an error: {value['error']}")
    if value.get("status") != "running":
        raise RuntimeError(f"browser runtime changed to {value.get('status')}")
    return value.get("matched") is True


def wait_for_firefox_flow_until(
    driver: Any,
    marker: str,
    deadline: float,
) -> tuple[bool, int]:
    waits = 0
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return False, waits
        waits += 1
        if wait_for_firefox_flow_marker(
            driver,
            marker,
            min(remaining, FIREFOX_FLOW_WAIT_CHUNK_SECONDS),
        ):
            return True, waits


def firefox_flow_actions(
    started: float,
    stability_started: float,
    stability_seconds: int,
) -> list[tuple[float, str, str]]:
    actions = [(started + 11, "keyboard", "m")]
    if stability_seconds:
        actions.extend(
            (
                (stability_started, "memory", "t0"),
                (stability_started + 300, "memory", "t5"),
                (stability_started + 600, "memory", "t10"),
                (
                    stability_started + stability_seconds - 20,
                    "keyboard",
                    "m",
                ),
            )
        )
    return sorted(actions)


def chrome_post_flow_actions(
    keyboard_probe_at: float,
    liveness_probe_at: float | None,
    memory_schedule: list[tuple[float, str]],
    memory_index: int,
    trace_at: float | None,
) -> list[tuple[float, str, str]]:
    actions = [
        (at, "memory", label) for at, label in memory_schedule[memory_index:]
    ]
    if keyboard_probe_at != float("inf"):
        actions.append((keyboard_probe_at, "keyboard", "m"))
    if liveness_probe_at is not None:
        actions.append((liveness_probe_at, "keyboard", "m"))
    if trace_at is not None:
        actions.append((trace_at, "trace", "performance"))
    return sorted(actions)


def sleep_until(deadline: float) -> None:
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return
        time.sleep(min(1, remaining))


def browser_console(driver: Any, phase: str) -> list[dict[str, Any]]:
    state = page_state(driver)
    result = []
    for entry in state.get("console_entries", []):
        if not isinstance(entry, dict):
            continue
        result.append(
            {
                "at_ms": entry.get("at_ms"),
                "level": entry.get("level", "log"),
                "message": str(entry.get("message", "")),
                "phase": phase,
            }
        )
    return result


def webdriver_console(
    driver: Any,
    phase: str,
    include_markers: bool = False,
) -> list[dict[str, Any]]:
    if driver.capabilities.get("browserName") != "chrome":
        return []
    result = []
    for entry in driver.get_log("browser"):
        raw_message = str(entry.get("message", ""))
        marker = marker_message(raw_message)
        if marker and not include_markers:
            continue
        level = str(entry.get("level", "INFO")).lower()
        result.append(
            {
                "level": "error" if level == "severe" else level,
                "message": marker or raw_message,
                "phase": phase,
                "timestamp": entry.get("timestamp"),
            }
        )
    return result


def flow_console(
    driver: Any,
    browser_name: str,
    phase: str,
    chrome_entries: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    if browser_name == "chrome":
        # ChromeDriver already receives every compatibility marker. Keep the
        # long stability loop out of the page isolate so its own polling does
        # not inflate the post-GC JavaScript heap under measurement.
        chrome_entries.extend(webdriver_console(driver, phase, include_markers=True))
        return chrome_entries
    return browser_console(driver, phase)


def final_flow_capture(
    driver: Any,
    browser_name: str,
    chrome_entries: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    entries = flow_console(driver, browser_name, "flow", chrome_entries)
    final_state = page_state(driver)
    if browser_name == "chrome":
        # PAGE_STATE can itself cause a final console entry. Drain after that
        # diagnostic so the evidence retains everything through final capture.
        entries = flow_console(driver, browser_name, "flow_final", chrome_entries)
    return entries, final_state


def failure_diagnostics(
    driver: Any,
    browser_name: str,
    chrome_entries: list[dict[str, Any]],
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    state: dict[str, Any] = {}
    entries = list(chrome_entries) if browser_name == "chrome" else []
    try:
        state = page_state(driver)
    except Exception:
        pass
    if browser_name == "chrome":
        try:
            entries.extend(webdriver_console(driver, "failure", include_markers=True))
        except Exception:
            pass
    elif state:
        for entry in state.get("console_entries", []):
            if not isinstance(entry, dict):
                continue
            entries.append(
                {
                    "at_ms": entry.get("at_ms"),
                    "level": entry.get("level", "log"),
                    "message": str(entry.get("message", "")),
                    "phase": "failure",
                }
            )
    return state, entries


def service_console(path: Path, runtime_bytes: int | None = None) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    result = []
    data = path.read_bytes()
    boundary = len(data)
    if runtime_bytes is not None:
        boundary = data.rfind(b"\n", 0, min(runtime_bytes, len(data))) + 1
    segments = (
        ((data[:boundary], "webdriver_service_runtime"), (data[boundary:], "webdriver_teardown"))
        if runtime_bytes is not None
        else ((data, "webdriver_service_runtime"),)
    )
    for raw, phase in segments:
        text = raw.decode("utf-8", errors="replace").replace("\0", "")
        for line in text.splitlines():
            if marker_message(line):
                continue
            lowered = line.lower()
            level = None
            if (
                "console.error:" in lowered
                or "javascript error:" in lowered
                or "fatal error:" in lowered
                or re.search(r"\b(error|fatal)\b", lowered)
            ):
                level = "error"
            elif (
                "console.warn:" in lowered
                or "javascript warning:" in lowered
                or re.search(r"\bwarn(?:ing)?\b", lowered)
            ):
                level = "warning"
            if level:
                result.append(
                    {
                        "level": level,
                        "message": line,
                        "phase": phase,
                    }
                )
    return result


def route_sequence(entries: list[dict[str, Any]]) -> list[str]:
    return [
        record["route"]
        for record in records(entries)
        if record.get("kind") == "route" and "route" in record
    ]


def normalized_routes(routes: list[str]) -> list[str]:
    result = []
    for route in routes:
        if route == "pause" or (result and result[-1] == route):
            continue
        result.append(route)
    return result


def settings_values(entries: list[dict[str, Any]], key: str) -> list[str]:
    return [
        record[key]
        for record in records(entries)
        if record.get("kind") == "settings" and key in record
    ]


def wait_for_route(driver: Any, expected: str, timeout: float = 10) -> None:
    wait_until(
        lambda: expected in route_sequence(browser_console(driver, "wait")),
        timeout,
        f"route {expected}",
    )


def wait_for_setting(driver: Any, key: str, expected: str, timeout: float = 10) -> None:
    wait_until(
        lambda: expected in settings_values(browser_console(driver, "wait"), key),
        timeout,
        f"{key}={expected}",
    )


def wait_for_storage_state(driver: Any, expected: str, timeout: float = 10) -> dict[str, Any]:
    return wait_until(
        lambda: (
            page_state(driver).get("storage")
            if (page_state(driver).get("storage") or {}).get("state") == expected
            else None
        ),
        timeout,
        f"browser storage state {expected}",
    )


def send_key(driver: Any, value: str) -> None:
    from selenium.webdriver.common.action_chains import ActionChains
    from selenium.webdriver.common.by import By

    driver.find_element(By.ID, "canvas").click()
    ActionChains(driver).send_keys(value).perform()


def click_canvas_gesture(driver: Any) -> None:
    from selenium.webdriver.common.action_chains import ActionChains
    from selenium.webdriver.common.by import By

    canvas = driver.find_element(By.ID, "canvas")
    rect = canvas.rect
    ActionChains(driver).move_to_element(canvas).move_by_offset(
        -int(rect["width"] / 2) + 2,
        -int(rect["height"] / 2) + 2,
    ).click().perform()


def performance_metric(metrics: dict[str, Any] | None, name: str) -> int | float | None:
    if not metrics:
        return None
    for metric in metrics.get("metrics", []):
        if metric.get("name") == name:
            return metric.get("value")
    return None


def values_within_percent(
    first: int | float | None,
    second: int | float | None,
    tolerance_percent: int | float,
) -> bool:
    if first is None or second is None or first <= 0 or second <= 0:
        return False
    return abs(first - second) * 100 / max(first, second) <= tolerance_percent


def canvas_geometry(driver: Any, width: int, height: int) -> dict[str, Any]:
    set_viewport(driver, width, height)
    time.sleep(0.25)
    canvas = page_state(driver).get("canvas") or {}
    actual = canvas.get("rect") or {}
    expected = expected_canvas_rect(width, height)
    return {
        "actual": actual,
        "canvas_size": {
            "height": canvas.get("height"),
            "width": canvas.get("width"),
        },
        "expected": expected,
        "pass": (
            canvas.get("width") == CANVAS_WIDTH
            and canvas.get("height") == CANVAS_HEIGHT
            and rect_matches(actual, expected)
        ),
        "probe_viewport": {"height": height, "width": width},
    }


def pointer_input_count(driver: Any) -> int:
    return sum(
        1
        for record in records(browser_console(driver, "pointer_probe"))
        if record.get("kind") == "input" and record.get("event_kind") == "mouse_1"
    )


def probe_pointer_alignment(driver: Any, geometry: dict[str, Any]) -> dict[str, Any]:
    from selenium.webdriver.common.action_chains import ActionChains
    from selenium.webdriver.common.by import By
    from selenium.webdriver.common.keys import Keys

    rect = geometry["actual"]
    target = logical_to_client(rect, POINTER_TARGET["x"], POINTER_TARGET["y"])
    offset_control = pointer_offset_control(rect, POINTER_TARGET["x"], POINTER_TARGET["y"])
    routes_before = route_sequence(browser_console(driver, "pointer_probe"))
    inputs_before = pointer_input_count(driver)
    canvas = driver.find_element(By.ID, "canvas")
    center_x = float(rect["x"]) + float(rect["width"]) / 2
    center_y = float(rect["y"]) + float(rect["height"]) / 2
    ActionChains(driver).move_to_element(canvas).move_by_offset(
        round(target["x"] - center_x),
        round(target["y"] - center_y),
    ).click().perform()

    def reached_expected_route() -> bool:
        routes = route_sequence(browser_console(driver, "pointer_probe"))
        return len(routes) > len(routes_before) and routes[-1] == POINTER_TARGET["route"]

    try:
        wait_until(reached_expected_route, 5, "letterboxed pointer target")
    except RuntimeError:
        pass
    routes_after = route_sequence(browser_console(driver, "pointer_probe"))
    inputs_after = pointer_input_count(driver)
    actual_route = routes_after[-1] if len(routes_after) > len(routes_before) else None
    result = {
        "actual_route": actual_route,
        "expected_route": POINTER_TARGET["route"],
        "input_observed": inputs_after > inputs_before,
        "logical": {"x": POINTER_TARGET["x"], "y": POINTER_TARGET["y"]},
        "offset_omission_control": offset_control,
        "pass": (
            actual_route == POINTER_TARGET["route"]
            and inputs_after > inputs_before
            and offset_control["pass"]
        ),
        "target_client": target,
    }

    if actual_route and actual_route != "title":
        route_count = len(routes_after)
        ActionChains(driver).send_keys(Keys.ESCAPE).perform()

        def returned_to_title() -> bool:
            routes = route_sequence(browser_console(driver, "pointer_probe"))
            return len(routes) > route_count and routes[-1] == "title"

        wait_until(returned_to_title, 5, "Title after pointer probe")
    return result


def collect_memory(driver: Any, browser_name: str, service_pid: int, label: str) -> dict[str, Any]:
    driver.execute_script(
        "console.info('GC_BROWSER|memory_probe|phase=start|label=' + arguments[0]);",
        label,
    )
    gc_method = "unavailable"
    performance_metrics = None
    runtime_heap_usage = None
    if browser_name == "chrome":
        try:
            driver.execute_cdp_cmd("HeapProfiler.collectGarbage", {})
            gc_method = "cdp HeapProfiler.collectGarbage"
        except Exception:
            gc_method = "unavailable"
        try:
            performance_metrics = driver.execute_cdp_cmd("Performance.getMetrics", {})
        except Exception:
            performance_metrics = None
        try:
            runtime_heap_usage = driver.execute_cdp_cmd("Runtime.getHeapUsage", {})
        except Exception:
            runtime_heap_usage = None
    else:
        try:
            collected = driver.execute_script(
                "if (window.windowUtils && window.windowUtils.garbageCollect) {"
                "window.windowUtils.garbageCollect(); return true; } return false;"
            )
            if collected:
                gc_method = "window.windowUtils.garbageCollect"
        except Exception:
            gc_method = "unavailable"
        time.sleep(0.25)

    # For Chrome, both CDP heap reads above are deliberately adjacent to the
    # forced GC. PAGE_STATE serializes page-side diagnostics and must run only
    # after the authoritative Performance metric and Runtime cross-check.
    state = page_state(driver)
    page_js_heap = state.get("js_heap_bytes")
    performance_js_heap = performance_metric(performance_metrics, "JSHeapUsedSize")
    runtime_js_heap = (
        runtime_heap_usage.get("usedSize") if isinstance(runtime_heap_usage, dict) else None
    )
    js_heap = performance_js_heap if performance_js_heap is not None else page_js_heap
    result = {
        "captured_at": utc_now(),
        "forced_gc": gc_method,
        "heap_crosscheck": {
            "pass": (
                values_within_percent(
                    performance_js_heap,
                    runtime_js_heap,
                    HEAP_CROSSCHECK_TOLERANCE_PERCENT,
                )
                if browser_name == "chrome"
                else None
            ),
            "tolerance_percent": HEAP_CROSSCHECK_TOLERANCE_PERCENT,
        },
        "js_heap_bytes": js_heap,
        "js_heap_source": (
            "cdp Performance.getMetrics.JSHeapUsedSize"
            if performance_js_heap is not None
            else "page performance.memory.usedJSHeapSize"
        ),
        "label": label,
        "page_js_heap_bytes": page_js_heap,
        "performance_metrics": performance_metrics,
        "process_tree": process_tree_rss(service_pid),
        "runtime_heap_usage": runtime_heap_usage,
        "sampling_order": (
            [
                "cdp HeapProfiler.collectGarbage",
                "cdp Performance.getMetrics.JSHeapUsedSize",
                "cdp Runtime.getHeapUsage.usedSize",
                "page PAGE_STATE",
                "process-tree RSS",
            ]
            if browser_name == "chrome"
            else [
                "forced garbage collection",
                "page PAGE_STATE",
                "process-tree RSS",
            ]
        ),
        "target_counts": {
            "documents": performance_metric(performance_metrics, "Documents"),
            "frames": performance_metric(performance_metrics, "Frames"),
        },
    }
    driver.execute_script(
        "console.info('GC_BROWSER|memory_probe|phase=end|label=' + arguments[0]);",
        label,
    )
    return result


def growth_percent(first: int | float | None, last: int | float | None) -> float | None:
    if first is None or last is None or first <= 0:
        return None
    return (last - first) * 100 / first


def sample_gate(entries: list[dict[str, Any]], flow_only: bool = True) -> dict[str, Any]:
    parsed = records(entries)
    current_sample = None
    probe_samples = set()
    for record in parsed:
        if record.get("kind") == "sample_start":
            current_sample = int(record.get("sample", "0"))
        elif record.get("kind") == "memory_probe" and current_sample:
            probe_samples.add(current_sample)
    flow_completions = [
        float(record["at_ms"])
        for record in parsed
        if record.get("kind") == "flow_complete" and "at_ms" in record
    ]
    last_complete_sample = (
        max(0, int((flow_completions[-1] / 1000 - 10) // 60))
        if flow_completions
        else 0
    )
    samples = [
        record
        for record in parsed
        if record.get("kind") == "sample"
        and record.get("partial") == "false"
        and float(record.get("duration_s", "0")) >= 59
        and int(record.get("sample", "0")) not in probe_samples
        and (not flow_only or int(record.get("sample", "0")) <= last_complete_sample)
    ]
    failures = []
    for index, sample in enumerate(samples, 1):
        limits = (
            ("update_p95_ms", 8),
            ("update_max_ms", 33),
            ("draw_p95_ms", 8),
            ("draw_max_ms", 33),
        )
        for key, limit in limits:
            value = float(sample.get(key, "inf"))
            if value > limit:
                failures.append(f"sample {index} {key}={value:.3f}>{limit}")
        if int(sample.get("frame_over_33_ms", "999")) >= 3:
            failures.append(f"sample {index} has >=3 frames over 33 ms")
        if int(sample.get("frame_over_250_ms", "999")) > 0:
            failures.append(f"sample {index} has a frame over 250 ms")
    input_latencies = [
        float(record["latency_ms"])
        for record in parsed
        if record.get("kind") == "input_latency" and "latency_ms" in record
    ]
    if input_latencies:
        ordered = sorted(input_latencies)
        p95 = ordered[max(0, (len(ordered) * 95 + 99) // 100 - 1)]
        if p95 > 100:
            failures.append(f"input latency p95={p95:.3f}>100 ms")
    return {
        "complete_samples": len(samples),
        "excluded_probe_samples": sorted(probe_samples),
        "failures": failures,
        "input_samples": len(input_latencies),
        "scope": "flow" if flow_only else "full_run",
        "pass": bool(samples) and bool(input_latencies) and not failures,
    }


def console_gate(
    entries: list[dict[str, Any]],
    classifications: list[re.Pattern[str]],
) -> dict[str, Any]:
    warnings = [
        entry
        for entry in entries
        if entry.get("level") in {"warn", "warning", "error"}
        and not str(entry.get("message", "")).startswith(("GC_BROWSER|", "GC_METRICS|"))
    ]
    classified = []
    unclassified = []
    for entry in warnings:
        message = str(entry.get("message", ""))
        hard_runtime_error = bool(HARD_RUNTIME_PATTERN.search(message))
        rule = (
            None
            if hard_runtime_error
            else next(
                (pattern.pattern for pattern in classifications if pattern.search(message)),
                None,
            )
        )
        if rule:
            classified.append({**entry, "classification": rule})
        else:
            unclassified.append(entry)
    return {
        "classified": classified,
        "pass": not unclassified,
        "unclassified": unclassified,
    }


def terminal_runtime_health(
    entries: list[dict[str, Any]],
    final_state: dict[str, Any],
) -> dict[str, Any]:
    error_markers = []
    for entry in entries:
        message = entry.get("message")
        if not isinstance(message, str):
            continue
        marker = parse_marker(message)
        if not marker or marker.get("source") != "GC_BROWSER" or marker.get("kind") != "error":
            continue
        error_markers.append(
            {
                "entry": {
                    key: entry.get(key)
                    for key in ("at_ms", "level", "message", "phase", "timestamp")
                    if entry.get(key) is not None
                },
                "marker": marker,
            }
        )
    status = final_state.get("status")
    return {
        "error_markers": error_markers,
        "pass": status == "running" and not error_markers,
        "status": status,
    }


def audio_probe_check(
    entries: list[dict[str, Any]],
    final_state: dict[str, Any],
) -> dict[str, Any]:
    match_times: list[float] = []
    observations: list[tuple[float, int, float, dict[str, str]]] = []
    invalid_observations: list[dict[str, Any]] = []
    settings_observations: list[tuple[float, dict[str, str]]] = []
    autoplay_warnings = 0
    for entry in entries:
        if "AudioContext was not allowed to start" in str(entry.get("message", "")):
            autoplay_warnings += 1
    for index, entry in enumerate(entries):
        message = entry.get("message")
        record = parse_marker(message) if isinstance(message, str) else None
        if not record:
            continue
        try:
            at_ms = float(record.get("at_ms", ""))
        except (TypeError, ValueError):
            if record.get("kind") == "audio":
                invalid_observations.append(
                    {"index": index, "reason": "invalid at_ms", "record": record}
                )
            continue
        if not math.isfinite(at_ms):
            if record.get("kind") == "audio":
                invalid_observations.append(
                    {"index": index, "reason": "non-finite at_ms", "record": record}
                )
            continue
        if record.get("kind") == "route" and record.get("route") == "match":
            match_times.append(at_ms)
        elif record.get("kind") == "audio":
            try:
                active_sources = int(record["active_sources"])
                volume = float(record["volume"])
                if active_sources < 0 or not math.isfinite(volume) or volume < 0:
                    raise ValueError
            except (KeyError, TypeError, ValueError):
                invalid_observations.append(
                    {
                        "index": index,
                        "reason": "invalid active_sources or volume",
                        "record": record,
                    }
                )
                continue
            observations.append((at_ms, active_sources, volume, record))
        elif record.get("kind") == "settings" and str(entry.get("phase", "")).startswith(
            ("flow", "match_focus")
        ):
            settings_observations.append(
                (at_ms, {**record, "phase": str(entry.get("phase"))})
            )
    match_at_ms = match_times[0] if match_times else None
    offsets_ms = (
        [at_ms - match_at_ms for at_ms, _, _, _ in observations]
        if match_at_ms is not None
        else []
    )
    ordered = all(
        later > earlier for earlier, later in zip(offsets_ms, offsets_ms[1:])
    )
    bounded = (
        len(observations) == AUDIO_PROBE_MAX_OBSERVATIONS
        and not invalid_observations
        and all(
            -100 <= offset <= (AUDIO_PROBE_SECONDS + AUDIO_PROBE_WAIT_SLACK_SECONDS) * 1000
            for offset in offsets_ms
        )
    )
    complete = (
        len(offsets_ms) == AUDIO_PROBE_MAX_OBSERVATIONS
        and not invalid_observations
        and ordered
        and offsets_ms[0] <= (AUDIO_PROBE_INTERVAL_SECONDS + 0.25) * 1000
        and offsets_ms[-1] >= (AUDIO_PROBE_SECONDS - AUDIO_PROBE_INTERVAL_SECONDS) * 1000
    )
    positive = any(
        active_sources > 0 and volume > 0
        for _, active_sources, volume, _ in observations
    )
    settings_before_match = (
        [
            record
            for at_ms, record in settings_observations
            if match_at_ms is not None and at_ms <= match_at_ms
        ][-1]
        if match_at_ms is not None
        and any(at_ms <= match_at_ms for at_ms, _ in settings_observations)
        else None
    )
    audible_settings = (
        settings_before_match is not None
        and settings_before_match.get("muted") == "false"
        and bool(observations)
        and all(volume > 0 for _, _, volume, _ in observations)
    )
    user_activated = bool(final_state.get("user_activation", {}).get("has_been_active"))
    return {
        "audible_settings": {
            "pass": audible_settings,
            "settings_before_match": settings_before_match,
            "volumes": [volume for _, _, volume, _ in observations],
        },
        "autoplay_warnings": autoplay_warnings,
        "bounded": bounded,
        "complete": complete,
        "duration_seconds": AUDIO_PROBE_SECONDS,
        "invalid_observations": invalid_observations,
        "match_at_ms": match_at_ms,
        "max_observations": AUDIO_PROBE_MAX_OBSERVATIONS,
        "observation_offsets_ms": offsets_ms,
        "observations": [record for _, _, _, record in observations],
        "ordered": ordered,
        "pass": (
            user_activated
            and match_at_ms is not None
            and audible_settings
            and bounded
            and complete
            and positive
            and autoplay_warnings == 0
        ),
        "positive_source_observed": positive,
        "user_activation": user_activated,
    }


def evidence_checks(
    browser_name: str,
    entries: list[dict[str, Any]],
    final_state: dict[str, Any],
    preflight: dict[str, Any],
    memories: list[dict[str, Any]],
    stability_seconds: int,
    warning_classifications: list[re.Pattern[str]],
) -> dict[str, Any]:
    parsed = records(entries)
    routes = route_sequence(entries)
    lifecycle = [
        record.get("event") for record in parsed if record.get("kind") == "lifecycle"
    ]
    input_kinds = [
        record.get("event_kind") for record in parsed if record.get("kind") == "input"
    ]
    renderer = str(final_state.get("renderer") or "")
    software = any(name in renderer.lower() for name in SOFTWARE_RENDERERS)
    first_memory = memories[0] if memories else {}
    last_memory = memories[-1] if memories else {}
    first_rss = first_memory.get("process_tree", {}).get("rss_kib")
    last_rss = last_memory.get("process_tree", {}).get("rss_kib")
    rss_growth = growth_percent(first_rss, last_rss)
    heap_growth = growth_percent(first_memory.get("js_heap_bytes"), last_memory.get("js_heap_bytes"))
    late_input = any(
        record.get("kind") == "input"
        and record.get("event_kind") == "key_m"
        and float(record.get("at_ms", "0")) >= max(0, stability_seconds - 30) * 1000
        for record in parsed
    )
    late_setting = any(
        record.get("kind") == "settings"
        and float(record.get("at_ms", "0")) >= max(0, stability_seconds - 30) * 1000
        for record in parsed
    )
    gamepad_inputs = {
        record.get("event_kind")
        for record in parsed
        if record.get("kind") == "input"
        and str(record.get("event_kind", "")).startswith("gamepad_")
    }
    standard_gamepad = any(
        gamepad.get("connected") and gamepad.get("mapping") == "standard"
        for gamepad in final_state.get("gamepads", [])
    )
    post_gc_samples = len(memories) == 3 and all(
        memory.get("forced_gc") != "unavailable" for memory in memories
    )
    heap_crosschecks = [
        memory.get("heap_crosscheck", {}).get("pass") for memory in memories
    ]
    heap_crosscheck_pass = (
        all(value is True for value in heap_crosschecks)
        if any(value is not None for value in heap_crosschecks)
        else None
    )
    document_counts = [
        memory.get("target_counts", {}).get("documents") for memory in memories
    ]
    frame_counts = [memory.get("target_counts", {}).get("frames") for memory in memories]
    one_document_baseline = (
        len(document_counts) == 3 and all(count == 1 for count in document_counts)
        if browser_name == "chrome"
        else None
    )
    acceptance_isolation = preflight.get("acceptance_isolation", {})
    isolated_acceptance = (
        acceptance_isolation.get("dedicated_flow_browser") is True
        and acceptance_isolation.get("distinct_profile") is True
        if browser_name == "chrome" and stability_seconds
        else None
    )
    memory_available = (
        len(memories) == 3
        and rss_growth is not None
        and heap_growth is not None
        and post_gc_samples
        and heap_crosscheck_pass is not False
        and one_document_baseline is not False
    )
    stability_performance = (
        sample_gate(entries, flow_only=False) if stability_seconds else None
    )
    runtime_console = [
        entry for entry in entries if entry.get("phase") != "webdriver_teardown"
    ]
    teardown_console = [
        entry for entry in entries if entry.get("phase") == "webdriver_teardown"
    ]
    return {
        "acceptance_isolation": {
            **acceptance_isolation,
            "one_document_baseline": one_document_baseline,
            "pass": (
                None
                if not stability_seconds
                else (
                    isolated_acceptance is True and one_document_baseline is True
                    if browser_name == "chrome"
                    else True
                )
            ),
        },
        "audio_after_gesture": audio_probe_check(entries, final_state),
        "artifact_boot": {
            "navigation_to_title_ms": preflight.get("navigation_to_title_ms"),
            "pass": preflight.get("title_state", {}).get("status") == "running",
            "soft_target_pass": (
                preflight.get("navigation_to_title_ms") is not None
                and float(preflight["navigation_to_title_ms"]) <= 2000
            ),
        },
        "browser_teardown": console_gate(teardown_console, warning_classifications),
        "clean_console": console_gate(runtime_console, warning_classifications),
        "complete_flow": {
            "pass": normalized_routes(routes)[-len(EXPECTED_FLOW) :] == EXPECTED_FLOW,
            "routes": routes,
        },
        "gamepad": {
            "devices": final_state.get("gamepads", []),
            "observed_inputs": sorted(value for value in gamepad_inputs if value),
            "pass": (
                standard_gamepad
                and "gamepad_a" in gamepad_inputs
                and "gamepad_b" in gamepad_inputs
            ),
        },
        "hardware_acceleration": {
            "pass": bool(renderer) and not software,
            "renderer": renderer or None,
            "vendor": final_state.get("vendor"),
        },
        "fullscreen": {
            "pass": bool(preflight.get("fullscreen_observed")),
        },
        "keyboard": {
            "observed_metric_kinds": input_kinds,
            "pass": all(
                any(
                    record.get("kind") == "input" and record.get("event_kind") == expected
                    for record in parsed
                )
                for expected in ("key_return", "key_escape")
            ),
        },
        "lifecycle": {
            "events": lifecycle,
            "pass": all(expected in lifecycle for expected in ("blur", "focus", "resize")),
        },
        "letterboxing": preflight.get(
            "letterboxing",
            {"pass": False, "reason": "missing letterbox observation"},
        ),
        "memory": {
            "available": memory_available,
            "document_counts": document_counts,
            "frame_counts": frame_counts,
            "heap_crosscheck_pass": heap_crosscheck_pass,
            "heap_growth_percent": heap_growth,
            "heap_source": first_memory.get("js_heap_source"),
            "one_document_baseline": one_document_baseline,
            "pass": (
                None
                if not memory_available
                else (
                    rss_growth <= 25
                    and heap_growth <= 25
                )
            ),
            "post_gc_samples": post_gc_samples,
            "rss_growth_percent": rss_growth,
            "samples": len(memories),
        },
        "performance": sample_gate(entries),
        "stability_performance": stability_performance,
        "persistence": {
            "flush_after_restore": preflight.get("flush_after_restore"),
            "flush_after_save": preflight.get("flush_after_save"),
            "muted_after_reload": preflight.get("muted_after_reload"),
            "muted_after_restore": preflight.get("muted_after_restore"),
            "pass": (
                preflight.get("flush_after_save") is True
                and preflight.get("muted_after_reload") == "true"
                and preflight.get("flush_after_restore") is True
                and preflight.get("muted_after_restore") == "false"
                and preflight.get("storage_after_reload", {}).get("state") == "ready"
                and preflight.get("storage_after_reload", {}).get("populate_count", 0) >= 1
            ),
            "storage_after_reload": preflight.get("storage_after_reload"),
        },
        "storage_unavailable": {
            "pass": preflight.get("storage_unavailable_boot_recoverable") is True,
            "state": preflight.get("storage_unavailable_state"),
        },
        "terminal_runtime_health": terminal_runtime_health(runtime_console, final_state),
        "runtime_stability": {
            "late_input": late_input,
            "late_settings_change": late_setting,
            "match_focus_recovery": preflight.get("match_focus_recovery"),
            "pass": (
                None
                if stability_seconds == 0
                else (
                    final_state.get("status") == "running"
                    and routes[-1:] == ["result"]
                    and late_input
                    and late_setting
                    and preflight.get("match_focus_recovery") is True
                )
            ),
            "requested_seconds": stability_seconds,
        },
    }


def run_preflight(driver: Any, url: str, width: int, height: int, output: Path) -> dict[str, Any]:
    from selenium.webdriver.common.keys import Keys
    from selenium.webdriver.common.by import By

    navigation_started = time.monotonic()
    driver.get(url)
    wait_until(lambda: page_state(driver).get("status") == "running", 30, "browser runtime")
    wait_for_storage_state(driver, "ready")
    set_viewport(driver, width, height)
    wait_for_route(driver, "title")
    title_state = page_state(driver)
    requested_geometry = canvas_geometry(driver, width, height)
    driver.save_screenshot(str(output / "title.png"))
    navigation_to_title_ms = (time.monotonic() - navigation_started) * 1000
    driver.find_element(By.ID, "canvas").click()
    send_key(driver, Keys.RETURN)
    wait_for_route(driver, "squad")
    send_key(driver, Keys.ESCAPE)
    wait_for_route(driver, "title")
    flush_count = int((page_state(driver).get("storage") or {}).get("flush_count") or 0)
    send_key(driver, "m")
    wait_for_setting(driver, "muted", "true")
    storage_after_save = wait_until(
        lambda: (
            page_state(driver).get("storage")
            if int((page_state(driver).get("storage") or {}).get("flush_count") or 0)
            > flush_count
            else None
        ),
        10,
        "settings flush",
    )
    flush_after_save = int(storage_after_save.get("flush_count") or 0) > flush_count

    probes = {"requested": requested_geometry}
    pointer_probes = {}
    for name, (probe_width, probe_height) in LETTERBOX_PROBES.items():
        geometry = canvas_geometry(driver, probe_width, probe_height)
        pointer = probe_pointer_alignment(driver, geometry)
        geometry["pointer_hit_testing"] = pointer
        probes[name] = geometry
        pointer_probes[name] = pointer
    tall = probes["tall"]
    letterboxing = {
        "actual": tall["actual"],
        "expected": tall["expected"],
        "pass": (
            all(probe["pass"] for probe in probes.values())
            and all(probe["pass"] for probe in pointer_probes.values())
        ),
        "pointer_hit_testing": {
            "pass": all(probe["pass"] for probe in pointer_probes.values()),
            "probes": pointer_probes,
        },
        "probe_viewport": tall["probe_viewport"],
        "probes": probes,
    }
    set_viewport(driver, width, height)
    time.sleep(0.25)

    fullscreen_observed = False
    try:
        send_key(driver, Keys.F11)
        wait_for_setting(driver, "fullscreen", "true")
        wait_until(lambda: page_state(driver).get("fullscreen") is True, 10, "fullscreen entry")
        send_key(driver, Keys.F11)
        wait_for_setting(driver, "fullscreen", "false")
        wait_until(lambda: page_state(driver).get("fullscreen") is False, 10, "fullscreen exit")
        fullscreen_observed = True
    except Exception:
        fullscreen_observed = False

    current = driver.current_window_handle
    driver.switch_to.new_window("tab")
    driver.get("data:text/html,<title>Focus probe</title>")
    time.sleep(0.25)
    driver.close()
    driver.switch_to.window(current)

    before_reload = browser_console(driver, "preflight")
    before_reload.extend(webdriver_console(driver, "preflight"))
    user_activation = page_state(driver).get("user_activation")
    driver.refresh()
    wait_until(lambda: page_state(driver).get("status") == "running", 30, "reloaded runtime")
    storage_after_reload = wait_for_storage_state(driver, "ready")
    set_viewport(driver, width, height)
    wait_for_route(driver, "title")
    after_reload = browser_console(driver, "persistence_reload")
    values = settings_values(after_reload, "muted")
    muted_after_reload = values[-1] if values else None
    after_reload.extend(webdriver_console(driver, "persistence_reload"))
    restore_flush_count = int(storage_after_reload.get("flush_count") or 0)
    send_key(driver, "m")
    wait_for_setting(driver, "muted", "false")
    storage_after_restore = wait_until(
        lambda: (
            page_state(driver).get("storage")
            if int((page_state(driver).get("storage") or {}).get("flush_count") or 0)
            > restore_flush_count
            else None
        ),
        10,
        "audible settings flush",
    )
    flush_after_restore = (
        int(storage_after_restore.get("flush_count") or 0) > restore_flush_count
    )

    driver.get(with_query(url, {"storage": "unavailable"}))
    wait_until(
        lambda: page_state(driver).get("status") == "running",
        30,
        "storage-unavailable runtime",
    )
    wait_for_storage_state(driver, "unavailable")
    set_viewport(driver, width, height)
    wait_for_route(driver, "title")
    send_key(driver, "m")
    wait_for_setting(driver, "muted", "true")
    storage_unavailable_state = wait_until(
        lambda: (
            page_state(driver).get("storage")
            if int(
                (page_state(driver).get("storage") or {}).get("skipped_flush_count") or 0
            )
            >= 1
            else None
        ),
        10,
        "recoverable unavailable-storage save",
    )
    storage_unavailable_page = page_state(driver)
    unavailable_console = browser_console(driver, "storage_unavailable")
    unavailable_console.extend(webdriver_console(driver, "storage_unavailable"))
    storage_unavailable_boot_recoverable = (
        storage_unavailable_page.get("status") == "running"
        and storage_unavailable_state.get("state") == "unavailable"
        and storage_unavailable_state.get("last_error", {}).get("recoverable") is True
        and "title" in route_sequence(unavailable_console)
        and "true" in settings_values(unavailable_console, "muted")
    )

    console = before_reload + after_reload + unavailable_console
    write_json(output / "preflight-console.json", console)
    return {
        "console": console,
        "flush_after_save": flush_after_save,
        "flush_after_restore": flush_after_restore,
        "fullscreen_observed": fullscreen_observed,
        "letterboxing": letterboxing,
        "muted_after_reload": muted_after_reload,
        "muted_after_restore": "false",
        "navigation_to_title_ms": navigation_to_title_ms,
        "storage_after_reload": storage_after_reload,
        "storage_unavailable_boot_recoverable": storage_unavailable_boot_recoverable,
        "storage_unavailable_state": storage_unavailable_state,
        "title_state": title_state,
        "user_activation": user_activation,
    }


def run_flow(
    driver: Any,
    browser_name: str,
    url: str,
    width: int,
    height: int,
    output: Path,
    flow_timeout: int,
    stability_seconds: int,
    trace_enabled: bool,
    chrome_entries: list[dict[str, Any]],
) -> dict[str, Any]:
    from selenium.webdriver.common.by import By

    flow_url = url + ("&" if "?" in url else "?") + urllib.parse.urlencode(
        {"arg": json.dumps(["--compat-flow"])}
    )
    driver.get(flow_url)
    wait_until(lambda: page_state(driver).get("status") == "running", 30, "flow runtime")
    dimensions = set_viewport(driver, width, height)
    click_canvas_gesture(driver)
    service_pid = driver.service.process.pid
    started = time.monotonic()
    flow_deadline = started + flow_timeout
    stability_started = started + 75
    end_at = (
        stability_started + stability_seconds
        if stability_seconds
        else started + flow_timeout
    )
    trace_at = started + 70 if trace_enabled else None
    keyboard_probe_at = started + 11
    liveness_probe_at = end_at - 20 if stability_seconds else None
    memory_schedule = (
        [
            (stability_started, "t0"),
            (stability_started + 300, "t5"),
            (stability_started + 600, "t10"),
        ]
        if stability_seconds
        else []
    )
    if browser_name == "firefox":
        return run_firefox_flow(
            driver,
            dimensions,
            output,
            flow_timeout,
            stability_seconds,
            service_pid,
            started,
            stability_started,
            end_at,
        )
    memory_index = 0
    memories = []
    performance_entries = []
    flow_completed = False
    match_focus_recovery = False
    match_focus_attempted = False
    audio_probe_wait_until = None
    native_log_drains_before_flow = 0
    post_flow_actions = []
    screenshot_saved = False

    while time.monotonic() <= end_at + 5:
        now = time.monotonic()
        if browser_name != "chrome":
            state = page_state(driver)
            if state.get("status") != "running":
                raise RuntimeError(f"browser runtime changed to {state.get('status')}")
        current_entries = flow_console(driver, browser_name, "flow", chrome_entries)
        native_log_drains_before_flow += 1
        runtime_errors = [
            record
            for record in records(current_entries)
            if record.get("source") == "GC_BROWSER" and record.get("kind") == "error"
        ]
        if runtime_errors:
            raise RuntimeError(f"browser runtime reported an error: {runtime_errors[-1]}")
        routes = route_sequence(current_entries)
        if not match_focus_attempted and normalized_routes(routes)[-1:] == ["match"]:
            match_focus_attempted = True
            audio_probe_wait_until = (
                time.monotonic() + AUDIO_PROBE_SECONDS + AUDIO_PROBE_WAIT_SLACK_SECONDS
            )
            route_count = len(routes)
            current = driver.current_window_handle
            driver.switch_to.new_window("tab")
            driver.get("data:text/html,<title>Match focus probe</title>")
            time.sleep(0.5)
            driver.close()
            driver.switch_to.window(current)

            def recovered_match() -> bool:
                nonlocal native_log_drains_before_flow
                new_routes = route_sequence(
                    flow_console(driver, browser_name, "match_focus", chrome_entries)
                )
                native_log_drains_before_flow += 1
                tail = new_routes[route_count:]
                return "pause" in tail and normalized_routes(tail)[-1:] == ["match"]

            wait_until(recovered_match, 15, "Match focus recovery")
            match_focus_recovery = True
        if normalized_routes(routes)[-len(EXPECTED_FLOW) :] == EXPECTED_FLOW:
            flow_completed = True
            if audio_probe_wait_until is None:
                audio_probe_wait_until = (
                    now + AUDIO_PROBE_SECONDS + AUDIO_PROBE_WAIT_SLACK_SECONDS
                )
            if stability_seconds or (
                audio_probe_wait_until is not None
                and now >= audio_probe_wait_until
            ):
                if not stability_seconds and not screenshot_saved:
                    driver.save_screenshot(str(output / "result.png"))
                    screenshot_saved = True
                break
        if not flow_completed and now > flow_deadline:
            raise RuntimeError(f"flow did not reach Result within {flow_timeout} seconds")
        if now >= keyboard_probe_at:
            send_key(driver, "m")
            keyboard_probe_at = float("inf")
        if liveness_probe_at and now >= liveness_probe_at:
            send_key(driver, "m")
            liveness_probe_at = None
        if memory_index < len(memory_schedule) and now >= memory_schedule[memory_index][0]:
            memories.append(
                collect_memory(
                    driver,
                    browser_name,
                    service_pid,
                    memory_schedule[memory_index][1],
                )
            )
            memory_index += 1
        if trace_at and now >= trace_at:
            performance_entries.extend(driver.get_log("performance"))
            trace_at = None
        time.sleep(1)

    if flow_completed and stability_seconds:
        post_flow_actions = chrome_post_flow_actions(
            keyboard_probe_at,
            liveness_probe_at,
            memory_schedule,
            memory_index,
            trace_at,
        )
        for action_at, action, value in post_flow_actions:
            sleep_until(action_at)
            if action == "keyboard":
                send_key(driver, value)
            elif action == "memory":
                memories.append(
                    collect_memory(
                        driver,
                        browser_name,
                        service_pid,
                        value,
                    )
                )
            else:
                performance_entries.extend(driver.get_log(value))
                trace_at = None
        sleep_until(end_at)
        if not screenshot_saved:
            driver.save_screenshot(str(output / "result.png"))
            screenshot_saved = True

    if trace_at:
        performance_entries.extend(driver.get_log("performance"))
    entries, final_state = final_flow_capture(driver, browser_name, chrome_entries)
    return {
        "console": entries,
        "dimensions": dimensions,
        "duration_seconds": time.monotonic() - started,
        "final_state": final_state,
        "flow_completed": flow_completed,
        "match_focus_recovery": match_focus_recovery,
        "measurement_collection": {
            "mode": "chrome_native_log_until_flow_scheduled_terminal_capture",
            "native_log_drains_before_flow": native_log_drains_before_flow,
            "short_run_audio_probe_waited": (
                not stability_seconds and audio_probe_wait_until is not None
            ),
            "periodic_native_log_drains_after_flow": 0,
            "periodic_page_state_probes": 0,
            "post_flow_scheduled_actions": [
                {"action": action, "value": value}
                for _, action, value in post_flow_actions
            ],
            "terminal_native_log_drains": 2,
        },
        "memory": memories,
        "performance_log": performance_entries,
    }


def run_firefox_flow(
    driver: Any,
    dimensions: dict[str, int],
    output: Path,
    flow_timeout: int,
    stability_seconds: int,
    service_pid: int,
    started: float,
    stability_started: float,
    end_at: float,
) -> dict[str, Any]:
    flow_deadline = started + flow_timeout
    memories = []
    flow_completed = False
    in_page_marker_waits = 0
    screenshot_saved = False

    wait_for_route(driver, "match", min(15, flow_timeout))
    audio_probe_wait_until = (
        time.monotonic() + AUDIO_PROBE_SECONDS + AUDIO_PROBE_WAIT_SLACK_SECONDS
    )
    routes = route_sequence(browser_console(driver, "match_focus"))
    route_count = len(routes)
    current = driver.current_window_handle
    driver.switch_to.new_window("tab")
    driver.get("data:text/html,<title>Match focus probe</title>")
    time.sleep(0.5)
    driver.close()
    driver.switch_to.window(current)

    def recovered_match() -> bool:
        new_routes = route_sequence(browser_console(driver, "match_focus"))
        tail = new_routes[route_count:]
        return "pause" in tail and normalized_routes(tail)[-1:] == ["match"]

    wait_until(recovered_match, 15, "Match focus recovery")

    def observe_flow_until(deadline: float) -> None:
        nonlocal flow_completed, in_page_marker_waits, screenshot_saved
        if flow_completed:
            return
        wait_deadline = min(deadline, flow_deadline)
        remaining = wait_deadline - time.monotonic()
        if remaining > 0:
            flow_completed, waits = wait_for_firefox_flow_until(
                driver,
                "GC_METRICS|flow_complete|",
                wait_deadline,
            )
            in_page_marker_waits += waits
        if flow_completed:
            if not stability_seconds and not screenshot_saved:
                driver.save_screenshot(str(output / "result.png"))
                screenshot_saved = True
        elif time.monotonic() >= flow_deadline:
            raise RuntimeError(f"flow did not reach Result within {flow_timeout} seconds")

    for action_at, action, value in firefox_flow_actions(
        started,
        stability_started,
        stability_seconds,
    ):
        observe_flow_until(action_at)
        sleep_until(action_at)
        if action == "keyboard":
            send_key(driver, value)
        else:
            memories.append(
                collect_memory(
                    driver,
                    "firefox",
                    service_pid,
                    value,
                )
            )

    observe_flow_until(flow_deadline)
    if not stability_seconds:
        sleep_until(audio_probe_wait_until)
    if stability_seconds:
        sleep_until(end_at)
        if not screenshot_saved:
            driver.save_screenshot(str(output / "result.png"))
            screenshot_saved = True

    entries, final_state = final_flow_capture(driver, "firefox", [])
    return {
        "console": entries,
        "dimensions": dimensions,
        "duration_seconds": time.monotonic() - started,
        "final_state": final_state,
        "flow_completed": flow_completed,
        "match_focus_recovery": True,
        "measurement_collection": {
            "async_wait_chunk_seconds": FIREFOX_FLOW_WAIT_CHUNK_SECONDS,
            "in_page_marker_waits": in_page_marker_waits,
            "mode": "firefox_scheduled_terminal_capture",
            "periodic_console_drains": 0,
            "periodic_page_state_probes": 0,
            "scheduled_memory_probes": len(memories),
            "short_run_audio_probe_waited": not stability_seconds,
        },
        "memory": memories,
        "performance_log": [],
    }


def run_viewport(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    base_url: str,
    viewport: tuple[int, int],
    output: Path,
    flow_timeout: int,
    stability_seconds: int,
    trace_enabled: bool,
    expected_manifest: dict[str, Any],
    warning_classifications: list[re.Pattern[str]],
) -> dict[str, Any]:
    width, height = viewport
    output.mkdir(parents=True, exist_ok=True)
    driver_log = output / "webdriver.log"
    dedicated_flow_browser = browser_name == "chrome" and stability_seconds > 0
    preflight_driver_log = (
        output / "preflight-webdriver.log" if dedicated_flow_browser else driver_log
    )
    active_driver_log = preflight_driver_log
    expected_revision = str(expected_manifest.get("source_revision"))
    driver = None
    capabilities: dict[str, Any] = {}
    preflight: dict[str, Any] = {"console": []}
    flow: dict[str, Any] | None = None
    failure: Exception | None = None
    failure_entries: list[dict[str, Any]] = []
    failure_state: dict[str, Any] = {}
    chrome_entries: list[dict[str, Any]] = []
    service_runtime_bytes = 0
    preflight_cleanup: dict[str, Any] = {
        "pass": not dedicated_flow_browser,
        "required": dedicated_flow_browser,
    }
    flow_cleanup: dict[str, Any] = {
        "pass": False,
        "reason": "flow browser did not launch",
    }
    try:
        validate_manifest(base_url, expected_manifest)
        driver = launch_browser(
            browser_name,
            binary,
            driver_path,
            preflight_driver_log,
            trace_enabled and not dedicated_flow_browser,
        )
        capabilities = driver.capabilities
        preflight_profile = profile_directory(capabilities)
        preflight = run_preflight(driver, base_url, width, height, output)
        if preflight["title_state"].get("build_id") != expected_revision:
            raise RuntimeError(
                "page build ID does not match manifest revision: "
                f"{preflight['title_state'].get('build_id')} != {expected_revision}"
            )
        if dedicated_flow_browser:
            preflight_session_id = driver.session_id
            preflight_cleanup = quit_browser_bounded(driver)
            driver = None
            if preflight_cleanup.get("pass") is not True:
                raise RuntimeError(
                    f"preflight browser cleanup failed: {preflight_cleanup}"
                )
            active_driver_log = driver_log
            driver = launch_browser(
                browser_name,
                binary,
                driver_path,
                driver_log,
                trace_enabled,
            )
            capabilities = driver.capabilities
            flow_profile = profile_directory(capabilities)
            preflight["acceptance_isolation"] = {
                "dedicated_flow_browser": True,
                "distinct_profile": bool(
                    preflight_profile
                    and flow_profile
                    and preflight_profile != flow_profile
                ),
                "flow_profile": flow_profile,
                "flow_session_id": driver.session_id,
                "preflight_profile": preflight_profile,
                "preflight_session_id": preflight_session_id,
            }
        else:
            preflight["acceptance_isolation"] = {
                "dedicated_flow_browser": False,
                "distinct_profile": False,
            }
        flow = run_flow(
            driver,
            browser_name,
            base_url,
            width,
            height,
            output,
            flow_timeout,
            stability_seconds,
            trace_enabled,
            chrome_entries,
        )
        if flow["final_state"].get("build_id") != expected_revision:
            raise RuntimeError(
                "flow build ID does not match manifest revision: "
                f"{flow['final_state'].get('build_id')} != {expected_revision}"
            )
        validate_manifest(base_url, expected_manifest)
    except Exception as error:
        failure = error
        if driver:
            capabilities = driver.capabilities
            failure_state, failure_entries = failure_diagnostics(
                driver,
                browser_name,
                chrome_entries,
            )
    finally:
        if driver:
            if active_driver_log.is_file():
                service_runtime_bytes = active_driver_log.stat().st_size
            flow_cleanup = quit_browser_bounded(driver)
            driver = None

    service_entries = (
        service_console(active_driver_log, service_runtime_bytes)
        if browser_name == "firefox"
        else []
    )
    browser_cleanup = {
        "flow": flow_cleanup,
        "pass": (
            preflight_cleanup.get("pass") is True
            and flow_cleanup.get("pass") is True
        ),
        "preflight": preflight_cleanup,
    }
    if browser_cleanup["pass"] is not True:
        service_entries.append(
            {
                "level": "error",
                "message": f"WebDriver cleanup failed: {browser_cleanup}",
                "phase": "webdriver_teardown",
            }
        )
    if failure:
        entries = preflight.get("console", []) + failure_entries + service_entries
        write_json(output / "console.json", entries)
        write_json(output / "memory.json", [])
        write_gzip_json(output / "performance-log.json.gz", [])
        report = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts" / "web_report.py"),
                str(output / "console.json"),
                "--require-flow",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        (output / "web-report.txt").write_text(
            report.stdout + report.stderr,
            encoding="utf-8",
        )
        result = {
            "browser": {
                "capabilities": capabilities,
                "binary": str(binary),
                "driver": str(driver_path),
            },
            "captured_at": utc_now(),
            "checks": {
                "browser_cleanup": browser_cleanup,
                "browser_teardown": console_gate(
                    [
                        entry
                        for entry in entries
                        if entry.get("phase") == "webdriver_teardown"
                    ],
                    warning_classifications,
                ),
                "clean_console": console_gate(
                    [
                        entry
                        for entry in entries
                        if entry.get("phase") != "webdriver_teardown"
                    ],
                    warning_classifications,
                ),
                "terminal_runtime_health": terminal_runtime_health(
                    [
                        entry
                        for entry in entries
                        if entry.get("phase") != "webdriver_teardown"
                    ],
                    failure_state,
                ),
            },
            "dimensions": None,
            "duration_seconds": None,
            "error": {
                "message": str(failure),
                "type": type(failure).__name__,
            },
            "final_state": failure_state,
            "preflight": {
                key: preflight.get(key)
                for key in (
                    "fullscreen_observed",
                    "flush_after_restore",
                    "flush_after_save",
                    "acceptance_isolation",
                    "letterboxing",
                    "muted_after_reload",
                    "muted_after_restore",
                    "navigation_to_title_ms",
                    "storage_after_reload",
                    "storage_unavailable_boot_recoverable",
                    "storage_unavailable_state",
                    "title_state",
                )
            },
            "stability_seconds": stability_seconds,
            "trace_enabled": trace_enabled,
            "viewport": {"height": height, "width": width},
            "web_report_exit_code": report.returncode,
        }
        write_json(output / "run.json", result)
        return result

    assert flow is not None
    entries = preflight["console"] + flow["console"] + service_entries
    write_json(output / "console.json", entries)
    write_json(output / "memory.json", flow["memory"])
    write_gzip_json(output / "performance-log.json.gz", flow["performance_log"])
    preflight["match_focus_recovery"] = flow["match_focus_recovery"]
    checks = evidence_checks(
        browser_name,
        entries,
        flow["final_state"],
        preflight,
        flow["memory"],
        stability_seconds,
        warning_classifications,
    )
    checks["browser_cleanup"] = browser_cleanup
    report = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "web_report.py"),
            str(output / "console.json"),
            "--require-flow",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    (output / "web-report.txt").write_text(
        report.stdout + report.stderr,
        encoding="utf-8",
    )
    result = {
        "browser": {
            "capabilities": capabilities,
            "binary": str(binary),
            "driver": str(driver_path),
        },
        "captured_at": utc_now(),
        "checks": checks,
        "dimensions": flow["dimensions"],
        "duration_seconds": flow["duration_seconds"],
        "final_state": {
            key: flow["final_state"].get(key)
            for key in (
                "build_id",
                "canvas",
                "events",
                "gamepads",
                "renderer",
                "status",
                "user_activation",
                "vendor",
                "viewport",
            )
        },
        "measurement_collection": flow.get("measurement_collection"),
        "preflight": {
            key: preflight.get(key)
            for key in (
                "fullscreen_observed",
                "flush_after_restore",
                "flush_after_save",
                "acceptance_isolation",
                "letterboxing",
                "match_focus_recovery",
                "muted_after_reload",
                "muted_after_restore",
                "navigation_to_title_ms",
                "storage_after_reload",
                "storage_unavailable_boot_recoverable",
                "storage_unavailable_state",
                "title_state",
            )
        },
        "stability_seconds": stability_seconds,
        "trace_enabled": trace_enabled,
        "viewport": {"height": height, "width": width},
        "web_report_exit_code": report.returncode,
    }
    write_json(output / "run.json", result)
    return result


def wait_for_firefox_heap_checkpoint(baseline: float, target_seconds: int, label: str) -> None:
    deadline = baseline + max(0, target_seconds - FIREFOX_HEAP_PROMPT_LEAD_SECONDS)
    while time.monotonic() < deadline:
        remaining = max(0, math.ceil(deadline - time.monotonic()))
        print(f"{label} checkpoint in {remaining} seconds...", flush=True)
        time.sleep(min(30, max(1, remaining)))


def capture_firefox_heap_checkpoint(
    driver: Any,
    game_handle: str,
    output: Path,
    label: str,
    target_seconds: int,
    baseline: float | None,
    expected_build_id: str,
) -> dict[str, Any]:
    if baseline is not None:
        wait_for_firefox_heap_checkpoint(baseline, target_seconds, label)
    driver.switch_to.window(game_handle)
    dimensions_before = set_viewport(driver, CANVAS_WIDTH, CANVAS_HEIGHT)
    state_before = page_state(driver)
    routes_before = normalized_routes(
        route_sequence(browser_console(driver, "firefox_heap_checkpoint"))
    )
    expected_viewport = {"height": CANVAS_HEIGHT, "width": CANVAS_WIDTH}
    before_pass = (
        state_before.get("status") == "running"
        and state_before.get("build_id") == expected_build_id
        and state_before.get("viewport") == expected_viewport
        and routes_before[-1:] == ["result"]
    )
    if not before_pass:
        raise RuntimeError(
            f"game tab failed its pre-{label} state/viewport/Result gate"
        )

    about_memory_path = output / f"about-memory-{label}.json.gz"
    snapshot_path = output / f"heap-{label}.fxsnapshot"
    for path in (about_memory_path, snapshot_path):
        if path.exists():
            raise RuntimeError(f"refusing reused Firefox heap evidence file: {path}")

    print("", flush=True)
    print(f"FIREFOX HEAP CHECKPOINT {label}", flush=True)
    print(
        "Keep DevTools undocked. In about:memory, click Minimize memory usage, "
        "then Measure and save:",
        flush=True,
    )
    print(f"  {about_memory_path}", flush=True)
    print(
        "Immediately return to the game tab's Memory panel, take a snapshot, "
        "and save:",
        flush=True,
    )
    print(f"  {snapshot_path}", flush=True)
    prompt_started_at = utc_now()
    prompt_started_ns = time.time_ns()
    prompt_started_monotonic = time.monotonic()
    input("Press Enter after both evidence files are fully saved: ")
    about_memory = file_evidence(about_memory_path, prompt_started_ns)
    snapshot = file_evidence(snapshot_path, prompt_started_ns)
    completed_monotonic = time.monotonic()
    elapsed_seconds = (
        0.0 if baseline is None else round(completed_monotonic - baseline, 3)
    )
    prompt_elapsed_seconds = (
        0.0 if baseline is None else round(prompt_started_monotonic - baseline, 3)
    )

    driver.switch_to.window(game_handle)
    dimensions_after = set_viewport(driver, CANVAS_WIDTH, CANVAS_HEIGHT)
    state_after = page_state(driver)
    routes_after = normalized_routes(
        route_sequence(browser_console(driver, "firefox_heap_checkpoint"))
    )
    viewport_pass = (
        state_after.get("status") == "running"
        and state_after.get("build_id") == expected_build_id
        and state_after.get("viewport") == expected_viewport
        and routes_after[-1:] == ["result"]
    )
    return {
        "about_memory": about_memory,
        "captured_at": utc_now(),
        "completed_elapsed_seconds": elapsed_seconds,
        "dimensions_after": dimensions_after,
        "dimensions_before": dimensions_before,
        "game_state_after": {
            "build_id": state_after.get("build_id"),
            "status": state_after.get("status"),
            "viewport": state_after.get("viewport"),
        },
        "game_state_before": {
            "build_id": state_before.get("build_id"),
            "status": state_before.get("status"),
            "viewport": state_before.get("viewport"),
        },
        "heap_snapshot": snapshot,
        "label": label,
        "pre_checkpoint_pass": before_pass,
        "prompt_elapsed_seconds": prompt_elapsed_seconds,
        "prompt_started_at": prompt_started_at,
        "routes_after": routes_after,
        "routes_before": routes_before,
        "timing_target_seconds": target_seconds,
        "timing_tolerance_seconds": FIREFOX_HEAP_TIMING_TOLERANCE_SECONDS,
        "viewport_pass": viewport_pass,
    }


def firefox_heap_extractor_source(
    checkpoints: list[dict[str, Any]],
    output: Path,
) -> str:
    specifications = [
        {
            "label": checkpoint["label"],
            "outputPath": str(
                (output / f"heap-{checkpoint['label']}-census.json").resolve()
            ),
            "snapshotPath": checkpoint["heap_snapshot"]["path"],
        }
        for checkpoint in checkpoints
    ]
    specs_json = json.dumps(specifications, ensure_ascii=True)
    return f"""// OMP-0 Firefox heap extractor schema 1.
// Paste this entire file into the isolated campaign profile's Browser Console.
(async () => {{
  const {{ Services }} = ChromeUtils.importESModule(
    "resource://gre/modules/Services.sys.mjs"
  );
  const specs = {specs_json};
  const results = [];
  for (const spec of specs) {{
    const snapshot = ChromeUtils.readHeapSnapshot(spec.snapshotPath);
    const report = snapshot.takeCensus({{
      breakdown: {{ by: "count", count: true, bytes: true }},
    }});
    const stat = await IOUtils.stat(spec.snapshotPath);
    const result = {{
      app_build_id: Services.appinfo.appBuildID,
      census_bytes: report.bytes,
      census_count: report.count,
      extractor_schema: 1,
      firefox_version: Services.appinfo.version,
      label: spec.label,
      platform_build_id: Services.appinfo.platformBuildID,
      snapshot_creation_time_us: snapshot.creationTime,
      snapshot_path: spec.snapshotPath,
      snapshot_sha256: await IOUtils.computeHexDigest(spec.snapshotPath, "sha256"),
      snapshot_size_bytes: stat.size,
    }};
    await IOUtils.writeJSON(spec.outputPath, result);
    results.push(result);
  }}
  return results;
}})();
"""


def read_firefox_heap_census(
    checkpoints: list[dict[str, Any]],
    output: Path,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    extractor_path = output / "firefox-heap-extractor.js"
    extractor_path.write_text(
        firefox_heap_extractor_source(checkpoints, output),
        encoding="utf-8",
    )
    extractor = file_evidence(extractor_path)
    expected_outputs = [
        output / f"heap-{checkpoint['label']}-census.json"
        for checkpoint in checkpoints
    ]
    if any(path.exists() for path in expected_outputs):
        raise RuntimeError("refusing reused Firefox heap census output")
    print("", flush=True)
    print("FIREFOX HEAP SNAPSHOT CENSUS", flush=True)
    print(
        "Open Firefox Browser Console (Ctrl+Shift+J) in this isolated profile, "
        "paste the entire extractor file below, and run it:",
        flush=True,
    )
    print(f"  {extractor_path}", flush=True)
    extractor_started_ns = time.time_ns()
    input("Press Enter after the console reports the three census results: ")

    results = []
    for checkpoint, result_path in zip(checkpoints, expected_outputs, strict=True):
        result_evidence = file_evidence(result_path, extractor_started_ns)
        try:
            value = json.loads(result_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise RuntimeError(f"invalid Firefox heap census output: {result_path}") from error
        snapshot = checkpoint["heap_snapshot"]
        census_bytes = value.get("census_bytes")
        census_count = value.get("census_count")
        if (
            value.get("extractor_schema") != 1
            or value.get("label") != checkpoint["label"]
            or value.get("snapshot_path") != snapshot["path"]
            or str(value.get("snapshot_sha256", "")).lower() != snapshot["sha256"]
            or value.get("snapshot_size_bytes") != snapshot["size_bytes"]
            or isinstance(census_bytes, bool)
            or not isinstance(census_bytes, int)
            or census_bytes <= 0
            or isinstance(census_count, bool)
            or not isinstance(census_count, int)
            or census_count <= 0
            or not isinstance(value.get("snapshot_creation_time_us"), int)
            or not isinstance(value.get("app_build_id"), str)
            or not value["app_build_id"]
            or not isinstance(value.get("platform_build_id"), str)
            or not value["platform_build_id"]
            or not isinstance(value.get("firefox_version"), str)
            or not value["firefox_version"]
        ):
            raise RuntimeError(
                f"Firefox heap census provenance mismatch at {checkpoint['label']}"
            )
        checkpoint["census"] = value
        checkpoint["census_output"] = result_evidence
        checkpoint["tab_heap_snapshot_census_bytes"] = census_bytes
        results.append(value)
    return results, extractor


def run_firefox_heap_companion(
    binary: Path,
    driver_path: Path,
    base_url: str,
    packet_root: Path,
    manifest: dict[str, Any],
    flow_timeout: int,
    expected_environment: dict[str, Any],
) -> dict[str, Any]:
    output = packet_root / "firefox-heap-companion"
    output.mkdir(parents=True, exist_ok=False)
    driver_log = output / "webdriver.log"
    binary_metadata = executable_metadata(binary)
    driver_metadata = executable_metadata(driver_path)
    expected_binary = expected_environment.get("browser_binary", {})
    expected_driver = expected_environment.get("browser_driver", {})
    asset_match = (
        binary_metadata.get("sha256") == expected_binary.get("sha256")
        and driver_metadata.get("sha256") == expected_driver.get("sha256")
    )
    if not asset_match:
        raise RuntimeError(
            "Firefox heap companion browser/driver assets do not match the automated row"
        )

    browser = None
    cleanup = {"pass": False, "reason": "browser did not launch"}
    flow: dict[str, Any] | None = None
    checkpoints: list[dict[str, Any]] = []
    capabilities: dict[str, Any] = {}
    session_id = None
    profile = None
    try:
        validate_manifest(base_url, manifest)
        browser = launch_browser(
            "firefox",
            binary,
            driver_path,
            driver_log,
            False,
            browser_console_enabled=True,
        )
        capabilities = browser.capabilities
        session_id = browser.session_id
        profile = profile_directory(capabilities)
        game_handle = browser.current_window_handle
        flow = run_flow(
            browser,
            "firefox",
            base_url,
            CANVAS_WIDTH,
            CANVAS_HEIGHT,
            output,
            flow_timeout,
            0,
            False,
            [],
        )
        flow_routes = normalized_routes(route_sequence(flow["console"]))
        if flow_routes[-len(EXPECTED_FLOW) :] != EXPECTED_FLOW:
            raise RuntimeError("Firefox heap companion did not reach stable Result")
        final_state = flow["final_state"]
        if final_state.get("build_id") != manifest.get("source_revision"):
            raise RuntimeError("Firefox heap companion build ID does not match the packet")
        if final_state.get("viewport") != {
            "height": CANVAS_HEIGHT,
            "width": CANVAS_WIDTH,
        }:
            raise RuntimeError("Firefox heap companion did not establish a 960x540 viewport")
        write_json(output / "flow-console.json", flow["console"])
        write_json(
            output / "session.json",
            {
                "browser_binary": binary_metadata,
                "browser_capabilities": capabilities,
                "browser_driver": driver_metadata,
                "clean_profile": True,
                "flow_final_state": final_state,
                "flow_routes": flow_routes,
                "profile": profile,
                "session_id": session_id,
                "viewport": {"height": CANVAS_HEIGHT, "width": CANVAS_WIDTH},
            },
        )

        print("", flush=True)
        print("ATTENDED FIREFOX TAB-HEAP COMPANION", flush=True)
        print(
            "The exact game tab is at stable Result in a fresh WebDriver profile with "
            "a verified 960x540 inner viewport.",
            flush=True,
        )
        print(
            "Open that tab's DevTools Memory panel in a separate undocked window. "
            "The runner will save each snapshot now and reduce it afterward with "
            "Firefox's own offline heap census.",
            flush=True,
        )
        input("Press Enter when the undocked Memory panel is ready: ")

        t0 = capture_firefox_heap_checkpoint(
            browser,
            game_handle,
            output,
            "t0",
            0,
            None,
            str(manifest.get("source_revision")),
        )
        checkpoints.append(t0)
        baseline = time.monotonic()
        for label, target_seconds in FIREFOX_HEAP_CHECKPOINTS[1:]:
            checkpoints.append(
                capture_firefox_heap_checkpoint(
                    browser,
                    game_handle,
                    output,
                    label,
                    target_seconds,
                    baseline,
                    str(manifest.get("source_revision")),
                )
            )
        census_results, extractor = read_firefox_heap_census(checkpoints, output)
        validate_manifest(base_url, manifest)
    finally:
        if browser:
            cleanup = quit_browser_bounded(browser)

    assert flow is not None
    totals = [
        float(checkpoint["tab_heap_snapshot_census_bytes"])
        for checkpoint in checkpoints
    ]
    growth_percent = ((totals[-1] - totals[0]) / totals[0]) * 100
    creation_times = [
        int(result["snapshot_creation_time_us"]) for result in census_results
    ]
    firefox_versions = {result["firefox_version"] for result in census_results}
    app_build_ids = {result["app_build_id"] for result in census_results}
    platform_build_ids = {result["platform_build_id"] for result in census_results}
    build_metadata_pass = (
        len(firefox_versions) == 1
        and len(app_build_ids) == 1
        and len(platform_build_ids) == 1
        and next(iter(firefox_versions), None) == capabilities.get("browserVersion")
    )
    session_metadata_pass = bool(session_id and profile)
    timing = firefox_heap_snapshot_timing(creation_times)
    timing_offsets_seconds = timing["offsets_seconds"]
    timing_pass = timing["pass"]
    viewport_pass = len(checkpoints) == 3 and all(
        checkpoint["viewport_pass"] is True for checkpoint in checkpoints
    )
    flow_routes = normalized_routes(route_sequence(flow["console"]))
    renderer = str(flow["final_state"].get("renderer") or "")
    hardware_acceleration_pass = bool(renderer) and not any(
        name in renderer.lower() for name in SOFTWARE_RENDERERS
    )
    driver_log_evidence = file_evidence(driver_log)
    summary = {
        "asset_match": asset_match,
        "browser_binary": binary_metadata,
        "browser_capabilities": capabilities,
        "browser_driver": driver_metadata,
        "build_metadata": {
            "app_build_ids": sorted(app_build_ids),
            "firefox_versions": sorted(firefox_versions),
            "pass": build_metadata_pass,
            "platform_build_ids": sorted(platform_build_ids),
        },
        "captured_at": utc_now(),
        "checkpoints": checkpoints,
        "clean_profile": True,
        "flow_pass": flow_routes[-len(EXPECTED_FLOW) :] == EXPECTED_FLOW,
        "formula": "(t10 - t0) / t0 * 100",
        "growth_percent": round(growth_percent, 6),
        "hardware_acceleration": {
            "pass": hardware_acceleration_pass,
            "renderer": renderer or None,
            "vendor": flow["final_state"].get("vendor"),
        },
        "heap_metric": {
            "breakdown": {"by": "count", "bytes": True, "count": True},
            "extractor": extractor,
            "name": "Firefox DevTools tab heap-snapshot census bytes",
            "source": "HeapSnapshot.takeCensus root report.bytes",
        },
        "manifest_sha256": file_evidence(packet_root / "manifest.json")["sha256"],
        "monotonic_non_decreasing": totals[0] <= totals[1] <= totals[2],
        "package_sha256": manifest.get("game_package", {}).get("sha256"),
        "pass": (
            asset_match
            and build_metadata_pass
            and session_metadata_pass
            and flow_routes[-len(EXPECTED_FLOW) :] == EXPECTED_FLOW
            and hardware_acceleration_pass
            and timing_pass
            and viewport_pass
            and growth_percent <= 25
            and cleanup.get("pass") is True
        ),
        "profile": profile,
        "quit": cleanup,
        "session_metadata_pass": session_metadata_pass,
        "session_id": session_id,
        "source_revision": manifest.get("source_revision"),
        "threshold_percent": 25,
        "timing_offsets_seconds": timing_offsets_seconds,
        "timing_pass": timing_pass,
        "viewport": {"height": CANVAS_HEIGHT, "width": CANVAS_WIDTH},
        "viewport_pass": viewport_pass,
        "webdriver_log": driver_log_evidence,
    }
    write_json(output / "firefox-heap-summary.json", summary)
    return summary


def result_passes(
    result: dict[str, Any],
    require_stability: bool,
    require_gamepad: bool = False,
    external_memory_companion: bool = False,
) -> bool:
    required = [
        "artifact_boot",
        "audio_after_gesture",
        "browser_cleanup",
        "clean_console",
        "complete_flow",
        "fullscreen",
        "hardware_acceleration",
        "keyboard",
        "letterboxing",
        "lifecycle",
        "performance",
        "persistence",
        "storage_unavailable",
        "terminal_runtime_health",
    ]
    if require_gamepad or require_stability:
        required.append("gamepad")
    if require_stability:
        required.extend(
            (
                "acceptance_isolation",
                "runtime_stability",
                "stability_performance",
            )
        )
        if not external_memory_companion:
            required.append("memory")
    return (
        result.get("web_report_exit_code") == 0
        and all(result.get("checks", {}).get(name, {}).get("pass") is True for name in required)
    )


def self_test() -> int:
    class FakeFirefoxWaitDriver:
        def __init__(self, value: dict[str, Any]) -> None:
            self.value = value
            self.timeout = None
            self.arguments = None

        def execute_script(self, _script: str, *_args: Any) -> Any:
            raise AssertionError("Firefox flow waits must not poll PAGE_STATE")

        def set_script_timeout(self, timeout: float) -> None:
            self.timeout = timeout

        def execute_async_script(self, script: str, *arguments: Any) -> dict[str, Any]:
            assert script == FIREFOX_FLOW_WAIT
            self.arguments = arguments
            return self.value

    class FakeFirefoxChunkDriver:
        def __init__(self) -> None:
            self.arguments: list[tuple[Any, ...]] = []
            self.timeouts: list[float] = []
            self.values = [
                {"error": None, "matched": False, "status": "running"},
                {"error": None, "matched": False, "status": "running"},
                {"error": None, "matched": True, "status": "running"},
            ]

        def execute_script(self, _script: str, *_args: Any) -> Any:
            raise AssertionError("Firefox flow waits must not poll PAGE_STATE")

        def set_script_timeout(self, timeout: float) -> None:
            self.timeouts.append(timeout)

        def execute_async_script(self, script: str, *arguments: Any) -> dict[str, Any]:
            assert script == FIREFOX_FLOW_WAIT
            self.arguments.append(arguments)
            return self.values.pop(0)

    class FakeChromeDriver:
        capabilities = {"browserName": "chrome"}

        def __init__(self) -> None:
            self.log_batches = [
                [
                    {
                        "level": "INFO",
                        "message": (
                            'http://example/player.js 1:1 "'
                            'GC_METRICS|route|route=title"'
                        ),
                        "timestamp": 1,
                    }
                ],
                [],
            ]

        def execute_script(self, _script: str) -> Any:
            raise AssertionError("Chrome flow polling must not inject page scripts")

        def get_log(self, log_type: str) -> list[dict[str, Any]]:
            assert log_type == "browser"
            return self.log_batches.pop(0)

    class MemoryOrderDriver:
        capabilities = {"browserName": "chrome"}

        def __init__(self) -> None:
            self.actions: list[str] = []

        def execute_script(self, script: str, *_args: Any) -> Any:
            if script == PAGE_STATE:
                self.actions.append("page_state")
                return {"js_heap_bytes": 99, "status": "running"}
            if "phase=start" in script:
                self.actions.append("marker_start")
            elif "phase=end" in script:
                self.actions.append("marker_end")
            return None

        def execute_cdp_cmd(self, method: str, _params: dict[str, Any]) -> dict[str, Any]:
            self.actions.append(method)
            if method == "Performance.getMetrics":
                return {
                    "metrics": [
                        {"name": "Documents", "value": 1},
                        {"name": "Frames", "value": 1},
                        {"name": "JSHeapUsedSize", "value": 100},
                    ]
                }
            if method == "Runtime.getHeapUsage":
                return {"usedSize": 101}
            return {}

    class FinalCaptureDriver:
        capabilities = {"browserName": "chrome"}

        def __init__(self) -> None:
            self.actions: list[str] = []
            self.log_batches = [
                [
                    {
                        "level": "INFO",
                        "message": 'http://example "GC_METRICS|route|route=result"',
                        "timestamp": 1,
                    }
                ],
                [
                    {
                        "level": "SEVERE",
                        "message": "late severe failure",
                        "timestamp": 2,
                    }
                ],
            ]

        def execute_script(self, script: str, *_args: Any) -> dict[str, Any]:
            assert script == PAGE_STATE
            self.actions.append("page_state")
            return {"status": "running"}

        def get_log(self, log_type: str) -> list[dict[str, Any]]:
            assert log_type == "browser"
            self.actions.append("get_log")
            return self.log_batches.pop(0)

    class FailureDriver:
        capabilities = {"browserName": "chrome"}

        def __init__(self) -> None:
            self.actions: list[str] = []

        def execute_script(self, script: str, *_args: Any) -> Any:
            assert script == PAGE_STATE
            self.actions.append("page_state")
            raise RuntimeError("page state unavailable")

        def get_log(self, log_type: str) -> list[dict[str, Any]]:
            assert log_type == "browser"
            self.actions.append("get_log")
            return [
                {
                    "level": "SEVERE",
                    "message": "failure-path severe entry",
                    "timestamp": 3,
                }
            ]

    assert parse_viewport("960x540") == (960, 540)
    windows_gpus = parse_windows_gpu_output(
        json.dumps(
            [
                {
                    "AdapterCompatibility": "Intel Corporation",
                    "DriverVersion": "32.0.101.6559",
                    "Name": "Intel(R) Arc(TM) Graphics",
                    "PNPDeviceID": "PCI\\VEN_8086",
                    "VideoProcessor": "Intel Arc",
                },
                {
                    "AdapterCompatibility": "Advanced Micro Devices, Inc.",
                    "DriverVersion": "32.0.21025.1006",
                    "Name": "AMD Radeon RX 7800 XT",
                    "PNPDeviceID": "PCI\\VEN_1002",
                    "VideoProcessor": "AMD Radeon Graphics Processor",
                },
            ]
        )
    )
    assert windows_gpus["available"] is True
    assert windows_gpus["truncated"] is False
    assert [adapter["name"] for adapter in windows_gpus["adapters"]] == [
        "Intel(R) Arc(TM) Graphics",
        "AMD Radeon RX 7800 XT",
    ]
    bounded_gpus = parse_windows_gpu_output(
        json.dumps(
            [{"Name": f"adapter-{index}"} for index in range(WINDOWS_GPU_MAX_ADAPTERS + 1)]
        )
    )
    assert len(bounded_gpus["adapters"]) == WINDOWS_GPU_MAX_ADAPTERS
    assert bounded_gpus["truncated"] is True
    assert parse_windows_gpu_output("not-json") == {
        "adapters": [],
        "available": False,
        "truncated": False,
    }
    assert expected_canvas_rect(800, 540) == {
        "height": 450,
        "width": 800,
        "x": 0,
        "y": 45,
    }
    assert expected_canvas_rect(1280, 540) == {
        "height": 540,
        "width": 960,
        "x": 160,
        "y": 0,
    }
    for width, height in DEFAULT_VIEWPORTS:
        assert expected_canvas_rect(width, height) == {
            "height": height,
            "width": width,
            "x": 0,
            "y": 0,
        }
    assert rect_matches(
        {"height": 540, "width": 959.9834, "x": 160.0166, "y": 0},
        expected_canvas_rect(1280, 540),
    )
    assert not rect_matches(
        {"height": 540, "width": 1280, "x": 0, "y": 0},
        expected_canvas_rect(1280, 540),
    )
    assert logical_to_client(expected_canvas_rect(800, 540), 480, 403) == {
        "x": 400,
        "y": 380.8333333333333,
    }
    assert logical_to_client(expected_canvas_rect(1280, 540), 480, 403) == {
        "x": 640,
        "y": 403,
    }
    old_wide_control = pointer_offset_control(
        expected_canvas_rect(1280, 540),
        360,
        390,
    )
    assert old_wide_control["logical_if_offset_omitted"] == {
        "x": 520,
        "y": 390,
    }
    assert old_wide_control["omitted_offset_hits_target"] is True
    assert old_wide_control["pass"] is False
    wide_control = pointer_offset_control(
        expected_canvas_rect(1280, 540),
        POINTER_TARGET["x"],
        POINTER_TARGET["y"],
    )
    assert wide_control["logical_if_offset_omitted"] == {
        "x": 640,
        "y": 403,
    }
    assert wide_control["omitted_offset_hits_target"] is False
    assert wide_control["pass"] is True
    tall_control = pointer_offset_control(
        expected_canvas_rect(800, 540),
        POINTER_TARGET["x"],
        POINTER_TARGET["y"],
    )
    assert tall_control["logical_if_offset_omitted"] == {
        "x": 480,
        "y": 457.0,
    }
    assert tall_control["omitted_offset_hits_target"] is False
    assert tall_control["pass"] is True
    assert with_query("http://127.0.0.1:8000/?arg=flow", {"storage": "unavailable"}) == (
        "http://127.0.0.1:8000/?arg=flow&storage=unavailable"
    )
    assert marker_message('http://example 1:2 "GC_METRICS|route|route=title"') == (
        "GC_METRICS|route|route=title"
    )
    assert parse_marker("GC_BROWSER|first_frame|at_ms=12.5") == {
        "source": "GC_BROWSER",
        "kind": "first_frame",
        "at_ms": "12.5",
    }
    assert parse_marker("GC_METRICS|input|kind=key_return|sequence=1") == {
        "source": "GC_METRICS",
        "kind": "input",
        "event_kind": "key_return",
        "sequence": "1",
    }
    healthy_audio_entries = [
        {
            "level": "log",
            "message": (
                "GC_METRICS|settings|at_ms=900|fullscreen=false|muted=false"
            ),
            "phase": "flow",
        },
        {
            "level": "log",
            "message": "GC_METRICS|route|at_ms=1000|route=match",
            "phase": "flow",
        }
    ]
    for observation in range(AUDIO_PROBE_MAX_OBSERVATIONS):
        healthy_audio_entries.append(
            {
                "level": "log",
                "message": (
                    "GC_METRICS|audio"
                    f"|active_sources={1 if observation == 4 else 0}"
                    f"|at_ms={1000 + observation * 500}|volume=1"
                ),
                "phase": "flow",
            }
        )
    healthy_audio = audio_probe_check(
        healthy_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert healthy_audio["pass"] is True
    assert healthy_audio["bounded"] is True
    assert healthy_audio["complete"] is True
    assert healthy_audio["positive_source_observed"] is True
    assert healthy_audio["audible_settings"]["pass"] is True
    assert healthy_audio["invalid_observations"] == []
    assert healthy_audio["ordered"] is True
    assert healthy_audio["observation_offsets_ms"] == [
        observation * 500 for observation in range(AUDIO_PROBE_MAX_OBSERVATIONS)
    ]
    focus_reentry_audio = audio_probe_check(
        [
            *healthy_audio_entries,
            {
                "level": "log",
                "message": "GC_METRICS|route|at_ms=2500|route=match",
            },
        ],
        {"user_activation": {"has_been_active": True}},
    )
    assert focus_reentry_audio["pass"] is True
    assert focus_reentry_audio["match_at_ms"] == 1000
    missing_audio = audio_probe_check(
        [
            {
                "level": "log",
                "message": "GC_METRICS|route|at_ms=1000|route=match",
            }
        ],
        {"user_activation": {"has_been_active": True}},
    )
    assert missing_audio["pass"] is False
    assert missing_audio["complete"] is False
    assert missing_audio["positive_source_observed"] is False
    silent_audio_entries = [
        {
            **entry,
            "message": str(entry["message"]).replace("active_sources=1", "active_sources=0"),
        }
        for entry in healthy_audio_entries
    ]
    silent_audio = audio_probe_check(
        silent_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert silent_audio["pass"] is False
    assert silent_audio["complete"] is True
    assert silent_audio["positive_source_observed"] is False
    muted_audio_entries = [
        {
            **entry,
            "message": str(entry["message"]).replace("muted=false", "muted=true"),
        }
        for entry in healthy_audio_entries
    ]
    muted_audio = audio_probe_check(
        muted_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert muted_audio["pass"] is False
    assert muted_audio["audible_settings"]["pass"] is False
    incomplete_audio = audio_probe_check(
        healthy_audio_entries[:-1],
        {"user_activation": {"has_been_active": True}},
    )
    assert incomplete_audio["pass"] is False
    assert incomplete_audio["bounded"] is False
    assert incomplete_audio["complete"] is False
    malformed_audio_entries = [
        {
            **entry,
            "message": (
                str(entry["message"]).replace("active_sources=0", "active_sources=invalid")
                if "at_ms=1000|volume=1" in str(entry["message"])
                else entry["message"]
            ),
        }
        for entry in healthy_audio_entries
    ]
    malformed_audio = audio_probe_check(
        malformed_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert malformed_audio["pass"] is False
    assert malformed_audio["complete"] is False
    assert malformed_audio["invalid_observations"][0]["reason"] == (
        "invalid active_sources or volume"
    )
    nonfinite_audio_entries = [
        {
            **entry,
            "message": (
                str(entry["message"]).replace("at_ms=1500", "at_ms=nan")
                if "GC_METRICS|audio" in str(entry["message"])
                else entry["message"]
            ),
        }
        for entry in healthy_audio_entries
    ]
    nonfinite_audio = audio_probe_check(
        nonfinite_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert nonfinite_audio["pass"] is False
    assert nonfinite_audio["invalid_observations"][0]["reason"] == "non-finite at_ms"
    missing_audio_field_entries = [
        {
            **entry,
            "message": (
                str(entry["message"]).replace("|volume=1", "")
                if "at_ms=1500|volume=1" in str(entry["message"])
                else entry["message"]
            ),
        }
        for entry in healthy_audio_entries
    ]
    missing_audio_field = audio_probe_check(
        missing_audio_field_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert missing_audio_field["pass"] is False
    assert missing_audio_field["invalid_observations"][0]["reason"] == (
        "invalid active_sources or volume"
    )
    nonfinite_volume_entries = [
        {
            **entry,
            "message": (
                str(entry["message"]).replace("|volume=1", "|volume=nan")
                if "at_ms=1500|volume=1" in str(entry["message"])
                else entry["message"]
            ),
        }
        for entry in healthy_audio_entries
    ]
    nonfinite_volume = audio_probe_check(
        nonfinite_volume_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert nonfinite_volume["pass"] is False
    assert nonfinite_volume["invalid_observations"][0]["reason"] == (
        "invalid active_sources or volume"
    )
    duplicate_audio_entries = [
        {
            **entry,
            "message": (
                str(entry["message"]).replace("at_ms=1500", "at_ms=1000")
                if "GC_METRICS|audio" in str(entry["message"])
                else entry["message"]
            ),
        }
        for entry in healthy_audio_entries
    ]
    duplicate_audio = audio_probe_check(
        duplicate_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert duplicate_audio["pass"] is False
    assert duplicate_audio["complete"] is False
    assert duplicate_audio["ordered"] is False
    out_of_order_audio_entries = [
        {
            **entry,
            "message": (
                str(entry["message"]).replace("at_ms=2000", "at_ms=1250")
                if "GC_METRICS|audio" in str(entry["message"])
                else entry["message"]
            ),
        }
        for entry in healthy_audio_entries
    ]
    out_of_order_audio = audio_probe_check(
        out_of_order_audio_entries,
        {"user_activation": {"has_been_active": True}},
    )
    assert out_of_order_audio["pass"] is False
    assert out_of_order_audio["complete"] is False
    assert out_of_order_audio["ordered"] is False
    fake_driver = FakeChromeDriver()
    chrome_entries: list[dict[str, Any]] = []
    assert route_sequence(flow_console(fake_driver, "chrome", "flow", chrome_entries)) == [
        "title"
    ]
    assert route_sequence(flow_console(fake_driver, "chrome", "flow", chrome_entries)) == [
        "title"
    ]
    memory_driver = MemoryOrderDriver()
    memory = collect_memory(memory_driver, "chrome", -1, "t0")
    assert memory_driver.actions == [
        "marker_start",
        "HeapProfiler.collectGarbage",
        "Performance.getMetrics",
        "Runtime.getHeapUsage",
        "page_state",
        "marker_end",
    ]
    assert memory["js_heap_bytes"] == 100
    assert memory["js_heap_source"] == "cdp Performance.getMetrics.JSHeapUsedSize"
    assert memory["runtime_heap_usage"]["usedSize"] == 101
    assert memory["target_counts"] == {"documents": 1, "frames": 1}
    final_driver = FinalCaptureDriver()
    final_entries, final_state = final_flow_capture(final_driver, "chrome", [])
    assert final_driver.actions == ["get_log", "page_state", "get_log"]
    assert final_state == {"status": "running"}
    assert any(
        entry.get("level") == "error" and entry.get("message") == "late severe failure"
        for entry in final_entries
    )
    accumulated = [
        {
            "level": "info",
            "message": "GC_METRICS|route|route=match",
            "phase": "flow",
        }
    ]
    failure_driver = FailureDriver()
    failure_state, failure_entries = failure_diagnostics(
        failure_driver,
        "chrome",
        accumulated,
    )
    assert failure_driver.actions == ["page_state", "get_log"]
    assert failure_state == {}
    assert route_sequence(failure_entries) == ["match"]
    assert any(
        entry.get("level") == "error"
        and entry.get("message") == "failure-path severe entry"
        for entry in failure_entries
    )
    assert performance_metric(
        {"metrics": [{"name": "JSHeapUsedSize", "value": 100}]},
        "JSHeapUsedSize",
    ) == 100
    assert values_within_percent(100, 104, HEAP_CROSSCHECK_TOLERANCE_PERCENT) is True
    assert values_within_percent(100, 106, HEAP_CROSSCHECK_TOLERANCE_PERCENT) is False
    assert growth_percent(100, 125) == 25
    firefox_actions = firefox_flow_actions(100, 175, 600)
    assert firefox_actions == [
        (111, "keyboard", "m"),
        (175, "memory", "t0"),
        (475, "memory", "t5"),
        (755, "keyboard", "m"),
        (775, "memory", "t10"),
    ]
    assert chrome_post_flow_actions(
        float("inf"),
        755,
        [(175, "t0"), (475, "t5"), (775, "t10")],
        1,
        None,
    ) == [
        (475, "memory", "t5"),
        (755, "keyboard", "m"),
        (775, "memory", "t10"),
    ]
    assert chrome_post_flow_actions(
        111,
        None,
        [(175, "t0")],
        0,
        170,
    ) == [
        (111, "keyboard", "m"),
        (170, "trace", "performance"),
        (175, "memory", "t0"),
    ]
    marker_driver = FakeFirefoxWaitDriver(
        {"error": None, "matched": True, "status": "running"}
    )
    assert (
        wait_for_firefox_flow_marker(
            marker_driver,
            "GC_METRICS|flow_complete|",
            2,
        )
        is True
    )
    assert marker_driver.timeout == 7
    assert marker_driver.arguments == ("GC_METRICS|flow_complete|", 2000)
    chunk_driver = FakeFirefoxChunkDriver()
    chunk_matched, chunk_waits = wait_for_firefox_flow_until(
        chunk_driver,
        "GC_METRICS|flow_complete|",
        time.monotonic() + 300,
    )
    assert chunk_matched is True
    assert chunk_waits == 3
    assert chunk_driver.timeouts == [95, 95, 95]
    assert chunk_driver.arguments == [
        ("GC_METRICS|flow_complete|", 90000),
        ("GC_METRICS|flow_complete|", 90000),
        ("GC_METRICS|flow_complete|", 90000),
    ]
    timeout_driver = FakeFirefoxWaitDriver(
        {"error": None, "matched": False, "status": "running"}
    )
    assert (
        wait_for_firefox_flow_marker(
            timeout_driver,
            "GC_METRICS|flow_complete|",
            2,
        )
        is False
    )
    failed_driver = FakeFirefoxWaitDriver(
        {"error": None, "matched": False, "status": "failed"}
    )
    try:
        wait_for_firefox_flow_marker(
            failed_driver,
            "GC_METRICS|flow_complete|",
            2,
        )
        raise AssertionError("failed Firefox runtime was accepted")
    except RuntimeError as error:
        assert "changed to failed" in str(error)
    error_driver = FakeFirefoxWaitDriver(
        {
            "error": "GC_BROWSER|error|message=failed",
            "matched": False,
            "status": "running",
        }
    )
    try:
        wait_for_firefox_flow_marker(
            error_driver,
            "GC_METRICS|flow_complete|",
            2,
        )
        raise AssertionError("Firefox runtime error was accepted")
    except RuntimeError as error:
        assert "GC_BROWSER|error|message=failed" in str(error)
    synthetic = [
        {
            "level": "log",
            "message": "GC_METRICS|sample_start|sample=1",
        },
        {
            "level": "log",
            "message": (
                "GC_METRICS|sample|sample=1|partial=false|duration_s=60"
                "|update_p95_ms=1|update_max_ms=2|draw_p95_ms=1|draw_max_ms=2"
                "|frame_over_33_ms=0|frame_over_250_ms=0"
            ),
        },
        {
            "level": "log",
            "message": "GC_METRICS|input_latency|latency_ms=2",
        },
        {
            "level": "log",
            "message": "GC_METRICS|flow_complete|at_ms=80000",
        },
        {
            "level": "log",
            "message": "GC_METRICS|sample_start|sample=2",
        },
        {
            "level": "log",
            "message": "GC_BROWSER|memory_probe|phase=start|label=t0",
        },
        {
            "level": "log",
            "message": (
                "GC_METRICS|sample|sample=2|partial=false|duration_s=60"
                "|update_p95_ms=1|update_max_ms=2|draw_p95_ms=1|draw_max_ms=2"
                "|frame_over_33_ms=0|frame_over_250_ms=1"
            ),
        },
    ]
    assert sample_gate(synthetic)["pass"] is True
    full_gate = sample_gate(synthetic, flow_only=False)
    assert full_gate["pass"] is True
    assert full_gate["excluded_probe_samples"] == [2]
    warning = {"level": "warning", "message": "known driver warning"}
    assert console_gate([warning], [re.compile("known driver")])["pass"] is True
    assert console_gate([warning], [])["pass"] is False
    fatal = {"level": "error", "message": "FATAL ERROR: causing a crash"}
    assert console_gate([fatal], [re.compile("FATAL")])["pass"] is False
    terminal_error = {
        "level": "error",
        "message": "GC_BROWSER|error|message=synthetic terminal failure|code=42",
        "phase": "flow_final",
        "timestamp": 123,
    }
    assert console_gate([terminal_error], [])["pass"] is True
    error_health = terminal_runtime_health([terminal_error], {"status": "running"})
    assert error_health["pass"] is False
    assert error_health["error_markers"] == [
        {
            "entry": terminal_error,
            "marker": {
                "source": "GC_BROWSER",
                "kind": "error",
                "message": "synthetic terminal failure",
                "code": "42",
            },
        }
    ]
    non_running_health = terminal_runtime_health([], {"status": "failed"})
    assert console_gate([], [])["pass"] is True
    assert non_running_health == {
        "error_markers": [],
        "pass": False,
        "status": "failed",
    }
    healthy_health = terminal_runtime_health(
        [{"level": "log", "message": "GC_BROWSER|first_frame|at_ms=12"}],
        {"status": "running"},
    )
    assert healthy_health == {
        "error_markers": [],
        "pass": True,
        "status": "running",
    }
    healthy_short_checks = {
        name: {"pass": True}
        for name in (
            "artifact_boot",
            "audio_after_gesture",
            "browser_cleanup",
            "clean_console",
            "complete_flow",
            "fullscreen",
            "hardware_acceleration",
            "keyboard",
            "letterboxing",
            "lifecycle",
            "performance",
            "persistence",
            "storage_unavailable",
        )
    }
    healthy_short_result = {
        "checks": {
            **healthy_short_checks,
            "terminal_runtime_health": healthy_health,
        },
        "web_report_exit_code": 0,
    }
    assert result_passes(healthy_short_result, require_stability=False) is True
    assert (
        result_passes(
            {
                **healthy_short_result,
                "checks": {
                    **healthy_short_result["checks"],
                    "browser_cleanup": {"pass": False},
                },
            },
            require_stability=False,
        )
        is False
    )
    assert (
        result_passes(
            healthy_short_result,
            require_stability=False,
            require_gamepad=True,
        )
        is False
    )
    assert (
        result_passes(
            {
                **healthy_short_result,
                "checks": {
                    **healthy_short_result["checks"],
                    "gamepad": {"pass": True},
                },
            },
            require_stability=False,
            require_gamepad=True,
        )
        is True
    )
    assert (
        result_passes(
            {
                **healthy_short_result,
                "checks": {
                    **healthy_short_checks,
                    "terminal_runtime_health": error_health,
                },
            },
            require_stability=False,
        )
        is False
    )
    assert (
        result_passes(
            {
                **healthy_short_result,
                "checks": {
                    **healthy_short_checks,
                    "terminal_runtime_health": non_running_health,
                },
            },
            require_stability=False,
        )
        is False
    )
    healthy_long_checks = {
        **healthy_short_result["checks"],
        "acceptance_isolation": {"pass": True},
        "gamepad": {"pass": True},
        "memory": {"pass": True},
        "runtime_stability": {"pass": True},
        "stability_performance": {"pass": True},
    }
    healthy_long_result = {
        **healthy_short_result,
        "checks": healthy_long_checks,
    }
    assert result_passes(healthy_long_result, require_stability=True) is True
    missing_firefox_heap_result = {
        **healthy_long_result,
        "checks": {
            **healthy_long_checks,
            "memory": {"pass": None},
        },
    }
    assert result_passes(missing_firefox_heap_result, require_stability=True) is False
    assert (
        result_passes(
            missing_firefox_heap_result,
            require_stability=True,
            external_memory_companion=True,
        )
        is True
    )
    extractor_source = firefox_heap_extractor_source(
        [
            {
                "heap_snapshot": {"path": r"C:\evidence\heap-t0.fxsnapshot"},
                "label": "t0",
            }
        ],
        Path(r"C:\evidence"),
    )
    assert "ChromeUtils.readHeapSnapshot" in extractor_source
    assert 'breakdown: { by: "count", count: true, bytes: true }' in extractor_source
    assert "snapshot_creation_time_us" in extractor_source
    assert "snapshot_sha256" in extractor_source
    assert "heap-t0-census.json" in extractor_source
    assert firefox_heap_snapshot_timing([1_000_000, 301_000_000, 601_000_000]) == {
        "offsets_seconds": [0.0, 300.0, 600.0],
        "pass": True,
    }
    assert (
        firefox_heap_snapshot_timing([1_000_000, 317_000_000, 601_000_000])["pass"]
        is False
    )
    assert (
        firefox_heap_snapshot_timing([1_000_000, 301_000_000, 301_000_000])["pass"]
        is False
    )

    class FakeStoppedProcess:
        pid = 123

        @staticmethod
        def poll() -> int:
            return 0

    class FakeService:
        process = FakeStoppedProcess()

    class FakeQuitDriver:
        service = FakeService()

        def __init__(self, fail: bool = False) -> None:
            self.fail = fail

        def quit(self) -> None:
            if self.fail:
                raise RuntimeError("synthetic quit failure")

    assert quit_browser_bounded(FakeQuitDriver(), 1)["pass"] is True
    failed_quit = quit_browser_bounded(FakeQuitDriver(fail=True), 1)
    assert failed_quit["pass"] is False
    assert failed_quit["error"] == "synthetic quit failure"
    print("browser matrix self-test: OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", choices=("chrome", "firefox"))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--driver", type=Path)
    parser.add_argument("--url", default="http://127.0.0.1:8000/")
    parser.add_argument("--viewport", action="append", type=parse_viewport)
    parser.add_argument("--flow-timeout", type=int, default=300)
    parser.add_argument("--stability-seconds", type=int, default=600)
    parser.add_argument(
        "--require-gamepad",
        action="store_true",
        help="require standard-mapped physical A/B observations in every viewport row",
    )
    parser.add_argument(
        "--external-memory-companion",
        action="store_true",
        help=(
            "complete a Firefox long row with its attended offline JS-heap "
            "snapshot companion"
        ),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / ".cache" / "omp0-browser-matrix" / datetime.now().strftime("%Y%m%d-%H%M%S"),
    )
    parser.add_argument(
        "--classify-warning",
        action="append",
        default=[],
        metavar="REGEX",
        help="record a known environment warning without failing the console gate",
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not args.browser:
        parser.error("--browser is required unless --self-test is used")
    if args.flow_timeout <= 0 or args.stability_seconds < 0:
        parser.error("timeouts must be positive (stability may be zero)")
    if args.external_memory_companion and (
        args.browser != "firefox" or args.stability_seconds <= 0
    ):
        parser.error(
            "--external-memory-companion requires Firefox with positive stability seconds"
        )

    binary, driver = resolve_assets(args.browser, args.binary, args.driver)
    viewports = args.viewport or DEFAULT_VIEWPORTS
    base_url = args.url.rstrip("/") + "/"
    manifest = fetch_json(urllib.parse.urljoin(base_url, "manifest.json"))
    if manifest.get("source_dirty") is not False:
        raise RuntimeError("refusing evidence capture from a dirty or unverified source tree")
    validate_manifest(base_url, manifest)
    try:
        warning_classifications = [re.compile(value) for value in args.classify_warning]
    except re.error as error:
        parser.error(f"invalid --classify-warning regex: {error}")
    root = args.output.resolve() / f"{args.browser}-{int(time.time())}"
    root.mkdir(parents=True, exist_ok=False)
    write_json(root / "manifest.json", manifest)
    environment = {
        "asset_resolution_mode": (
            "explicit_override" if args.binary or args.driver else "selenium_manager_stable"
        ),
        "browser_binary": executable_metadata(binary),
        "browser_channel_requested": "stable",
        "browser_driver": executable_metadata(driver),
        "captured_at": utc_now(),
        "clean_profile": True,
        "extensions_disabled": True,
        "hardware_acceleration_requested": True,
        "memory_evidence_mode": (
            "external_firefox_companion"
            if args.external_memory_companion
            else "automated"
        ),
        "os": os_metadata(),
        "physical_gamepad_required": args.require_gamepad,
        "selenium": selenium_metadata(),
        "served_url": base_url,
        "warning_classifications": [pattern.pattern for pattern in warning_classifications],
    }
    windows_gpu = environment["os"].get("windows_video_controllers", {})
    environment_checks = {
        "gpu_metadata": {
            "pass": (
                windows_gpu.get("available") is True
                and bool(windows_gpu.get("adapters"))
                if os.name == "nt"
                else True
            ),
            "required": os.name == "nt",
        },
        "stable_browser_requested": {
            "pass": not (args.binary or args.driver),
            "requested": "stable",
            "resolution_mode": (
                "explicit_override"
                if args.binary or args.driver
                else "selenium_manager_stable"
            ),
        },
    }
    write_json(root / "environment.json", environment)

    results = []
    for index, viewport in enumerate(viewports):
        width, height = viewport
        stability = args.stability_seconds if index == 0 else 0
        print(
            f"collecting {args.browser} {width}x{height}"
            + (f" with {stability}s stability" if stability else ""),
            flush=True,
        )
        results.append(
            run_viewport(
                args.browser,
                binary,
                driver,
                base_url,
                viewport,
                root / "viewports" / f"{width}x{height}",
                args.flow_timeout,
                stability,
                index == 0 and args.browser == "chrome",
                manifest,
                warning_classifications,
            )
        )
    automated_pass = all(
        result_passes(
            result,
            require_stability=index == 0 and args.stability_seconds > 0,
            require_gamepad=args.require_gamepad,
            external_memory_companion=(
                args.external_memory_companion
                and index == 0
                and args.stability_seconds > 0
            ),
        )
        for index, result in enumerate(results)
    ) and all(check["pass"] is True for check in environment_checks.values())
    memory_companion: dict[str, Any] = {
        "required": args.external_memory_companion,
        "status": "not_required",
    }
    if args.external_memory_companion:
        if not automated_pass:
            memory_companion["status"] = "skipped_automated_failure"
        else:
            try:
                companion_summary = run_firefox_heap_companion(
                    binary,
                    driver,
                    base_url,
                    root,
                    manifest,
                    args.flow_timeout,
                    environment,
                )
                memory_companion = {
                    "pass": companion_summary.get("pass") is True,
                    "required": True,
                    "status": (
                        "pass" if companion_summary.get("pass") is True else "fail"
                    ),
                    "summary": str(
                        root
                        / "firefox-heap-companion"
                        / "firefox-heap-summary.json"
                    ),
                }
            except Exception as error:
                memory_companion = {
                    "error": {
                        "message": str(error),
                        "type": type(error).__name__,
                    },
                    "pass": False,
                    "required": True,
                    "status": "error",
                }
    packet_pass = automated_pass and (
        not args.external_memory_companion
        or memory_companion.get("pass") is True
    )
    write_json(
        root / "summary.json",
        {
            "automated_pass": automated_pass,
            "browser": args.browser,
            "captured_at": utc_now(),
            "environment": environment,
            "environment_checks": environment_checks,
            "manifest": manifest,
            "memory_companion": memory_companion,
            "pass": packet_pass,
            "results": results,
        },
    )
    print(f"evidence packet: {root}")
    return 0 if packet_pass else 1


if __name__ == "__main__":
    raise SystemExit(main())
