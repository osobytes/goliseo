#!/usr/bin/env python3
"""Orchestrate OMP-2 rollback validation in native LÖVE, Chrome, and Firefox."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
from dataclasses import dataclass
from datetime import UTC, datetime
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any, Iterable

from browser_determinism import (
    bounded_log_tail,
    launch,
    quit_browser_bounded,
    validate_provenance,
)
from browser_matrix import (
    executable_metadata,
    os_metadata,
    resolve_assets,
    selenium_metadata,
    validate_manifest,
)
from web_serve import ArtifactHandler


ROOT = Path(__file__).resolve().parents[1]
HISTORICAL_SOCCER_EVIDENCE = (
    ROOT / "docs" / "online" / "evidence" / "omp2_rollback_linux_2026-07-24.json"
)
HISTORICAL_SOCCER_TAPE_DIGEST = "881917e3ba798703"
MARKER_PREFIX = "GC_ROLLBACK_VALIDATION"
METRICS_PREFIX = "GC_ROLLBACK_METRICS"
TIMINGS_PREFIX = "GC_ROLLBACK_TIMINGS"
RESULT_REQUIRED_FIELDS = ("schema", "suite", "success", "logical_digest", "case_count")
NETWORK_SEEDS = (2001, 2002, 2003)
NATIVE_PROFILES = ("clean", "omp0_parity", "playable", "stress")
BROWSER_FULL_PROFILES = ("clean", "playable")
BROWSER_CPU_SCENARIOS = ("complete_fixture", "combat")
BROWSER_CPU_FIXTURES = {
    "complete_fixture": "omp1-nebula-orion-eight-streams-v2",
    "combat": "omp2-combat-rollback-v1",
}
CAMPAIGNS = ("all", "matrix", "soak")
STRESS_PROFILE = "stress"
SCENARIOS = (
    "possession_change",
    "tackle",
    "shot",
    "goal",
    "kickoff",
    "aerial",
    "keeper_action",
    "repeated_rollback",
    "full_time",
)
SOAK_NETWORK_SEEDS = (2001, 2002, 2003, 2001, 2002)
DEFAULT_TIMEOUT_SECONDS = 7200
MIN_BROWSER_SOAK_TIMEOUT_SECONDS = 5400
POLL_SECONDS = 0.2
ERROR_MARKERS = (
    "GC_BROWSER|error|",
    "GC_BROWSER|window_error|",
    "GC_BROWSER|unhandled_rejection|",
    "GC_ROLLBACK_VALIDATION|failure|",
)
SOAK_SAMPLES = ("warmup", "120", "360", "600", "final")
EXTERNAL_MEMORY_SAMPLES = SOAK_SAMPLES
EXPECTED_PROFILE_DIGEST = "5fbf1e0d51a6f4d5"
MAX_MEMORY_GROWTH_RATIO = 0.10
MAX_SNAPSHOT_COUNT = 31
MAX_SNAPSHOT_BYTES = 768 * 1024
MAX_HISTORY_BYTES = 1024 * 1024
MAX_P95_WORK_MS = 16.67
MAX_ROLLBACK_P999_MS = 33.3
MAX_ROLLBACK_P999_US = 33300
ROLLBACK_PERCENTILE = 0.999
GATE_CONTRACT = "5"
MAX_BROWSER_P95_WORK_RATIO = 6.7
MAX_BROWSER_ROLLBACK_P999_RATIO = 11.7
BROWSER_CPU_CALIBRATION_RUNS = ("30060058593", "30065880550")
BROWSER_CPU_DIAGNOSTIC_RUN = "30075505461"
BROWSER_CPU_CALIBRATION_MARGIN = 0.15
BROWSER_CPU_CALIBRATION_MAX_P95_WORK_RATIO = 13.975 / 2.410
BROWSER_CPU_CALIBRATION_MAX_ROLLBACK_P999_RATIO = 24.800 / 2.440
BROWSER_CPU_CASE_FIELDS = frozenset(
    {
        "case",
        "client_hash",
        "cpu_gate",
        "cpu_gate_applied",
        "cpu_gate_mode",
        "event_confirmed_combat",
        "event_confirmed_digest",
        "event_reference_digest",
        "event_residue",
        "expected_failure",
        "fixture",
        "game_gate",
        "gate_contract",
        "hidden_progress",
        "history_gate",
        "initial_hash",
        "lab_success",
        "late_tick",
        "max_depth",
        "network_seed",
        "peak_history_bytes",
        "peak_snapshot_bytes",
        "peak_snapshots",
        "profile",
        "reference_hash",
        "resimulated",
        "rollbacks",
        "sample",
        "scenario",
        "scenario_pass",
        "schema",
        "snapshot_gate",
        "snapshot_version",
        "status",
        "success",
        "tape_digest",
        "tape_version",
    }
)
BROWSER_CPU_RESULT_FIELDS = frozenset(
    {"case_count", "logical_digest", "schema", "success", "suite"}
)
BROWSER_CPU_RUNTIME_FIELDS = frozenset(
    {
        "gate_contract",
        "input_version",
        "love",
        "profile_digest",
        "snapshot_versions",
        "suite",
        "tape_versions",
        "tick_rate",
    }
)
BROWSER_CPU_METRIC_FIELDS = frozenset(
    {
        "capture_calls",
        "capture_ms",
        "case",
        "max_rollback_ms",
        "max_update_wall_ms",
        "p95_update_wall_ms",
        "p95_work_ms",
        "peak_history_bytes",
        "peak_snapshot_bytes",
        "profile",
        "resimulation_calls",
        "resimulation_ms",
        "restore_calls",
        "restore_ms",
        "rollback_calls",
        "rollback_ms",
        "rollback_over_33_3_count",
        "rollback_p999_ms",
        "rollback_percentile",
        "rollback_percentile_method",
        "rollback_sample_count",
        "rollback_timing_evidence",
        "simulation_calls",
        "simulation_ms",
        "work_samples",
    }
)


def validate_historical_soccer_evidence() -> None:
    try:
        evidence = json.loads(HISTORICAL_SOCCER_EVIDENCE.read_text(encoding="utf-8"))
        actual = evidence["simulation_contract"]["rollback_tape_digest"]
    except (FileNotFoundError, KeyError, json.JSONDecodeError) as error:
        raise RuntimeError("historical soccer rollback evidence is unavailable") from error
    if actual != HISTORICAL_SOCCER_TAPE_DIGEST:
        raise RuntimeError(
            "historical soccer rollback tape digest changed: "
            f"{actual!r} != {HISTORICAL_SOCCER_TAPE_DIGEST!r}"
        )


BROWSER_CONSOLE_WAIT_SCRIPT = """
const cursor = arguments[0];
const timeoutMs = arguments[1];
const done = arguments[arguments.length - 1];
const state = window.__GALACTIC_CUP__ || {};
const entries = state.console_entries || [];

function result(timedOut) {
  const current = window.__GALACTIC_CUP__ || {};
  const currentEntries = current.console_entries || entries;
  const nextCursor = currentEntries.length;
  const delta = currentEntries.slice(cursor, nextCursor)
    .map((entry) => String(entry.message || ""));
  for (let index = cursor; index < nextCursor; index += 1) {
    const entry = currentEntries[index];
    if (entry && typeof entry === "object") {
      entry.message = "";
    }
  }
  return {
    cursor: nextCursor,
    entries: delta,
    status: current.status || null,
    timed_out: timedOut
  };
}

if (entries.length > cursor) {
  done(result(false));
} else {
  const originalPush = entries.push;
  let finished = false;
  let deadlineTimer = null;
  let settleTimer = null;
  function finish(timedOut) {
    if (finished) {
      return;
    }
    finished = true;
    if (entries.push === observedPush) {
      entries.push = originalPush;
    }
    if (deadlineTimer !== null) {
      window.clearTimeout(deadlineTimer);
    }
    if (settleTimer !== null) {
      window.clearTimeout(settleTimer);
    }
    done(result(timedOut));
  }
  function observedPush() {
    const pushed = originalPush.apply(entries, arguments);
    if (settleTimer === null) {
      settleTimer = window.setTimeout(() => finish(false), 0);
    }
    return pushed;
  }
  entries.push = observedPush;
  deadlineTimer = window.setTimeout(() => finish(true), timeoutMs);
}
"""


@dataclass(frozen=True)
class ValidationMarker:
    """One stable logical marker emitted by the Lua validation campaign."""

    kind: str
    fields: dict[str, str]
    raw: str


@dataclass(frozen=True)
class ProcessIdentity:
    """A PID plus Linux start time, which protects teardown checks from PID reuse."""

    pid: int
    start_ticks: int


@dataclass(frozen=True)
class ProcessInfo:
    """The /proc fields needed for process-tree memory and CPU accounting."""

    identity: ProcessIdentity
    parent_pid: int
    rss_bytes: int
    cpu_seconds: float


@dataclass(frozen=True)
class RuntimeMetric:
    """A nonlogical wall-time observation paired with one logical case."""

    fields: dict[str, str]
    kind: str
    raw: str


@dataclass(frozen=True)
class RollbackTimingSeries:
    """Raw quantized rollback durations for independent tail validation."""

    case: str
    raw: str
    samples_us: tuple[int, ...]


def utc_now() -> str:
    return datetime.now(UTC).isoformat()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(path)


def parse_marker(line: str) -> ValidationMarker:
    parts = line.split("|")
    if len(parts) < 4 or parts[0] != MARKER_PREFIX:
        raise RuntimeError(f"invalid rollback validation marker: {line}")
    kind = parts[1]
    if kind not in {"case", "result"}:
        raise RuntimeError(f"unexpected rollback validation marker kind {kind!r}")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, separator, value = part.partition("=")
        if not separator or not key or key in fields:
            raise RuntimeError(f"invalid rollback validation marker field: {part}")
        fields[key] = value
    return ValidationMarker(kind=kind, fields=fields, raw=line)


def markers_from_messages(messages: Iterable[str]) -> list[ValidationMarker]:
    markers = []
    for message in messages:
        if message.startswith(MARKER_PREFIX + "|"):
            markers.append(parse_marker(message))
    return markers


def parse_runtime_metric(line: str) -> RuntimeMetric:
    parts = line.split("|")
    if len(parts) < 4 or parts[0] != METRICS_PREFIX:
        raise RuntimeError(f"invalid rollback runtime metric: {line}")
    kind = parts[1]
    if kind not in {"case", "runtime"}:
        raise RuntimeError(f"unexpected rollback runtime metric kind {kind!r}")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, separator, value = part.partition("=")
        if not separator or not key or key in fields:
            raise RuntimeError(f"invalid rollback runtime metric field: {part}")
        fields[key] = value
    return RuntimeMetric(fields=fields, kind=kind, raw=line)


def runtime_metrics_from_messages(messages: Iterable[str]) -> list[RuntimeMetric]:
    metrics = []
    for message in messages:
        if message.startswith(METRICS_PREFIX + "|"):
            metrics.append(parse_runtime_metric(message))
    return metrics


def parse_rollback_timings(line: str) -> RollbackTimingSeries:
    parts = line.split("|")
    if len(parts) < 7 or parts[0] != TIMINGS_PREFIX or parts[1] != "case":
        raise RuntimeError(f"invalid rollback timing series: {line[:200]}")
    fields: dict[str, str] = {}
    for part in parts[2:]:
        key, separator, value = part.partition("=")
        if not separator or not key or key in fields:
            raise RuntimeError(f"invalid rollback timing series field: {part[:200]}")
        fields[key] = value
    required = {"case", "gate_contract", "sample_count", "samples", "unit"}
    missing = sorted(required.difference(fields))
    if missing:
        raise RuntimeError(f"rollback timing series omits fields: {', '.join(missing)}")
    if fields["gate_contract"] != GATE_CONTRACT:
        raise RuntimeError("rollback timing series uses a stale gate contract")
    if fields["unit"] != "microseconds":
        raise RuntimeError("rollback timing series uses an unsupported unit")
    declared_count = positive_integer(fields["sample_count"], "rollback timing sample_count")
    raw_samples = fields["samples"].split(",")
    if len(raw_samples) != declared_count:
        raise RuntimeError(
            f"rollback timing series declares {declared_count} samples but emits "
            f"{len(raw_samples)}"
        )
    samples = tuple(
        non_negative_integer(value, "rollback timing sample") for value in raw_samples
    )
    return RollbackTimingSeries(
        case=fields["case"],
        raw=line,
        samples_us=samples,
    )


def rollback_timings_from_messages(messages: Iterable[str]) -> list[RollbackTimingSeries]:
    timings = []
    for message in messages:
        if message.startswith(TIMINGS_PREFIX + "|"):
            timings.append(parse_rollback_timings(message))
    return timings


def rejected_case(marker: ValidationMarker) -> str | None:
    """Reject explicit failed gates without constraining the evolving case schema."""

    fields = marker.fields
    expected_failure = fields.get("expected_failure") == "1"
    if fields.get("gate") in {"fail", "failed"}:
        return "gate"
    if fields.get("pass") == "0":
        return "pass"
    if fields.get("success") == "0" and not expected_failure:
        return "success"
    if fields.get("status") in {"fail", "failed", "error"} and not expected_failure:
        return "status"
    return None


def validate_marker_set(
    markers: list[ValidationMarker],
    expected_suite: str,
) -> ValidationMarker:
    if not markers:
        raise RuntimeError(f"{expected_suite} emitted no rollback validation markers")
    results = [marker for marker in markers if marker.kind == "result"]
    cases = [marker for marker in markers if marker.kind == "case"]
    if len(results) != 1:
        raise RuntimeError(f"{expected_suite} emitted {len(results)} result markers, expected one")
    result = results[0]
    missing = [field for field in RESULT_REQUIRED_FIELDS if not result.fields.get(field)]
    if missing:
        raise RuntimeError(f"{expected_suite} result omits required fields: {', '.join(missing)}")
    if result.fields["schema"] != "1":
        raise RuntimeError(
            f"{expected_suite} result schema is {result.fields['schema']!r}, expected '1'"
        )
    if result.fields["suite"] != expected_suite:
        raise RuntimeError(
            f"{expected_suite} command emitted suite {result.fields['suite']!r}"
        )
    if result.fields["success"] != "1":
        raise RuntimeError(f"{expected_suite} result reports success={result.fields['success']!r}")
    try:
        declared_count = int(result.fields["case_count"])
    except ValueError as error:
        raise RuntimeError(f"{expected_suite} case_count is not an integer") from error
    if declared_count <= 0 or declared_count != len(cases):
        raise RuntimeError(
            f"{expected_suite} declared {declared_count} cases but emitted {len(cases)}"
        )
    raw_cases = [marker.raw for marker in cases]
    if len(set(raw_cases)) != len(raw_cases):
        raise RuntimeError(f"{expected_suite} emitted duplicate case markers")
    for marker in cases:
        failed_field = rejected_case(marker)
        if failed_field is not None:
            raise RuntimeError(
                f"{expected_suite} case reports a failed {failed_field} gate: {marker.raw}"
            )
    if markers[-1].kind != "result":
        raise RuntimeError(f"{expected_suite} emitted logical markers after its result")
    return result


def positive_integer(value: str, description: str) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise RuntimeError(f"{description} is not an integer") from error
    if parsed <= 0:
        raise RuntimeError(f"{description} must be positive")
    return parsed


def non_negative_integer(value: str, description: str) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise RuntimeError(f"{description} is not an integer") from error
    if parsed < 0:
        raise RuntimeError(f"{description} must be non-negative")
    return parsed


def expected_case_plan(
    suite: str,
    arguments: tuple[str, ...],
) -> list[dict[str, str]]:
    plan: list[dict[str, str]] = []

    def full_case(profile: str, seed: int) -> dict[str, str]:
        return {
            "case": f"full-{profile}-{seed}",
            "network_seed": str(seed),
            "profile": profile,
            "scenario": "complete_fixture",
        }

    def combat_case(profile: str, seed: int, case_id: str | None = None) -> dict[str, str]:
        return {
            "case": case_id or f"combat-{profile}-{seed}",
            "network_seed": str(seed),
            "profile": profile,
            "scenario": "combat",
        }

    def scenario_case(scenario: str, profile: str, seed: int) -> dict[str, str]:
        return {
            "case": f"scenario-{scenario}-{profile}-{seed}",
            "network_seed": str(seed),
            "profile": profile,
            "scenario": scenario,
        }

    if suite == "native":
        if arguments:
            raise RuntimeError("native validation does not accept case filters")
        for profile in NATIVE_PROFILES:
            for seed in NETWORK_SEEDS:
                plan.append(full_case(profile, seed))
                plan.append(combat_case(profile, seed))
        for seed in NETWORK_SEEDS:
            for scenario in SCENARIOS:
                plan.append(scenario_case(scenario, STRESS_PROFILE, seed))
            plan.append(
                combat_case(
                    STRESS_PROFILE,
                    seed,
                    f"combat-stress-evidence-{seed}",
                )
            )
    elif suite == "browser-full":
        if len(arguments) != 2:
            raise RuntimeError("browser-full requires profile and network seed")
        profile, raw_seed = arguments
        if profile not in NATIVE_PROFILES or not raw_seed.isdigit():
            raise RuntimeError("browser-full received an unsupported profile or seed")
        seed = int(raw_seed)
        if seed not in NETWORK_SEEDS:
            raise RuntimeError("browser-full received an unsupported network seed")
        plan.append(full_case(profile, seed))
        plan.append(combat_case(profile, seed))
    elif suite == "browser-stress":
        if len(arguments) != 2 or arguments[0] != STRESS_PROFILE:
            raise RuntimeError("browser-stress requires the pinned stress profile and seed")
        if not arguments[1].isdigit() or int(arguments[1]) not in NETWORK_SEEDS:
            raise RuntimeError("browser-stress received an unsupported network seed")
        seed = int(arguments[1])
        for scenario in SCENARIOS:
            plan.append(scenario_case(scenario, STRESS_PROFILE, seed))
        plan.append(
            combat_case(
                STRESS_PROFILE,
                seed,
                f"combat-stress-evidence-{seed}",
            )
        )
    elif suite == "late-window":
        if arguments:
            raise RuntimeError("late-window validation does not accept case filters")
        for delay in (30, 31):
            plan.append(
                {
                    "case": f"delay-{delay}",
                    "network_seed": str(delay),
                    "profile": f"delay_{delay}",
                    "scenario": "late_window",
                }
            )
    elif suite == "soak":
        if arguments:
            raise RuntimeError("soak validation does not accept case filters")
        for index, seed in enumerate(SOAK_NETWORK_SEEDS, start=1):
            plan.append(
                combat_case(
                    "playable",
                    seed,
                    f"combat-soak-{index}-{seed}",
                )
            )
            plan.append(
                {
                    "case": f"soak-{index}-{seed}",
                    "network_seed": str(seed),
                    "profile": "playable",
                    "scenario": "complete_fixture",
                }
            )
    else:
        raise RuntimeError(f"no pinned case plan for suite {suite!r}")
    return plan


def validate_case_plan(
    markers: list[ValidationMarker],
    suite: str,
    arguments: tuple[str, ...],
) -> None:
    cases = [marker for marker in markers if marker.kind == "case"]
    expected = expected_case_plan(suite, arguments)
    if len(cases) != len(expected):
        raise RuntimeError(
            f"{suite} emitted {len(cases)} cases, expected pinned plan of {len(expected)}"
        )
    for index, (marker, planned) in enumerate(zip(cases, expected, strict=True), start=1):
        mismatches = [
            f"{key}={marker.fields.get(key)!r}"
            for key, value in planned.items()
            if marker.fields.get(key) != value
        ]
        if mismatches:
            raise RuntimeError(
                f"{suite} case {index} differs from the pinned plan: " + ", ".join(mismatches)
            )


def cpu_gate_mode(suite: str, profile: str, browser_runtime: bool) -> str:
    """Derive per-case CPU ownership from the pinned campaign contract."""

    if profile != "playable" or suite == "soak":
        return "diagnostic"
    if browser_runtime:
        if suite != "browser-full":
            raise RuntimeError(
                f"{suite} cannot defer playable CPU acceptance to the browser aggregate"
            )
        return "normalized_deferred"
    return "absolute"


def validate_case_integrity(
    markers: list[ValidationMarker],
    suite: str,
    browser_runtime: bool = False,
) -> None:
    for marker in (row for row in markers if row.kind == "case"):
        fields = marker.fields
        case_id = fields.get("case", "<unknown>")
        required = (
            "client_hash",
            "cpu_gate",
            "cpu_gate_applied",
            "cpu_gate_mode",
            "event_confirmed_digest",
            "event_confirmed_combat",
            "event_reference_digest",
            "event_residue",
            "expected_failure",
            "game_gate",
            "gate_contract",
            "history_gate",
            "hidden_progress",
            "initial_hash",
            "lab_success",
            "peak_history_bytes",
            "peak_snapshot_bytes",
            "peak_snapshots",
            "profile",
            "reference_hash",
            "resimulated",
            "scenario_pass",
            "snapshot_version",
            "snapshot_gate",
            "success",
            "tape_digest",
            "tape_version",
        )
        missing = [name for name in required if fields.get(name) in {None, ""}]
        if missing:
            raise RuntimeError(
                f"{case_id} omits integrity fields: {', '.join(missing)}"
            )
        if fields["success"] != "1":
            raise RuntimeError(f"{case_id} was not accepted")
        if fields["scenario_pass"] != "1":
            raise RuntimeError(f"{case_id} did not cover its declared scenario")
        if fields["hidden_progress"] != "0":
            raise RuntimeError(f"{case_id} made hidden progress after a terminal result")
        combat_case = fields.get("scenario") == "combat"
        expected_tape_version = "2" if combat_case else "1"
        expected_snapshot_version = "6" if combat_case else "5"
        if fields["tape_version"] != expected_tape_version:
            raise RuntimeError(
                f"{case_id} reports tape_version={fields['tape_version']!r}, "
                f"expected {expected_tape_version!r}"
            )
        if fields["snapshot_version"] != expected_snapshot_version:
            raise RuntimeError(
                f"{case_id} reports snapshot_version={fields['snapshot_version']!r}, "
                f"expected {expected_snapshot_version!r}"
            )
        for field in ("initial_hash", "reference_hash", "client_hash", "tape_digest"):
            if not re.fullmatch(r"[0-9a-f]{16}", fields[field]):
                raise RuntimeError(f"{case_id} reports malformed {field}")
        confirmed_combat = non_negative_integer(
            fields["event_confirmed_combat"],
            f"{case_id} event_confirmed_combat",
        )
        if combat_case and confirmed_combat == 0:
            raise RuntimeError(f"{case_id} did not confirm a combat event")
        if not combat_case and confirmed_combat != 0:
            raise RuntimeError(f"{case_id} soccer fixture reported combat events")
        resimulated = non_negative_integer(fields["resimulated"], f"{case_id} resimulated")
        if combat_case and fields["profile"] != "clean" and resimulated == 0:
            raise RuntimeError(f"{case_id} did not exercise combat resimulation")
        if fields["gate_contract"] != GATE_CONTRACT:
            raise RuntimeError(
                f"{case_id} reports gate_contract={fields['gate_contract']!r}"
            )
        if fields["cpu_gate_applied"] not in {"0", "1"}:
            raise RuntimeError(
                f"{case_id} reports cpu_gate_applied={fields['cpu_gate_applied']!r}"
            )
        expected_mode = cpu_gate_mode(suite, fields["profile"], browser_runtime)
        if fields["cpu_gate_mode"] != expected_mode:
            raise RuntimeError(
                f"{case_id} reports cpu_gate_mode={fields['cpu_gate_mode']!r}, "
                f"expected {expected_mode!r}"
            )
        expected_applied = expected_mode == "absolute"
        if (fields["cpu_gate_applied"] == "1") != expected_applied:
            raise RuntimeError(
                f"{case_id} CPU gate ownership differs from the {suite} contract"
            )
        if expected_mode == "absolute":
            expected_cpu_gates = {"1"}
        elif expected_mode == "normalized_deferred":
            expected_cpu_gates = {"deferred"}
        else:
            expected_cpu_gates = {"not_applied"}
        if fields["cpu_gate"] not in expected_cpu_gates:
            raise RuntimeError(
                f"{case_id} reports cpu_gate={fields['cpu_gate']!r} "
                f"for cpu_gate_mode={fields['cpu_gate_mode']!r}"
            )
        for gate in ("snapshot_gate", "history_gate", "game_gate"):
            if fields[gate] != "1":
                raise RuntimeError(f"{case_id} reports {gate}={fields[gate]!r}")
        expected_failure = fields["expected_failure"] == "1"
        if not expected_failure:
            if fields["lab_success"] != "1":
                raise RuntimeError(f"{case_id} laboratory result failed unexpectedly")
            if fields["reference_hash"] != fields["client_hash"]:
                raise RuntimeError(f"{case_id} client hash did not converge to authority")
            if fields["event_reference_digest"] != fields["event_confirmed_digest"]:
                raise RuntimeError(f"{case_id} confirmed event digest differs from authority")
            if fields["event_residue"] != "0":
                raise RuntimeError(f"{case_id} retained speculative event residue")
        if fields["profile"] == "playable":
            snapshot_count = non_negative_integer(
                fields["peak_snapshots"],
                f"{case_id} peak_snapshots",
            )
            snapshot_bytes = non_negative_integer(
                fields["peak_snapshot_bytes"],
                f"{case_id} peak_snapshot_bytes",
            )
            history_bytes = non_negative_integer(
                fields["peak_history_bytes"],
                f"{case_id} peak_history_bytes",
            )
            if snapshot_count > MAX_SNAPSHOT_COUNT:
                raise RuntimeError(f"{case_id} exceeded the 31-snapshot gate")
            if snapshot_bytes >= MAX_SNAPSHOT_BYTES:
                raise RuntimeError(f"{case_id} exceeded the 768 KiB snapshot gate")
            if history_bytes >= MAX_HISTORY_BYTES:
                raise RuntimeError(f"{case_id} exceeded the 1 MiB history gate")


def finite_non_negative_float(value: str, description: str) -> float:
    try:
        parsed = float(value)
    except ValueError as error:
        raise RuntimeError(f"{description} is not numeric") from error
    if not math.isfinite(parsed) or parsed < 0:
        raise RuntimeError(f"{description} must be finite and non-negative")
    return parsed


def nearest_rank_integer(values: tuple[int, ...], percentile: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * percentile) - 1)]


def validate_runtime_metrics(
    metrics: list[RuntimeMetric],
    timings: list[RollbackTimingSeries],
    markers: list[ValidationMarker],
    suite: str,
    browser_runtime: bool = False,
) -> None:
    runtimes = [metric for metric in metrics if metric.kind == "runtime"]
    if len(runtimes) != 1:
        raise RuntimeError(f"emitted {len(runtimes)} runtime provenance rows, expected one")
    runtime = runtimes[0].fields
    expected_runtime = {
        "input_version": "2",
        "gate_contract": GATE_CONTRACT,
        "love": "11.5.0",
        "snapshot_versions": "5,6",
        "suite": suite,
        "tape_versions": "1,2",
        "tick_rate": "60",
    }
    mismatches = [
        f"{key}={runtime.get(key)!r}"
        for key, value in expected_runtime.items()
        if runtime.get(key) != value
    ]
    if mismatches:
        raise RuntimeError("runtime provenance mismatch: " + ", ".join(mismatches))
    if runtime.get("profile_digest") != EXPECTED_PROFILE_DIGEST:
        raise RuntimeError(
            "runtime profile_digest does not match the frozen playable envelope"
        )

    metrics = [metric for metric in metrics if metric.kind == "case"]
    cases = [marker for marker in markers if marker.kind == "case"]
    if len(metrics) != len(cases):
        raise RuntimeError(
            f"emitted {len(metrics)} runtime metric rows for {len(cases)} validation cases"
        )
    case_ids = {case.fields["case"] for case in cases}
    timing_by_case: dict[str, RollbackTimingSeries] = {}
    for series in timings:
        if series.case not in case_ids:
            raise RuntimeError(f"rollback timing series names unknown case {series.case!r}")
        if series.case in timing_by_case:
            raise RuntimeError(f"duplicate rollback timing series for {series.case}")
        timing_by_case[series.case] = series

    seen: set[str] = set()
    numeric_fields = (
        "p95_work_ms",
        "rollback_p999_ms",
        "max_rollback_ms",
        "p95_update_wall_ms",
        "max_update_wall_ms",
        "simulation_ms",
        "capture_ms",
        "restore_ms",
        "resimulation_ms",
        "rollback_ms",
    )
    count_fields = (
        "capture_calls",
        "simulation_calls",
        "restore_calls",
        "resimulation_calls",
        "rollback_calls",
        "rollback_over_33_3_count",
        "rollback_sample_count",
    )
    for case, metric in zip(cases, metrics, strict=True):
        fields = metric.fields
        case_id = case.fields["case"]
        if fields.get("case") != case_id:
            raise RuntimeError(
                f"runtime metric {fields.get('case')!r} is out of order for {case_id}"
            )
        if case_id in seen:
            raise RuntimeError(f"duplicate runtime metric for {case_id}")
        seen.add(case_id)
        if fields.get("profile") != case.fields["profile"]:
            raise RuntimeError(f"{case_id} runtime metric profile does not match its case")
        missing = [
            name
            for name in (
                *numeric_fields,
                *count_fields,
                "rollback_percentile",
                "rollback_percentile_method",
                "rollback_timing_evidence",
                "work_samples",
            )
            if fields.get(name) in {None, ""}
        ]
        if missing:
            raise RuntimeError(
                f"{case_id} runtime metric omits fields: {', '.join(missing)}"
            )
        values = {
            name: finite_non_negative_float(fields[name], f"{case_id} {name}")
            for name in numeric_fields
        }
        positive_integer(fields["work_samples"], f"{case_id} work_samples")
        counts = {
            name: non_negative_integer(fields[name], f"{case_id} {name}")
            for name in count_fields
        }
        if counts["simulation_calls"] <= 0:
            raise RuntimeError(f"{case_id} recorded no simulation timing samples")
        if fields["rollback_percentile"] != "0.999":
            raise RuntimeError(f"{case_id} reports an unsupported rollback percentile")
        if fields["rollback_percentile_method"] != "nearest_rank":
            raise RuntimeError(f"{case_id} reports an unsupported percentile method")
        expected_timing_evidence = (
            "aggregate_diagnostic" if suite == "soak" else "raw"
        )
        if fields["rollback_timing_evidence"] != expected_timing_evidence:
            raise RuntimeError(
                f"{case_id} reports rollback_timing_evidence="
                f"{fields['rollback_timing_evidence']!r}, expected "
                f"{expected_timing_evidence!r}"
            )
        if counts["rollback_sample_count"] != counts["rollback_calls"]:
            raise RuntimeError(f"{case_id} rollback sample count differs from call count")
        sample_count = counts["rollback_sample_count"]
        over_count = counts["rollback_over_33_3_count"]
        if values["max_rollback_ms"] < values["rollback_p999_ms"]:
            raise RuntimeError(f"{case_id} rollback maximum is below p99.9")
        if over_count > sample_count:
            raise RuntimeError(f"{case_id} over-budget count exceeds its sample count")
        if sample_count == 0:
            if (
                values["rollback_p999_ms"] != 0
                or values["max_rollback_ms"] != 0
                or over_count != 0
            ):
                raise RuntimeError(f"{case_id} reports a nonzero empty rollback diagnostic")
        else:
            maximum_reaches_threshold = (
                values["max_rollback_ms"] >= MAX_ROLLBACK_P999_MS
            )
            if maximum_reaches_threshold != (over_count > 0):
                raise RuntimeError(
                    f"{case_id} rollback maximum disagrees with its over-budget count"
                )
            p999_tail_slots = (
                sample_count
                - math.ceil(sample_count * ROLLBACK_PERCENTILE)
                + 1
            )
            p999_reaches_threshold = (
                values["rollback_p999_ms"] >= MAX_ROLLBACK_P999_MS
            )
            if p999_reaches_threshold != (over_count >= p999_tail_slots):
                raise RuntimeError(
                    f"{case_id} rollback p99.9 disagrees with its over-budget count"
                )
        logical_rollbacks = non_negative_integer(
            case.fields.get("rollbacks", ""),
            f"{case_id} logical rollbacks",
        )
        if counts["rollback_calls"] != logical_rollbacks:
            raise RuntimeError(f"{case_id} timing calls differ from logical rollbacks")
        series = timing_by_case.pop(case_id, None)
        if suite == "soak":
            if series is not None:
                raise RuntimeError(f"{case_id} soak emitted raw rollback timings")
            samples_us: tuple[int, ...] = ()
        elif counts["rollback_calls"] == 0:
            if series is not None:
                raise RuntimeError(f"{case_id} emitted timings without rollback calls")
            samples_us = ()
        else:
            if series is None:
                raise RuntimeError(f"{case_id} omitted raw rollback timings")
            samples_us = series.samples_us
            if len(samples_us) != counts["rollback_calls"]:
                raise RuntimeError(f"{case_id} raw timing count differs from rollback calls")
        if suite != "soak":
            p999_ms = nearest_rank_integer(samples_us, ROLLBACK_PERCENTILE) / 1000
            maximum_ms = nearest_rank_integer(samples_us, 1) / 1000
            recomputed_over_count = sum(
                sample >= MAX_ROLLBACK_P999_US for sample in samples_us
            )
            if not math.isclose(
                values["rollback_p999_ms"],
                p999_ms,
                rel_tol=0.0,
                abs_tol=0.0000001,
            ):
                raise RuntimeError(f"{case_id} reported p99.9 differs from raw timings")
            if not math.isclose(
                values["max_rollback_ms"],
                maximum_ms,
                rel_tol=0.0,
                abs_tol=0.0000001,
            ):
                raise RuntimeError(f"{case_id} reported maximum differs from raw timings")
            if counts["rollback_over_33_3_count"] != recomputed_over_count:
                raise RuntimeError(f"{case_id} over-budget count differs from raw timings")
        expected_mode = cpu_gate_mode(suite, case.fields["profile"], browser_runtime)
        expected_applied = expected_mode == "absolute"
        if (case.fields["cpu_gate_applied"] == "1") != expected_applied:
            raise RuntimeError(
                f"{case_id} CPU metric ownership differs from the {suite} contract"
            )
        if expected_applied:
            if values["p95_work_ms"] >= MAX_P95_WORK_MS:
                raise RuntimeError(
                    f"{case_id} p95 work {values['p95_work_ms']:.6f} ms "
                    f"does not meet the <{MAX_P95_WORK_MS} ms gate"
                )
            if values["rollback_p999_ms"] >= MAX_ROLLBACK_P999_MS:
                raise RuntimeError(
                    f"{case_id} p99.9 rollback {values['rollback_p999_ms']:.6f} ms "
                    f"does not meet the <{MAX_ROLLBACK_P999_MS} ms gate"
                )
    if timing_by_case:
        raise RuntimeError("unconsumed rollback timing series remain after validation")


def runtime_metric_record(metrics: list[RuntimeMetric]) -> dict[str, Any]:
    payload = "\n".join(metric.raw for metric in metrics)
    encoded = (payload + ("\n" if payload else "")).encode()
    return {
        "marker_sha256": sha256_bytes(encoded),
        "rows": [
            {
                "fields": metric.fields,
                "kind": metric.kind,
                "marker": metric.raw,
            }
            for metric in metrics
        ],
    }


def rollback_timing_record(timings: list[RollbackTimingSeries]) -> dict[str, Any]:
    payload = "\n".join(series.raw for series in timings)
    encoded = (payload + ("\n" if payload else "")).encode()
    return {
        "marker_sha256": sha256_bytes(encoded),
        "series": [
            {
                "case": series.case,
                "sample_count": len(series.samples_us),
                "samples_us": list(series.samples_us),
            }
            for series in timings
        ],
    }


def validate_soak_contract(markers: list[ValidationMarker]) -> dict[str, ValidationMarker]:
    cases = [marker for marker in markers if marker.kind == "case"]
    checkpoint_cases = [marker for marker in cases if marker.fields.get("sample") != "none"]
    combat_cases = [marker for marker in cases if marker.fields.get("sample") == "none"]
    emitted_samples = [marker.fields.get("sample") for marker in checkpoint_cases]
    if emitted_samples != list(SOAK_SAMPLES):
        raise RuntimeError(
            "soak checkpoint order is "
            f"{emitted_samples!r}, expected {list(SOAK_SAMPLES)!r}"
        )
    by_sample: dict[str, ValidationMarker] = {}
    for marker in checkpoint_cases:
        sample = marker.fields.get("sample")
        if sample not in SOAK_SAMPLES:
            raise RuntimeError(f"soak case has unexpected sample {sample!r}")
        if sample in by_sample:
            raise RuntimeError(f"soak emitted duplicate {sample} checkpoint")
        if marker.fields.get("forced_gc") != "1":
            raise RuntimeError(f"soak {sample} checkpoint did not force garbage collection")
        positive_integer(
            marker.fields.get("lua_heap_bytes", ""),
            f"soak {sample} lua_heap_bytes",
        )
        if not re.fullmatch(r"[0-9a-f]{16}", marker.fields.get("logical_digest", "")):
            raise RuntimeError(f"soak {sample} logical_digest is not canonical 16-hex")
        if marker.fields.get("success") != "1":
            raise RuntimeError(f"soak {sample} checkpoint reports failure")
        by_sample[sample] = marker
    missing = [sample for sample in SOAK_SAMPLES if sample not in by_sample]
    if missing:
        raise RuntimeError(f"soak omitted checkpoints: {', '.join(missing)}")
    if combat_cases and len(combat_cases) != len(SOAK_NETWORK_SEEDS):
        raise RuntimeError("soak emitted the wrong number of bounded combat cases")
    for marker in combat_cases:
        if marker.fields.get("scenario") != "combat":
            raise RuntimeError("soak emitted an unexpected non-checkpoint case")
        if marker.fields.get("forced_gc") is not None:
            raise RuntimeError("bounded combat cases cannot become memory checkpoints")
    return by_sample


def validate_late_window_contract(markers: list[ValidationMarker]) -> None:
    cases = [marker for marker in markers if marker.kind == "case"]
    supported = next(
        (marker for marker in cases if marker.fields.get("case") == "delay-30"),
        None,
    )
    if supported is None:
        raise RuntimeError("late-window omitted the supported delay-30 correction")
    if (
        supported.fields.get("lab_success") != "1"
        or supported.fields.get("status") != "converged"
        or supported.fields.get("max_depth") != "30"
    ):
        raise RuntimeError(
            "delay-30 did not converge at the exact supported rollback depth"
        )
    expected_failures = [
        marker
        for marker in cases
        if marker.kind == "case" and marker.fields.get("expected_failure") == "1"
    ]
    if len(expected_failures) != 1:
        raise RuntimeError(
            "late-window must emit exactly one accepted over-window terminal case"
        )
    fields = expected_failures[0].fields
    expected = {
        "hidden_progress": "0",
        "lab_success": "0",
        "late_tick": "0",
        "status": "late_input_unrecoverable",
        "success": "1",
    }
    mismatches = [
        f"{key}={fields.get(key)!r}"
        for key, value in expected.items()
        if fields.get(key) != value
    ]
    if mismatches:
        raise RuntimeError(
            "late-window over-window terminal contract failed: " + ", ".join(mismatches)
        )


def growth_gate(values: dict[str, int], label: str) -> dict[str, Any]:
    baseline = values["warmup"]
    terminal = values["final"]
    growth_ratio = max(0.0, (terminal - baseline) / baseline)
    peak_sample = max(values, key=lambda sample: values[sample])
    peak = values[peak_sample]
    peak_growth_ratio = max(0.0, (peak - baseline) / baseline)
    passed = growth_ratio <= MAX_MEMORY_GROWTH_RATIO + 1e-12
    return {
        "baseline_bytes": baseline,
        "growth_percent": round(growth_ratio * 100, 6),
        "label": label,
        "limit_percent": MAX_MEMORY_GROWTH_RATIO * 100,
        "measurement": "final_vs_warmup",
        "pass": passed,
        "peak_bytes": peak,
        "peak_growth_percent": round(peak_growth_ratio * 100, 6),
        "peak_sample": peak_sample,
        "samples": values,
        "terminal_bytes": terminal,
        "terminal_sample": "final",
    }


def soak_memory_evidence(
    markers: list[ValidationMarker],
    resources: dict[str, Any],
    browser_name: str | None,
) -> dict[str, Any]:
    by_sample = validate_soak_contract(markers)
    checkpoints = {
        row.get("validation_marker"): row
        for row in resources.get("checkpoints", [])
        if row.get("validation_marker") is not None
    }
    lua_values = {
        sample: positive_integer(
            by_sample[sample].fields["lua_heap_bytes"],
            f"soak {sample} lua_heap_bytes",
        )
        for sample in SOAK_SAMPLES
    }
    rss_values: dict[str, int] = {}
    js_heap_values: dict[str, int] = {}
    for sample in EXTERNAL_MEMORY_SAMPLES:
        checkpoint = checkpoints.get(by_sample[sample].raw)
        if checkpoint is None:
            raise RuntimeError(f"soak {sample} has no external process checkpoint")
        rss_bytes = checkpoint.get("rss_bytes")
        if not isinstance(rss_bytes, int) or rss_bytes <= 0:
            raise RuntimeError(f"soak {sample} process RSS is unavailable")
        rss_values[sample] = rss_bytes
        if browser_name == "chrome":
            js_heap = checkpoint.get("js_heap")
            used_bytes = js_heap.get("used_bytes") if isinstance(js_heap, dict) else None
            if not isinstance(used_bytes, int) or used_bytes <= 0:
                raise RuntimeError(f"soak {sample} Chrome JS heap is unavailable")
            js_heap_values[sample] = used_bytes
    gates = {
        "lua_heap": growth_gate(lua_values, "Lua heap"),
        "process_rss": growth_gate(rss_values, "process-tree RSS"),
    }
    if browser_name == "chrome":
        gates["js_heap"] = growth_gate(js_heap_values, "Chrome JS heap")
    return {
        "gates": gates,
        "pass": all(gate["pass"] for gate in gates.values()),
    }


def compare_fresh_markers(
    first: list[ValidationMarker],
    second: list[ValidationMarker],
) -> str:
    first_lines = [marker.raw for marker in first]
    second_lines = [marker.raw for marker in second]
    if first_lines != second_lines:
        mismatch = 0
        while (
            mismatch < len(first_lines)
            and mismatch < len(second_lines)
            and first_lines[mismatch] == second_lines[mismatch]
        ):
            mismatch += 1
        first_value = first_lines[mismatch] if mismatch < len(first_lines) else "<missing>"
        second_value = second_lines[mismatch] if mismatch < len(second_lines) else "<missing>"
        raise RuntimeError(
            "fresh native rollback validation markers disagreed at "
            f"index {mismatch}: {first_value!r} != {second_value!r}"
        )
    payload = ("\n".join(first_lines) + "\n").encode()
    return sha256_bytes(payload)


def source_provenance() -> dict[str, Any]:
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.strip()
    status = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout
    return {
        "dirty": bool(status.strip()),
        "revision": revision,
    }


def system_provenance() -> dict[str, Any]:
    return {
        "environment": {
            key: os.environ.get(key)
            for key in (
                "CI",
                "DISPLAY",
                "GALLIUM_DRIVER",
                "LIBGL_ALWAYS_SOFTWARE",
                "SE_CACHE_PATH",
            )
        },
        "machine": platform.machine(),
        "os": os_metadata(),
        "platform": platform.platform(),
        "python_runtime": executable_metadata(Path(sys.executable)),
        "python_version": platform.python_version(),
    }


def read_process_table() -> dict[int, ProcessInfo]:
    """Read a best-effort Linux process table without adding a psutil dependency."""

    if not Path("/proc").is_dir():
        return {}
    clock_ticks = os.sysconf("SC_CLK_TCK")
    page_size = os.sysconf("SC_PAGE_SIZE")
    result: dict[int, ProcessInfo] = {}
    for stat_path in Path("/proc").glob("[0-9]*/stat"):
        try:
            payload = stat_path.read_text(encoding="utf-8")
            close = payload.rfind(")")
            if close < 0:
                continue
            pid = int(payload[: payload.find(" ")])
            fields = payload[close + 2 :].split()
            parent_pid = int(fields[1])
            user_ticks = int(fields[11])
            system_ticks = int(fields[12])
            start_ticks = int(fields[19])
            rss_pages = int(fields[21])
            identity = ProcessIdentity(pid=pid, start_ticks=start_ticks)
            result[pid] = ProcessInfo(
                identity=identity,
                parent_pid=parent_pid,
                rss_bytes=max(0, rss_pages) * page_size,
                cpu_seconds=(user_ticks + system_ticks) / clock_ticks,
            )
        except (FileNotFoundError, PermissionError, ProcessLookupError, ValueError):
            continue
    return result


def validation_process_census() -> dict[ProcessIdentity, tuple[str, ...]]:
    """Find native validation commands, including helpers detached by an AppImage."""

    table = read_process_table()
    matches: dict[ProcessIdentity, tuple[str, ...]] = {}
    root_argument = str(ROOT)
    for pid, info in table.items():
        try:
            raw = Path(f"/proc/{pid}/cmdline").read_bytes()
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        arguments = tuple(
            value.decode("utf-8", errors="replace")
            for value in raw.split(b"\0")
            if value
        )
        if root_argument in arguments and "--rollback-validation" in arguments:
            matches[info.identity] = arguments
    return matches


def browser_process_census(
    browser_name: str,
    binary: Path,
    driver_path: Path,
) -> dict[ProcessIdentity, str]:
    """Find browser-family executables, including helpers that detached early."""

    assert browser_name in {"chrome", "firefox"}
    resolved_binary = binary.resolve()
    resolved_driver = driver_path.resolve()
    helper_names = (
        {"chrome", "chrome_crashpad_handler", "chromium", "chromium-browser", "google-chrome"}
        if browser_name == "chrome"
        else {"crashreporter", "firefox", "firefox-bin"}
    )
    matches: dict[ProcessIdentity, str] = {}
    for pid, info in read_process_table().items():
        try:
            executable = Path(os.readlink(f"/proc/{pid}/exe")).resolve()
        except (FileNotFoundError, PermissionError, ProcessLookupError, OSError):
            continue
        if (
            executable == resolved_binary
            or executable == resolved_driver
            or (
                executable.parent == resolved_binary.parent
                and executable.name in helper_names
            )
        ):
            matches[info.identity] = str(executable)
    return matches


class ProcessTreeSampler:
    """Track a process and every descendant seen during a bounded campaign."""

    def __init__(self, root_pid: int) -> None:
        self.root_pid = root_pid
        self._root_identity: ProcessIdentity | None = None
        self.started = time.monotonic()
        self._known: set[ProcessIdentity] = set()
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._sample_loop, daemon=True)
        self._peak_rss_bytes = 0
        self._peak_process_count = 0
        self._latest_cpu_seconds = 0.0
        self._checkpoints: list[dict[str, Any]] = []
        self._available = Path("/proc").is_dir()
        self._sample()
        self._thread.start()

    def _sample_loop(self) -> None:
        while not self._stop.wait(POLL_SECONDS):
            self._sample()

    def _sample(self) -> dict[str, Any]:
        table = read_process_table()
        if not table:
            return {
                "cpu_seconds": None,
                "process_count": None,
                "rss_bytes": None,
            }
        with self._lock:
            root = table.get(self.root_pid)
            if self._root_identity is None and root is not None:
                self._root_identity = root.identity
            known_pids = {
                identity.pid
                for identity in self._known
                if table.get(identity.pid)
                and table[identity.pid].identity.start_ticks == identity.start_ticks
            }
            selected = set(known_pids)
            if (
                root is not None
                and self._root_identity is not None
                and root.identity == self._root_identity
            ):
                selected.add(self.root_pid)
            changed = True
            while changed:
                changed = False
                for pid, info in table.items():
                    if pid not in selected and info.parent_pid in selected:
                        selected.add(pid)
                        changed = True
            infos = [table[pid] for pid in selected if pid in table]
            for info in infos:
                self._known.add(info.identity)
            rss_bytes = sum(info.rss_bytes for info in infos)
            cpu_seconds = sum(info.cpu_seconds for info in infos)
            self._peak_rss_bytes = max(self._peak_rss_bytes, rss_bytes)
            self._peak_process_count = max(self._peak_process_count, len(infos))
            self._latest_cpu_seconds = max(self._latest_cpu_seconds, cpu_seconds)
            return {
                "cpu_seconds": round(cpu_seconds, 6),
                "process_count": len(infos),
                "rss_bytes": rss_bytes,
            }

    def checkpoint(self, label: str) -> dict[str, Any]:
        sample = self._sample()
        row = {
            "elapsed_seconds": round(time.monotonic() - self.started, 6),
            "label": label,
            **sample,
        }
        with self._lock:
            self._checkpoints.append(row)
        return row

    def alive_identities(self) -> list[ProcessIdentity]:
        table = read_process_table()
        with self._lock:
            return sorted(
                (
                    identity
                    for identity in self._known
                    if table.get(identity.pid)
                    and table[identity.pid].identity.start_ticks == identity.start_ticks
                ),
                key=lambda identity: identity.pid,
            )

    def finish(self) -> dict[str, Any]:
        self._stop.set()
        self._thread.join(timeout=2)
        self._sample()
        with self._lock:
            return {
                "available": self._available,
                "checkpoints": list(self._checkpoints),
                "cpu_seconds": round(self._latest_cpu_seconds, 6)
                if self._available
                else None,
                "peak_process_count": self._peak_process_count if self._available else None,
                "peak_rss_bytes": self._peak_rss_bytes if self._available else None,
            }


def wait_identities_gone(
    sampler: ProcessTreeSampler,
    timeout_seconds: float,
) -> list[ProcessIdentity]:
    deadline = time.monotonic() + timeout_seconds
    alive = sampler.alive_identities()
    while alive and time.monotonic() < deadline:
        time.sleep(0.05)
        alive = sampler.alive_identities()
    return alive


def terminate_identities(identities: Iterable[ProcessIdentity], sent_signal: int) -> None:
    table = read_process_table()
    for identity in identities:
        current = table.get(identity.pid)
        if current is None or current.identity.start_ticks != identity.start_ticks:
            continue
        try:
            os.kill(identity.pid, sent_signal)
        except (PermissionError, ProcessLookupError):
            continue


def wait_validation_processes_gone(
    baseline: set[ProcessIdentity],
    timeout_seconds: float,
) -> dict[ProcessIdentity, tuple[str, ...]]:
    deadline = time.monotonic() + timeout_seconds
    alive = {
        identity: arguments
        for identity, arguments in validation_process_census().items()
        if identity not in baseline
    }
    while alive and time.monotonic() < deadline:
        time.sleep(0.05)
        alive = {
            identity: arguments
            for identity, arguments in validation_process_census().items()
            if identity not in baseline
        }
    return alive


def wait_browser_processes_gone(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    baseline: set[ProcessIdentity],
    timeout_seconds: float,
) -> dict[ProcessIdentity, str]:
    deadline = time.monotonic() + timeout_seconds
    alive = {
        identity: executable
        for identity, executable in browser_process_census(
            browser_name,
            binary,
            driver_path,
        ).items()
        if identity not in baseline
    }
    while alive and time.monotonic() < deadline:
        time.sleep(0.05)
        alive = {
            identity: executable
            for identity, executable in browser_process_census(
                browser_name,
                binary,
                driver_path,
            ).items()
            if identity not in baseline
        }
    return alive


def finish_browser_census(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    baseline: set[ProcessIdentity],
) -> dict[str, Any]:
    detached = wait_browser_processes_gone(
        browser_name,
        binary,
        driver_path,
        baseline,
        2,
    )
    detected = len(detached)
    signals: list[str] = []
    if detached:
        terminate_identities(detached, signal.SIGTERM)
        signals.append("TERM")
        detached = wait_browser_processes_gone(
            browser_name,
            binary,
            driver_path,
            baseline,
            2,
        )
    if detached:
        terminate_identities(detached, signal.SIGKILL)
        signals.append("KILL")
        detached = wait_browser_processes_gone(
            browser_name,
            binary,
            driver_path,
            baseline,
            2,
        )
    return {
        "detached_orphan_count": detected,
        "detached_remaining_process_count": len(detached),
        "detached_signals": signals,
    }


def finish_process_tree(
    process: subprocess.Popen[Any],
    sampler: ProcessTreeSampler,
    timed_out: bool,
    validation_baseline: set[ProcessIdentity],
) -> dict[str, Any]:
    signals: list[str] = []
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            signals.append("TERM")
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
                signals.append("KILL")
            except ProcessLookupError:
                pass
            process.wait(timeout=5)
    alive = wait_identities_gone(sampler, 2)
    detected_orphans = len(alive)
    if alive:
        terminate_identities(alive, signal.SIGTERM)
        alive = wait_identities_gone(sampler, 2)
    if alive:
        terminate_identities(alive, signal.SIGKILL)
        alive = wait_identities_gone(sampler, 2)
    detached = wait_validation_processes_gone(validation_baseline, 2)
    detected_detached = len(detached)
    detached_signals: list[str] = []
    if detached:
        terminate_identities(detached, signal.SIGTERM)
        detached_signals.append("TERM")
        detached = wait_validation_processes_gone(validation_baseline, 2)
    if detached:
        terminate_identities(detached, signal.SIGKILL)
        detached_signals.append("KILL")
        detached = wait_validation_processes_gone(validation_baseline, 2)
    sampler.checkpoint("teardown")
    return {
        "detached_orphan_count": detected_detached,
        "detached_remaining_process_count": len(detached),
        "detached_signals": detached_signals,
        "detected_orphan_count": detected_orphans,
        "orphan_free": (
            not alive
            and not detached
            and detected_orphans == 0
            and detected_detached == 0
        ),
        "remaining_process_count": len(alive),
        "signals": signals,
    }


def bounded_tail(path: Path, max_lines: int = 80, max_characters: int = 12000) -> str:
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        return f"<log unavailable: {error}>"
    value = "\n".join(lines[-max_lines:])
    if len(value) > max_characters:
        value = "<truncated>\n" + value[-max_characters:]
    return value or "<log empty>"


def command_executable(command: str) -> Path:
    candidate = Path(command)
    resolved = candidate if candidate.parent != Path(".") else Path(shutil.which(command) or "")
    if not resolved or not resolved.is_file():
        raise RuntimeError(f"runtime executable does not exist: {command}")
    return resolved.resolve()


def marker_record(
    markers: list[ValidationMarker],
    result: ValidationMarker,
) -> dict[str, Any]:
    encoded = ("\n".join(marker.raw for marker in markers) + "\n").encode()
    return {
        "case_count": int(result.fields["case_count"]),
        "logical_digest": result.fields["logical_digest"],
        "logical_marker_sha256": sha256_bytes(encoded),
        "markers": [marker.raw for marker in markers],
        "result_fields": result.fields,
    }


def run_native_once(
    love_bin: Path,
    suite: str,
    arguments: tuple[str, ...],
    log_path: Path,
    timeout_seconds: int,
    enforce_plan: bool = True,
) -> dict[str, Any]:
    command = [
        str(love_bin),
        str(ROOT),
        "--rollback-validation",
        suite,
        *arguments,
        "--external-sample-ack",
    ]
    log_path.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    timed_out = False
    messages: list[str] = []
    reader_errors: list[str] = []
    validation_baseline = set(validation_process_census())
    if validation_baseline:
        identities = ", ".join(
            f"{identity.pid}:{identity.start_ticks}"
            for identity in sorted(validation_baseline, key=lambda row: row.pid)
        )
        raise RuntimeError(
            "native rollback validation requires a serialized runtime lane; "
            f"pre-existing validation processes: {identities}"
        )
    with log_path.open("w", encoding="utf-8") as log:
        process = subprocess.Popen(
            command,
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            text=True,
            bufsize=1,
        )
        sampler = ProcessTreeSampler(process.pid)
        sampler.checkpoint("started")

        def read_output() -> None:
            stream = process.stdout
            if stream is None:
                reader_errors.append("native process stdout pipe is unavailable")
                return
            try:
                for raw_line in stream:
                    log.write(raw_line)
                    log.flush()
                    line = raw_line.rstrip("\r\n")
                    messages.append(line)
                    if line.startswith(MARKER_PREFIX + "|"):
                        marker = parse_marker(line)
                        checkpoint = sampler.checkpoint(
                            f"marker-{len(markers_from_messages(messages))}-{marker.kind}"
                        )
                        checkpoint["validation_marker"] = marker.raw
                        if (
                            marker.kind == "case"
                            and marker.fields.get("sample") == "final"
                            and marker.fields.get("forced_gc") == "1"
                        ):
                            if process.stdin is None:
                                raise RuntimeError(
                                    "native final sample acknowledgement pipe is unavailable"
                                )
                            process.stdin.write("GC_ROLLBACK_SAMPLE_ACK\n")
                            process.stdin.flush()
            except Exception as error:
                reader_errors.append(str(error))

        reader = threading.Thread(target=read_output, daemon=True)
        reader.start()
        try:
            try:
                process.wait(timeout=timeout_seconds)
            except subprocess.TimeoutExpired:
                timed_out = True
        finally:
            try:
                teardown = finish_process_tree(
                    process,
                    sampler,
                    timed_out,
                    validation_baseline,
                )
                reader.join(timeout=5)
                if reader.is_alive():
                    reader_errors.append("native output reader did not stop")
            finally:
                if process.stdin is not None:
                    process.stdin.close()
                resources = sampler.finish()
    duration_seconds = round(time.monotonic() - started, 6)
    if reader_errors:
        raise RuntimeError(f"native {suite} output reader failed: {reader_errors[0]}")
    if timed_out:
        raise RuntimeError(
            f"native {suite} timed out after {timeout_seconds}s\n{bounded_tail(log_path)}"
        )
    if process.returncode != 0:
        raise RuntimeError(
            f"native {suite} exited {process.returncode}\n{bounded_tail(log_path)}"
        )
    if not teardown["orphan_free"]:
        raise RuntimeError(f"native {suite} left processes behind after bounded teardown")
    markers = markers_from_messages(messages)
    runtime_metrics = runtime_metrics_from_messages(messages)
    rollback_timings = rollback_timings_from_messages(messages)
    result = validate_marker_set(markers, suite)
    if enforce_plan:
        validate_case_plan(markers, suite, arguments)
        validate_case_integrity(markers, suite)
        validate_runtime_metrics(runtime_metrics, rollback_timings, markers, suite)
    if suite == "late-window":
        validate_late_window_contract(markers)
    record = {
        "arguments": list(arguments),
        "command": command,
        "duration_seconds": duration_seconds,
        "log": {
            "path": str(log_path.resolve()),
            "sha256": sha256_file(log_path),
            "size_bytes": log_path.stat().st_size,
        },
        **marker_record(markers, result),
        "resources": resources,
        "rollback_timings": rollback_timing_record(rollback_timings),
        "runtime_metrics": runtime_metric_record(runtime_metrics),
        "suite": suite,
        "teardown": teardown,
    }
    if suite == "soak":
        record["soak_memory"] = soak_memory_evidence(markers, resources, None)
    return record


def native_shard_plan() -> list[tuple[str, tuple[str, ...]]]:
    """Split independent native matrix cases into fresh bounded processes."""

    plan: list[tuple[str, tuple[str, ...]]] = []
    for profile in NATIVE_PROFILES:
        for network_seed in NETWORK_SEEDS:
            plan.append(("browser-full", (profile, str(network_seed))))
    for network_seed in NETWORK_SEEDS:
        plan.append(("browser-stress", (STRESS_PROFILE, str(network_seed))))
    return plan


def native_aggregate_record(
    run_number: int,
    shards: list[dict[str, Any]],
) -> dict[str, Any]:
    markers = [
        parse_marker(raw)
        for shard in shards
        for raw in shard["markers"]
    ]
    validate_case_plan(markers, "native", ())
    validate_case_integrity(markers, "native")
    encoded = ("\n".join(marker.raw for marker in markers) + "\n").encode()
    return {
        "case_count": sum(shard["case_count"] for shard in shards),
        "duration_seconds": round(
            sum(float(shard["duration_seconds"]) for shard in shards),
            6,
        ),
        "logical_marker_sha256": sha256_bytes(encoded),
        "markers": [marker.raw for marker in markers],
        "result_count": sum(1 for marker in markers if marker.kind == "result"),
        "run": run_number,
        "shard_count": len(shards),
        "shards": shards,
        "suite": "native-sharded",
    }


def native_campaign_plan(campaign: str = "all") -> list[tuple[str, tuple[str, ...]]]:
    if campaign not in CAMPAIGNS:
        raise ValueError(f"unknown rollback campaign {campaign!r}")
    plan: list[tuple[str, tuple[str, ...]]] = []
    if campaign in {"all", "matrix"}:
        plan.extend(native_shard_plan())
        plan.append(("late-window", ()))
    if campaign in {"all", "soak"}:
        plan.append(("soak", ()))
    return plan


def native_matrix(
    evidence: dict[str, Any],
    love_bin: Path,
    raw_root: Path,
    timeout_seconds: int,
    campaign: str,
) -> None:
    native: dict[str, Any] = {
        "matrix_process_model": "fresh_process_per_full_case_and_stress_seed",
        "plan": [
            {"arguments": list(arguments), "suite": suite}
            for suite, arguments in native_campaign_plan(campaign)
        ],
        "campaign": campaign,
        "persistent_soak": campaign in {"all", "soak"},
        "runtime": executable_metadata(love_bin),
        "fresh_runs": [],
    }
    evidence["native"] = native
    if campaign in {"all", "matrix"}:
        for run_number in (1, 2):
            shards: list[dict[str, Any]] = []
            for shard_number, (suite, arguments) in enumerate(
                native_shard_plan(),
                start=1,
            ):
                slug = "-".join((suite, *arguments))
                shard = run_native_once(
                    love_bin,
                    suite,
                    arguments,
                    raw_root / f"native-{run_number}-{shard_number:02d}-{slug}.log",
                    timeout_seconds,
                )
                shard["shard"] = shard_number
                shards.append(shard)
            native["fresh_runs"].append(
                native_aggregate_record(run_number, shards)
            )
        first_markers = [
            parse_marker(marker) for marker in native["fresh_runs"][0]["markers"]
        ]
        second_markers = [
            parse_marker(marker) for marker in native["fresh_runs"][1]["markers"]
        ]
        native["fresh_marker_sha256"] = compare_fresh_markers(
            first_markers,
            second_markers,
        )
        native["fresh_runs_agree"] = True
        native["late_window"] = run_native_once(
            love_bin,
            "late-window",
            (),
            raw_root / "native-late-window.log",
            timeout_seconds,
        )
    if campaign in {"all", "soak"}:
        native["soak"] = run_native_once(
            love_bin,
            "soak",
            (),
            raw_root / "native-soak.log",
            timeout_seconds,
        )
        if not native["soak"]["soak_memory"]["pass"]:
            raise RuntimeError(
                "native soak exceeded the 10% terminal forced-GC growth gate"
            )


def browser_js_heap(driver: Any, browser_name: str) -> dict[str, Any] | None:
    if browser_name != "chrome":
        return None
    try:
        metrics = driver.execute_cdp_cmd("Performance.getMetrics", {}).get("metrics", [])
        values = {
            str(row.get("name")): row.get("value")
            for row in metrics
            if isinstance(row, dict)
        }
        return {
            "total_bytes": int(values["JSHeapTotalSize"]),
            "used_bytes": int(values["JSHeapUsedSize"]),
        }
    except Exception:
        return None


def browser_checkpoint(
    sampler: ProcessTreeSampler,
    driver: Any,
    browser_name: str,
    label: str,
    force_js_gc: bool = False,
) -> dict[str, Any]:
    console_entries_discarded = False
    js_gc_forced = False
    if force_js_gc and browser_name == "chrome":
        driver.execute_cdp_cmd("Runtime.discardConsoleEntries", {})
        console_entries_discarded = True
        driver.execute_cdp_cmd("HeapProfiler.collectGarbage", {})
        js_gc_forced = True
    row = sampler.checkpoint(label)
    row["js_heap"] = browser_js_heap(driver, browser_name)
    row["js_console_entries_discarded"] = console_entries_discarded
    row["js_gc_forced"] = js_gc_forced
    return row


def browser_teardown(
    driver: Any,
    sampler: ProcessTreeSampler,
    browser_name: str,
    binary: Path,
    driver_path: Path,
    browser_baseline: set[ProcessIdentity],
) -> tuple[dict[str, Any], dict[str, Any]]:
    teardown_error = None
    try:
        teardown = quit_browser_bounded(driver)
    except Exception as error:
        teardown_error = str(error)
        teardown = {
            "fallback": True,
            "process_group": None,
            "quit_error": teardown_error,
            "service_exit_code": None,
            "signals": [],
        }
    alive = wait_identities_gone(sampler, 2)
    detected_orphans = len(alive)
    if alive:
        terminate_identities(alive, signal.SIGTERM)
        alive = wait_identities_gone(sampler, 2)
    if alive:
        terminate_identities(alive, signal.SIGKILL)
        alive = wait_identities_gone(sampler, 2)
    sampler.checkpoint("teardown")
    detached = finish_browser_census(
        browser_name,
        binary,
        driver_path,
        browser_baseline,
    )
    teardown["detected_orphan_count"] = detected_orphans
    teardown.update(detached)
    teardown["orphan_free"] = (
        not alive
        and detected_orphans == 0
        and detached["detached_orphan_count"] == 0
        and detached["detached_remaining_process_count"] == 0
    )
    teardown["remaining_process_count"] = len(alive)
    teardown["teardown_error"] = teardown_error
    resources = sampler.finish()
    return teardown, resources


def wait_for_browser_console_entries(
    driver: Any,
    cursor: int,
    timeout_seconds: float,
) -> dict[str, Any]:
    """Wait in-page for new console entries without repeated WebDriver polling."""

    assert cursor >= 0
    assert timeout_seconds > 0
    wait_ms = max(1, math.ceil(timeout_seconds * 1000))
    value = driver.execute_async_script(
        BROWSER_CONSOLE_WAIT_SCRIPT,
        cursor,
        wait_ms,
    )
    if not isinstance(value, dict):
        raise RuntimeError("browser returned malformed console wait state")
    entries = value.get("entries")
    next_cursor = value.get("cursor")
    timed_out = value.get("timed_out")
    if (
        not isinstance(entries, list)
        or not isinstance(next_cursor, int)
        or isinstance(next_cursor, bool)
        or next_cursor < cursor
        or next_cursor - cursor != len(entries)
        or not isinstance(timed_out, bool)
    ):
        raise RuntimeError("browser returned malformed console wait fields")
    return {
        "cursor": next_cursor,
        "entries": [str(entry) for entry in entries],
        "status": value.get("status"),
        "timed_out": timed_out,
    }


def set_webdriver_command_timeout(driver: Any, timeout_seconds: float) -> None:
    """Set Selenium's HTTP read timeout beyond the in-page async-script bound."""

    assert timeout_seconds > 0
    command_executor = getattr(driver, "command_executor", None)
    client_config = getattr(command_executor, "client_config", None)
    if client_config is None or not hasattr(client_config, "timeout"):
        raise RuntimeError("WebDriver command timeout configuration is unavailable")
    client_config.timeout = timeout_seconds


def validated_browser_version(driver: Any, browser_name: str) -> str:
    """Read an exact dotted-numeric browser version from raw WebDriver capabilities."""

    capabilities = getattr(driver, "capabilities", None)
    if not isinstance(capabilities, dict):
        raise RuntimeError(f"{browser_name} WebDriver capabilities are malformed")
    reported_browser = capabilities.get("browserName")
    if reported_browser != browser_name:
        raise RuntimeError(
            f"{browser_name} WebDriver reports browserName={reported_browser!r}"
        )
    version = capabilities.get("browserVersion")
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]+)+", version):
        raise RuntimeError(
            f"{browser_name} WebDriver reports malformed browserVersion={version!r}"
        )
    return version


def run_browser_once(
    browser_name: str,
    binary: Path,
    driver_path: Path,
    base_url: str,
    suite: str,
    arguments: tuple[str, ...],
    log_path: Path,
    timeout_seconds: int,
) -> dict[str, Any]:
    driver_log = log_path.with_suffix(".webdriver.log")
    started = time.monotonic()
    browser_baseline = set(browser_process_census(browser_name, binary, driver_path))
    try:
        driver = launch(browser_name, binary, driver_path, driver_log)
    except Exception as error:
        detached = finish_browser_census(
            browser_name,
            binary,
            driver_path,
            browser_baseline,
        )
        raise RuntimeError(
            f"{browser_name} {suite} launch failed: {error}\n"
            f"detached process cleanup: {json.dumps(detached, sort_keys=True)}\n"
            f"{bounded_log_tail(driver_log)}"
        ) from error
    process = getattr(getattr(driver, "service", None), "process", None)
    if process is None or getattr(process, "pid", None) is None:
        try:
            driver.quit()
        finally:
            detached = finish_browser_census(
                browser_name,
                binary,
                driver_path,
                browser_baseline,
            )
            raise RuntimeError(
                f"{browser_name} {suite} WebDriver process is unavailable; "
                f"detached cleanup={json.dumps(detached, sort_keys=True)}"
            )
    sampler = ProcessTreeSampler(process.pid)
    resource_checkpoints: list[dict[str, Any]] = []
    messages: list[str] = []
    markers: list[ValidationMarker] = []
    result: ValidationMarker | None = None
    teardown: dict[str, Any]
    resources: dict[str, Any]
    browser_version: str
    try:
        browser_version = validated_browser_version(driver, browser_name)
        if browser_name == "chrome":
            driver.execute_cdp_cmd("Performance.enable", {})
        resource_checkpoints.append(browser_checkpoint(sampler, driver, browser_name, "started"))
        driver.set_page_load_timeout(min(timeout_seconds, 300))
        webdriver_command_timeout_seconds = timeout_seconds + 10
        driver.set_script_timeout(webdriver_command_timeout_seconds)
        set_webdriver_command_timeout(driver, webdriver_command_timeout_seconds)
        query_arguments = [
            "--rollback-validation",
            suite,
            *arguments,
            "--browser-runtime",
        ]
        query = urllib.parse.urlencode(
            {"arg": json.dumps(query_arguments, separators=(",", ":"))}
        )
        driver.get(f"{base_url}?{query}")
        deadline = time.monotonic() + timeout_seconds
        console_cursor = 0
        observed_marker_count = 0
        while time.monotonic() < deadline:
            remaining_seconds = deadline - time.monotonic()
            if remaining_seconds <= 0:
                break
            state = wait_for_browser_console_entries(
                driver,
                console_cursor,
                remaining_seconds,
            )
            console_cursor = state["cursor"]
            entries = state["entries"]
            messages.extend(entries)
            failures = [
                message
                for message in entries
                if any(error_marker in message for error_marker in ERROR_MARKERS)
            ]
            if failures:
                raise RuntimeError(f"{browser_name} {suite} runtime failure: {failures[0]}")
            markers = markers_from_messages(messages)
            while observed_marker_count < len(markers):
                marker = markers[observed_marker_count]
                checkpoint = browser_checkpoint(
                    sampler,
                    driver,
                    browser_name,
                    f"marker-{observed_marker_count + 1}-{marker.kind}",
                    force_js_gc=marker.fields.get("forced_gc") == "1",
                )
                checkpoint["validation_marker"] = marker.raw
                resource_checkpoints.append(checkpoint)
                observed_marker_count += 1
            results = [marker for marker in markers if marker.kind == "result"]
            if len(results) > 1:
                raise RuntimeError(f"{browser_name} {suite} emitted duplicate results")
            if results:
                result = validate_marker_set(markers, suite)
                break
            if state["timed_out"]:
                break
        if result is None:
            raise RuntimeError(
                f"{browser_name} {suite} timed out after {timeout_seconds}s without a result"
            )
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text("\n".join(messages) + "\n", encoding="utf-8")
        resource_checkpoints.append(
            browser_checkpoint(sampler, driver, browser_name, "completed")
        )
    finally:
        if messages:
            log_path.parent.mkdir(parents=True, exist_ok=True)
            log_path.write_text("\n".join(messages) + "\n", encoding="utf-8")
        teardown, resources = browser_teardown(
            driver,
            sampler,
            browser_name,
            binary,
            driver_path,
            browser_baseline,
        )
    if teardown["teardown_error"] is not None:
        raise RuntimeError(
            f"{browser_name} {suite} teardown failed: {teardown['teardown_error']}"
        )
    if not teardown["orphan_free"]:
        raise RuntimeError(f"{browser_name} {suite} left browser processes after teardown")
    if result is None:
        raise RuntimeError(f"{browser_name} {suite} produced no validated result")
    validate_case_plan(markers, suite, arguments)
    validate_case_integrity(markers, suite, browser_runtime=True)
    runtime_metrics = runtime_metrics_from_messages(messages)
    rollback_timings = rollback_timings_from_messages(messages)
    validate_runtime_metrics(
        runtime_metrics,
        rollback_timings,
        markers,
        suite,
        browser_runtime=True,
    )
    js_heap_samples = [
        row["js_heap"] for row in resource_checkpoints if row.get("js_heap") is not None
    ]
    resources["browser_checkpoints"] = resource_checkpoints
    resources["js_heap_peak_used_bytes"] = (
        max(row["used_bytes"] for row in js_heap_samples) if js_heap_samples else None
    )
    resources["js_heap_peak_total_bytes"] = (
        max(row["total_bytes"] for row in js_heap_samples) if js_heap_samples else None
    )
    record = {
        "arguments": list(arguments),
        "browser": browser_name,
        "browser_version": browser_version,
        "duration_seconds": round(time.monotonic() - started, 6),
        "log": {
            "path": str(log_path.resolve()),
            "sha256": sha256_file(log_path),
            "size_bytes": log_path.stat().st_size,
        },
        **marker_record(markers, result),
        "resources": resources,
        "rollback_timings": rollback_timing_record(rollback_timings),
        "runtime_metrics": runtime_metric_record(runtime_metrics),
        "suite": suite,
        "teardown": teardown,
        "webdriver_command_timeout_seconds": webdriver_command_timeout_seconds,
        "webdriver_log": {
            "path": str(driver_log.resolve()),
            "sha256": sha256_file(driver_log),
            "size_bytes": driver_log.stat().st_size,
        },
    }
    if suite == "soak":
        record["soak_memory"] = soak_memory_evidence(
            markers,
            resources,
            browser_name,
        )
    return record


def artifact_provenance(artifact: Path, allow_dirty: bool) -> dict[str, Any]:
    manifest_path = artifact / "manifest.json"
    if not manifest_path.is_file():
        raise RuntimeError(f"browser artifact manifest is missing: {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    validate_provenance(manifest, allow_dirty)
    return {
        "manifest": manifest,
        "manifest_path": str(manifest_path.resolve()),
        "manifest_sha256": sha256_file(manifest_path),
    }


def browser_plan(campaign: str = "all") -> list[tuple[str, tuple[str, ...]]]:
    if campaign not in CAMPAIGNS:
        raise ValueError(f"unknown rollback campaign {campaign!r}")
    plan = []
    if campaign in {"all", "matrix"}:
        for network_seed in NETWORK_SEEDS:
            for profile in BROWSER_FULL_PROFILES:
                plan.append(("browser-full", (profile, str(network_seed))))
        for network_seed in NETWORK_SEEDS:
            plan.append(("browser-stress", (STRESS_PROFILE, str(network_seed))))
    if campaign in {"all", "soak"}:
        plan.append(("soak", ()))
    return plan


def browser_suite_timeout_seconds(suite: str, timeout_seconds: int) -> int:
    """Scale the browser timeout for the five-fixture persistent soak."""

    if suite == "soak":
        return max(timeout_seconds, MIN_BROWSER_SOAK_TIMEOUT_SECONDS)
    return timeout_seconds


def browser_cpu_case(
    run: dict[str, Any],
    browser_name: str,
    run_index: int,
) -> tuple[list[tuple[tuple[str, str, str], dict[str, Any]]], list[str]]:
    """Validate and extract both exact browser-full CPU cases from one process."""

    label = f"{browser_name} browser-full run {run_index}"
    reasons: list[str] = []
    if run.get("browser") != browser_name:
        reasons.append(
            f"{label} reports browser={run.get('browser')!r}, expected {browser_name!r}"
        )
    arguments = run.get("arguments")
    if not isinstance(arguments, list) or len(arguments) != 2:
        reasons.append(f"{label} has malformed arguments")
        return [], reasons
    profile, seed = arguments
    if not isinstance(profile, str) or not isinstance(seed, str):
        reasons.append(f"{label} has non-string profile or seed arguments")
        return [], reasons
    if profile not in BROWSER_FULL_PROFILES:
        reasons.append(f"{label} has unexpected profile {profile!r}")
        return [], reasons
    if seed not in {str(value) for value in NETWORK_SEEDS}:
        reasons.append(f"{label} has unexpected network seed {seed!r}")
        return [], reasons
    expected_plan = expected_case_plan("browser-full", (profile, seed))

    raw_markers = run.get("markers")
    if not isinstance(raw_markers, list):
        reasons.append(f"{label} omits validation markers")
        return [], reasons
    if not all(isinstance(raw, str) for raw in raw_markers):
        reasons.append(f"{label} contains a non-string validation marker")
        return [], reasons
    try:
        markers = [parse_marker(raw) for raw in raw_markers]
    except RuntimeError as error:
        reasons.append(f"{label} has malformed validation markers: {error}")
        return [], reasons
    case_markers = [marker for marker in markers if marker.kind == "case"]
    result_markers = [marker for marker in markers if marker.kind == "result"]
    if len(markers) != 3 or len(case_markers) != 2 or len(result_markers) != 1:
        reasons.append(
            f"{label} has {len(case_markers)} case and {len(result_markers)} result "
            "markers, expected exactly two cases and one result"
        )
        return [], reasons
    result = result_markers[0]
    if set(result.fields) != BROWSER_CPU_RESULT_FIELDS:
        reasons.append(
            f"{label} result marker schema differs from contract: "
            f"missing={sorted(BROWSER_CPU_RESULT_FIELDS.difference(result.fields))}, "
            f"extra={sorted(set(result.fields).difference(BROWSER_CPU_RESULT_FIELDS))}"
        )
    expected_result = {
        "case_count": "2",
        "schema": "1",
        "success": "1",
        "suite": "browser-full",
    }
    result_mismatches = [
        f"{name}={result.fields.get(name)!r}"
        for name, value in expected_result.items()
        if result.fields.get(name) != value
    ]
    if not re.fullmatch(r"[0-9a-f]{16}", result.fields.get("logical_digest", "")):
        result_mismatches.append(
            f"logical_digest={result.fields.get('logical_digest')!r}"
        )
    if result_mismatches:
        reasons.append(f"{label} result marker mismatch: {', '.join(result_mismatches)}")
    try:
        validate_marker_set(markers, "browser-full")
        validate_case_plan(markers, "browser-full", (profile, seed))
        validate_case_integrity(markers, "browser-full", browser_runtime=True)
    except RuntimeError as error:
        reasons.append(f"{label} marker contract failed: {error}")
    for marker_index, marker in enumerate(case_markers, start=1):
        if set(marker.fields) != BROWSER_CPU_CASE_FIELDS:
            reasons.append(
                f"{label} case marker {marker_index} schema differs from contract: "
                f"missing={sorted(BROWSER_CPU_CASE_FIELDS.difference(marker.fields))}, "
                f"extra={sorted(set(marker.fields).difference(BROWSER_CPU_CASE_FIELDS))}"
            )
        scenario = marker.fields.get("scenario")
        expected_marker_values = {
            "fixture": BROWSER_CPU_FIXTURES.get(scenario, ""),
            "late_tick": "none",
            "sample": "none",
            "schema": "1",
            "status": "converged",
        }
        marker_mismatches = [
            f"{name}={marker.fields.get(name)!r}"
            for name, value in expected_marker_values.items()
            if marker.fields.get(name) != value
        ]
        if marker_mismatches:
            reasons.append(
                f"{label} case marker {marker_index} values differ from contract: "
                + ", ".join(marker_mismatches)
            )
        for name in (
            "max_depth",
            "peak_history_bytes",
            "peak_snapshot_bytes",
            "peak_snapshots",
            "resimulated",
            "rollbacks",
        ):
            try:
                non_negative_integer(
                    marker.fields.get(name, ""),
                    f"{label} case marker {marker_index} {name}",
                )
            except RuntimeError as error:
                reasons.append(str(error))
        for name in ("event_confirmed_digest", "event_reference_digest"):
            if not re.fullmatch(r"[0-9a-f]{16}", marker.fields.get(name, "")):
                reasons.append(
                    f"{label} case marker {marker_index} reports malformed {name}"
                )

    marker_payload = ("\n".join(marker.raw for marker in markers) + "\n").encode()
    declared_marker_digest = run.get("logical_marker_sha256")
    if (
        not isinstance(declared_marker_digest, str)
        or declared_marker_digest != sha256_bytes(marker_payload)
    ):
        reasons.append(f"{label} logical marker digest is missing or mismatched")
    if run.get("case_count") != 2:
        reasons.append(f"{label} record case_count={run.get('case_count')!r}, expected 2")
    if run.get("logical_digest") != result.fields.get("logical_digest"):
        reasons.append(f"{label} record logical_digest differs from its result marker")
    if run.get("result_fields") != result.fields:
        reasons.append(f"{label} record result_fields differ from its result marker")

    runtime_metrics = run.get("runtime_metrics")
    if not isinstance(runtime_metrics, dict):
        reasons.append(f"{label} omits runtime metric record")
        return [], reasons
    if set(runtime_metrics) != {"marker_sha256", "rows"}:
        reasons.append(
            f"{label} runtime metric record schema differs from contract"
        )
    rows = runtime_metrics.get("rows") if isinstance(runtime_metrics, dict) else None
    if not isinstance(rows, list):
        reasons.append(f"{label} omits runtime metric rows")
        return [], reasons
    parsed_metrics: list[RuntimeMetric] = []
    for row_index, row in enumerate(rows, start=1):
        if not isinstance(row, dict) or set(row) != {"fields", "kind", "marker"}:
            reasons.append(f"{label} runtime metric row {row_index} has malformed schema")
            continue
        fields = row["fields"]
        kind = row["kind"]
        raw = row["marker"]
        if (
            not isinstance(fields, dict)
            or not all(
                isinstance(name, str) and isinstance(value, str)
                for name, value in fields.items()
            )
            or not isinstance(kind, str)
            or not isinstance(raw, str)
        ):
            reasons.append(f"{label} runtime metric row {row_index} has malformed types")
            continue
        try:
            parsed = parse_runtime_metric(raw)
        except RuntimeError as error:
            reasons.append(
                f"{label} runtime metric row {row_index} is malformed: {error}"
            )
            continue
        if parsed.kind != kind or parsed.fields != fields:
            reasons.append(
                f"{label} runtime metric row {row_index} differs from its marker"
            )
            continue
        parsed_metrics.append(parsed)
    runtime_rows = [metric for metric in parsed_metrics if metric.kind == "runtime"]
    case_rows = [metric for metric in parsed_metrics if metric.kind == "case"]
    if len(rows) != 3 or len(runtime_rows) != 1 or len(case_rows) != 2:
        reasons.append(
            f"{label} has {len(runtime_rows)} runtime and {len(case_rows)} case "
            "metrics, expected exactly one runtime and two cases"
        )
    expected_metric_order = [
        ("runtime", None),
        *[("case", planned["case"]) for planned in expected_plan],
    ]
    actual_metric_order = [
        (metric.kind, metric.fields.get("case")) for metric in parsed_metrics
    ]
    if actual_metric_order != expected_metric_order:
        reasons.append(
            f"{label} runtime metric order differs from contract: "
            f"{actual_metric_order!r}"
        )
    declared_metric_digest = runtime_metrics.get("marker_sha256")
    metric_payload = ("\n".join(metric.raw for metric in parsed_metrics) + "\n").encode()
    if (
        not isinstance(declared_metric_digest, str)
        or declared_metric_digest != sha256_bytes(metric_payload)
    ):
        reasons.append(f"{label} runtime metric digest is missing or mismatched")
    if len(runtime_rows) == 1:
        runtime_fields = runtime_rows[0].fields
        if set(runtime_fields) != BROWSER_CPU_RUNTIME_FIELDS:
            reasons.append(
                f"{label} runtime provenance schema differs from contract: "
                f"missing={sorted(BROWSER_CPU_RUNTIME_FIELDS.difference(runtime_fields))}, "
                f"extra={sorted(set(runtime_fields).difference(BROWSER_CPU_RUNTIME_FIELDS))}"
            )
        expected_runtime = {
            "gate_contract": GATE_CONTRACT,
            "input_version": "2",
            "love": "11.5.0",
            "profile_digest": EXPECTED_PROFILE_DIGEST,
            "snapshot_versions": "5,6",
            "suite": "browser-full",
            "tape_versions": "1,2",
            "tick_rate": "60",
        }
        runtime_mismatches = [
            f"{name}={runtime_fields.get(name)!r}"
            for name, value in expected_runtime.items()
            if runtime_fields.get(name) != value
        ]
        if runtime_mismatches:
            reasons.append(
                f"{label} runtime provenance mismatch: {', '.join(runtime_mismatches)}"
            )

    extracted: list[tuple[tuple[str, str, str], dict[str, Any]]] = []
    numeric_names = (
        "capture_ms",
        "max_rollback_ms",
        "max_update_wall_ms",
        "p95_update_wall_ms",
        "p95_work_ms",
        "resimulation_ms",
        "restore_ms",
        "rollback_ms",
        "rollback_p999_ms",
        "simulation_ms",
    )
    count_names = (
        "capture_calls",
        "peak_history_bytes",
        "peak_snapshot_bytes",
        "resimulation_calls",
        "restore_calls",
        "rollback_calls",
        "rollback_over_33_3_count",
        "rollback_sample_count",
        "simulation_calls",
    )
    for planned in expected_plan:
        case_id = planned["case"]
        matching_metrics = [
            metric for metric in case_rows if metric.fields.get("case") == case_id
        ]
        if len(matching_metrics) != 1:
            reasons.append(
                f"{label} has {len(matching_metrics)} runtime metrics for {case_id}, expected one"
            )
            continue
        metric = matching_metrics[0]
        fields = metric.fields
        metric_reasons: list[str] = []
        if set(fields) != BROWSER_CPU_METRIC_FIELDS:
            metric_reasons.append(
                "schema differs from contract: "
                f"missing={sorted(BROWSER_CPU_METRIC_FIELDS.difference(fields))}, "
                f"extra={sorted(set(fields).difference(BROWSER_CPU_METRIC_FIELDS))}"
            )
        if fields.get("profile") != profile:
            metric_reasons.append(
                f"profile={fields.get('profile')!r}, expected {profile!r}"
            )
        if fields.get("rollback_percentile") != "0.999":
            metric_reasons.append("rollback_percentile is not 0.999")
        if fields.get("rollback_percentile_method") != "nearest_rank":
            metric_reasons.append("rollback_percentile_method is not nearest_rank")
        if fields.get("rollback_timing_evidence") != "raw":
            metric_reasons.append("rollback_timing_evidence is not raw")
        numeric: dict[str, float] = {}
        counts: dict[str, int] = {}
        for name in numeric_names:
            value = fields.get(name)
            if not isinstance(value, str):
                metric_reasons.append(f"{name} is missing")
                continue
            try:
                numeric[name] = finite_non_negative_float(
                    value,
                    f"{label} {case_id} {name}",
                )
            except RuntimeError as error:
                metric_reasons.append(str(error))
        for name in count_names:
            value = fields.get(name)
            if not isinstance(value, str):
                metric_reasons.append(f"{name} is missing")
                continue
            try:
                counts[name] = non_negative_integer(
                    value,
                    f"{label} {case_id} {name}",
                )
            except RuntimeError as error:
                metric_reasons.append(str(error))
        work_samples = fields.get("work_samples")
        if isinstance(work_samples, str):
            try:
                positive_integer(work_samples, f"{label} {case_id} work_samples")
            except RuntimeError as error:
                metric_reasons.append(str(error))
        else:
            metric_reasons.append("work_samples is missing")
        if counts.get("simulation_calls", 0) <= 0:
            metric_reasons.append("simulation_calls must be positive")
        if (
            "rollback_sample_count" in counts
            and "rollback_calls" in counts
            and counts["rollback_sample_count"] != counts["rollback_calls"]
        ):
            metric_reasons.append("rollback sample count differs from rollback calls")
        if (
            "max_rollback_ms" in numeric
            and "rollback_p999_ms" in numeric
            and numeric["max_rollback_ms"] < numeric["rollback_p999_ms"]
        ):
            metric_reasons.append("rollback maximum is below p99.9")
        if (
            "rollback_over_33_3_count" in counts
            and "rollback_sample_count" in counts
            and counts["rollback_over_33_3_count"] > counts["rollback_sample_count"]
        ):
            metric_reasons.append("over-budget count exceeds rollback sample count")
        marker = next(
            (candidate for candidate in case_markers if candidate.fields.get("case") == case_id),
            None,
        )
        if marker is None:
            metric_reasons.append("has no matching logical case marker")
        else:
            if (
                "rollback_calls" in counts
                and marker.fields.get("rollbacks") != str(counts["rollback_calls"])
            ):
                metric_reasons.append("rollback calls differ from logical marker")
            for name in ("peak_history_bytes", "peak_snapshot_bytes"):
                if name in counts and marker.fields.get(name) != str(counts[name]):
                    metric_reasons.append(f"{name} differs from logical marker")
        if {
            "max_rollback_ms",
            "rollback_p999_ms",
        }.issubset(numeric) and {
            "rollback_over_33_3_count",
            "rollback_sample_count",
        }.issubset(counts):
            sample_count = counts["rollback_sample_count"]
            over_count = counts["rollback_over_33_3_count"]
            if sample_count == 0:
                if (
                    numeric["rollback_p999_ms"] != 0
                    or numeric["max_rollback_ms"] != 0
                    or over_count != 0
                ):
                    metric_reasons.append("empty rollback diagnostics are nonzero")
            else:
                maximum_reaches_threshold = (
                    numeric["max_rollback_ms"] >= MAX_ROLLBACK_P999_MS
                )
                if maximum_reaches_threshold != (over_count > 0):
                    metric_reasons.append(
                        "rollback maximum disagrees with over-budget count"
                    )
                tail_slots = (
                    sample_count
                    - math.ceil(sample_count * ROLLBACK_PERCENTILE)
                    + 1
                )
                p999_reaches_threshold = (
                    numeric["rollback_p999_ms"] >= MAX_ROLLBACK_P999_MS
                )
                if p999_reaches_threshold != (over_count >= tail_slots):
                    metric_reasons.append(
                        "rollback p99.9 disagrees with over-budget count"
                    )
        if metric_reasons:
            reasons.append(
                f"{label} runtime metric {case_id} invalid: "
                + ", ".join(metric_reasons)
            )
            continue
        extracted.append(
            (
                (planned["scenario"], profile, seed),
                {
                    "case": case_id,
                    "max_rollback_ms": numeric["max_rollback_ms"],
                    "p95_work_ms": numeric["p95_work_ms"],
                    "rollback_over_33_3_count": counts[
                        "rollback_over_33_3_count"
                    ],
                    "rollback_p999_ms": numeric["rollback_p999_ms"],
                    "scenario": planned["scenario"],
                },
            )
        )
    if reasons:
        return [], reasons
    if len(extracted) != len(BROWSER_CPU_SCENARIOS):
        return [], [f"{label} did not extract both browser-full CPU cases"]
    return extracted, []


def browser_cpu_acceptance(
    runs: list[dict[str, Any]],
    browser_name: str,
) -> dict[str, Any]:
    """Apply the strict same-run, same-runtime, seed-paired browser CPU contract."""

    reasons: list[str] = []
    rows: dict[tuple[str, str, str], dict[str, Any]] = {}
    browser_versions: set[str] = set()
    for run_index, run in enumerate(runs, start=1):
        if not isinstance(run, dict):
            reasons.append(
                f"{browser_name} browser run {run_index} is not an evidence object"
            )
            continue
        if run.get("suite") != "browser-full":
            continue
        browser_version = run.get("browser_version")
        if isinstance(browser_version, str) and re.fullmatch(
            r"[0-9]+(?:\.[0-9]+)+",
            browser_version,
        ):
            browser_versions.add(browser_version)
        else:
            reasons.append(
                f"{browser_name} browser-full run {run_index} has malformed browser_version"
            )
        run_rows, row_reasons = browser_cpu_case(run, browser_name, run_index)
        reasons.extend(row_reasons)
        for key, row in run_rows:
            if key in rows:
                reasons.append(
                    f"{browser_name} has duplicate {key[0]} {key[1]} control "
                    f"for seed {key[2]}"
                )
                continue
            rows[key] = row
    if len(browser_versions) != 1:
        reasons.append(
            f"{browser_name} controls report {len(browser_versions)} browser versions, "
            "expected exactly one"
        )

    pairs: list[dict[str, Any]] = []
    for scenario in BROWSER_CPU_SCENARIOS:
        for seed_value in NETWORK_SEEDS:
            seed = str(seed_value)
            clean = rows.get((scenario, "clean", seed))
            playable = rows.get((scenario, "playable", seed))
            if clean is None:
                reasons.append(
                    f"{browser_name} is missing the {scenario} clean control for seed {seed}"
                )
            if playable is None:
                reasons.append(
                    f"{browser_name} is missing the {scenario} playable case for seed {seed}"
                )
            if clean is None or playable is None:
                continue
            clean_p95 = clean["p95_work_ms"]
            if not math.isfinite(clean_p95) or clean_p95 <= 0:
                reasons.append(
                    f"{browser_name} {scenario} seed {seed} clean p95 denominator "
                    "must be finite and >0"
                )
                continue
            p95_ratio = playable["p95_work_ms"] / clean_p95
            rollback_ratio = playable["rollback_p999_ms"] / clean_p95
            pair_reasons = []
            if p95_ratio >= MAX_BROWSER_P95_WORK_RATIO:
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} "
                    f"p95_work_ratio={p95_ratio:.9f} "
                    f"does not meet <{MAX_BROWSER_P95_WORK_RATIO:.1f}"
                )
            if rollback_ratio >= MAX_BROWSER_ROLLBACK_P999_RATIO:
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} "
                    f"rollback_p999_ratio={rollback_ratio:.9f} "
                    f"does not meet <{MAX_BROWSER_ROLLBACK_P999_RATIO:.1f}"
                )
            reasons.extend(pair_reasons)
            pairs.append(
                {
                    "absolute_diagnostics": {
                        "clean_max_rollback_ms": clean["max_rollback_ms"],
                        "clean_p95_work_ms": clean_p95,
                        "clean_rollback_over_33_3_count": clean[
                            "rollback_over_33_3_count"
                        ],
                        "clean_rollback_p999_ms": clean["rollback_p999_ms"],
                        "playable_max_rollback_ms": playable["max_rollback_ms"],
                        "playable_p95_work_ms": playable["p95_work_ms"],
                        "playable_rollback_over_33_3_count": playable[
                            "rollback_over_33_3_count"
                        ],
                        "playable_rollback_p999_ms": playable[
                            "rollback_p999_ms"
                        ],
                    },
                    "cases": {
                        "clean": clean["case"],
                        "playable": playable["case"],
                    },
                    "pass": not pair_reasons,
                    "ratios": {
                        "p95_work_over_clean_p95": round(p95_ratio, 9),
                        "rollback_p999_over_clean_p95": round(rollback_ratio, 9),
                    },
                    "reasons": pair_reasons,
                    "scenario": scenario,
                    "seed": seed_value,
                }
            )
    expected_keys = {
        (scenario, profile, str(seed))
        for scenario in BROWSER_CPU_SCENARIOS
        for profile in BROWSER_FULL_PROFILES
        for seed in NETWORK_SEEDS
    }
    unexpected_keys = sorted(set(rows).difference(expected_keys))
    for scenario, profile, seed in unexpected_keys:
        reasons.append(
            f"{browser_name} has unexpected {scenario} {profile} control for seed {seed}"
        )
    if len(rows) != len(expected_keys):
        reasons.append(
            f"{browser_name} collected {len(rows)} unique scenario/profile rows, "
            f"expected {len(expected_keys)}"
        )
    return {
        "browser": browser_name,
        "browser_version": (
            next(iter(browser_versions)) if len(browser_versions) == 1 else None
        ),
        "calibration": {
            "accepted_run_ids": list(BROWSER_CPU_CALIBRATION_RUNS),
            "diagnostic_failed_run_id": BROWSER_CPU_DIAGNOSTIC_RUN,
            "margin_over_accepted_maximum": BROWSER_CPU_CALIBRATION_MARGIN,
            "max_accepted_p95_work_ratio": round(
                BROWSER_CPU_CALIBRATION_MAX_P95_WORK_RATIO,
                9,
            ),
            "max_accepted_rollback_p999_ratio": round(
                BROWSER_CPU_CALIBRATION_MAX_ROLLBACK_P999_RATIO,
                9,
            ),
        },
        "gate_contract": int(GATE_CONTRACT),
        "method": "same_run_same_runtime_scenario_seed_paired",
        "pairs": pairs,
        "pass": not reasons
        and len(pairs) == len(BROWSER_CPU_SCENARIOS) * len(NETWORK_SEEDS),
        "reasons": reasons,
        "thresholds": {
            "comparison": "strict_less_than",
            "max_p95_work_over_clean_p95": MAX_BROWSER_P95_WORK_RATIO,
            "max_rollback_p999_over_clean_p95": MAX_BROWSER_ROLLBACK_P999_RATIO,
        },
    }


def browser_matrix(
    evidence: dict[str, Any],
    artifact: Path,
    browsers: list[str],
    raw_root: Path,
    timeout_seconds: int,
    allow_dirty: bool,
    campaign: str,
) -> None:
    provenance = artifact_provenance(artifact, allow_dirty)
    source = evidence["source"]
    if provenance["manifest"].get("source_revision") != source["revision"]:
        raise RuntimeError("browser artifact source revision does not match the checkout")
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        lambda *args, **kwargs: ArtifactHandler(
            *args,
            directory=str(artifact),
            **kwargs,
        ),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{server.server_port}/"
    browser_evidence: dict[str, Any] = {
        "artifact": provenance,
        "campaign": campaign,
        "plan": [
            {"arguments": list(arguments), "suite": suite}
            for suite, arguments in browser_plan(campaign)
        ],
        "runtimes": {},
        "selenium": selenium_metadata(),
    }
    evidence["browser"] = browser_evidence
    try:
        validate_manifest(base_url, provenance["manifest"])
        for browser_name in browsers:
            binary, driver_path = resolve_assets(browser_name, None, None)
            runtime: dict[str, Any] = {
                "binary": executable_metadata(binary),
                "driver": executable_metadata(driver_path),
                "runs": [],
            }
            browser_evidence["runtimes"][browser_name] = runtime
            for run_number, (suite, arguments) in enumerate(
                browser_plan(campaign),
                start=1,
            ):
                slug = "-".join((browser_name, suite, *arguments))
                suite_timeout_seconds = browser_suite_timeout_seconds(
                    suite,
                    timeout_seconds,
                )
                run = run_browser_once(
                    browser_name,
                    binary,
                    driver_path,
                    base_url,
                    suite,
                    arguments,
                    raw_root / f"{slug}.log",
                    suite_timeout_seconds,
                )
                run["timeout_seconds"] = suite_timeout_seconds
                run["run"] = run_number
                runtime["runs"].append(run)
                if suite == "soak" and not run["soak_memory"]["pass"]:
                    raise RuntimeError(
                        f"{browser_name} soak exceeded the 10% terminal "
                        "forced-GC growth gate"
                    )
            if campaign in {"all", "matrix"}:
                cpu_acceptance = browser_cpu_acceptance(
                    runtime["runs"],
                    browser_name,
                )
                runtime["cpu_acceptance"] = cpu_acceptance
                if not cpu_acceptance["pass"]:
                    reason = "; ".join(cpu_acceptance["reasons"])
                    raise RuntimeError(
                        f"{browser_name} aggregate browser CPU acceptance failed: {reason}"
                    )
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
    if thread.is_alive():
        raise RuntimeError("browser artifact server did not stop cleanly")


def run_self_test() -> None:
    validate_historical_soccer_evidence()
    if Path("/proc").is_dir():
        current = read_process_table().get(os.getpid())
        if current is None:
            raise RuntimeError("browser process census self-test cannot identify itself")
        executable = Path(sys.executable).resolve()
        census = browser_process_census("firefox", executable, executable)
        if current.identity not in census:
            raise RuntimeError("browser process census self-test missed an exact executable")
        if wait_browser_processes_gone(
            "firefox",
            executable,
            executable,
            set(census),
            0,
        ):
            raise RuntimeError("browser process census baseline self-test reported a false orphan")

    case = f"{MARKER_PREFIX}|case|schema=1|id=fixture|success=1"
    result = (
        f"{MARKER_PREFIX}|result|schema=1|suite=native|success=1|"
        "logical_digest=abc123|case_count=1"
    )
    markers = markers_from_messages(["noise", case, result])
    validated = validate_marker_set(markers, "native")
    if validated.fields["logical_digest"] != "abc123":
        raise RuntimeError("marker parsing self-test lost the logical digest")
    if compare_fresh_markers(markers, list(markers)) != sha256_bytes(
        (case + "\n" + result + "\n").encode()
    ):
        raise RuntimeError("fresh marker digest self-test failed")

    invalid_sets = [
        [],
        markers + [validated],
        [parse_marker(result.replace("case_count=1", "case_count=2"))],
        [
            parse_marker(case.replace("success=1", "success=0")),
            validated,
        ],
        [
            parse_marker(case),
            parse_marker(result.replace("success=1", "success=0")),
        ],
    ]
    for invalid in invalid_sets:
        try:
            validate_marker_set(invalid, "native")
        except RuntimeError:
            pass
        else:
            raise RuntimeError("invalid marker set passed self-test")
    try:
        compare_fresh_markers(markers, markers[:-1])
    except RuntimeError:
        pass
    else:
        raise RuntimeError("fresh marker disagreement passed self-test")

    expected_plan = [
        ("browser-full", ("clean", "2001")),
        ("browser-full", ("playable", "2001")),
        ("browser-full", ("clean", "2002")),
        ("browser-full", ("playable", "2002")),
        ("browser-full", ("clean", "2003")),
        ("browser-full", ("playable", "2003")),
        ("browser-stress", ("stress", "2001")),
        ("browser-stress", ("stress", "2002")),
        ("browser-stress", ("stress", "2003")),
        ("soak", ()),
    ]
    if browser_plan() != expected_plan:
        raise RuntimeError("browser matrix plan self-test failed")
    if browser_plan("matrix") != expected_plan[:-1]:
        raise RuntimeError("browser runtime-matrix campaign plan self-test failed")
    if browser_plan("soak") != [("soak", ())]:
        raise RuntimeError("browser soak campaign plan self-test failed")
    if browser_plan("matrix") + browser_plan("soak") != browser_plan("all"):
        raise RuntimeError("split browser campaigns do not reconstruct the full plan")
    if browser_suite_timeout_seconds("browser-full", 1800) != 1800:
        raise RuntimeError("single-fixture browser timeout scaling self-test failed")
    if browser_suite_timeout_seconds("soak", 1800) != 5400:
        raise RuntimeError("browser soak timeout scaling self-test failed")
    if browser_suite_timeout_seconds("soak", 7200) != 7200:
        raise RuntimeError("browser soak timeout upper-bound self-test failed")

    class FakeConsoleWaitDriver:
        def execute_async_script(self, script: str, cursor: int, timeout_ms: int) -> Any:
            if script != BROWSER_CONSOLE_WAIT_SCRIPT or timeout_ms != 1250:
                raise RuntimeError("browser console wait invocation self-test failed")
            delta_position = script.find("const delta =")
            scrub_position = script.find('entry.message = "";')
            if delta_position < 0 or scrub_position <= delta_position:
                raise RuntimeError("browser console wait scrubs messages before copying its delta")
            if "settleTimer = window.setTimeout(() => finish(false), 0);" not in script:
                raise RuntimeError("browser console wait does not batch synchronous marker rows")
            return {
                "cursor": cursor + 2,
                "entries": ["one", "two"],
                "status": "running",
                "timed_out": False,
            }

    console_wait = wait_for_browser_console_entries(FakeConsoleWaitDriver(), 3, 1.25)
    if console_wait != {
        "cursor": 5,
        "entries": ["one", "two"],
        "status": "running",
        "timed_out": False,
    }:
        raise RuntimeError("browser console wait result self-test failed")

    class FakeCheckpointSampler:
        def checkpoint(self, label: str) -> dict[str, Any]:
            return {"label": label}

    class FakeCheckpointDriver:
        def __init__(self) -> None:
            self.calls: list[str] = []

        def execute_cdp_cmd(
            self,
            method: str,
            _params: dict[str, Any],
        ) -> dict[str, Any]:
            self.calls.append(method)
            if method == "Performance.getMetrics":
                return {
                    "metrics": [
                        {"name": "JSHeapTotalSize", "value": 2000},
                        {"name": "JSHeapUsedSize", "value": 1000},
                    ]
                }
            return {}

    fake_checkpoint_driver = FakeCheckpointDriver()
    forced_checkpoint = browser_checkpoint(
        FakeCheckpointSampler(),  # type: ignore[arg-type]
        fake_checkpoint_driver,
        "chrome",
        "forced",
        force_js_gc=True,
    )
    if fake_checkpoint_driver.calls != [
        "Runtime.discardConsoleEntries",
        "HeapProfiler.collectGarbage",
        "Performance.getMetrics",
    ]:
        raise RuntimeError("Chrome forced-GC checkpoint ordering self-test failed")
    if (
        forced_checkpoint["js_heap"] != {"total_bytes": 2000, "used_bytes": 1000}
        or forced_checkpoint["js_console_entries_discarded"] is not True
        or forced_checkpoint["js_gc_forced"] is not True
    ):
        raise RuntimeError("Chrome forced-GC checkpoint evidence self-test failed")

    class FakeClientConfig:
        timeout = 120.0

    class FakeCommandExecutor:
        client_config = FakeClientConfig()

    class FakeCommandDriver:
        command_executor = FakeCommandExecutor()

    fake_command_driver = FakeCommandDriver()
    set_webdriver_command_timeout(fake_command_driver, 1810.0)
    if fake_command_driver.command_executor.client_config.timeout != 1810.0:
        raise RuntimeError("WebDriver command timeout self-test failed")

    class FakeCapabilityDriver:
        def __init__(self, capabilities: Any) -> None:
            self.capabilities = capabilities

    if (
        validated_browser_version(
            FakeCapabilityDriver(
                {"browserName": "firefox", "browserVersion": "153.0"}
            ),
            "firefox",
        )
        != "153.0"
    ):
        raise RuntimeError("raw browser-version bridge self-test lost the version")
    malformed_capabilities = (
        None,
        {},
        {"browserName": "chrome", "browserVersion": "153.0"},
        {"browserName": "firefox"},
        {"browserName": "firefox", "browserVersion": None},
        {"browserName": "firefox", "browserVersion": ""},
        {"browserName": "firefox", "browserVersion": 153.0},
        {"browserName": "firefox", "browserVersion": "None"},
        {"browserName": "firefox", "browserVersion": "153"},
        {"browserName": "firefox", "browserVersion": "153.0 beta"},
    )
    for capabilities in malformed_capabilities:
        try:
            validated_browser_version(
                FakeCapabilityDriver(capabilities),
                "firefox",
            )
        except RuntimeError:
            pass
        else:
            raise RuntimeError("malformed raw browser version passed self-test")
    expected_counts = {
        ("native", ()): 54,
        ("browser-full", ("clean", "2001")): 2,
        ("browser-stress", ("stress", "2001")): 10,
        ("late-window", ()): 2,
        ("soak", ()): 10,
    }
    for (suite, arguments), count in expected_counts.items():
        if len(expected_case_plan(suite, arguments)) != count:
            raise RuntimeError(f"{suite} pinned case-plan self-test failed")
    flattened_native_shards = [
        case
        for suite, arguments in native_shard_plan()
        for case in expected_case_plan(suite, arguments)
    ]
    if flattened_native_shards != expected_case_plan("native", ()):
        raise RuntimeError("native shard plan changed the pinned case order")
    expected_native_matrix = [*native_shard_plan(), ("late-window", ())]
    if native_campaign_plan("matrix") != expected_native_matrix:
        raise RuntimeError("native runtime-matrix campaign plan self-test failed")
    if native_campaign_plan("soak") != [("soak", ())]:
        raise RuntimeError("native soak campaign plan self-test failed")
    if (
        native_campaign_plan("matrix") + native_campaign_plan("soak")
        != native_campaign_plan()
    ):
        raise RuntimeError("split native campaigns do not reconstruct the full plan")

    calibrated_p95 = (
        math.ceil(
            BROWSER_CPU_CALIBRATION_MAX_P95_WORK_RATIO
            * (1 + BROWSER_CPU_CALIBRATION_MARGIN)
            * 10
        )
        / 10
    )
    calibrated_rollback = (
        math.ceil(
            BROWSER_CPU_CALIBRATION_MAX_ROLLBACK_P999_RATIO
            * (1 + BROWSER_CPU_CALIBRATION_MARGIN)
            * 10
        )
        / 10
    )
    if (
        calibrated_p95 != MAX_BROWSER_P95_WORK_RATIO
        or calibrated_rollback != MAX_BROWSER_ROLLBACK_P999_RATIO
    ):
        raise RuntimeError("browser CPU calibration margin self-test failed")

    def synthetic_browser_cpu_run(
        profile: str,
        seed: int,
        p95_work_ms: float,
        rollback_p999_ms: float,
        *,
        combat_p95_work_ms: float | None = None,
        combat_rollback_p999_ms: float | None = None,
        browser_name: str = "firefox",
        browser_version: str = "153.0",
        marker_seed: int | None = None,
    ) -> dict[str, Any]:
        emitted_seed = marker_seed or seed
        cpu_gate = "deferred" if profile == "playable" else "not_applied"
        cpu_mode = "normalized_deferred" if profile == "playable" else "diagnostic"
        combat_p95 = (
            combat_p95_work_ms
            if combat_p95_work_ms is not None
            else p95_work_ms * 1.2
        )
        combat_rollback = (
            combat_rollback_p999_ms
            if combat_rollback_p999_ms is not None
            else rollback_p999_ms * 1.2
        )

        def logical_case(scenario: str) -> ValidationMarker:
            combat = scenario == "combat"
            prefix = "combat" if combat else "full"
            case_id = f"{prefix}-{profile}-{emitted_seed}"
            rollbacks = 0 if profile == "clean" else 8
            peak_snapshot_bytes = 688660 if combat else 611274
            peak_history_bytes = 743170 if combat else 700000
            event_digest = "0000000000000004" if combat else "0000000000000003"
            return parse_marker(
                f"{MARKER_PREFIX}|case|schema=1|case={case_id}|scenario={scenario}|"
                f"fixture={BROWSER_CPU_FIXTURES[scenario]}|"
                f"profile={profile}|network_seed={emitted_seed}|success=1|"
                "lab_success=1|expected_failure=0|status=converged|late_tick=none|"
                "hidden_progress=0|scenario_pass=1|"
                f"tape_version={'2' if combat else '1'}|"
                f"snapshot_version={'6' if combat else '5'}|"
                f"tape_digest={'1111111111111111' if combat else HISTORICAL_SOCCER_TAPE_DIGEST}|"
                "initial_hash=0000000000000001|reference_hash=0000000000000002|"
                f"client_hash=0000000000000002|rollbacks={rollbacks}|max_depth=8|"
                f"resimulated={20 if profile == 'playable' else 0}|"
                f"peak_snapshots=31|peak_snapshot_bytes={peak_snapshot_bytes}|"
                f"peak_history_bytes={peak_history_bytes}|"
                f"event_reference_digest={event_digest}|"
                f"event_confirmed_digest={event_digest}|"
                f"event_confirmed_combat={14 if combat else 0}|event_residue=0|"
                f"sample=none|gate_contract={GATE_CONTRACT}|cpu_gate={cpu_gate}|"
                f"cpu_gate_applied=0|cpu_gate_mode={cpu_mode}|snapshot_gate=1|"
                "history_gate=1|game_gate=1"
            )

        logical_cases = [
            logical_case("complete_fixture"),
            logical_case("combat"),
        ]
        result = parse_marker(
            f"{MARKER_PREFIX}|result|schema=1|suite=browser-full|success=1|"
            "logical_digest=0000000000000005|case_count=2"
        )
        markers = [*logical_cases, result]
        runtime_metric = parse_runtime_metric(
            f"{METRICS_PREFIX}|runtime|love=11.5.0|suite=browser-full|"
            f"gate_contract={GATE_CONTRACT}|profile_digest={EXPECTED_PROFILE_DIGEST}|"
            "input_version=2|tape_versions=1,2|snapshot_versions=5,6|tick_rate=60"
        )

        def case_metric(
            scenario: str,
            case_p95_work_ms: float,
            case_rollback_p999_ms: float,
        ) -> RuntimeMetric:
            combat = scenario == "combat"
            prefix = "combat" if combat else "full"
            case_id = f"{prefix}-{profile}-{seed}"
            rollback_calls = 0 if profile == "clean" else 8
            over_count = (
                1
                if rollback_calls > 0
                and case_rollback_p999_ms >= MAX_ROLLBACK_P999_MS
                else 0
            )
            peak_snapshot_bytes = 688660 if combat else 611274
            peak_history_bytes = 743170 if combat else 700000
            return parse_runtime_metric(
                f"{METRICS_PREFIX}|case|case={case_id}|profile={profile}|"
                f"p95_work_ms={case_p95_work_ms:.6f}|"
                f"rollback_p999_ms={case_rollback_p999_ms:.6f}|"
                f"max_rollback_ms={case_rollback_p999_ms:.6f}|"
                f"rollback_sample_count={rollback_calls}|"
                f"rollback_over_33_3_count={over_count}|"
                "rollback_percentile=0.999|rollback_percentile_method=nearest_rank|"
                "rollback_timing_evidence=raw|p95_update_wall_ms=1.000000|"
                "max_update_wall_ms=2.000000|simulation_ms=3.000000|"
                "capture_ms=4.000000|restore_ms=5.000000|"
                "resimulation_ms=6.000000|rollback_ms=7.000000|capture_calls=80|"
                f"simulation_calls=80|restore_calls={rollback_calls}|"
                f"resimulation_calls={rollback_calls}|rollback_calls={rollback_calls}|"
                f"work_samples=80|peak_snapshot_bytes={peak_snapshot_bytes}|"
                f"peak_history_bytes={peak_history_bytes}"
            )

        metrics = [
            runtime_metric,
            case_metric("complete_fixture", p95_work_ms, rollback_p999_ms),
            case_metric("combat", combat_p95, combat_rollback),
        ]
        marker_payload = ("\n".join(marker.raw for marker in markers) + "\n").encode()
        metric_payload = ("\n".join(metric.raw for metric in metrics) + "\n").encode()
        return {
            "arguments": [profile, str(seed)],
            "browser": browser_name,
            "browser_version": browser_version,
            "case_count": 2,
            "logical_digest": result.fields["logical_digest"],
            "logical_marker_sha256": sha256_bytes(marker_payload),
            "markers": [marker.raw for marker in markers],
            "result_fields": result.fields,
            "runtime_metrics": {
                "marker_sha256": sha256_bytes(metric_payload),
                "rows": [
                    {
                        "fields": metric.fields,
                        "kind": metric.kind,
                        "marker": metric.raw,
                    }
                    for metric in metrics
                ]
            },
            "suite": "browser-full",
        }

    def synthetic_browser_cpu_matrix(
        scale: float,
        soccer_rollback_regression_seeds: tuple[int, ...] = (),
        combat_p95_regression_seeds: tuple[int, ...] = (),
    ) -> list[dict[str, Any]]:
        runs = []
        for index, seed in enumerate(NETWORK_SEEDS):
            for profile in BROWSER_FULL_PROFILES:
                clean_p95 = (2.0 + index * 0.25) * scale
                combat_clean_p95 = clean_p95 * 1.2
                if profile == "clean":
                    p95_work = clean_p95
                    combat_p95_work = combat_clean_p95
                    rollback_p999 = 0.0
                    combat_rollback_p999 = 0.0
                else:
                    p95_work = clean_p95 * 5.5
                    rollback_ratio = 9.5
                    if seed in soccer_rollback_regression_seeds:
                        rollback_ratio = MAX_BROWSER_ROLLBACK_P999_RATIO + 0.1
                    rollback_p999 = clean_p95 * rollback_ratio
                    combat_p95_ratio = 5.4
                    if seed in combat_p95_regression_seeds:
                        combat_p95_ratio = MAX_BROWSER_P95_WORK_RATIO + 0.1
                    combat_p95_work = combat_clean_p95 * combat_p95_ratio
                    combat_rollback_p999 = combat_clean_p95 * 9.0
                runs.append(
                    synthetic_browser_cpu_run(
                        profile,
                        seed,
                        p95_work,
                        rollback_p999,
                        combat_p95_work_ms=combat_p95_work,
                        combat_rollback_p999_ms=combat_rollback_p999,
                    )
                )
        return runs

    proportional_slowdown = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(1.8),
        "firefox",
    )
    if not proportional_slowdown["pass"] or len(proportional_slowdown["pairs"]) != 6:
        raise RuntimeError("proportional browser slowdown did not pass normalization")
    multi_family_regression = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(
            1.0,
            soccer_rollback_regression_seeds=(2001,),
            combat_p95_regression_seeds=(2002,),
        ),
        "firefox",
    )
    expected_regressions = (
        "complete_fixture seed 2001 rollback_p999_ratio",
        "combat seed 2002 p95_work_ratio",
    )
    if multi_family_regression["pass"] or not all(
        any(fragment in reason for reason in multi_family_regression["reasons"])
        for fragment in expected_regressions
    ):
        raise RuntimeError("aggregate browser CPU failure omitted a case-family violation")
    complete_controls = synthetic_browser_cpu_matrix(1.0)

    def replace_control(
        controls: list[dict[str, Any]],
        replacement: dict[str, Any],
    ) -> list[dict[str, Any]]:
        arguments = replacement["arguments"]
        return [
            replacement if control["arguments"] == arguments else control
            for control in controls
        ]

    exact_p95_boundary = browser_cpu_acceptance(
        replace_control(
            complete_controls,
            synthetic_browser_cpu_run(
                "playable",
                2001,
                2.0 * MAX_BROWSER_P95_WORK_RATIO,
                2.0 * 9.5,
                combat_p95_work_ms=2.4 * 5.4,
                combat_rollback_p999_ms=2.4 * 9.0,
            ),
        ),
        "firefox",
    )
    if exact_p95_boundary["pass"] or not any(
        "complete_fixture seed 2001 p95_work_ratio=6.700000000" in reason
        for reason in exact_p95_boundary["reasons"]
    ):
        raise RuntimeError("exact browser p95 ratio threshold passed strict gate")
    exact_rollback_boundary = browser_cpu_acceptance(
        replace_control(
            complete_controls,
            synthetic_browser_cpu_run(
                "playable",
                2002,
                2.25 * 5.5,
                2.25 * MAX_BROWSER_ROLLBACK_P999_RATIO,
                combat_p95_work_ms=2.7 * 5.4,
                combat_rollback_p999_ms=2.7 * 9.0,
            ),
        ),
        "firefox",
    )
    if exact_rollback_boundary["pass"] or not any(
        "complete_fixture seed 2002 rollback_p999_ratio=11.700000000" in reason
        for reason in exact_rollback_boundary["reasons"]
    ):
        raise RuntimeError("exact browser rollback ratio threshold passed strict gate")
    missing_control = browser_cpu_acceptance(complete_controls[:-1], "firefox")
    if missing_control["pass"] or not any(
        "missing the complete_fixture playable case for seed 2003" in reason
        for reason in missing_control["reasons"]
    ):
        raise RuntimeError("missing browser CPU control passed normalization")
    duplicate_control = browser_cpu_acceptance(
        [*complete_controls, complete_controls[0]],
        "firefox",
    )
    if duplicate_control["pass"] or not any(
        "duplicate complete_fixture clean control for seed 2001" in reason
        for reason in duplicate_control["reasons"]
    ):
        raise RuntimeError("duplicate browser CPU control passed normalization")
    mismatched_controls = [
        synthetic_browser_cpu_run(
            "clean",
            2001,
            2.0,
            0.0,
            marker_seed=2002,
        ),
        *complete_controls[1:],
    ]
    mismatched_control = browser_cpu_acceptance(mismatched_controls, "firefox")
    if mismatched_control["pass"] or not any(
        "marker contract failed" in reason for reason in mismatched_control["reasons"]
    ):
        raise RuntimeError("mismatched browser CPU control passed normalization")
    non_string_marker_run = {
        **complete_controls[0],
        "markers": [*complete_controls[0]["markers"], 42],
    }
    non_string_marker = browser_cpu_acceptance(
        [non_string_marker_run, *complete_controls[1:]],
        "firefox",
    )
    if non_string_marker["pass"] or not any(
        "non-string validation marker" in reason
        for reason in non_string_marker["reasons"]
    ):
        raise RuntimeError("non-string browser CPU marker passed normalization")
    malformed_metric_run = {
        **complete_controls[0],
        "runtime_metrics": {
            **complete_controls[0]["runtime_metrics"],
            "rows": [
                *complete_controls[0]["runtime_metrics"]["rows"],
                {"kind": "case"},
            ],
        },
    }
    malformed_metric = browser_cpu_acceptance(
        [malformed_metric_run, *complete_controls[1:]],
        "firefox",
    )
    if (
        malformed_metric["pass"]
        or "firefox browser-full run 1 runtime metric row 4 has malformed schema"
        not in malformed_metric["reasons"]
    ):
        raise RuntimeError("malformed extra browser CPU metric passed normalization")
    mismatched_logical_digest_run = {
        **complete_controls[0],
        "logical_marker_sha256": "0" * 64,
    }
    mismatched_logical_digest = browser_cpu_acceptance(
        [mismatched_logical_digest_run, *complete_controls[1:]],
        "firefox",
    )
    if (
        mismatched_logical_digest["pass"]
        or "firefox browser-full run 1 logical marker digest is missing or mismatched"
        not in mismatched_logical_digest["reasons"]
    ):
        raise RuntimeError("mismatched browser logical marker digest passed normalization")
    mismatched_metric_digest_run = {
        **complete_controls[0],
        "runtime_metrics": {
            **complete_controls[0]["runtime_metrics"],
            "marker_sha256": "0" * 64,
        },
    }
    mismatched_metric_digest = browser_cpu_acceptance(
        [mismatched_metric_digest_run, *complete_controls[1:]],
        "firefox",
    )
    if (
        mismatched_metric_digest["pass"]
        or "firefox browser-full run 1 runtime metric digest is missing or mismatched"
        not in mismatched_metric_digest["reasons"]
    ):
        raise RuntimeError("mismatched browser runtime metric digest passed normalization")

    def metric_record(rows: list[dict[str, Any]]) -> dict[str, Any]:
        payload = ("\n".join(row["marker"] for row in rows) + "\n").encode()
        return {"marker_sha256": sha256_bytes(payload), "rows": rows}

    missing_combat_metric_run = {
        **complete_controls[0],
        "runtime_metrics": metric_record(
            complete_controls[0]["runtime_metrics"]["rows"][:2]
        ),
    }
    missing_combat_metric = browser_cpu_acceptance(
        [missing_combat_metric_run, *complete_controls[1:]],
        "firefox",
    )
    if missing_combat_metric["pass"] or not any(
        "0 runtime metrics for combat-clean-2001" in reason
        for reason in missing_combat_metric["reasons"]
    ):
        raise RuntimeError("missing combat CPU metric passed normalization")
    duplicated_combat_rows = [
        *complete_controls[0]["runtime_metrics"]["rows"],
        complete_controls[0]["runtime_metrics"]["rows"][2],
    ]
    duplicate_combat_metric_run = {
        **complete_controls[0],
        "runtime_metrics": metric_record(duplicated_combat_rows),
    }
    duplicate_combat_metric = browser_cpu_acceptance(
        [duplicate_combat_metric_run, *complete_controls[1:]],
        "firefox",
    )
    if duplicate_combat_metric["pass"] or not any(
        "2 runtime metrics for combat-clean-2001" in reason
        for reason in duplicate_combat_metric["reasons"]
    ):
        raise RuntimeError("duplicate combat CPU metric passed normalization")

    def marker_record(raw_markers: list[str]) -> dict[str, Any]:
        parsed = [parse_marker(raw) for raw in raw_markers]
        result_marker = next(marker for marker in parsed if marker.kind == "result")
        payload = ("\n".join(raw_markers) + "\n").encode()
        return {
            "case_count": int(result_marker.fields["case_count"]),
            "logical_digest": result_marker.fields["logical_digest"],
            "logical_marker_sha256": sha256_bytes(payload),
            "markers": raw_markers,
            "result_fields": result_marker.fields,
        }

    missing_combat_markers = [
        complete_controls[0]["markers"][0],
        complete_controls[0]["markers"][2],
    ]
    missing_combat_marker_run = {
        **complete_controls[0],
        **marker_record(missing_combat_markers),
    }
    missing_combat_marker = browser_cpu_acceptance(
        [missing_combat_marker_run, *complete_controls[1:]],
        "firefox",
    )
    if missing_combat_marker["pass"] or not any(
        "expected exactly two cases and one result" in reason
        for reason in missing_combat_marker["reasons"]
    ):
        raise RuntimeError("missing combat logical marker passed normalization")
    corrupted_combat_markers = [
        complete_controls[0]["markers"][0],
        complete_controls[0]["markers"][1].replace(
            "event_confirmed_combat=14",
            "event_confirmed_combat=0",
        ),
        complete_controls[0]["markers"][2],
    ]
    corrupted_combat_marker_run = {
        **complete_controls[0],
        **marker_record(corrupted_combat_markers),
    }
    corrupted_combat_marker = browser_cpu_acceptance(
        [corrupted_combat_marker_run, *complete_controls[1:]],
        "firefox",
    )
    if corrupted_combat_marker["pass"] or not any(
        "did not confirm a combat event" in reason
        for reason in corrupted_combat_marker["reasons"]
    ):
        raise RuntimeError("corrupted combat logical marker passed normalization")
    wrong_browser_controls = [
        synthetic_browser_cpu_run("clean", 2001, 2.0, 0.0, browser_name="chrome"),
        *complete_controls[1:],
    ]
    wrong_browser = browser_cpu_acceptance(wrong_browser_controls, "firefox")
    if wrong_browser["pass"] or not any(
        "reports browser='chrome'" in reason for reason in wrong_browser["reasons"]
    ):
        raise RuntimeError("cross-browser CPU control passed normalization")
    mixed_versions = [
        synthetic_browser_cpu_run(
            "clean",
            2001,
            2.0,
            0.0,
            browser_version="154.0",
        ),
        *complete_controls[1:],
    ]
    mixed_version = browser_cpu_acceptance(mixed_versions, "firefox")
    if mixed_version["pass"] or not any(
        "report 2 browser versions" in reason for reason in mixed_version["reasons"]
    ):
        raise RuntimeError("mixed browser-version controls passed normalization")
    zero_denominator_controls = [
        synthetic_browser_cpu_run("clean", 2001, 0.0, 0.0),
        *complete_controls[1:],
    ]
    zero_denominator = browser_cpu_acceptance(zero_denominator_controls, "firefox")
    if zero_denominator["pass"] or not any(
        "complete_fixture seed 2001 clean p95 denominator must be finite and >0"
        in reason
        for reason in zero_denominator["reasons"]
    ):
        raise RuntimeError("zero browser CPU control denominator passed normalization")
    try:
        raise_on_interruption(signal.SIGTERM, None)
    except InterruptedError as error:
        if "SIGTERM" not in str(error):
            raise RuntimeError("interruption handler lost the signal name") from error
    else:
        raise RuntimeError("interruption handler self-test failed")

    integrity_case = parse_marker(
        f"{MARKER_PREFIX}|case|schema=1|case=integrity|scenario=complete_fixture|"
        "profile=playable|success=1|"
        "lab_success=1|expected_failure=0|hidden_progress=0|scenario_pass=1|"
        "gate_contract=5|cpu_gate=1|cpu_gate_applied=1|cpu_gate_mode=absolute|"
        "snapshot_gate=1|"
        "history_gate=1|game_gate=1|rollbacks=6903|"
        "tape_version=1|snapshot_version=5|"
        "initial_hash=0000000000000001|reference_hash=0000000000000002|"
        "client_hash=0000000000000002|tape_digest=881917e3ba798703|resimulated=42|"
        "event_reference_digest=0000000000000003|"
        "event_confirmed_digest=0000000000000003|event_confirmed_combat=0|"
        "event_residue=0|peak_snapshots=31|"
        "peak_snapshot_bytes=614399|peak_history_bytes=1048575"
    )
    validate_case_integrity([integrity_case], "native")

    def expect_integrity_failure(raw: str, suite: str, description: str) -> None:
        try:
            validate_case_integrity([parse_marker(raw)], suite)
        except RuntimeError:
            return
        raise RuntimeError(f"{description} passed self-test")

    expect_integrity_failure(
        integrity_case.raw.replace("gate_contract=5", "gate_contract=4"),
        "native",
        "contract-4 case",
    )
    expect_integrity_failure(
        integrity_case.raw.replace("peak_snapshots=31", "peak_snapshots=32"),
        "native",
        "over-budget playable case",
    )
    near_snapshot_limit = parse_marker(
        integrity_case.raw.replace("peak_snapshot_bytes=614399", "peak_snapshot_bytes=786431")
    )
    validate_case_integrity([near_snapshot_limit], "native")
    expect_integrity_failure(
        integrity_case.raw.replace("peak_snapshot_bytes=614399", "peak_snapshot_bytes=786432"),
        "native",
        "768 KiB inclusive snapshot limit",
    )
    soak_case = parse_marker(
        integrity_case.raw.replace(
            "cpu_gate=1|cpu_gate_applied=1|cpu_gate_mode=absolute",
            "cpu_gate=not_applied|cpu_gate_applied=0|cpu_gate_mode=diagnostic",
        )
    )
    validate_case_integrity([soak_case], "soak")
    deferred_browser_case = parse_marker(
        integrity_case.raw.replace(
            "cpu_gate=1|cpu_gate_applied=1|cpu_gate_mode=absolute",
            "cpu_gate=deferred|cpu_gate_applied=0|cpu_gate_mode=normalized_deferred",
        )
    )
    validate_case_integrity(
        [deferred_browser_case],
        "browser-full",
        browser_runtime=True,
    )
    for inconsistent, inconsistent_suite in (
        (soak_case.raw.replace("cpu_gate=not_applied", "cpu_gate=1"), "soak"),
        (soak_case.raw, "native"),
        (integrity_case.raw, "soak"),
        (integrity_case.raw.replace("profile=playable", "profile=stress"), "native"),
    ):
        expect_integrity_failure(
            inconsistent,
            inconsistent_suite,
            "inconsistent CPU gate ownership",
        )
    try:
        validate_case_integrity(
            [integrity_case],
            "browser-full",
            browser_runtime=True,
        )
    except RuntimeError:
        pass
    else:
        raise RuntimeError("absolute browser CPU ownership passed self-test")

    def timing_series(case_id: str, samples_us: tuple[int, ...]) -> RollbackTimingSeries:
        return parse_rollback_timings(
            "|".join(
                (
                    TIMINGS_PREFIX,
                    "case",
                    "gate_contract=5",
                    f"case={case_id}",
                    f"sample_count={len(samples_us)}",
                    "unit=microseconds",
                    "samples=" + ",".join(str(value) for value in samples_us),
                )
            )
        )

    def metric_for_samples(
        samples_us: tuple[int, ...],
        p95_work_ms: str = "1.25",
        timing_evidence: str = "raw",
    ) -> RuntimeMetric:
        p999_ms = nearest_rank_integer(samples_us, ROLLBACK_PERCENTILE) / 1000
        maximum_ms = nearest_rank_integer(samples_us, 1) / 1000
        over_count = sum(sample >= MAX_ROLLBACK_P999_US for sample in samples_us)
        return parse_runtime_metric(
            f"{METRICS_PREFIX}|case|case=integrity|profile=playable|"
            f"p95_work_ms={p95_work_ms}|rollback_p999_ms={p999_ms:.6f}|"
            f"max_rollback_ms={maximum_ms:.6f}|"
            f"rollback_sample_count={len(samples_us)}|"
            f"rollback_over_33_3_count={over_count}|"
            "rollback_percentile=0.999|rollback_percentile_method=nearest_rank|"
            f"rollback_timing_evidence={timing_evidence}|"
            "p95_update_wall_ms=3|max_update_wall_ms=4|simulation_ms=5|"
            "capture_ms=6|restore_ms=7|resimulation_ms=8|rollback_ms=9|"
            "capture_calls=10|simulation_calls=11|restore_calls=12|"
            f"resimulation_calls=13|rollback_calls={len(samples_us)}|work_samples=15"
        )

    runtime_provenance = parse_runtime_metric(
        f"{METRICS_PREFIX}|runtime|love=11.5.0|suite=native|"
        f"gate_contract={GATE_CONTRACT}|profile_digest={EXPECTED_PROFILE_DIGEST}|input_version=2|"
        "tape_versions=1,2|snapshot_versions=5,6|tick_rate=60"
    )
    passing_samples = (10000,) * 6897 + (33301, 33400, 34000, 35000, 40000, 46040)
    passing_timing = timing_series("integrity", passing_samples)
    passing_metric = metric_for_samples(passing_samples)
    validate_runtime_metrics(
        [runtime_provenance, passing_metric],
        [passing_timing],
        [integrity_case],
        "native",
    )
    if passing_metric.fields["max_rollback_ms"] != "46.040000":
        raise RuntimeError("raw 46.04 ms maximum was not preserved diagnostically")
    if passing_metric.fields["rollback_over_33_3_count"] != "6":
        raise RuntimeError("six-over-budget p99.9 boundary self-test failed")

    def expect_runtime_failure(
        runtime: RuntimeMetric,
        metric: RuntimeMetric,
        timings: list[RollbackTimingSeries],
        case_marker: ValidationMarker,
        suite: str,
        description: str,
    ) -> None:
        try:
            validate_runtime_metrics(
                [runtime, metric],
                timings,
                [case_marker],
                suite,
            )
        except RuntimeError:
            return
        raise RuntimeError(f"{description} passed self-test")

    threshold_samples = (10000,) * 6896 + (
        33300,
        33301,
        34000,
        35000,
        36000,
        40000,
        46040,
    )
    threshold_timing = timing_series("integrity", threshold_samples)
    threshold_metric = metric_for_samples(threshold_samples)
    expect_runtime_failure(
        runtime_provenance,
        threshold_metric,
        [threshold_timing],
        integrity_case,
        "native",
        "seven-over-budget exact-threshold p99.9 metric",
    )
    if threshold_metric.fields["rollback_p999_ms"] != "33.300000":
        raise RuntimeError("exact p99.9 threshold self-test did not reach 33.3 ms")

    malformed_timing_lines = (
        passing_timing.raw.replace("|unit=microseconds", ""),
        passing_timing.raw.replace("gate_contract=5", "gate_contract=4"),
        passing_timing.raw.replace("samples=10000", "samples=bad", 1),
        passing_timing.raw.replace("samples=10000", "samples=-1", 1),
        passing_timing.raw.replace("sample_count=6903", "sample_count=6902"),
    )
    for malformed in malformed_timing_lines:
        try:
            parse_rollback_timings(malformed)
        except RuntimeError:
            pass
        else:
            raise RuntimeError("malformed raw rollback timings passed self-test")

    expect_runtime_failure(
        runtime_provenance,
        passing_metric,
        [],
        integrity_case,
        "native",
        "missing raw rollback timings",
    )
    expect_runtime_failure(
        runtime_provenance,
        passing_metric,
        [passing_timing, passing_timing],
        integrity_case,
        "native",
        "duplicate raw rollback timing series",
    )
    expect_runtime_failure(
        runtime_provenance,
        passing_metric,
        [timing_series("unknown-case", passing_samples)],
        integrity_case,
        "native",
        "unknown-case raw rollback timing series",
    )
    expect_runtime_failure(
        runtime_provenance,
        passing_metric,
        [passing_timing],
        parse_marker(integrity_case.raw.replace("rollbacks=6903", "rollbacks=6902")),
        "native",
        "logical rollback and timing call-count drift",
    )
    for mismatched_metric, description in (
        (
            parse_runtime_metric(
                passing_metric.raw.replace("rollback_p999_ms=10.000000", "rollback_p999_ms=10.001000")
            ),
            "mismatched reported p99.9",
        ),
        (
            parse_runtime_metric(
                passing_metric.raw.replace("max_rollback_ms=46.040000", "max_rollback_ms=46.039000")
            ),
            "mismatched reported maximum",
        ),
        (
            parse_runtime_metric(
                passing_metric.raw.replace(
                    "rollback_over_33_3_count=6",
                    "rollback_over_33_3_count=5",
                )
            ),
            "mismatched over-budget count",
        ),
        (
            parse_runtime_metric(
                passing_metric.raw.replace(
                    "rollback_sample_count=6903",
                    "rollback_sample_count=6902",
                )
            ),
            "mismatched rollback sample count",
        ),
    ):
        expect_runtime_failure(
            runtime_provenance,
            mismatched_metric,
            [passing_timing],
            integrity_case,
            "native",
            description,
        )

    contract_4_runtime = parse_runtime_metric(
        runtime_provenance.raw.replace("gate_contract=5", "gate_contract=4")
    )
    expect_runtime_failure(
        contract_4_runtime,
        passing_metric,
        [passing_timing],
        integrity_case,
        "native",
        "contract-4 runtime provenance",
    )
    stale_profile = parse_runtime_metric(
        runtime_provenance.raw.replace(EXPECTED_PROFILE_DIGEST, "0000000000000000")
    )
    expect_runtime_failure(
        stale_profile,
        passing_metric,
        [passing_timing],
        integrity_case,
        "native",
        "stale network-profile digest",
    )
    expect_runtime_failure(
        runtime_provenance,
        metric_for_samples(passing_samples, "16.67"),
        [passing_timing],
        integrity_case,
        "native",
        "over-budget p95 work metric",
    )
    browser_provenance = parse_runtime_metric(
        runtime_provenance.raw.replace("suite=native", "suite=browser-full")
    )
    deferred_threshold_metric = metric_for_samples(
        threshold_samples,
        p95_work_ms="16.67",
    )
    validate_runtime_metrics(
        [browser_provenance, deferred_threshold_metric],
        [threshold_timing],
        [deferred_browser_case],
        "browser-full",
        browser_runtime=True,
    )

    soak_provenance = parse_runtime_metric(
        runtime_provenance.raw.replace("suite=native", "suite=soak")
    )
    soak_metric = metric_for_samples(
        threshold_samples,
        timing_evidence="aggregate_diagnostic",
    )
    validate_runtime_metrics(
        [soak_provenance, soak_metric],
        [],
        [soak_case],
        "soak",
    )
    expect_runtime_failure(
        soak_provenance,
        soak_metric,
        [threshold_timing],
        soak_case,
        "soak",
        "soak raw rollback timing series",
    )
    expect_runtime_failure(
        soak_provenance,
        threshold_metric,
        [],
        soak_case,
        "soak",
        "soak raw timing evidence ownership",
    )
    for inconsistent_soak, description in (
        (
            parse_runtime_metric(
                soak_metric.raw.replace(
                    "max_rollback_ms=46.040000",
                    "max_rollback_ms=33.299000",
                )
            ),
            "soak maximum below p99.9",
        ),
        (
            parse_runtime_metric(
                soak_metric.raw.replace(
                    "rollback_over_33_3_count=7",
                    "rollback_over_33_3_count=0",
                )
            ),
            "soak maximum and over-budget count disagreement",
        ),
        (
            parse_runtime_metric(
                soak_metric.raw.replace(
                    "rollback_p999_ms=33.300000",
                    "rollback_p999_ms=10.000000",
                )
            ),
            "soak p99.9 and over-budget count disagreement",
        ),
    ):
        expect_runtime_failure(
            soak_provenance,
            inconsistent_soak,
            [],
            soak_case,
            "soak",
            description,
        )

    soak_cases = [
        parse_marker(
            f"{MARKER_PREFIX}|case|schema=1|sample={sample}|forced_gc=1|"
            f"lua_heap_bytes={1000 + index * 10}|logical_digest={index:016x}|success=1"
        )
        for index, sample in enumerate(SOAK_SAMPLES)
    ]
    soak_result = parse_marker(
        f"{MARKER_PREFIX}|result|schema=1|suite=soak|success=1|"
        "logical_digest=soak|case_count=5"
    )
    soak_markers = [*soak_cases, soak_result]
    validate_marker_set(soak_markers, "soak")
    soak_resources = {
        "checkpoints": [
            {
                "js_heap": {"used_bytes": 2000 + index * 10},
                "rss_bytes": 3000 + index * 10,
                "validation_marker": soak_cases[index].raw,
            }
            for index in range(len(SOAK_SAMPLES))
        ]
    }
    soak_gate = soak_memory_evidence(soak_markers, soak_resources, "chrome")
    if not soak_gate["pass"]:
        raise RuntimeError("passing soak memory self-test failed")
    missing_final_resources = {
        "checkpoints": list(soak_resources["checkpoints"][:-1])
    }
    try:
        soak_memory_evidence(soak_markers, missing_final_resources, "chrome")
    except RuntimeError:
        pass
    else:
        raise RuntimeError("missing final external memory checkpoint passed self-test")
    inclusive_growth = growth_gate(
        {"warmup": 1000, "middle": 1090, "final": 1100},
        "inclusive-threshold",
    )
    if not inclusive_growth["pass"]:
        raise RuntimeError("inclusive memory-growth threshold failed self-test")
    if growth_gate({"warmup": 1000, "middle": 1090, "final": 1101}, "over-threshold")[
        "pass"
    ]:
        raise RuntimeError("over-threshold memory growth passed self-test")
    transient_peak = growth_gate(
        {"warmup": 1000, "middle": 1200, "final": 1090},
        "transient-peak",
    )
    if (
        not transient_peak["pass"]
        or transient_peak["growth_percent"] != 9.0
        or transient_peak["measurement"] != "final_vs_warmup"
        or transient_peak["peak_growth_percent"] != 20.0
        or transient_peak["terminal_bytes"] != 1090
    ):
        raise RuntimeError("transient memory peak self-test failed")
    soak_resources["checkpoints"][-1]["rss_bytes"] = 4000
    if soak_memory_evidence(soak_markers, soak_resources, "chrome")["pass"]:
        raise RuntimeError("over-budget soak memory self-test passed")

    late_case = parse_marker(
        f"{MARKER_PREFIX}|case|schema=1|success=1|lab_success=0|expected_failure=1|"
        "case=delay-31|status=late_input_unrecoverable|late_tick=0|hidden_progress=0"
    )
    supported_late_case = parse_marker(
        f"{MARKER_PREFIX}|case|schema=1|success=1|lab_success=1|expected_failure=0|"
        "case=delay-30|status=converged|late_tick=none|hidden_progress=0|max_depth=30"
    )
    validate_late_window_contract([supported_late_case, late_case])

    with tempfile.TemporaryDirectory(prefix="gc-rollback-self-test-") as temp:
        script = Path(temp) / "fake-love"
        script.write_text(
            "#!/usr/bin/env python3\n"
            f"print({case!r})\n"
            f"print({result!r})\n",
            encoding="utf-8",
        )
        script.chmod(0o755)
        record = run_native_once(
            script,
            "native",
            (),
            Path(temp) / "fake.log",
            5,
            enforce_plan=False,
        )
        if record["case_count"] != 1 or not record["teardown"]["orphan_free"]:
            raise RuntimeError("fake native launcher self-test failed")
        held_case = case + "|sample=final|forced_gc=1"
        held_script = Path(temp) / "fake-love-held"
        held_script.write_text(
            "#!/usr/bin/env python3\n"
            "import sys\n"
            f"print({held_case!r}, flush=True)\n"
            "if sys.stdin.readline().strip() != 'GC_ROLLBACK_SAMPLE_ACK':\n"
            "    raise SystemExit(2)\n"
            f"print({result!r}, flush=True)\n",
            encoding="utf-8",
        )
        held_script.chmod(0o755)
        held_record = run_native_once(
            held_script,
            "native",
            (),
            Path(temp) / "fake-held.log",
            5,
            enforce_plan=False,
        )
        held_checkpoint = next(
            row
            for row in held_record["resources"]["checkpoints"]
            if row.get("validation_marker") == held_case
        )
        if held_checkpoint.get("rss_bytes", 0) <= 0:
            raise RuntimeError("terminal sample acknowledgement race self-test failed")
        detached_script = Path(temp) / "fake-love-detached"
        detached_script.write_text(
            "#!/usr/bin/env python3\n"
            "import subprocess\n"
            "import sys\n"
            "subprocess.Popen(\n"
            "    [sys.executable, '-c', 'import time; time.sleep(60)', sys.argv[1], sys.argv[2]],\n"
            "    start_new_session=True,\n"
            "    stdout=subprocess.DEVNULL,\n"
            "    stderr=subprocess.DEVNULL,\n"
            ")\n",
            encoding="utf-8",
        )
        detached_script.chmod(0o755)
        try:
            run_native_once(
                detached_script,
                "native",
                (),
                Path(temp) / "fake-detached.log",
                5,
                enforce_plan=False,
            )
        except RuntimeError as error:
            if "left processes behind" not in str(error):
                raise
        else:
            raise RuntimeError("detached native helper passed teardown self-test")
        if validation_process_census():
            raise RuntimeError("detached native helper survived teardown self-test")
    print("rollback validation orchestration self-test: OK")


def default_output() -> Path:
    directory = Path(tempfile.mkdtemp(prefix="galactic-cup-omp2-rollback-"))
    return directory / "omp2_rollback.json"


def raise_on_interruption(sent_signal: int, _frame: Any) -> None:
    """Turn terminal signals into exceptions so bounded teardown always runs."""

    try:
        signal_name = signal.Signals(sent_signal).name
    except ValueError:
        signal_name = str(sent_signal)
    raise InterruptedError(f"rollback validation interrupted by {signal_name}")


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--mode",
        choices=("native", "browser", "full"),
        default="native",
    )
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--campaign", choices=CAMPAIGNS, default="all")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--love-bin", default=os.environ.get("LOVE_BIN", "love"))
    parser.add_argument(
        "--browser",
        action="append",
        choices=("chrome", "firefox"),
        dest="browsers",
    )
    parser.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_arguments()
    if args.self_test:
        run_self_test()
        return 0
    validate_historical_soccer_evidence()
    if args.timeout_seconds <= 0:
        raise SystemExit("--timeout-seconds must be positive")
    if args.mode in {"browser", "full"} and args.artifact is None:
        raise SystemExit("--artifact is required for browser and full modes")

    output = (args.output or default_output()).resolve()
    raw_root = output.parent / (output.stem + "-raw")
    source = source_provenance()
    evidence: dict[str, Any] = {
        "generated_at": utc_now(),
        "campaign": args.campaign,
        "gate_contract": int(GATE_CONTRACT),
        "mode": args.mode,
        "pass": False,
        "schema": 1,
        "source": source,
        "system": system_provenance(),
    }
    signal.signal(signal.SIGINT, raise_on_interruption)
    signal.signal(signal.SIGTERM, raise_on_interruption)
    try:
        if source["dirty"] and not args.allow_dirty:
            raise RuntimeError("rollback validation refuses a dirty source checkout")
        if raw_root.exists() and any(raw_root.iterdir()):
            raise RuntimeError(f"raw evidence directory is not empty: {raw_root}")
        raw_root.mkdir(parents=True, exist_ok=True)
        if args.mode in {"native", "full"}:
            love_bin = command_executable(args.love_bin)
            native_matrix(
                evidence,
                love_bin,
                raw_root,
                args.timeout_seconds,
                args.campaign,
            )
        if args.mode in {"browser", "full"}:
            browser_matrix(
                evidence,
                args.artifact.resolve(),
                args.browsers or ["chrome", "firefox"],
                raw_root,
                args.timeout_seconds,
                args.allow_dirty,
                args.campaign,
            )
        evidence["pass"] = True
        evidence["completed_at"] = utc_now()
        write_json(output, evidence)
        print(f"rollback validation: PASS ({output})")
        return 0
    except Exception as error:
        evidence["completed_at"] = utc_now()
        evidence["error"] = str(error)
        write_json(output, evidence)
        print(f"rollback validation: FAIL ({output})", file=sys.stderr)
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
