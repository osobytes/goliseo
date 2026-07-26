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
from collections.abc import Callable, Iterable
from concurrent.futures import ThreadPoolExecutor
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
    "hash": "fnv1a64-canonical-snapshot-v11",
    "final_hash": "f5bc4aaade430afb",
    "sequence_digest": "b4e10bdddd965a31",
    "score": "0-0",
    "outcome": "draw",
    "snapshot_bytes": "21937",
    "coverage": "tackle,aerial,keeper,full_time",
    "events": "catch:1,claim:4,header:2,pass:5,reception:1,shot:1,tackle:147,touch:173",
    "love": "11.5.0",
}
REQUIRED_PROTOCOL_FIELDS = {
    "schema": "1",
    "manifest_id": "659947cdd13d7d68",
    "transcript_id": "5772846370555e41",
    "messages": "13",
}
REQUIRED_INPUT_PROTOCOL_FIELDS = {
    "schema": "1",
    "input": "2",
    "history": "6",
    "delay": "3",
    "vectors": "2",
    "guest": "4c85cafb2a373f45",
    "host": "ac5249f8efb62759",
    "max_bytes": "755",
}
ERROR_MARKERS = (
    "GC_BROWSER|error|",
    "GC_BROWSER|window_error|",
    "GC_BROWSER|unhandled_rejection|",
    "GC_DETERMINISM|failure|",
)
# Every browser the sharded workflow must schedule. The gate pins this set, so a
# shard that is never scheduled, is skipped, or is cancelled uploads nothing and
# is detected by name instead of relying on a rolled-up matrix job result.
SHARD_BROWSERS = ("chrome", "firefox")
SHARD_ARTIFACT_PREFIX = "omp1-determinism-"
EVIDENCE_SCHEMA = "omp1-browser-determinism-1"
# The proof is that independent processes agree, so a shard is only evidence if
# it carries at least two records produced by distinct WebDriver process groups.
MINIMUM_FRESH_RUNS = 2


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
    # The pinned set is exhaustive, so an unpinned field would otherwise be a
    # value a browser shard could vary without any shard or gate noticing.
    unpinned = sorted(set(fields) - set(REQUIRED_FIELDS))
    if unpinned:
        raise RuntimeError(f"determinism marker carries unpinned fields: {', '.join(unpinned)}")
    return fields


def parse_protocol_marker(line: str) -> dict[str, str]:
    parts = line.split("|")
    if parts[:2] != ["GC_PROTOCOL", "golden"]:
        raise RuntimeError(f"invalid protocol golden marker: {line}")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, separator, value = part.partition("=")
        if not separator or not key or key in fields:
            raise RuntimeError(f"invalid protocol golden marker field: {part}")
        fields[key] = value
    if fields != REQUIRED_PROTOCOL_FIELDS:
        raise RuntimeError(
            f"protocol golden marker mismatch: "
            f"expected {REQUIRED_PROTOCOL_FIELDS}, got {fields}"
        )
    return fields


def parse_input_protocol_marker(line: str) -> dict[str, str]:
    parts = line.split("|")
    if parts[:2] != ["GC_INPUT_PROTOCOL", "golden"]:
        raise RuntimeError(f"invalid input protocol golden marker: {line}")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, separator, value = part.partition("=")
        if not separator or not key or key in fields:
            raise RuntimeError(f"invalid input protocol golden marker field: {part}")
        fields[key] = value
    if fields != REQUIRED_INPUT_PROTOCOL_FIELDS:
        raise RuntimeError(
            f"input protocol golden marker mismatch: "
            f"expected {REQUIRED_INPUT_PROTOCOL_FIELDS}, got {fields}"
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
        protocol_markers: list[str] = []
        input_protocol_markers: list[str] = []
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
                protocol_markers = [
                    message
                    for message in messages
                    if message.startswith("GC_PROTOCOL|golden|")
                ]
                input_protocol_markers = [
                    message
                    for message in messages
                    if message.startswith("GC_INPUT_PROTOCOL|golden|")
                ]
                break
            time.sleep(0.5)
        if len(markers) != 1:
            raise RuntimeError(
                f"{browser_name} run {run_number} timed out without exactly one result marker"
            )
        if len(protocol_markers) != 1:
            raise RuntimeError(
                f"{browser_name} run {run_number} omitted the unique protocol golden marker"
            )
        if len(input_protocol_markers) != 1:
            raise RuntimeError(
                f"{browser_name} run {run_number} omitted the unique input protocol golden marker"
            )
        if state.get("status") != "running":
            raise RuntimeError(
                f"{browser_name} loader status is {state.get('status')!r}, expected 'running'"
            )
        fields = parse_marker(markers[0])
        protocol_fields = parse_protocol_marker(protocol_markers[0])
        input_protocol_fields = parse_input_protocol_marker(input_protocol_markers[0])
        record = {
            "browser": browser_name,
            "browser_version": str(driver.capabilities.get("browserVersion")),
            "duration_seconds": round(time.monotonic() - started, 3),
            "fields": fields,
            "marker": markers[0],
            "protocol": protocol_fields,
            "protocol_marker": protocol_markers[0],
            "input_protocol": input_protocol_fields,
            "input_protocol_marker": input_protocol_markers[0],
            "run": run_number,
        }
    finally:
        teardown = quit_browser_bounded(driver)
    if record is None:
        raise RuntimeError(f"{browser_name} run {run_number} produced no record")
    record["teardown"] = teardown
    return record


def shard_output_name(browser_name: str) -> str:
    return f"omp1-browser-determinism-{browser_name}.json"


def expected_shard_evidence() -> dict[str, str]:
    """Pinned artifact name -> browser for every shard the gate demands."""
    return {f"{SHARD_ARTIFACT_PREFIX}{name}": name for name in SHARD_BROWSERS}


def require_complete_shards(names: Iterable[str]) -> None:
    """Fail closed unless exactly the pinned shard evidence arrived, once each."""
    seen = list(names)
    expected = expected_shard_evidence()
    duplicates = sorted({name for name in seen if seen.count(name) > 1})
    # No CI evidence tree can trip this: actions/upload-artifact enforces unique
    # names per run and a directory listing cannot repeat an entry. It is part of
    # this function's contract rather than of its caller's input, so the
    # self-test asserts it by calling here directly instead of via the gate.
    if duplicates:
        raise RuntimeError(
            f"browser determinism evidence has duplicate shards: {', '.join(duplicates)}"
        )
    missing = sorted(set(expected) - set(seen))
    if missing:
        raise RuntimeError(
            f"browser determinism evidence is missing shards: {', '.join(missing)}"
        )
    unpinned = sorted(set(seen) - set(expected))
    if unpinned:
        raise RuntimeError(
            f"browser determinism evidence has unpinned shards: {', '.join(unpinned)}"
        )


def marker_matches_golden(fields: Any) -> bool:
    """Whether one record's determinism marker equals the pinned golden.

    Split out so the per-record golden pin is a seam the self-test can switch
    off. It is the stronger of the gate's two cross-runtime checks — it compares
    each runtime against a recorded truth — but it is also the one that makes
    `require_cross_runtime_agreement` unable to fail in a passing CI run. Being
    able to disable exactly this check is what lets the self-test prove the
    other one works rather than assume it.
    """
    return fields == REQUIRED_FIELDS


def load_shard_evidence(
    root: Path,
    artifact_name: str,
    browser_name: str,
    revision: str,
    marker_check: Callable[[Any], bool] = marker_matches_golden,
) -> Any:
    """Load one shard's evidence, rejecting anything that is not its own proof."""
    path = root / artifact_name / shard_output_name(browser_name)
    if not path.is_file():
        raise RuntimeError(f"shard {artifact_name} did not upload {path.name}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"shard {artifact_name} evidence is unreadable: {error}") from error
    if not isinstance(payload, dict):
        raise RuntimeError(f"shard {artifact_name} evidence is not an object")
    if payload.get("schema") != EVIDENCE_SCHEMA:
        raise RuntimeError(
            f"shard {artifact_name} reports schema {payload.get('schema')!r}, "
            f"expected {EVIDENCE_SCHEMA!r}"
        )
    if payload.get("pass") is not True:
        raise RuntimeError(f"shard {artifact_name} did not pass")
    if payload.get("shard") != browser_name:
        raise RuntimeError(
            f"shard {artifact_name} declares shard {payload.get('shard')!r}, "
            f"expected {browser_name!r}"
        )
    if payload.get("runtime_commit") != RUNTIME_COMMIT:
        raise RuntimeError(f"shard {artifact_name} pinned a different love.js commit")
    if payload.get("source_dirty") is not False:
        raise RuntimeError(f"shard {artifact_name} was produced from a dirty checkout")
    if payload.get("manifest_source_revision") != revision:
        raise RuntimeError(
            f"shard {artifact_name} was built at revision "
            f"{payload.get('manifest_source_revision')!r}, expected {revision!r}"
        )
    records = payload.get("records")
    if not isinstance(records, list) or len(records) < MINIMUM_FRESH_RUNS:
        raise RuntimeError(
            f"shard {artifact_name} carries fewer than {MINIMUM_FRESH_RUNS} fresh executions"
        )
    groups: list[Any] = []
    versions: set[str] = set()
    for record in records:
        if not isinstance(record, dict):
            raise RuntimeError(f"shard {artifact_name} has a malformed record")
        if record.get("browser") != browser_name:
            raise RuntimeError(
                f"shard {artifact_name} carries a {record.get('browser')!r} record"
            )
        if not marker_check(record.get("fields")):
            raise RuntimeError(
                f"shard {artifact_name} run {record.get('run')} did not match the pinned marker"
            )
        if record.get("protocol") != REQUIRED_PROTOCOL_FIELDS:
            raise RuntimeError(
                f"shard {artifact_name} run {record.get('run')} did not match the protocol golden"
            )
        if record.get("input_protocol") != REQUIRED_INPUT_PROTOCOL_FIELDS:
            raise RuntimeError(
                f"shard {artifact_name} run {record.get('run')} "
                "did not match the input protocol golden"
            )
        teardown = record.get("teardown")
        if not isinstance(teardown, dict) or teardown.get("process_group") is None:
            raise RuntimeError(
                f"shard {artifact_name} run {record.get('run')} has no process-group evidence"
            )
        groups.append(teardown["process_group"])
        versions.add(str(record.get("browser_version")))
    if len(set(groups)) != len(groups):
        raise RuntimeError(
            f"shard {artifact_name} reused a browser process group across its fresh runs"
        )
    if len(versions) != 1:
        raise RuntimeError(f"shard {artifact_name} mixed browser versions across its fresh runs")
    return payload


def require_cross_runtime_agreement(payloads: dict[str, Any]) -> tuple[dict[str, str], int]:
    """Assert every fresh execution, in every shard, carries identical fields.

    Deliberately independent of the golden pin `load_shard_evidence` applies per
    record. That one asks "does each runtime match what we recorded"; this one
    asks "do the runtimes match each other". They share no data and no code, so
    a future relaxation of either — a field allowed to vary, a check refactored
    into vacuity — cannot silently retire cross-runtime agreement. Redundant
    while both stand, which is the point of stating it twice in a gate whose
    whole job is proving determinism.
    """
    canonical: dict[str, str] | None = None
    canonical_source = ""
    executions = 0
    for browser_name, payload in sorted(payloads.items()):
        for record in payload["records"]:
            executions += 1
            if canonical is None:
                canonical = record["fields"]
                canonical_source = f"{browser_name} run {record['run']}"
            elif record["fields"] != canonical:
                raise RuntimeError(
                    f"{browser_name} run {record['run']} disagreed with {canonical_source}"
                )
    if canonical is None:
        raise RuntimeError("browser determinism evidence carries no fresh executions")
    return canonical, executions


def aggregate_shards(
    root: Path,
    revision: str,
    shard_result: str,
    marker_check: Callable[[Any], bool] = marker_matches_golden,
) -> dict[str, Any]:
    """Require every pinned shard and re-assert agreement across the runtimes.

    `marker_check` is a seam, not a knob: CI always uses the default pinned
    golden. `shard_gate_self_test` passes a permissive one so a record that is
    structurally valid but carries divergent hashes can reach
    `require_cross_runtime_agreement`, which is the only way to prove that
    assertion still catches divergence on its own.
    """
    # Necessary, never sufficient: GitHub rolls a matrix job up to one result,
    # which cannot see a shard that was never scheduled. The manifest below can.
    if shard_result != "success":
        raise RuntimeError(f"browser determinism shards did not succeed: result={shard_result}")
    if not root.is_dir():
        raise RuntimeError(f"browser determinism evidence root is missing: {root}")
    require_complete_shards(sorted(entry.name for entry in root.iterdir() if entry.is_dir()))
    payloads: dict[str, Any] = {}
    for artifact_name, browser_name in sorted(expected_shard_evidence().items()):
        payloads[browser_name] = load_shard_evidence(
            root, artifact_name, browser_name, revision, marker_check
        )
    canonical, executions = require_cross_runtime_agreement(payloads)
    print(
        f"browser determinism gate: {executions} fresh executions across "
        f"{len(payloads)} shards agree at {canonical['final_hash']}/"
        f"{canonical['sequence_digest']}"
    )
    return {"final_hash": canonical["final_hash"], "shards": sorted(payloads)}


def build_shard_fixture(root: Path, revision: str) -> None:
    group = 1000
    for browser_name in SHARD_BROWSERS:
        records = []
        for run_number in (1, 2):
            group += 1
            records.append(
                {
                    "browser": browser_name,
                    "browser_version": "1.0",
                    "duration_seconds": 1.0,
                    "fields": dict(REQUIRED_FIELDS),
                    "marker": "GC_DETERMINISM|result|",
                    "protocol": dict(REQUIRED_PROTOCOL_FIELDS),
                    "protocol_marker": "GC_PROTOCOL|golden|",
                    "input_protocol": dict(REQUIRED_INPUT_PROTOCOL_FIELDS),
                    "input_protocol_marker": "GC_INPUT_PROTOCOL|golden|",
                    "run": run_number,
                    "teardown": {"process_group": group},
                }
            )
        directory = root / f"{SHARD_ARTIFACT_PREFIX}{browser_name}"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / shard_output_name(browser_name)).write_text(
            json.dumps(
                {
                    "manifest_source_revision": revision,
                    "pass": True,
                    "records": records,
                    "runtime_commit": RUNTIME_COMMIT,
                    "schema": EVIDENCE_SCHEMA,
                    "shard": browser_name,
                    "source_dirty": False,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )


def shard_gate_self_test() -> None:
    revision = "a" * 40
    with tempfile.TemporaryDirectory(prefix="gc-determinism-gate-self-test-") as temp:
        root = Path(temp) / "evidence"
        build_shard_fixture(root, revision)
        aggregate_shards(root, revision, "success")

        for result in ("failure", "cancelled", "skipped", ""):
            try:
                aggregate_shards(root, revision, result)
            except RuntimeError:
                pass
            else:
                raise RuntimeError(f"gate accepted a {result!r} shard result")

        # A shard that was never scheduled uploads nothing; drop each in turn.
        for artifact_name in expected_shard_evidence():
            hostile = Path(temp) / f"missing-{artifact_name}"
            build_shard_fixture(hostile, revision)
            dropped = hostile / artifact_name
            for child in dropped.iterdir():
                child.unlink()
            dropped.rmdir()
            try:
                aggregate_shards(hostile, revision, "success")
            except RuntimeError as error:
                if artifact_name not in str(error):
                    raise RuntimeError(f"gate failure did not name {artifact_name}") from error
            else:
                raise RuntimeError(f"gate accepted a run without {artifact_name}")

        unpinned = Path(temp) / "unpinned"
        build_shard_fixture(unpinned, revision)
        (unpinned / f"{SHARD_ARTIFACT_PREFIX}webkit").mkdir()
        try:
            aggregate_shards(unpinned, revision, "success")
        except RuntimeError:
            pass
        else:
            raise RuntimeError("gate accepted an unpinned shard")

        empty = Path(temp) / "empty"
        empty.mkdir()
        try:
            aggregate_shards(empty, revision, "success")
        except RuntimeError:
            pass
        else:
            raise RuntimeError("gate accepted a run with no shard evidence at all")

        mutations: tuple[tuple[str, Any], ...] = (
            ("failed shard", lambda payload: payload.update(**{"pass": False})),
            ("stale contract", lambda payload: payload.update(schema="omp1-legacy")),
            ("cross runtime", lambda payload: payload.update(shard="firefox")),
            ("cross revision", lambda payload: payload.update(manifest_source_revision="b" * 40)),
            ("dirty checkout", lambda payload: payload.update(source_dirty=True)),
            ("foreign runtime commit", lambda payload: payload.update(runtime_commit="0" * 40)),
            ("single execution", lambda payload: payload.update(records=payload["records"][:1])),
            (
                "reused process",
                lambda payload: payload["records"][1]["teardown"].update(
                    process_group=payload["records"][0]["teardown"]["process_group"]
                ),
            ),
            (
                "mixed versions",
                lambda payload: payload["records"][1].update(browser_version="2.0"),
            ),
            (
                "foreign record",
                lambda payload: payload["records"][1].update(browser="firefox"),
            ),
            (
                "unpinned marker",
                lambda payload: payload["records"][1]["fields"].update(final_hash="0" * 16),
            ),
            (
                "protocol drift",
                lambda payload: payload["records"][1]["protocol"].update(messages="12"),
            ),
            (
                "input protocol drift",
                lambda payload: payload["records"][1]["input_protocol"].update(max_bytes="756"),
            ),
        )
        for label, mutate in mutations:
            hostile = Path(temp) / f"hostile-{label.replace(' ', '-')}"
            build_shard_fixture(hostile, revision)
            target = hostile / f"{SHARD_ARTIFACT_PREFIX}chrome" / shard_output_name("chrome")
            payload = json.loads(target.read_text(encoding="utf-8"))
            mutate(payload)
            target.write_text(
                json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
            )
            try:
                aggregate_shards(hostile, revision, "success")
            except RuntimeError:
                pass
            else:
                raise RuntimeError(f"gate accepted {label} evidence")

        # A directory listing cannot repeat an entry, so the aggregate path can
        # never reach the duplicate branch. Assert its contract where it lives.
        try:
            require_complete_shards(sorted(expected_shard_evidence()) * 2)
        except RuntimeError as error:
            if "duplicate" not in str(error):
                raise RuntimeError("duplicate shard names failed for the wrong reason") from error
        else:
            raise RuntimeError("gate accepted duplicate shard evidence")

        # Cross-runtime divergence is the assertion the single-job runner made
        # in process. The whole gate must reject it...
        diverged = Path(temp) / "diverged"
        build_shard_fixture(diverged, revision)
        target = diverged / f"{SHARD_ARTIFACT_PREFIX}firefox" / shard_output_name("firefox")
        payload = json.loads(target.read_text(encoding="utf-8"))
        for record in payload["records"]:
            record["fields"]["sequence_digest"] = "0" * 16
        target.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        try:
            aggregate_shards(diverged, revision, "success")
        except RuntimeError:
            pass
        else:
            raise RuntimeError("gate accepted runtimes that disagreed with each other")

        # ...but that alone proves nothing about which check did the rejecting:
        # the per-record golden pin fires first and would hide a
        # require_cross_runtime_agreement that had rotted into a no-op. Switch
        # the pin off, leaving records that are structurally valid but carry
        # divergent hashes, and require the comparison to catch them by itself.
        def accept_any_marker(_fields: Any) -> bool:
            return True

        control = Path(temp) / "unpinned-control"
        build_shard_fixture(control, revision)
        aggregate_shards(control, revision, "success", accept_any_marker)
        try:
            aggregate_shards(diverged, revision, "success", accept_any_marker)
        except RuntimeError as error:
            if "disagreed with" not in str(error):
                raise RuntimeError(
                    "cross-runtime divergence failed for the wrong reason"
                ) from error
        else:
            raise RuntimeError("the cross-runtime comparison does not catch divergence on its own")

        # The mirror image, which pins the other assertion: every shard agreeing
        # on a hash that is not the recorded one. The comparison above is blind
        # to that by construction, so only the golden pin can reject it.
        agreed_wrong = Path(temp) / "agreed-wrong"
        build_shard_fixture(agreed_wrong, revision)
        for browser_name in SHARD_BROWSERS:
            wrong = (
                agreed_wrong
                / f"{SHARD_ARTIFACT_PREFIX}{browser_name}"
                / shard_output_name(browser_name)
            )
            payload = json.loads(wrong.read_text(encoding="utf-8"))
            for record in payload["records"]:
                record["fields"]["final_hash"] = "0" * 16
            wrong.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        try:
            aggregate_shards(agreed_wrong, revision, "success")
        except RuntimeError as error:
            if "pinned marker" not in str(error):
                raise RuntimeError("agreed-wrong evidence failed for the wrong reason") from error
        else:
            raise RuntimeError("gate accepted shards agreeing on a hash the golden does not pin")
    print("browser determinism shard gate self-test: OK")


def self_test() -> None:
    protocol_marker = (
        "GC_PROTOCOL|golden|schema=1|manifest_id=659947cdd13d7d68"
        "|transcript_id=5772846370555e41|messages=13"
    )
    if parse_protocol_marker(protocol_marker) != REQUIRED_PROTOCOL_FIELDS:
        raise RuntimeError("protocol golden marker self-test failed")
    try:
        parse_protocol_marker(protocol_marker.replace("messages=13", "messages=12"))
    except RuntimeError:
        pass
    else:
        raise RuntimeError("protocol golden marker accepted a changed vector count")
    input_protocol_marker = (
        "GC_INPUT_PROTOCOL|golden|schema=1|input=2|history=6|delay=3|vectors=2"
        "|guest=4c85cafb2a373f45|host=ac5249f8efb62759|max_bytes=755"
    )
    if parse_input_protocol_marker(input_protocol_marker) != REQUIRED_INPUT_PROTOCOL_FIELDS:
        raise RuntimeError("input protocol golden marker self-test failed")
    try:
        parse_input_protocol_marker(input_protocol_marker.replace("max_bytes=755", "max_bytes=756"))
    except RuntimeError:
        pass
    else:
        raise RuntimeError("input protocol golden marker accepted a changed maximum size")
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
    try:
        parse_marker(
            "GC_DETERMINISM|result|"
            + "|".join(f"{key}={value}" for key, value in REQUIRED_FIELDS.items())
            + "|unpinned=1"
        )
    except RuntimeError:
        pass
    else:
        raise RuntimeError("marker parser accepted an unpinned field")

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
    shard_gate_self_test()
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
    parser.add_argument(
        "--run-concurrency",
        type=int,
        default=1,
        help=(
            "Fresh processes to run at once within a browser. They stay fully "
            "independent and the comparison is hash equality, so contention "
            "cannot alter the result (default: 1)."
        ),
    )
    parser.add_argument("--timeout-seconds", type=int, default=1200)
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--mode",
        choices=("campaign", "aggregate"),
        default="campaign",
        help="campaign runs the browsers; aggregate gates the uploaded shard evidence.",
    )
    parser.add_argument("--evidence-root", type=Path)
    parser.add_argument("--revision")
    parser.add_argument("--shard-result", default="")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if args.mode == "aggregate":
        if args.evidence_root is None or not args.revision:
            parser.error("--evidence-root and --revision are required for --mode aggregate")
        aggregate_shards(args.evidence_root.resolve(), args.revision, args.shard_result)
        return 0
    if args.artifact is None or args.output is None:
        parser.error("--artifact and --output are required unless --self-test is used")
    browsers = args.browsers or list(SHARD_BROWSERS)
    if args.runs < 2:
        raise SystemExit("--runs must be at least 2")
    if args.run_concurrency < 1:
        raise SystemExit("--run-concurrency must be at least 1")

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
            run_numbers = range(1, args.runs + 1)

            def execute(run_number: int, name: str = browser_name) -> dict[str, Any]:
                return run_once(
                    name,
                    binary,
                    driver_path,
                    url,
                    log_dir,
                    run_number,
                    args.timeout_seconds,
                )

            if args.run_concurrency > 1:
                # Each run owns a separate browser process group and writes its
                # own WebDriver log, so the runs share nothing but the artifact.
                with ThreadPoolExecutor(max_workers=args.run_concurrency) as pool:
                    browser_records = list(pool.map(execute, run_numbers))
            else:
                browser_records = [execute(run_number) for run_number in run_numbers]
            groups = [record["teardown"]["process_group"] for record in browser_records]
            if len(set(groups)) != len(groups):
                raise RuntimeError(f"fresh {browser_name} runs shared a browser process group")
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
            "run_concurrency": args.run_concurrency,
            "runtime_commit": RUNTIME_COMMIT,
            "schema": EVIDENCE_SCHEMA,
            "shard": browsers[0] if len(browsers) == 1 else None,
            "source_dirty": manifest.get("source_dirty"),
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
