#!/usr/bin/env python3
"""Run the frozen OMP-1 Lua determinism suite in real Chrome and Firefox."""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import tempfile
import threading
import time
import urllib.parse
from datetime import UTC, datetime
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any

from browser_matrix import resolve_assets, validate_manifest
from web_serve import ArtifactHandler


ROOT = Path(__file__).resolve().parents[1]
RUNTIME_COMMIT = "495c5eb7eb55b54aaadfc21405c58f50a6d819c4"
RUNTIME_REPOSITORY = "https://github.com/2dengine/love.js"
RUNTIME_ARCHIVE_SHA256 = "89b56e7953935d6cb06c454d0ee0c0d8903e433b9a94d1d6d501fb8b516f5ff6"
REQUIRED_FIELDS = {
    "schema": "1",
    "fixture": "omp1-nebula-orion-eight-streams-v2",
    "build": "omp1-determinism-v1",
    "source": "issue-39-canonical-recording-v1",
    "content": "nebula-orion-showcase-content-v1",
    "config": "field=960x540;duration=120;max_goals=3;tick_rate=60",
    "tuning": "defaults",
    "seed": "19",
    "tick_rate": "60",
    "ticks": "7201",
    "boundaries": "7202",
    "hash": "fnv1a64-canonical-snapshot-v8",
    "final_hash": "459b141c4c0b8729",
    "sequence_digest": "1ab003a12c1f9a19",
    "score": "0-0",
    "outcome": "draw",
    "snapshot_bytes": "22488",
    "coverage": "tackle,aerial,keeper,full_time",
    "events": "catch:1,claim:4,header:2,pass:5,reception:1,shot:1,tackle:147,touch:173",
    "love": "11.5.0",
}
ERROR_MARKERS = (
    "GC_BROWSER|error|",
    "GC_BROWSER|window_error|",
    "GC_BROWSER|unhandled_rejection|",
    "GC_DETERMINISM|failure|",
)


def parse_marker(line: str) -> dict[str, str]:
    parts = line.split("|")
    if parts[:2] != ["GC_DETERMINISM", "result"]:
        raise RuntimeError(f"invalid determinism marker: {line}")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, separator, value = part.partition("=")
        if not separator or not key or key in fields:
            raise RuntimeError(f"invalid determinism marker field: {part}")
        fields[key] = value
    for key, expected in REQUIRED_FIELDS.items():
        if fields.get(key) != expected:
            raise RuntimeError(
                f"determinism marker {key} mismatch: expected {expected}, got {fields.get(key)}"
            )
    return fields


def validate_provenance(manifest: dict[str, Any], allow_dirty: bool) -> None:
    source_dirty = manifest.get("source_dirty")
    if not isinstance(source_dirty, bool):
        raise RuntimeError("browser artifact source_dirty must be a boolean")
    if source_dirty and not allow_dirty:
        raise RuntimeError("browser determinism refuses a dirty source manifest")
    runtime = manifest.get("runtime")
    if not isinstance(runtime, dict):
        raise RuntimeError("browser artifact runtime metadata is missing")
    expected = {
        "archive_sha256": RUNTIME_ARCHIVE_SHA256,
        "commit": RUNTIME_COMMIT,
        "repository": RUNTIME_REPOSITORY,
    }
    for key, value in expected.items():
        if runtime.get(key) != value:
            raise RuntimeError(
                f"browser artifact runtime {key} mismatch: "
                f"expected {value}, got {runtime.get(key)}"
            )


def driver_version(path: Path) -> str:
    result = subprocess.run(
        [str(path), "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    return (result.stdout or result.stderr).strip()


def chrome_arguments(ci: bool) -> tuple[str, ...]:
    arguments = [
        "--headless=new",
        "--disable-dev-shm-usage",
        "--disable-extensions",
        "--no-default-browser-check",
        "--no-first-run",
    ]
    if ci:
        arguments.append("--no-sandbox")
    return tuple(arguments)


def firefox_arguments(ci: bool) -> tuple[str, ...]:
    if ci:
        return ()
    return ("-headless",)


def firefox_preferences(ci: bool) -> dict[str, bool | int]:
    preferences: dict[str, bool | int] = {
        "extensions.autoDisableScopes": 15,
        "extensions.enabledScopes": 0,
    }
    if ci:
        preferences.update(
            {
                "webgl.force-enabled": True,
                "gfx.webrender.software": True,
                "gfx.x11-egl.force-disabled": True,
            }
        )
    return preferences


def bounded_log_tail(path: Path, max_lines: int = 40, max_characters: int = 6000) -> str:
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        return f"<webdriver log unavailable: {error}>"
    tail = "\n".join(lines[-max_lines:])
    if len(tail) > max_characters:
        tail = "<truncated>\n" + tail[-max_characters:]
    return tail or "<webdriver log empty>"


def launch(browser_name: str, binary: Path, driver: Path, log: Path) -> Any:
    if browser_name == "chrome":
        from selenium import webdriver
        from selenium.webdriver.chrome.options import Options
        from selenium.webdriver.chrome.service import Service

        options = Options()
        options.binary_location = str(binary)
        for argument in chrome_arguments(os.environ.get("CI") == "true"):
            options.add_argument(argument)
        return webdriver.Chrome(
            service=Service(
                str(driver),
                log_output=str(log),
                popen_kw={"start_new_session": True},
            ),
            options=options,
        )

    from selenium import webdriver
    from selenium.webdriver.firefox.options import Options
    from selenium.webdriver.firefox.service import Service

    options = Options()
    options.binary_location = str(binary)
    ci = os.environ.get("CI") == "true"
    for argument in firefox_arguments(ci):
        options.add_argument(argument)
    for name, value in firefox_preferences(ci).items():
        options.set_preference(name, value)
    return webdriver.Firefox(
        service=Service(
            str(driver),
            log_output=str(log),
            popen_kw={"start_new_session": True},
        ),
        options=options,
    )


def wait_process(process: Any, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if process.poll() is not None:
            return True
        time.sleep(0.05)
    return process.poll() is not None


def process_group_alive(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    return True


def wait_group_gone(group_alive: Any, pgid: int, timeout_seconds: float) -> bool:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if not group_alive(pgid):
            return True
        time.sleep(0.05)
    return not group_alive(pgid)


def quit_browser_bounded(
    driver: Any,
    timeout_seconds: float = 30,
    term_wait_seconds: float = 5,
    kill_wait_seconds: float = 5,
    getpgid: Any = os.getpgid,
    killpg: Any = os.killpg,
    group_alive: Any = process_group_alive,
) -> dict[str, Any]:
    process = getattr(getattr(driver, "service", None), "process", None)
    if process is None or getattr(process, "pid", None) is None:
        raise RuntimeError("WebDriver service process is unavailable for bounded teardown")
    pgid = getpgid(process.pid)
    quit_errors: list[str] = []

    def quit_driver() -> None:
        try:
            driver.quit()
        except Exception as error:  # teardown must still reap the process group
            quit_errors.append(str(error))

    thread = threading.Thread(target=quit_driver, daemon=True)
    thread.start()
    thread.join(timeout_seconds)
    service_exited = wait_process(process, 1)
    group_exited = wait_group_gone(group_alive, pgid, 1)
    fallback = thread.is_alive() or not service_exited or not group_exited
    signals: list[str] = []
    if fallback:
        try:
            killpg(pgid, signal.SIGTERM)
            signals.append("TERM")
        except ProcessLookupError:
            pass
        wait_process(process, term_wait_seconds)
        if not wait_group_gone(group_alive, pgid, term_wait_seconds):
            try:
                killpg(pgid, signal.SIGKILL)
                signals.append("KILL")
            except ProcessLookupError:
                pass
            wait_process(process, kill_wait_seconds)
            if not wait_group_gone(group_alive, pgid, kill_wait_seconds):
                raise RuntimeError(
                    f"WebDriver process group {pgid} survived bounded TERM/KILL teardown"
                )
    return {
        "fallback": fallback,
        "process_group": pgid,
        "quit_error": quit_errors[0] if quit_errors else None,
        "service_exit_code": process.poll(),
        "signals": signals,
    }


def console_state(driver: Any) -> dict[str, Any]:
    value = driver.execute_script(
        """
        const state = window.__GALACTIC_CUP__ || {};
        return {
          status: state.status || null,
          entries: (state.console_entries || []).map((entry) => String(entry.message || ""))
        };
        """
    )
    if not isinstance(value, dict):
        raise RuntimeError("browser returned malformed compatibility state")
    return value


def run_once(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    url: str,
    output: Path,
    run_number: int,
    timeout_seconds: int,
) -> dict[str, Any]:
    started = time.monotonic()
    log = output / f"{browser_name}-{run_number}-webdriver.log"
    try:
        driver = launch(browser_name, binary, driver_path, log)
    except Exception as error:
        raise RuntimeError(
            f"{browser_name} run {run_number} launch failed: {error}\n"
            f"bounded webdriver log tail:\n{bounded_log_tail(log)}"
        ) from error
    record: dict[str, Any] | None = None
    try:
        driver.set_page_load_timeout(90)
        driver.get(url)
        deadline = time.monotonic() + timeout_seconds
        state: dict[str, Any] = {}
        markers: list[str] = []
        while time.monotonic() < deadline:
            state = console_state(driver)
            entries = state.get("entries")
            if not isinstance(entries, list):
                raise RuntimeError("browser console entries are malformed")
            messages = [str(entry) for entry in entries]
            failures = [
                message
                for message in messages
                if any(marker in message for marker in ERROR_MARKERS)
            ]
            if failures:
                raise RuntimeError(f"{browser_name} runtime failure: {failures[0]}")
            markers = [
                message
                for message in messages
                if message.startswith("GC_DETERMINISM|result|")
            ]
            if markers:
                break
            time.sleep(0.5)
        if len(markers) != 1:
            raise RuntimeError(
                f"{browser_name} run {run_number} timed out without exactly one result marker"
            )
        if state.get("status") != "running":
            raise RuntimeError(
                f"{browser_name} loader status is {state.get('status')!r}, expected 'running'"
            )
        fields = parse_marker(markers[0])
        record = {
            "browser": browser_name,
            "browser_version": str(driver.capabilities.get("browserVersion")),
            "duration_seconds": round(time.monotonic() - started, 3),
            "fields": fields,
            "marker": markers[0],
            "run": run_number,
        }
    finally:
        teardown = quit_browser_bounded(driver)
    if record is None:
        raise RuntimeError(f"{browser_name} run {run_number} produced no record")
    record["teardown"] = teardown
    return record


def self_test() -> None:
    if "--no-sandbox" not in chrome_arguments(True):
        raise RuntimeError("CI Chrome arguments omit --no-sandbox")
    if "--no-sandbox" in chrome_arguments(False):
        raise RuntimeError("local Chrome arguments unexpectedly disable the sandbox")
    if firefox_arguments(False) != ("-headless",):
        raise RuntimeError("local Firefox arguments omit headless mode")
    if firefox_arguments(True):
        raise RuntimeError("CI Firefox arguments unexpectedly enable headless mode")
    ci_only_firefox_preferences = {
        "webgl.force-enabled": True,
        "gfx.webrender.software": True,
        "gfx.x11-egl.force-disabled": True,
    }
    local_firefox_preferences = firefox_preferences(False)
    expected_ci_firefox_preferences = local_firefox_preferences.copy()
    expected_ci_firefox_preferences.update(ci_only_firefox_preferences)
    if firefox_preferences(True) != expected_ci_firefox_preferences:
        raise RuntimeError("CI Firefox preferences do not match software GL requirements")
    if any(name in local_firefox_preferences for name in ci_only_firefox_preferences):
        raise RuntimeError("local Firefox preferences unexpectedly force software WebGL")

    class FakeProcess:
        def __init__(self) -> None:
            self.pid = 123
            self.return_code: int | None = None

        def poll(self) -> int | None:
            return self.return_code

    class FakeService:
        def __init__(self, process: FakeProcess) -> None:
            self.process = process

    class FakeDriver:
        def __init__(self, process: FakeProcess, blocked: bool) -> None:
            self.service = FakeService(process)
            self.blocked = blocked

        def quit(self) -> None:
            if self.blocked:
                time.sleep(1)
            else:
                self.service.process.return_code = 0

    process = FakeProcess()
    normal = quit_browser_bounded(
        FakeDriver(process, False),
        timeout_seconds=0.01,
        getpgid=lambda _pid: 456,
        killpg=lambda _pgid, _signal: None,
        group_alive=lambda _pgid: False,
    )
    if normal["fallback"] or normal["service_exit_code"] != 0:
        raise RuntimeError("normal bounded-teardown self-test failed")

    process = FakeProcess()
    seen_signals: list[int] = []
    fake_group_alive = True

    def fake_killpg(_pgid: int, sent_signal: int) -> None:
        nonlocal fake_group_alive
        seen_signals.append(sent_signal)
        process.return_code = -sent_signal
        fake_group_alive = False

    fallback = quit_browser_bounded(
        FakeDriver(process, True),
        timeout_seconds=0.01,
        term_wait_seconds=0.01,
        kill_wait_seconds=0.01,
        getpgid=lambda _pid: 789,
        killpg=fake_killpg,
        group_alive=lambda _pgid: fake_group_alive,
    )
    if not fallback["fallback"] or seen_signals != [signal.SIGTERM]:
        raise RuntimeError("forced bounded-teardown self-test failed")

    fields = parse_marker(
        "GC_DETERMINISM|result|"
        + "|".join(f"{key}={value}" for key, value in REQUIRED_FIELDS.items())
    )
    if fields != REQUIRED_FIELDS:
        raise RuntimeError("marker parser self-test failed")

    for invalid_dirty in (None, "false", 0):
        invalid_manifest = {
            "source_dirty": invalid_dirty,
            "runtime": {
                "archive_sha256": RUNTIME_ARCHIVE_SHA256,
                "commit": RUNTIME_COMMIT,
                "repository": RUNTIME_REPOSITORY,
            },
        }
        try:
            validate_provenance(invalid_manifest, allow_dirty=True)
        except RuntimeError:
            pass
        else:
            raise RuntimeError("non-boolean source_dirty passed provenance self-test")

    with tempfile.TemporaryDirectory(prefix="gc-browser-determinism-self-test-") as temp:
        artifact = Path(temp)
        (artifact / "payload.bin").write_bytes(b"tampered")
        bad_hash = "0" * 64
        manifest = {
            "files": {"payload.bin": bad_hash},
            "game_package": {"path": "payload.bin", "sha256": bad_hash},
        }
        server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            lambda *a, **k: ArtifactHandler(*a, directory=str(artifact), **k),
        )
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            try:
                validate_manifest(f"http://127.0.0.1:{server.server_port}/", manifest)
            except RuntimeError:
                pass
            else:
                raise RuntimeError("tampered served artifact passed manifest self-test")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
    print("browser determinism self-test: OK")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--browser",
        action="append",
        choices=("chrome", "firefox"),
        dest="browsers",
        help="Required browser; repeat to select both (default: both).",
    )
    parser.add_argument("--runs", type=int, default=2)
    parser.add_argument("--timeout-seconds", type=int, default=1200)
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.artifact is None or args.output is None:
        parser.error("--artifact and --output are required unless --self-test is used")
    browsers = args.browsers or ["chrome", "firefox"]
    if args.runs < 2:
        raise SystemExit("--runs must be at least 2")

    artifact = args.artifact.resolve()
    manifest = json.loads((artifact / "manifest.json").read_text(encoding="utf-8"))
    validate_provenance(manifest, args.allow_dirty)

    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        lambda *a, **k: ArtifactHandler(*a, directory=str(artifact), **k),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{server.server_port}/"
    try:
        validate_manifest(base_url, manifest)
        query = urllib.parse.urlencode(
            {
                "arg": json.dumps(
                    ["--determinism", "--browser-runtime"],
                    separators=(",", ":"),
                )
            }
        )
        url = f"{base_url}?{query}"
        output = args.output.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        log_dir = output.parent / (output.stem + "-logs")
        log_dir.mkdir(parents=True, exist_ok=True)

        records: list[dict[str, Any]] = []
        assets: dict[str, dict[str, str]] = {}
        for browser_name in browsers:
            binary, driver_path = resolve_assets(browser_name, None, None)
            assets[browser_name] = {
                "binary": str(binary),
                "driver": str(driver_path),
                "driver_version": driver_version(driver_path),
            }
            browser_records = [
                run_once(
                    browser_name,
                    binary,
                    driver_path,
                    url,
                    log_dir,
                    run_number,
                    args.timeout_seconds,
                )
                for run_number in range(1, args.runs + 1)
            ]
            first_fields = browser_records[0]["fields"]
            for record in browser_records[1:]:
                if record["fields"] != first_fields:
                    raise RuntimeError(f"fresh {browser_name} runs disagreed")
            records.extend(browser_records)
        canonical = records[0]["fields"]
        for record in records[1:]:
            if record["fields"] != canonical:
                raise RuntimeError(
                    f"{record['browser']} run {record['run']} disagreed with the runtime matrix"
                )
        result = {
            "artifact": str(artifact),
            "assets": assets,
            "generated_at": datetime.now(UTC).isoformat(),
            "manifest_source_revision": manifest.get("source_revision"),
            "pass": True,
            "records": records,
            "runtime_commit": RUNTIME_COMMIT,
        }
        output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        for record in records:
            print(
                f"{record['browser']} run {record['run']}: "
                f"{record['browser_version']} {record['duration_seconds']}s "
                f"{record['fields']['sequence_digest']}"
            )
        print(f"browser determinism: {len(records)} fresh executions agree")
        return 0
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    raise SystemExit(main())
