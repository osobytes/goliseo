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
import statistics
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
from rollback_ci import attribution_from_environment
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
CAMPAIGNS = ("all", "matrix", "soak", "stress")
BROWSER_ONLY_CAMPAIGNS = frozenset({"stress"})
# Only the seed-partitioned runtime matrix shards. The short stress campaign is
# already its own job, and the persistent soak is one continuous process.
SHARDED_CAMPAIGNS = frozenset({"matrix"})
BROWSER_RUNTIMES = ("chrome", "firefox")
SEED_SHARDS = tuple(str(seed) for seed in NETWORK_SEEDS)
TAIL_SHARD = "tail"
NATIVE_SHARDS = (*SEED_SHARDS, TAIL_SHARD)
BROWSER_MATRIX_SHARDS = SEED_SHARDS
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
# Both ends of the growth ratio average this many checkpoints instead of reading one
# sample. The checkpoint series has a natural spread of ~13%, so a single-sample
# denominator made the verdict depend on where `warmup` happened to land rather than on
# retained memory: two runs of identical code differing by 0.04% in mean heap scored
# 0.000% and 11.572%. Averaging both ends removes that. It costs sensitivity — a
# sustained leak is now detected from ~13.5% rather than ~10% — which is the price of
# not resolving a 10% threshold against a 13% noise band. More checkpoints would buy the
# sensitivity back; the checkpoint itself is sub-second.
SOAK_GROWTH_WINDOW = 2
MAX_SNAPSHOT_COUNT = 31
MAX_SNAPSHOT_BYTES = 768 * 1024
MAX_HISTORY_BYTES = 1024 * 1024
MAX_P95_WORK_MS = 16.67
MAX_ROLLBACK_P999_MS = 33.3
MAX_ROLLBACK_P999_US = 33300
ROLLBACK_PERCENTILE = 0.999
# Nearest-rank p99.9 returns ordered[ceil(n * 0.999) - 1]. That index is the last element --
# the plain maximum -- whenever ceil(n * ROLLBACK_PERCENTILE) == n. For integer n,
# ceil(n * p) < n holds exactly when n * p <= n - 1, i.e. n * (1 - p) >= 1, i.e.
# n >= 1 / (1 - ROLLBACK_PERCENTILE) = 1000. So p99.9 is the maximum for every n <= 999 and
# becomes a genuine tail percentile at n == 1000, the smallest sample count that keeps at
# least one sample strictly above the reported rank. Below this floor the statistic is a
# maximum wearing a percentile's name, and comparing it against a ratio threshold calibrated
# on ~6900-sample distributions rejects healthy runs whenever one of a handful of samples is
# slow. Cases below the floor are recorded diagnostically instead of gated.
MIN_ROLLBACK_P999_SAMPLE_COUNT = 1000
ROLLBACK_P999_GATE_APPLIED = "gated"
ROLLBACK_P999_GATE_DIAGNOSTIC = "diagnostic_sample_count_below_p999_floor"
ROLLBACK_P999_GATE_ERROR = "error_sample_count_unavailable"
ROLLBACK_P999_GATE_UNCALIBRATED = "error_scenario_thresholds_uncalibrated"
GATE_CONTRACT = "7"
MAX_BROWSER_P95_WORK_RATIO = 6.7
# The browser rollback tail is normalized against the playable case's OWN p95 work rather than
# against the paired clean control's p95 work. Both profiles run in the same job on the same
# runner, but they are two separate browser sessions, so the clean control draws its own sample
# of how loaded that machine happened to be. The numerator here is a tail statistic with roughly
# seven samples above the p99.9 rank; the clean control's p95 is a smooth statistic over
# thousands. Dividing one by the other does not cancel runner noise, it amplifies it, and the
# chrome seed 2001 shard of the first sharded campaign shows the failure directly: playable p95 /
# clean p95 read 2.984 where every other recorded pair reads 5.01-6.80, because the clean session
# hit a slow patch its playable partner did not. The playable case's own p95 work shares the
# time window and the workload with the tail it normalizes, so it tracks the machine the tail was
# actually measured on. Over the 42 recorded unsharded complete_fixture pairs the two statistics
# are equally tight (cv 0.085 against 0.086); over the six sharded pairs the composite spreads to
# cv 0.198 while the normalized ratio holds at 0.118.
MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95 = 2.6
# Absolute backstop, applied alongside the normalized ratio, and scaled by the runner this shard
# landed on -- see MAX_BROWSER_RUNNER_SCALE below. 39.7 ms is the ceiling for a shard running at
# the speed of the campaign's other shards; a shard measurably slower than its own peers is
# compared against a proportionally larger number.
#
# This is a CI noise ceiling, not a frame-budget claim. It is the largest tail in the 42-pair
# calibration population carried through BROWSER_CPU_CALIBRATION_MARGIN (34.520 * 1.15), and
# nothing more. Do not read it as the browser analogue of MAX_ROLLBACK_P999_MS = 33.3: that number
# is a deliberate two-frame claim at 60 fps, whereas 39.7 is 2.38 frames and asserts nothing about
# frame pacing. The browser matrix does not make frame-budget claims.
#
# Its job is bounding, not detection. Normalizing the tail by a measurement from the same session
# is what makes the gate stable, and it is also what makes the normalized ratio EXACTLY invariant
# under a proportional regression: (k * p999) / (k * p95) == p999 / p95 for every k, however
# large. That is a null derivative, not reduced sensitivity, and it is a blind spot contract 7
# introduces -- contract 6 divided by the clean control, an independent reference, so a
# proportional regression did inflate it. The proportional class is therefore bounded by
# MAX_BROWSER_P95_WORK_RATIO and this ceiling alone, and both are loose: across the 48 recorded
# complete_fixture pairs a uniform slowdown of the playable build must reach a median factor of
# 1.256 before either fires, and 1.573 on the pair whose clean control misreported. The self-test
# pins the invariance as a known bounded property rather than leaving it unexamined.
MAX_BROWSER_ROLLBACK_P999_MS = 39.7
# The ceiling is corrected for runner speed by the shard's own p95 work measured against the
# median of the campaign's OTHER shards for the same browser and scenario (#230).
#
# Why a correction was needed. The ceiling was the one gate in contract 7 that was not
# runner-relative, so it converted runner speed into a verdict. Run 30238674582 recorded
# complete_fixture chrome seed 2003 at a 48.400 ms tail on a 21.840 ms p95 work, against a healthy
# peer mean of 15.81 ms and a healthy peer maximum of 17.09 ms: the whole job ran about 38% slow.
# The normalized ratio, which is runner-relative, read 2.216 against its 2.6 threshold and passed.
# Two gates disagreed about the same measurements, and the absolute one turned main red.
#
# Why the peer shards are the reference. The six matrix shards run concurrently on comparable
# hosted runners, so the ratio of one shard's p95 work to its peers' is a measure of that runner
# and of nothing else. Critically it is invariant to the build: a regression that slows the
# playable case moves every shard together, leaving the ratio at 1 and the ceiling at 39.7 ms, so
# the correction cannot be bought by regressing the code. Only a shard that is slow relative to
# its own campaign -- the runner-noise signature -- earns slack.
#
# Why not the paired clean control (option 1 of #230). Scaling by the clean control's p95 work is
# algebraically the contract-6 composite ratio that #188 and #190 removed, and it is measurably
# worse. Over the 24 gated complete_fixture pairs of the four replayed campaigns, p99.9 over clean
# p95 spans 7.929-12.692 with the HEALTHY maximum (firefox seed 2002 of run 30231753972, 12.692)
# ABOVE the false red this constant exists to fix (11.748): the statistic cannot even separate the
# two. Calibrated the way every other threshold here is, it would sit at 14.6, which on a typical
# 3.2 ms clean control is a 46.7 ms ceiling -- looser than the 39.7 ms it replaces. Modelled
# against a uniform proportional slowdown of the whole campaign, the median detection factor is
# 1.267 for the fixed ceiling, 1.301 for this peer-relative one, and 1.501 for the clean-scaled
# one. The peer reference costs 2.7% of proportional sensitivity; the clean control costs 18%.
#
# Why not a binary runner-health precondition (option 2 of #230). Downgrading the ceiling to
# diagnostic when the shard is an outlier needs an outlier threshold, and the recorded data does
# not leave room for one: the largest healthy peer-relative slowdown is 1.161 and the false red is
# 1.344, so the project's 15% margin over the healthy maximum lands at 1.335 -- 0.7% below the
# value it must exempt. That is a threshold inside its own noise band, the #191 defect, on a new
# constant. A continuous correction has no such knife edge: it degrades smoothly and clears the
# false red by 9.3%.
#
# The correction never tightens the ceiling. A shard faster than its peers is still measured
# against 39.7 ms, so nothing that passed the fixed ceiling can fail the scaled one -- replaying
# the four recorded campaigns shows exactly one verdict change, the false red. Scaling downwards
# would apply the constant to a population it was not fitted on: firefox seed 2002 of run
# 30231753972 read 13.580 ms of p95 work against an 18.120 ms peer median and a 33.760 ms tail,
# and a two-sided correction would have failed that healthy pair at 29.75 ms.
BROWSER_CEILING_RUNNER_REFERENCE = "peer_median_playable_p95_work_ms"
# At least two peers, so the reference is a median of a real set rather than a single shard's own
# noise. Three seed shards per browser give every pair exactly two.
MIN_BROWSER_CEILING_PEER_COUNT = 2
# The largest correction the ceiling will grant, so an absurd peer ratio cannot buy unbounded
# exemption. Derived like every other browser threshold here: the worst runner slowdown ever
# recorded, chrome seed 2003 of run 30238674582 at 21.840 / 16.2550, carried through
# BROWSER_CPU_CALIBRATION_MARGIN by the same math.ceil(x * 1.15 * 10) / 10 the tail thresholds
# use. The self-test pins that derivation. Past this the correction stops growing; it is a clamp,
# not an error, because a pair under the capped ceiling is still under a ceiling.
BROWSER_RUNNER_SCALE_CALIBRATION_RUN = "30238674582"
BROWSER_RUNNER_SCALE_CALIBRATION_MAX = 21.840 / 16.2550
MAX_BROWSER_RUNNER_SCALE = 1.6
# The peer set exists only where more than one seed is in scope, which is the aggregate. A
# single-seed shard job records the ceiling as deferred and the rollback gate applies it when it
# merges the three shards; aggregate_browser_evidence refuses evidence in which a gated pair never
# had its ceiling applied, so the deferral is always redeemed. This is what stops a slow runner
# failing its own shard job before the campaign that could exonerate it has finished.
ROLLBACK_CEILING_GATE_DEFERRED = "deferred_to_aggregate_peer_set"
ROLLBACK_CEILING_GATE_ERROR = "error_runner_reference_unavailable"
# 2.6 and 39.7 ms are calibrated on the complete_fixture distribution and on nothing else. combat
# sits permanently below MIN_ROLLBACK_P999_SAMPLE_COUNT so it is never gated today, but the
# recorded combat pairs reach a normalized ratio of 3.437 at six samples: reusing these numbers
# there would re-create exactly the false failure #178 removed. Namespacing the thresholds by
# scenario makes that a fail-closed error rather than a comment somebody can overlook. A scenario
# that clears the sample floor without an entry here fails the pair, so #179 has to calibrate
# before it can gate.
BROWSER_ROLLBACK_TAIL_CALIBRATED_SCENARIOS = frozenset({"complete_fixture"})
BROWSER_CPU_CALIBRATION_RUNS = ("30060058593", "30065880550")
BROWSER_CPU_DIAGNOSTIC_RUN = "30075505461"
BROWSER_CPU_CALIBRATION_MARGIN = 0.15
BROWSER_CPU_CALIBRATION_MAX_P95_WORK_RATIO = 13.975 / 2.410
# Contract 7 recalibrated the rollback tail gate on the 42 complete_fixture pairs recorded by the
# seven consecutive runs below -- the largest accepted population available -- because the
# statistic itself changed and the contract-5 calibration pair predates it. The maxima are the
# worst single pair in that population: chrome seed 2002 of run 30170553072 for the normalized
# ratio, and firefox seed 2001 of run 30179056065 for the absolute tail. Both are carried through
# BROWSER_CPU_CALIBRATION_MARGIN exactly as the contract-5 thresholds were.
BROWSER_TAIL_CALIBRATION_RUNS = (
    "30163492726",
    "30168105822",
    "30170553072",
    "30175632519",
    "30176706801",
    "30179056065",
    "30181277777",
)
BROWSER_TAIL_CALIBRATION_MAX_ROLLBACK_P999_OVER_PLAYABLE_P95 = 24.295 / 10.825
BROWSER_TAIL_CALIBRATION_MAX_ROLLBACK_P999_MS = 34.520
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
        seeds = NETWORK_SEEDS
        if arguments:
            if len(arguments) != 1 or arguments[0] not in SEED_SHARDS:
                raise RuntimeError(
                    "native validation accepts at most one pinned network seed"
                )
            seeds = (int(arguments[0]),)
        for profile in NATIVE_PROFILES:
            for seed in seeds:
                plan.append(full_case(profile, seed))
                plan.append(combat_case(profile, seed))
        for seed in seeds:
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
        expected_snapshot_version = "12" if combat_case else "11"
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


def rollback_p999_gate_decision(sample_count: Any) -> tuple[bool, str]:
    """Decide whether a playable case's rollback p99.9 ratio is eligible for its gate.

    The ratio threshold is only comparable to a calibrated bound when the case recorded
    enough rollbacks for nearest-rank p99.9 to be a tail percentile rather than the
    maximum sample; see MIN_ROLLBACK_P999_SAMPLE_COUNT. At exactly that floor a single
    sample sits above the reported rank, so the floor is the boundary of the formula
    rather than a threshold of strong tail confidence.

    The browser combat scenario records six to eight rollbacks by construction, so it sits
    below the floor permanently and its browser p99.9 ratio is diagnostic indefinitely.
    Native combat playable cases keep the absolute < 33.3 ms p99.9 gate; see
    docs/online/omp2_rollback_validation.md and issue #179.

    ROLLBACK_P999_GATE_ERROR is defence in depth, not the production fail-closed path:
    browser_cpu_case already requires rollback_sample_count to parse through
    non_negative_integer and drops the whole case, with its own rejection reason, when it
    does not. This branch exists so a refactor that loosens that upstream check cannot
    turn a missing count into a silent exemption.
    """

    if (
        not isinstance(sample_count, int)
        or isinstance(sample_count, bool)
        or sample_count < 0
    ):
        return False, ROLLBACK_P999_GATE_ERROR
    if sample_count < MIN_ROLLBACK_P999_SAMPLE_COUNT:
        return False, ROLLBACK_P999_GATE_DIAGNOSTIC
    return True, ROLLBACK_P999_GATE_APPLIED


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
        "snapshot_versions": "11,12",
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
    ordered = list(values)
    if not ordered:
        raise RuntimeError(f"{label} growth gate received no checkpoints")
    # Collapse to single-sample ends when there are too few checkpoints to average
    # without the two windows overlapping.
    window = SOAK_GROWTH_WINDOW if len(ordered) >= 2 * SOAK_GROWTH_WINDOW + 1 else 1
    baseline_samples = ordered[:window]
    terminal_samples = ordered[-window:]
    baseline = sum(values[sample] for sample in baseline_samples) / window
    terminal = sum(values[sample] for sample in terminal_samples) / window
    if baseline <= 0:
        raise RuntimeError(
            f"{label} growth gate baseline is not positive: {baseline!r}"
        )
    growth_ratio = max(0.0, (terminal - baseline) / baseline)
    peak_sample = max(values, key=lambda sample: values[sample])
    peak = values[peak_sample]
    peak_growth_ratio = max(0.0, (peak - baseline) / baseline)
    passed = growth_ratio <= MAX_MEMORY_GROWTH_RATIO + 1e-12
    return {
        "baseline_bytes": round(baseline),
        "baseline_samples": baseline_samples,
        "growth_percent": round(growth_ratio * 100, 6),
        "label": label,
        "limit_percent": MAX_MEMORY_GROWTH_RATIO * 100,
        "measurement": "terminal_window_vs_baseline_window",
        "pass": passed,
        "peak_bytes": peak,
        "peak_growth_percent": round(peak_growth_ratio * 100, 6),
        "peak_sample": peak_sample,
        "samples": values,
        "terminal_bytes": round(terminal),
        "terminal_samples": terminal_samples,
        "window": window,
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


def require_nonempty_plan(
    plan: list[tuple[str, tuple[str, ...]]],
    runtime: str,
    campaign: str,
    shard: str | None,
) -> list[tuple[str, tuple[str, ...]]]:
    """Refuse to run a campaign that would validate nothing and report success.

    An empty plan is the one shape that could produce a passing evidence
    artifact without doing any work, which is exactly what the rollback gate
    exists to prevent. Every reachable campaign/shard combination owns cases, so
    an empty plan is a contract bug rather than a valid selection.
    """

    if not plan:
        raise ValueError(
            f"the {runtime} {campaign} campaign selected no cases for shard "
            f"{shard!r}; refusing to emit empty rollback evidence"
        )
    return plan


def native_shard_plan(
    network_seed: str | None = None,
) -> list[tuple[str, tuple[str, ...]]]:
    """Split independent native matrix cases into fresh bounded processes."""

    if network_seed is not None and network_seed not in SEED_SHARDS:
        raise ValueError(f"unknown native network seed shard {network_seed!r}")
    seeds = NETWORK_SEEDS if network_seed is None else (int(network_seed),)
    plan: list[tuple[str, tuple[str, ...]]] = []
    for profile in NATIVE_PROFILES:
        for seed in seeds:
            plan.append(("browser-full", (profile, str(seed))))
    for seed in seeds:
        plan.append(("browser-stress", (STRESS_PROFILE, str(seed))))
    return plan


def native_aggregate_record(
    run_number: int,
    shards: list[dict[str, Any]],
    network_seed: str | None = None,
) -> dict[str, Any]:
    markers = [
        parse_marker(raw)
        for shard in shards
        for raw in shard["markers"]
    ]
    arguments = () if network_seed is None else (network_seed,)
    validate_case_plan(markers, "native", arguments)
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
        "network_seed": network_seed,
        "run": run_number,
        "shard_count": len(shards),
        "shards": shards,
        "suite": "native-sharded",
    }


def native_campaign_plan(
    campaign: str = "all",
    shard: str | None = None,
) -> list[tuple[str, tuple[str, ...]]]:
    """Select the pinned native plan, optionally restricted to one CI shard.

    ``shard`` is a network seed for the seed-partitioned matrix cases, or
    ``tail`` for the seed-independent late-window pair and persistent soak.
    """

    if campaign not in CAMPAIGNS:
        raise ValueError(f"unknown rollback campaign {campaign!r}")
    if campaign in BROWSER_ONLY_CAMPAIGNS:
        raise ValueError(
            f"the {campaign} campaign is browser-only; native runs own the full matrix"
        )
    if shard is not None and shard not in NATIVE_SHARDS:
        raise ValueError(f"unknown native rollback shard {shard!r}")
    if shard in SEED_SHARDS and campaign not in {"all", "matrix"}:
        raise ValueError(
            f"the native {campaign} campaign has no work for network seed shard "
            f"{shard}; the seed-independent late-window pair and persistent soak "
            f"live in the {TAIL_SHARD!r} shard"
        )
    plan: list[tuple[str, tuple[str, ...]]] = []
    if shard == TAIL_SHARD:
        if campaign in {"all", "matrix"}:
            plan.append(("late-window", ()))
        if campaign in {"all", "soak"}:
            plan.append(("soak", ()))
        return require_nonempty_plan(plan, "native", campaign, shard)
    if campaign in {"all", "matrix"}:
        plan.extend(native_shard_plan(shard))
        if shard is None:
            plan.append(("late-window", ()))
    if campaign in {"all", "soak"} and shard is None:
        plan.append(("soak", ()))
    return require_nonempty_plan(plan, "native", campaign, shard)


def native_matrix(
    evidence: dict[str, Any],
    love_bin: Path,
    raw_root: Path,
    timeout_seconds: int,
    campaign: str,
    shard: str | None = None,
) -> None:
    plan = native_campaign_plan(campaign, shard)
    matrix_plan = [entry for entry in plan if entry[0].startswith("browser-")]
    network_seed = shard if shard in SEED_SHARDS else None
    native: dict[str, Any] = {
        "matrix_process_model": "fresh_process_per_full_case_and_stress_seed",
        "plan": [
            {"arguments": list(arguments), "suite": suite}
            for suite, arguments in plan
        ],
        "campaign": campaign,
        "network_seed": network_seed,
        "persistent_soak": ("soak", ()) in plan,
        "runtime": executable_metadata(love_bin),
        "shard": shard,
        "fresh_runs": [],
    }
    evidence["native"] = native
    if matrix_plan:
        for run_number in (1, 2):
            shards: list[dict[str, Any]] = []
            for shard_number, (suite, arguments) in enumerate(matrix_plan, start=1):
                slug = "-".join((suite, *arguments))
                process = run_native_once(
                    love_bin,
                    suite,
                    arguments,
                    raw_root / f"native-{run_number}-{shard_number:02d}-{slug}.log",
                    timeout_seconds,
                )
                process["shard"] = shard_number
                shards.append(process)
            native["fresh_runs"].append(
                native_aggregate_record(run_number, shards, network_seed)
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
    if ("late-window", ()) in plan:
        native["late_window"] = run_native_once(
            love_bin,
            "late-window",
            (),
            raw_root / "native-late-window.log",
            timeout_seconds,
        )
    if ("soak", ()) in plan:
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


def browser_plan(
    campaign: str = "all",
    shard: str | None = None,
) -> list[tuple[str, tuple[str, ...]]]:
    """Select the pinned browser plan, optionally restricted to one network seed.

    Each seed keeps its clean control immediately before its playable case, so a
    seed shard still measures both halves of a CPU pair in one job.
    """

    if campaign not in CAMPAIGNS:
        raise ValueError(f"unknown rollback campaign {campaign!r}")
    if shard is not None and shard not in BROWSER_MATRIX_SHARDS:
        raise ValueError(f"unknown browser rollback shard {shard!r}")
    if shard is not None and campaign not in SHARDED_CAMPAIGNS:
        raise ValueError(
            "only the browser runtime matrix is sharded by network seed; the short "
            "stress campaign is already its own job and the persistent soak is one "
            "continuous memory-retention process"
        )
    seeds = NETWORK_SEEDS if shard is None else (int(shard),)
    plan = []
    if campaign in {"all", "matrix"}:
        for network_seed in seeds:
            for profile in BROWSER_FULL_PROFILES:
                plan.append(("browser-full", (profile, str(network_seed))))
    if campaign in {"all", "matrix", "stress"}:
        for network_seed in seeds:
            plan.append(("browser-stress", (STRESS_PROFILE, str(network_seed))))
    if campaign in {"all", "soak"}:
        plan.append(("soak", ()))
    return require_nonempty_plan(plan, "browser", campaign, shard)


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
            "snapshot_versions": "11,12",
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
                    "rollback_sample_count": counts["rollback_sample_count"],
                    "scenario": planned["scenario"],
                },
            )
        )
    if reasons:
        return [], reasons
    if len(extracted) != len(BROWSER_CPU_SCENARIOS):
        return [], [f"{label} did not extract both browser-full CPU cases"]
    return extracted, []


def browser_ceiling_gate(
    playable_p95: float,
    peers: list[float],
    expected_peer_count: int,
) -> dict[str, Any]:
    """Scale the absolute rollback ceiling by this shard's own runner speed.

    ``peers`` are the playable p95 work values of the campaign's other shards for the same
    browser and scenario, measured concurrently on comparable hardware. Their median is the
    reference: a shard slower than it is compared against a proportionally larger ceiling, a
    shard faster than it is compared against the unmodified one. The correction is invariant
    to the build, because a regression moves every shard together.

    Fails closed in both directions it can. Fewer than ``MIN_BROWSER_CEILING_PEER_COUNT``
    peers in scope is a deferral, not an exemption -- the aggregate has the peer set and
    ``aggregate_browser_evidence`` refuses evidence in which a gated pair never had its
    ceiling applied. A peer set that is incomplete or carries a non-finite or non-positive
    measurement where one was expected is an error that fails the pair, so a vanished or
    collapsed reference can never be read as "no ceiling".
    """

    block: dict[str, Any] = {
        "applied": False,
        "base_ms": MAX_BROWSER_ROLLBACK_P999_MS,
        "effective_ms": None,
        "max_runner_scale": MAX_BROWSER_RUNNER_SCALE,
        "peer_count": len(peers),
        "peer_reference_p95_work_ms": None,
        "reference": BROWSER_CEILING_RUNNER_REFERENCE,
        "runner_scale": None,
        "runner_scale_clamped": False,
        "status": ROLLBACK_CEILING_GATE_DEFERRED,
    }
    if expected_peer_count < MIN_BROWSER_CEILING_PEER_COUNT:
        return block
    if len(peers) != expected_peer_count or not all(
        math.isfinite(peer) and peer > 0 for peer in peers
    ):
        block["status"] = ROLLBACK_CEILING_GATE_ERROR
        return block
    reference = statistics.median(peers)
    if not math.isfinite(reference) or reference <= 0:
        block["status"] = ROLLBACK_CEILING_GATE_ERROR
        return block
    measured_scale = playable_p95 / reference
    runner_scale = min(max(measured_scale, 1.0), MAX_BROWSER_RUNNER_SCALE)
    block.update(
        {
            "applied": True,
            "effective_ms": round(MAX_BROWSER_ROLLBACK_P999_MS * runner_scale, 9),
            "peer_reference_p95_work_ms": round(reference, 9),
            "runner_scale": round(runner_scale, 9),
            "runner_scale_clamped": measured_scale > MAX_BROWSER_RUNNER_SCALE,
            "status": ROLLBACK_P999_GATE_APPLIED,
        }
    )
    return block


def unredeemed_ceiling_deferrals(acceptance: dict[str, Any]) -> list[str]:
    """Name every gated pair whose absolute ceiling was never applied.

    The shard jobs defer the ceiling because a single shard has no peers to measure its runner
    against (#230). This is where that debt is collected: a pair the tail gate enforced must
    have had its ceiling enforced too, or the campaign produced evidence in which nothing ever
    applied it.
    """

    return [
        f"{pair['scenario']} seed {pair['seed']}"
        for pair in acceptance["pairs"]
        if pair["rollback_p999_gate"]["applied"]
        and not pair["rollback_p999_gate"]["absolute_ceiling"]["applied"]
    ]


def browser_cpu_acceptance(
    runs: list[dict[str, Any]],
    browser_name: str,
    seeds: tuple[int, ...] = NETWORK_SEEDS,
) -> dict[str, Any]:
    """Apply the strict same-shard, same-runtime, seed-paired browser CPU contract.

    ``seeds`` narrows the contract to one CI shard's own network seed. Every
    supplied seed must contribute a complete clean/playable pair for both
    scenario families, and no run may carry a seed outside the scope.
    """

    unknown = [seed for seed in seeds if seed not in NETWORK_SEEDS]
    if not seeds or unknown:
        raise ValueError(f"browser CPU acceptance received unpinned seeds {unknown!r}")
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

    # The runner-speed reference for the absolute ceiling. Keyed by scenario and seed so a pair
    # is only ever compared against the same fixture on the campaign's other shards.
    peer_p95_work: dict[tuple[str, str], float] = {
        (key[0], key[2]): row["p95_work_ms"]
        for key, row in rows.items()
        if key[1] == "playable"
    }

    pairs: list[dict[str, Any]] = []
    for scenario in BROWSER_CPU_SCENARIOS:
        for seed_value in seeds:
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
            playable_p95 = playable["p95_work_ms"]
            # The tail normalizer must be a real measurement before it can divide anything.
            # finite_non_negative_float upstream admits 0.0, so this is the only thing standing
            # between an absent or collapsed playable p95 and a ZeroDivisionError -- or worse, an
            # inflated denominator that quietly excuses a slow tail. Fail the pair instead.
            if not math.isfinite(playable_p95) or playable_p95 <= 0:
                reasons.append(
                    f"{browser_name} {scenario} seed {seed} playable p95 work normalizer "
                    "must be finite and >0"
                )
                continue
            playable_rollback_p999 = playable["rollback_p999_ms"]
            p95_ratio = playable_p95 / clean_p95
            rollback_ratio = playable_rollback_p999 / clean_p95
            normalized_rollback_ratio = playable_rollback_p999 / playable_p95
            pair_reasons = []
            if p95_ratio >= MAX_BROWSER_P95_WORK_RATIO:
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} "
                    f"p95_work_ratio={p95_ratio:.9f} "
                    f"does not meet <{MAX_BROWSER_P95_WORK_RATIO:.1f}"
                )
            rollback_sample_count = playable.get("rollback_sample_count")
            rollback_gate_applied, rollback_gate_status = rollback_p999_gate_decision(
                rollback_sample_count
            )
            # Redundant with the browser_cpu_case count validation that already rejects such
            # a case upstream; kept so the aggregate can never silently exempt a pair.
            if rollback_gate_status == ROLLBACK_P999_GATE_ERROR:
                rollback_sample_count = None
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} playable case reports no usable "
                    "rollback_sample_count, so the rollback p99.9 gate fails closed"
                )
            # The thresholds below are calibrated on complete_fixture alone. A scenario that
            # clears the sample floor without its own calibration must fail rather than borrow
            # them: the recorded combat pairs reach a normalized ratio of 3.437, so borrowing 2.6
            # would re-create the false failure #178 removed. This is the guard that stops the
            # numbers being reused silently when #179 lifts the floor for combat.
            if (
                rollback_gate_applied
                and scenario not in BROWSER_ROLLBACK_TAIL_CALIBRATED_SCENARIOS
            ):
                rollback_gate_applied = False
                rollback_gate_status = ROLLBACK_P999_GATE_UNCALIBRATED
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} cleared the rollback p99.9 sample "
                    "floor, but the tail thresholds are calibrated for "
                    f"{sorted(BROWSER_ROLLBACK_TAIL_CALIBRATED_SCENARIOS)} only, so the gate "
                    "fails closed until they are recalibrated for this scenario"
                )
            if (
                rollback_gate_applied
                and normalized_rollback_ratio
                >= MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95
            ):
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} "
                    f"rollback_p999_over_playable_p95={normalized_rollback_ratio:.9f} "
                    f"does not meet <{MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95:.1f}"
                )
            # The absolute ceiling rides the same sample-count floor as the normalized ratio. At
            # six to eight samples rollback_p999_ms is the worst single sample, and #177 recorded
            # one 43.545 ms combat sample on a build with a 7.1 ms native peak -- runner noise,
            # not a regression. Applying a millisecond ceiling there would re-create exactly the
            # false failure #178 removed.
            #
            # It is also the one gate here that is stated in milliseconds, so it is the one that
            # has to be told how fast the machine under it was. The peer shards say so; see
            # browser_ceiling_gate and MAX_BROWSER_RUNNER_SCALE.
            ceiling_gate = browser_ceiling_gate(
                playable_p95,
                [
                    peer_p95_work[(scenario, str(peer_seed))]
                    for peer_seed in seeds
                    if peer_seed != seed_value
                    and (scenario, str(peer_seed)) in peer_p95_work
                ],
                len(seeds) - 1,
            )
            if not rollback_gate_applied:
                # Whatever held back the normalized ratio holds back the ceiling too, and the
                # published status says which of the two reasons it was.
                ceiling_gate["applied"] = False
                ceiling_gate["status"] = rollback_gate_status
            elif ceiling_gate["status"] == ROLLBACK_CEILING_GATE_ERROR:
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} has no usable runner reference "
                    f"for the absolute rollback ceiling ({ceiling_gate['peer_count']} of "
                    f"{len(seeds) - 1} peer shards reported a finite positive p95 work), so "
                    "the ceiling fails closed"
                )
            elif ceiling_gate["applied"] and (
                playable_rollback_p999 >= ceiling_gate["effective_ms"]
            ):
                pair_reasons.append(
                    f"{browser_name} {scenario} seed {seed} "
                    f"playable_rollback_p999_ms={playable_rollback_p999:.6f} "
                    f"does not meet <{ceiling_gate['effective_ms']:.6f} "
                    f"({MAX_BROWSER_ROLLBACK_P999_MS:.1f} ms base ceiling at a "
                    f"{ceiling_gate['runner_scale']:.6f} runner scale over a "
                    f"{ceiling_gate['peer_reference_p95_work_ms']:.6f} ms peer median p95 work)"
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
                        "clean_rollback_sample_count": clean.get(
                            "rollback_sample_count"
                        ),
                        "playable_max_rollback_ms": playable["max_rollback_ms"],
                        "playable_p95_work_ms": playable["p95_work_ms"],
                        "playable_rollback_over_33_3_count": playable[
                            "rollback_over_33_3_count"
                        ],
                        "playable_rollback_p999_ms": playable[
                            "rollback_p999_ms"
                        ],
                        "playable_rollback_sample_count": rollback_sample_count,
                    },
                    "cases": {
                        "clean": clean["case"],
                        "playable": playable["case"],
                    },
                    "pass": not pair_reasons,
                    "ratios": {
                        "p95_work_over_clean_p95": round(p95_ratio, 9),
                        # Retained as a diagnostic only. Contract 6 gated on this; contract 7
                        # gates on rollback_p999_over_playable_p95 instead, and keeping the old
                        # figure published lets a reader compare the two across the archive.
                        "rollback_p999_over_clean_p95": round(rollback_ratio, 9),
                        "rollback_p999_over_playable_p95": round(
                            normalized_rollback_ratio,
                            9,
                        ),
                    },
                    "reasons": pair_reasons,
                    "rollback_p999_gate": {
                        "absolute_ceiling": ceiling_gate,
                        "applied": rollback_gate_applied,
                        "composite_ratio": round(rollback_ratio, 9),
                        "minimum_sample_count": MIN_ROLLBACK_P999_SAMPLE_COUNT,
                        "normalized_ratio": round(normalized_rollback_ratio, 9),
                        "normalizer": "playable_p95_work_ms",
                        "playable_rollback_p999_ms": playable_rollback_p999,
                        "sample_count": rollback_sample_count,
                        "status": rollback_gate_status,
                    },
                    "scenario": scenario,
                    "seed": seed_value,
                }
            )
    expected_keys = {
        (scenario, profile, str(seed))
        for scenario in BROWSER_CPU_SCENARIOS
        for profile in BROWSER_FULL_PROFILES
        for seed in seeds
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
            "max_accepted_rollback_p999_ms": (
                BROWSER_TAIL_CALIBRATION_MAX_ROLLBACK_P999_MS
            ),
            "max_accepted_rollback_p999_over_playable_p95": round(
                BROWSER_TAIL_CALIBRATION_MAX_ROLLBACK_P999_OVER_PLAYABLE_P95,
                9,
            ),
            "max_accepted_runner_scale": round(
                BROWSER_RUNNER_SCALE_CALIBRATION_MAX,
                9,
            ),
            "rollback_tail_accepted_run_ids": list(BROWSER_TAIL_CALIBRATION_RUNS),
            "runner_scale_accepted_run_id": BROWSER_RUNNER_SCALE_CALIBRATION_RUN,
        },
        "gate_contract": int(GATE_CONTRACT),
        "method": "same_shard_same_runtime_scenario_seed_paired",
        "pairs": pairs,
        "pass": not reasons
        and len(pairs) == len(BROWSER_CPU_SCENARIOS) * len(seeds),
        "reasons": reasons,
        "scope": "aggregate" if tuple(seeds) == NETWORK_SEEDS else "shard",
        "seeds": [int(seed) for seed in seeds],
        "thresholds": {
            "comparison": "strict_less_than",
            "max_p95_work_over_clean_p95": MAX_BROWSER_P95_WORK_RATIO,
            # Base value. The enforced ceiling is this scaled by the pair's own runner scale,
            # published per pair as rollback_p999_gate.absolute_ceiling.effective_ms.
            "max_rollback_p999_ms": MAX_BROWSER_ROLLBACK_P999_MS,
            "max_rollback_p999_over_playable_p95": (
                MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95
            ),
            "max_runner_scale": MAX_BROWSER_RUNNER_SCALE,
            "min_rollback_p999_peer_count": MIN_BROWSER_CEILING_PEER_COUNT,
            "min_rollback_p999_sample_count": MIN_ROLLBACK_P999_SAMPLE_COUNT,
            "runner_scale_reference": BROWSER_CEILING_RUNNER_REFERENCE,
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
    shard: str | None = None,
) -> None:
    plan = browser_plan(campaign, shard)
    seeds = NETWORK_SEEDS if shard is None else (int(shard),)
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
        "network_seed": shard,
        "plan": [
            {"arguments": list(arguments), "suite": suite}
            for suite, arguments in plan
        ],
        "runtimes": {},
        "selenium": selenium_metadata(),
        "shard": shard,
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
            for run_number, (suite, arguments) in enumerate(plan, start=1):
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
                    seeds,
                )
                runtime["cpu_acceptance"] = cpu_acceptance
                if not cpu_acceptance["pass"]:
                    reason = "; ".join(cpu_acceptance["reasons"])
                    raise RuntimeError(
                        f"{browser_name} {cpu_acceptance['scope']} browser CPU "
                        f"acceptance failed: {reason}"
                    )
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
    if thread.is_alive():
        raise RuntimeError("browser artifact server did not stop cleanly")


def expected_shard_evidence() -> dict[str, dict[str, Any]]:
    """Pin every artifact a complete campaign uploads before the gate may pass.

    This covers the short stress jobs as well as the sharded long jobs, because
    the gate downloads every ``omp2-rollback-*`` artifact in the run: an artifact
    the manifest does not account for is as much a contract break as a missing
    one. ``rollback_ci.EXPECTED_ARTIFACTS`` restates this set for reuse
    discovery, and the self-test asserts the two never drift.
    """

    expected: dict[str, dict[str, Any]] = {}
    for shard in NATIVE_SHARDS:
        expected[f"omp2-rollback-native-{shard}"] = {
            "browser": None,
            "campaign": "all",
            "mode": "native",
            "shard": shard,
        }
    for browser_name in BROWSER_RUNTIMES:
        for shard in BROWSER_MATRIX_SHARDS:
            expected[f"omp2-rollback-{browser_name}-matrix-{shard}"] = {
                "browser": browser_name,
                "campaign": "matrix",
                "mode": "browser",
                "shard": shard,
            }
        expected[f"omp2-rollback-{browser_name}-soak"] = {
            "browser": browser_name,
            "campaign": "soak",
            "mode": "browser",
            "shard": None,
        }
        expected[f"omp2-rollback-{browser_name}-stress"] = {
            "browser": browser_name,
            "campaign": "stress",
            "mode": "browser",
            "shard": None,
        }
    return expected


def require_complete_shards(found: Iterable[str]) -> list[str]:
    """Fail closed unless the pinned shard set is present exactly once each.

    A vanished, skipped, or cancelled shard uploads no evidence, so its pinned
    name is missing here even when GitHub rolls the matrix job up to success.
    """

    names = list(found)
    expected = frozenset(expected_shard_evidence())
    missing = sorted(expected.difference(names))
    unexpected = sorted(frozenset(names).difference(expected))
    duplicates = sorted({name for name in names if names.count(name) > 1})
    problems: list[str] = []
    if missing:
        problems.append(f"missing rollback shard evidence: {', '.join(missing)}")
    if unexpected:
        problems.append(f"unpinned rollback shard evidence: {', '.join(unexpected)}")
    if duplicates:
        problems.append(f"duplicate rollback shard evidence: {', '.join(duplicates)}")
    if problems:
        raise RuntimeError("; ".join(problems))
    return sorted(expected)


def load_shard_evidence(
    root: Path,
    name: str,
    identity: dict[str, Any],
    revision: str,
) -> dict[str, Any]:
    """Read one shard's uploaded evidence and prove it is the pinned shard."""

    path = root / name / f"{name}.json"
    if not path.is_file():
        raise RuntimeError(f"rollback shard {name} uploaded no evidence at {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise RuntimeError(f"rollback shard {name} evidence is unreadable: {error}") from error
    if not isinstance(payload, dict):
        raise RuntimeError(f"rollback shard {name} evidence root is not an object")
    if payload.get("schema") != 1:
        raise RuntimeError(f"rollback shard {name} reports an unsupported schema")
    if payload.get("gate_contract") != int(GATE_CONTRACT):
        raise RuntimeError(f"rollback shard {name} was produced under another gate contract")
    if payload.get("pass") is not True:
        raise RuntimeError(
            f"rollback shard {name} did not pass: {payload.get('error', 'no reason recorded')}"
        )
    for field in ("mode", "campaign"):
        if payload.get(field) != identity[field]:
            raise RuntimeError(
                f"rollback shard {name} reports {field}={payload.get(field)!r}, "
                f"expected {identity[field]!r}"
            )
    source = payload.get("source")
    if not isinstance(source, dict):
        raise RuntimeError(f"rollback shard {name} omits source provenance")
    if source.get("revision") != revision:
        raise RuntimeError(
            f"rollback shard {name} was produced at revision "
            f"{source.get('revision')!r}, expected {revision!r}"
        )
    if source.get("dirty") is not False:
        raise RuntimeError(f"rollback shard {name} was produced from a dirty checkout")
    section = payload.get(identity["mode"])
    if not isinstance(section, dict):
        raise RuntimeError(f"rollback shard {name} omits its {identity['mode']} evidence")
    if section.get("shard") != identity["shard"]:
        raise RuntimeError(
            f"rollback shard {name} reports shard={section.get('shard')!r}, "
            f"expected {identity['shard']!r}"
        )
    if identity["browser"] is not None:
        runtimes = section.get("runtimes")
        if not isinstance(runtimes, dict) or set(runtimes) != {identity["browser"]}:
            raise RuntimeError(
                f"rollback shard {name} did not record exactly the "
                f"{identity['browser']} runtime"
            )
    return payload


def aggregate_native_evidence(shards: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """Reassemble the pinned 54-case native plan from its per-seed shards."""

    covered: list[str] = []
    per_shard: dict[str, int] = {}
    for seed in SEED_SHARDS:
        native = shards[f"omp2-rollback-native-{seed}"]["native"]
        fresh_runs = native.get("fresh_runs")
        if not isinstance(fresh_runs, list) or len(fresh_runs) != 2:
            raise RuntimeError(f"native seed shard {seed} did not record two fresh runs")
        if native.get("fresh_runs_agree") is not True:
            raise RuntimeError(f"native seed shard {seed} fresh runs did not agree")
        for run in fresh_runs:
            raw_markers = run.get("markers")
            if not isinstance(raw_markers, list):
                raise RuntimeError(f"native seed shard {seed} omits validation markers")
            markers = [parse_marker(raw) for raw in raw_markers]
            validate_case_plan(markers, "native", (seed,))
            validate_case_integrity(markers, "native")
        first = [parse_marker(raw) for raw in fresh_runs[0]["markers"]]
        cases = [marker.fields["case"] for marker in first if marker.kind == "case"]
        per_shard[seed] = len(cases)
        covered.extend(cases)
    expected_cases = [case["case"] for case in expected_case_plan("native", ())]
    missing = sorted(set(expected_cases).difference(covered))
    unexpected = sorted(set(covered).difference(expected_cases))
    if missing or unexpected or len(covered) != len(expected_cases):
        raise RuntimeError(
            f"sharded native evidence covers {len(covered)} of {len(expected_cases)} "
            f"pinned cases: missing={missing}, unexpected={unexpected}"
        )
    tail = shards[f"omp2-rollback-native-{TAIL_SHARD}"]["native"]
    late_window = tail.get("late_window")
    if not isinstance(late_window, dict) or late_window.get("suite") != "late-window":
        raise RuntimeError("native tail shard omitted the late-window pair")
    soak = tail.get("soak")
    if not isinstance(soak, dict) or soak.get("soak_memory", {}).get("pass") is not True:
        raise RuntimeError("native tail shard omitted a passing persistent soak")
    return {
        "case_count": len(covered),
        "cases_per_shard": per_shard,
        "late_window_cases": late_window.get("case_count"),
        "shard_key": "network_seed",
        "soak_memory": soak["soak_memory"],
    }


def aggregate_browser_evidence(shards: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """Evaluate all six clean/playable pairs per browser across the seed shards."""

    runtimes: dict[str, Any] = {}
    for browser_name in BROWSER_RUNTIMES:
        runs: list[dict[str, Any]] = []
        for seed in BROWSER_MATRIX_SHARDS:
            name = f"omp2-rollback-{browser_name}-matrix-{seed}"
            runtime = shards[name]["browser"]["runtimes"][browser_name]
            shard_acceptance = runtime.get("cpu_acceptance")
            if not isinstance(shard_acceptance, dict):
                raise RuntimeError(f"{name} omitted its shard CPU acceptance")
            if shard_acceptance.get("seeds") != [int(seed)]:
                raise RuntimeError(f"{name} scoped its CPU acceptance to the wrong seed")
            if shard_acceptance.get("pass") is not True:
                raise RuntimeError(f"{name} shard CPU acceptance did not pass")
            shard_runs = runtime.get("runs")
            if not isinstance(shard_runs, list):
                raise RuntimeError(f"{name} omitted its browser runs")
            runs.extend(shard_runs)
        acceptance = browser_cpu_acceptance(runs, browser_name)
        if not acceptance["pass"]:
            reason = "; ".join(acceptance["reasons"])
            raise RuntimeError(
                f"{browser_name} aggregate browser CPU acceptance failed: {reason}"
            )
        deferred = unredeemed_ceiling_deferrals(acceptance)
        if deferred:
            raise RuntimeError(
                f"{browser_name} aggregate evidence gated the rollback tail without applying "
                f"the absolute ceiling to {deferred}"
            )
        soak_name = f"omp2-rollback-{browser_name}-soak"
        soak_runs = shards[soak_name]["browser"]["runtimes"][browser_name].get("runs")
        if not isinstance(soak_runs, list):
            raise RuntimeError(f"{soak_name} omitted its browser runs")
        soaks = [run for run in soak_runs if run.get("suite") == "soak"]
        if len(soaks) != 1:
            raise RuntimeError(f"{soak_name} did not record exactly one persistent soak")
        soak_memory = soaks[0].get("soak_memory")
        if not isinstance(soak_memory, dict) or soak_memory.get("pass") is not True:
            raise RuntimeError(f"{soak_name} did not pass the memory growth gate")
        stress_name = f"omp2-rollback-{browser_name}-stress"
        stress_runs = shards[stress_name]["browser"]["runtimes"][browser_name].get("runs")
        if not isinstance(stress_runs, list):
            raise RuntimeError(f"{stress_name} omitted its browser runs")
        stress_seeds = sorted(
            run.get("arguments", [None, None])[1]
            for run in stress_runs
            if run.get("suite") == "browser-stress"
        )
        if stress_seeds != sorted(SEED_SHARDS):
            raise RuntimeError(
                f"{stress_name} covered stress seeds {stress_seeds}, "
                f"expected {sorted(SEED_SHARDS)}"
            )
        runtimes[browser_name] = {
            "cpu_acceptance": acceptance,
            "soak_memory": soak_memory,
            "stress_seeds": stress_seeds,
        }
    return {"runtimes": runtimes, "shard_key": "network_seed"}


def aggregate_shards(evidence: dict[str, Any], evidence_root: Path) -> None:
    """Merge every uploaded shard and apply the campaign-wide acceptance."""

    root = evidence_root.resolve()
    if not root.is_dir():
        raise RuntimeError(f"rollback shard evidence root is missing: {root}")
    names = require_complete_shards(
        entry.name for entry in root.iterdir() if entry.is_dir()
    )
    revision = evidence["source"]["revision"]
    identities = expected_shard_evidence()
    shards = {
        name: load_shard_evidence(root, name, identities[name], revision)
        for name in names
    }
    evidence["aggregate"] = {
        "browser": aggregate_browser_evidence(shards),
        "evidence_root": str(root),
        "native": aggregate_native_evidence(shards),
        "shards": {
            name: {
                "generated_at": shards[name].get("generated_at"),
                "sha256": sha256_file(root / name / f"{name}.json"),
            }
            for name in names
        },
    }


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
    if browser_plan("stress") != expected_plan[6:-1]:
        raise RuntimeError("browser stress campaign plan self-test failed")
    if browser_plan("stress") != [
        step for step in browser_plan("matrix") if step[0] == "browser-stress"
    ]:
        raise RuntimeError("browser stress campaign is not a subset of the runtime matrix")
    try:
        native_campaign_plan("stress")
    except ValueError as error:
        if "browser-only" not in str(error):
            raise RuntimeError("native stress rejection reports the wrong reason") from error
    else:
        raise RuntimeError("browser-only stress campaign was accepted natively")
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

    # Pin the derivation behind MIN_ROLLBACK_P999_SAMPLE_COUNT: nearest-rank p99.9 is the
    # maximum sample for every n below the floor and a strictly smaller order statistic at the
    # floor itself. tuple(range(n)) is strictly increasing, so its maximum is n - 1.
    for collapsed_count in (8, 999, MIN_ROLLBACK_P999_SAMPLE_COUNT - 1):
        ascending = tuple(range(collapsed_count))
        if (
            nearest_rank_integer(ascending, ROLLBACK_PERCENTILE)
            != collapsed_count - 1
        ):
            raise RuntimeError(
                f"p99.9 nearest rank at n={collapsed_count} is not the maximum sample"
            )
    floor_samples = tuple(range(MIN_ROLLBACK_P999_SAMPLE_COUNT))
    if nearest_rank_integer(floor_samples, ROLLBACK_PERCENTILE) != (
        MIN_ROLLBACK_P999_SAMPLE_COUNT - 2
    ):
        raise RuntimeError(
            "p99.9 nearest rank at the minimum sample count is not strictly below the maximum"
        )
    if nearest_rank_integer(floor_samples, 1) != MIN_ROLLBACK_P999_SAMPLE_COUNT - 1:
        raise RuntimeError("nearest-rank maximum self-test failed")
    for sample_count, expected_decision in (
        (MIN_ROLLBACK_P999_SAMPLE_COUNT, (True, ROLLBACK_P999_GATE_APPLIED)),
        (MIN_ROLLBACK_P999_SAMPLE_COUNT + 1, (True, ROLLBACK_P999_GATE_APPLIED)),
        (6903, (True, ROLLBACK_P999_GATE_APPLIED)),
        (0, (False, ROLLBACK_P999_GATE_DIAGNOSTIC)),
        (8, (False, ROLLBACK_P999_GATE_DIAGNOSTIC)),
        (
            MIN_ROLLBACK_P999_SAMPLE_COUNT - 1,
            (False, ROLLBACK_P999_GATE_DIAGNOSTIC),
        ),
        (None, (False, ROLLBACK_P999_GATE_ERROR)),
        ("1000", (False, ROLLBACK_P999_GATE_ERROR)),
        (1000.0, (False, ROLLBACK_P999_GATE_ERROR)),
        (True, (False, ROLLBACK_P999_GATE_ERROR)),
        (-1, (False, ROLLBACK_P999_GATE_ERROR)),
    ):
        if rollback_p999_gate_decision(sample_count) != expected_decision:
            raise RuntimeError(
                f"rollback p99.9 gate decision for {sample_count!r} is not "
                f"{expected_decision!r}"
            )

    for shard in SEED_SHARDS:
        if native_campaign_plan("all", shard) != native_shard_plan(shard):
            raise RuntimeError(f"native seed shard {shard} campaign plan self-test failed")
        shard_cases = [
            case
            for suite, arguments in native_campaign_plan("all", shard)
            for case in expected_case_plan(suite, arguments)
        ]
        if shard_cases != expected_case_plan("native", (shard,)) or len(shard_cases) != 18:
            raise RuntimeError(f"native seed shard {shard} changed the pinned case order")
    if native_campaign_plan("all", TAIL_SHARD) != [("late-window", ()), ("soak", ())]:
        raise RuntimeError("native tail shard campaign plan self-test failed")
    if native_campaign_plan("matrix", TAIL_SHARD) != [("late-window", ())]:
        raise RuntimeError("native tail matrix campaign plan self-test failed")
    if native_campaign_plan("soak", TAIL_SHARD) != [("soak", ())]:
        raise RuntimeError("native tail soak campaign plan self-test failed")
    for rejected_campaign, rejected_shard in (
        ("soak", "2001"),
        ("soak", "2002"),
        ("soak", "2003"),
        ("all", "2004"),
        ("sprint", "2001"),
    ):
        try:
            native_campaign_plan(rejected_campaign, rejected_shard)
        except ValueError:
            pass
        else:
            raise RuntimeError(
                f"native {rejected_campaign} campaign emitted a plan for shard "
                f"{rejected_shard!r}"
            )
    for runtime_label in ("native", "browser"):
        try:
            require_nonempty_plan([], runtime_label, "soak", "2001")
        except ValueError as error:
            if "refusing to emit empty rollback evidence" not in str(error):
                raise RuntimeError(
                    f"{runtime_label} empty-plan guard reason self-test failed"
                ) from error
        else:
            raise RuntimeError(f"{runtime_label} empty rollback plan passed self-test")
    sharded_native_cases = sorted(
        case["case"]
        for shard in SEED_SHARDS
        for case in expected_case_plan("native", (shard,))
    )
    if sharded_native_cases != sorted(
        case["case"] for case in expected_case_plan("native", ())
    ):
        raise RuntimeError("native seed shards do not reconstruct the pinned 54-case plan")
    for shard in SEED_SHARDS:
        if browser_plan("matrix", shard) != [
            ("browser-full", ("clean", shard)),
            ("browser-full", ("playable", shard)),
            ("browser-stress", ("stress", shard)),
        ]:
            raise RuntimeError(f"browser seed shard {shard} lost its clean/playable pair")
    sharded_browser_plan = [
        entry for shard in SEED_SHARDS for entry in browser_plan("matrix", shard)
    ]
    if sorted(sharded_browser_plan) != sorted(browser_plan("matrix")):
        raise RuntimeError("browser seed shards do not reconstruct the runtime matrix")
    for campaign, rejected_shard in (
        ("soak", "2001"),
        ("stress", "2001"),
        ("all", "2001"),
        ("matrix", TAIL_SHARD),
    ):
        try:
            browser_plan(campaign, rejected_shard)
        except ValueError:
            pass
        else:
            raise RuntimeError(
                f"browser {campaign} campaign accepted shard {rejected_shard!r}"
            )

    def carry_calibration_margin(accepted_maximum: float) -> float:
        return (
            math.ceil(
                accepted_maximum * (1 + BROWSER_CPU_CALIBRATION_MARGIN) * 10
            )
            / 10
        )

    for accepted_maximum, threshold, label in (
        (
            BROWSER_CPU_CALIBRATION_MAX_P95_WORK_RATIO,
            MAX_BROWSER_P95_WORK_RATIO,
            "p95 work ratio",
        ),
        (
            BROWSER_TAIL_CALIBRATION_MAX_ROLLBACK_P999_OVER_PLAYABLE_P95,
            MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95,
            "rollback p99.9 over playable p95",
        ),
        (
            BROWSER_TAIL_CALIBRATION_MAX_ROLLBACK_P999_MS,
            MAX_BROWSER_ROLLBACK_P999_MS,
            "rollback p99.9 absolute ceiling",
        ),
        (
            BROWSER_RUNNER_SCALE_CALIBRATION_MAX,
            MAX_BROWSER_RUNNER_SCALE,
            "absolute ceiling runner scale cap",
        ),
    ):
        if carry_calibration_margin(accepted_maximum) != threshold:
            raise RuntimeError(
                f"browser CPU calibration margin self-test failed for {label}"
            )
        if accepted_maximum >= threshold:
            raise RuntimeError(
                f"browser CPU threshold for {label} does not clear its accepted maximum"
            )

    def synthetic_browser_cpu_run(
        profile: str,
        seed: int,
        p95_work_ms: float,
        rollback_p999_ms: float,
        *,
        combat_p95_work_ms: float | None = None,
        combat_rollback_p999_ms: float | None = None,
        rollback_samples: int = MIN_ROLLBACK_P999_SAMPLE_COUNT,
        combat_rollback_samples: int | None = None,
        browser_name: str = "firefox",
        browser_version: str = "153.0",
        marker_seed: int | None = None,
    ) -> dict[str, Any]:
        emitted_seed = marker_seed or seed
        combat_samples = (
            combat_rollback_samples
            if combat_rollback_samples is not None
            else rollback_samples
        )

        def sample_count(scenario: str) -> int:
            if profile == "clean":
                return 0
            return combat_samples if scenario == "combat" else rollback_samples

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
            rollbacks = sample_count(scenario)
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
                f"snapshot_version={'12' if combat else '11'}|"
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
            "input_version=2|tape_versions=1,2|snapshot_versions=11,12|tick_rate=60"
        )

        def case_metric(
            scenario: str,
            case_p95_work_ms: float,
            case_rollback_p999_ms: float,
        ) -> RuntimeMetric:
            combat = scenario == "combat"
            prefix = "combat" if combat else "full"
            case_id = f"{prefix}-{profile}-{seed}"
            rollback_calls = sample_count(scenario)
            # The validator cross-checks p99.9 against the over-budget count: an over-budget
            # p99.9 requires at least as many over-budget samples as the nearest-rank tail
            # holds, which is one slot for every n below the floor and grows above it.
            tail_slots = (
                rollback_calls
                - math.ceil(rollback_calls * ROLLBACK_PERCENTILE)
                + 1
            )
            over_count = (
                tail_slots
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

    # Healthy shapes, taken from the recorded complete_fixture population: playable p95 work runs
    # about 5.5x the paired clean control, and the playable rollback p99.9 runs about 1.85x the
    # playable p95 (the median of the 48 recorded pairs is 1.844).
    HEALTHY_WORK_RATIO = 5.5
    HEALTHY_TAIL_RATIO = 1.85
    HEALTHY_COMBAT_WORK_RATIO = 5.4
    HEALTHY_COMBAT_TAIL_RATIO = 1.7

    # The browser combat fixture is a pinned 80-tick campaign that produces six to eight rollbacks
    # by construction, so the synthetic matrix mirrors production and keeps combat below the
    # sample floor. Tests that need a gated combat pair ask for one explicitly.
    OBSERVED_COMBAT_ROLLBACK_SAMPLES = 8

    # The three shards land on three different runners, so the default fixture gives them three
    # different machine speeds. That spread is exactly what the absolute ceiling's runner scale
    # reads, and equal entries put every shard on scale 1.0.
    SHARD_MACHINE_P95_MS = (2.0, 2.25, 2.5)

    def synthetic_browser_cpu_matrix(
        scale: float,
        soccer_rollback_regression_seeds: tuple[int, ...] = (),
        combat_p95_regression_seeds: tuple[int, ...] = (),
        clean_control_scale: float = 1.0,
        playable_scale: float = 1.0,
        machine_p95_ms: tuple[float, float, float] = SHARD_MACHINE_P95_MS,
    ) -> list[dict[str, Any]]:
        """Build a seed-paired browser-full matrix.

        ``scale`` multiplies every measurement, so the ratio gates must be blind to it.
        ``clean_control_scale`` multiplies only the clean control's p95 work, reproducing the
        per-shard clean/playable disagreement that #188 measured on real hardware.
        ``playable_scale`` multiplies only the playable case, modelling a regression that slows
        the playable build uniformly while the clean control stays where it was.
        ``machine_p95_ms`` sets each shard's runner speed, which is what the absolute ceiling's
        peer-relative runner scale is derived from.
        """

        runs = []
        for index, seed in enumerate(NETWORK_SEEDS):
            for profile in BROWSER_FULL_PROFILES:
                machine_p95 = machine_p95_ms[index] * scale
                combat_machine_p95 = machine_p95 * 1.2
                if profile == "clean":
                    p95_work = machine_p95 * clean_control_scale
                    combat_p95_work = combat_machine_p95 * clean_control_scale
                    rollback_p999 = 0.0
                    combat_rollback_p999 = 0.0
                else:
                    p95_work = machine_p95 * HEALTHY_WORK_RATIO * playable_scale
                    tail_ratio = HEALTHY_TAIL_RATIO
                    if seed in soccer_rollback_regression_seeds:
                        tail_ratio = MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95 + 0.1
                    rollback_p999 = p95_work * tail_ratio
                    combat_p95_ratio = HEALTHY_COMBAT_WORK_RATIO
                    if seed in combat_p95_regression_seeds:
                        combat_p95_ratio = MAX_BROWSER_P95_WORK_RATIO + 0.1
                    combat_p95_work = combat_machine_p95 * combat_p95_ratio
                    combat_rollback_p999 = combat_p95_work * HEALTHY_COMBAT_TAIL_RATIO
                runs.append(
                    synthetic_browser_cpu_run(
                        profile,
                        seed,
                        p95_work,
                        rollback_p999,
                        combat_p95_work_ms=combat_p95_work,
                        combat_rollback_p999_ms=combat_rollback_p999,
                        combat_rollback_samples=OBSERVED_COMBAT_ROLLBACK_SAMPLES,
                    )
                )
        return runs

    proportional_slowdown = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(1.4),
        "firefox",
    )
    if not proportional_slowdown["pass"] or len(proportional_slowdown["pairs"]) != 6:
        raise RuntimeError("proportional browser slowdown did not pass normalization")
    if proportional_slowdown["scope"] != "aggregate":
        raise RuntimeError("complete browser CPU acceptance lost its aggregate scope")
    sharded_controls = synthetic_browser_cpu_matrix(1.4)
    for shard_index, shard_seed in enumerate(NETWORK_SEEDS):
        shard_runs = sharded_controls[shard_index * 2 : shard_index * 2 + 2]
        shard_acceptance = browser_cpu_acceptance(shard_runs, "firefox", (shard_seed,))
        if not shard_acceptance["pass"] or len(shard_acceptance["pairs"]) != 2:
            raise RuntimeError(f"seed {shard_seed} shard CPU acceptance self-test failed")
        if (
            shard_acceptance["scope"] != "shard"
            or shard_acceptance["seeds"] != [shard_seed]
        ):
            raise RuntimeError(f"seed {shard_seed} shard CPU acceptance lost its scope")
        # A shard job holds one seed, so it has no peers to measure its runner against and
        # cannot apply the absolute ceiling. It says so rather than quietly applying a value
        # calibrated for a machine it cannot see; the aggregate below collects the debt.
        for shard_pair in shard_acceptance["pairs"]:
            shard_ceiling = shard_pair["rollback_p999_gate"]["absolute_ceiling"]
            if (
                shard_ceiling["applied"]
                or shard_ceiling["effective_ms"] is not None
                or shard_ceiling["peer_count"] != 0
            ):
                raise RuntimeError(
                    f"seed {shard_seed} applied the absolute ceiling without a peer set: "
                    f"{shard_ceiling}"
                )
            expected_shard_status = (
                ROLLBACK_CEILING_GATE_DEFERRED
                if shard_pair["rollback_p999_gate"]["applied"]
                else shard_pair["rollback_p999_gate"]["status"]
            )
            if shard_ceiling["status"] != expected_shard_status:
                raise RuntimeError(
                    f"seed {shard_seed} misreported its deferred ceiling: {shard_ceiling}"
                )
        # A shard's deferral is a debt the rollback gate collects when it merges the shards.
        # aggregate_browser_evidence refuses evidence that still owes it, so shard-scoped
        # evidence must be exactly what that guard rejects.
        if unredeemed_ceiling_deferrals(shard_acceptance) != [
            f"complete_fixture seed {shard_seed}"
        ]:
            raise RuntimeError(
                f"seed {shard_seed} shard evidence did not record a ceiling debt: "
                f"{unredeemed_ceiling_deferrals(shard_acceptance)}"
            )
    for aggregate_pair in proportional_slowdown["pairs"]:
        aggregate_ceiling = aggregate_pair["rollback_p999_gate"]["absolute_ceiling"]
        if (
            aggregate_pair["rollback_p999_gate"]["applied"]
            != aggregate_ceiling["applied"]
            or aggregate_ceiling["peer_count"] != MIN_BROWSER_CEILING_PEER_COUNT
        ):
            raise RuntimeError(
                "the aggregate did not redeem a shard's deferred ceiling: "
                f"{aggregate_ceiling}"
            )
    if unredeemed_ceiling_deferrals(proportional_slowdown):
        raise RuntimeError(
            "aggregate evidence still owed a ceiling: "
            f"{unredeemed_ceiling_deferrals(proportional_slowdown)}"
        )
    if browser_cpu_acceptance(sharded_controls, "firefox") != proportional_slowdown:
        raise RuntimeError("merged seed shards did not reconstruct the aggregate contract")
    foreign_seed = browser_cpu_acceptance(sharded_controls, "firefox", (2001,))
    if foreign_seed["pass"] or not any(
        "unexpected complete_fixture clean control for seed 2002" in reason
        for reason in foreign_seed["reasons"]
    ):
        raise RuntimeError("shard-scoped browser CPU acceptance accepted a foreign seed")
    incomplete_aggregate = browser_cpu_acceptance(sharded_controls[:4], "firefox")
    if incomplete_aggregate["pass"] or not any(
        "missing the complete_fixture clean control for seed 2003" in reason
        for reason in incomplete_aggregate["reasons"]
    ):
        raise RuntimeError("aggregate browser CPU acceptance accepted a vanished shard")
    for unpinned in ((), (2004,)):
        try:
            browser_cpu_acceptance(sharded_controls, "firefox", unpinned)
        except ValueError:
            pass
        else:
            raise RuntimeError(f"browser CPU acceptance accepted seeds {unpinned!r}")

    # The ratio gates are scale-free by construction, which is exactly why normalization on its
    # own could excuse a build that is simply slower everywhere. The absolute ceiling is the part
    # that is not scale-free: push the same healthy shape far enough and it must fail.
    #
    # It fires on seed 2002 rather than on the slowest shard, and that is the runner scale working
    # as intended. A uniform slowdown multiplies every shard equally, so no shard moves relative
    # to its peers and every runner scale is exactly what it was on the healthy matrix: seed 2003
    # is 17.6% slower than its peers by construction and keeps the 17.6% larger ceiling it had
    # before the slowdown, so the gate binds on the middle shard, whose scale is 1.0. The
    # correction equalizes sensitivity across shards; it does not lower it.
    gross_slowdown = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(1.8),
        "firefox",
    )
    if gross_slowdown["pass"] or not any(
        "complete_fixture seed 2002 playable_rollback_p999_ms" in reason
        for reason in gross_slowdown["reasons"]
    ):
        raise RuntimeError(
            "the absolute rollback ceiling did not backstop a proportional slowdown"
        )
    if any(
        "rollback_p999_over_playable_p95" in reason
        for reason in gross_slowdown["reasons"]
    ):
        raise RuntimeError(
            "a proportional slowdown moved the normalized rollback ratio"
        )

    # The proportional blind spot, pinned as a known bounded property rather than left unexamined.
    #
    # A regression that slows the playable build uniformly multiplies the tail and its normalizer
    # by the same factor, so the normalized ratio does not move AT ALL -- the derivative is zero,
    # not merely small. Contract 6 did not have this blind spot, because it divided by the clean
    # control, which such a regression leaves untouched. Contract 7 accepts it in exchange for
    # removing the clean control's session noise, and bounds it with the work-ratio gate and the
    # absolute ceiling.
    #
    # These assertions state the bound in both directions: below the bound the class is invisible,
    # above it the backstops fire, and the normalized ratio is unchanged either way. Tightening
    # this is a deliberate future decision (see #191 for the work gate's own calibration), and
    # these assertions are what will notice when someone makes it.
    def normalized_ratios(acceptance: dict[str, Any]) -> list[float]:
        return [
            pair["ratios"]["rollback_p999_over_playable_p95"]
            for pair in acceptance["pairs"]
            if pair["scenario"] == "complete_fixture"
        ]

    baseline_normalized = normalized_ratios(
        browser_cpu_acceptance(synthetic_browser_cpu_matrix(1.0), "firefox")
    )
    # 1.2 sits under the work-ratio bound for this fixture (5.5 * 1.2 = 6.6 against 6.7), so a 20%
    # uniform playable regression clears every gate. That is the documented gap.
    undetected_proportional = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(1.0, playable_scale=1.2),
        "firefox",
    )
    if not undetected_proportional["pass"]:
        raise RuntimeError(
            "the proportional blind spot changed shape; re-derive the documented bound: "
            f"{undetected_proportional['reasons']}"
        )
    if normalized_ratios(undetected_proportional) != baseline_normalized:
        raise RuntimeError("a proportional regression moved the normalized rollback ratio")
    # 1.3 crosses the work-ratio bound (5.5 * 1.3 = 7.15), and the backstop fires.
    detected_proportional = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(1.0, playable_scale=1.3),
        "firefox",
    )
    if detected_proportional["pass"] or not any(
        "complete_fixture seed 2001 p95_work_ratio" in reason
        for reason in detected_proportional["reasons"]
    ):
        raise RuntimeError("no backstop caught a 1.3x uniform playable regression")
    if normalized_ratios(detected_proportional) != baseline_normalized:
        raise RuntimeError("a proportional regression moved the normalized rollback ratio")
    if any(
        "rollback_p999_over_playable_p95" in reason
        for reason in detected_proportional["reasons"]
    ):
        raise RuntimeError(
            "the normalized ratio claimed credit for catching a proportional regression"
        )
    # #188's mechanism: the clean control is a separate browser session, so it can record a
    # slower machine than the playable case it is paired with. Contract 6 divided the playable
    # tail by that control, so this pair read 1 / 0.55 = 1.8x its true composite ratio. The
    # normalized gate is measured entirely inside the playable session and does not move.
    noisy_clean_control = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(1.0, clean_control_scale=1 / 0.55),
        "firefox",
    )
    if not noisy_clean_control["pass"]:
        raise RuntimeError(
            "an anomalously slow clean control failed the runner-relative gate: "
            f"{noisy_clean_control['reasons']}"
        )
    noisy_pair = next(
        pair
        for pair in noisy_clean_control["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2001
    )
    if round(noisy_pair["ratios"]["rollback_p999_over_playable_p95"], 6) != round(
        HEALTHY_TAIL_RATIO,
        6,
    ):
        raise RuntimeError("clean-control noise leaked into the normalized rollback ratio")
    if noisy_pair["ratios"]["rollback_p999_over_clean_p95"] >= (
        HEALTHY_WORK_RATIO * HEALTHY_TAIL_RATIO
    ):
        raise RuntimeError("the composite diagnostic did not record the clean-control noise")
    multi_family_regression = browser_cpu_acceptance(
        synthetic_browser_cpu_matrix(
            1.0,
            soccer_rollback_regression_seeds=(2001,),
            combat_p95_regression_seeds=(2002,),
        ),
        "firefox",
    )
    expected_regressions = (
        "complete_fixture seed 2001 rollback_p999_over_playable_p95",
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
    # Seed 2002 clean controls are 2.25 ms (complete_fixture) and 2.7 ms (combat) at scale 1.0.
    # 12.5 ms of playable p95 work puts the exact-threshold tail at 32.5 ms: under the absolute
    # ceiling, so this exercises the normalized ratio on its own, and exact at the six decimal
    # places the metric markers carry, so the boundary is not decided by a rounding artifact.
    exact_rollback_boundary = browser_cpu_acceptance(
        replace_control(
            complete_controls,
            synthetic_browser_cpu_run(
                "playable",
                2002,
                12.5,
                12.5 * MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95,
                combat_p95_work_ms=2.7 * 5.4,
                combat_rollback_p999_ms=2.7 * 5.4 * 1.7,
            ),
        ),
        "firefox",
    )
    if exact_rollback_boundary["pass"] or not any(
        "complete_fixture seed 2002 rollback_p999_over_playable_p95=2.600000000" in reason
        for reason in exact_rollback_boundary["reasons"]
    ):
        raise RuntimeError("exact browser rollback ratio threshold passed strict gate")
    if any(
        "seed 2002 playable_rollback_p999_ms" in reason
        for reason in exact_rollback_boundary["reasons"]
    ):
        raise RuntimeError("the absolute ceiling fired below its own threshold")
    # The absolute ceiling, exercised on its own at both of the values it can take.
    #
    # First the base value, on a matrix whose three shards run at the same speed, so every runner
    # scale is exactly 1.0 and the enforced ceiling is the unmodified 39.7 ms. 3.0 ms machines put
    # playable p95 work at 16.5 ms, which holds the normalized ratio at 39.7 / 16.5 = 2.406 and
    # the work ratio at 5.5, so neither of the other two gates can be what fires.
    def uniform_shard_playable(
        seed: int,
        machine_p95: float,
        rollback_p999_ms: float,
    ) -> dict[str, Any]:
        combat_p95 = machine_p95 * 1.2 * HEALTHY_COMBAT_WORK_RATIO
        return synthetic_browser_cpu_run(
            "playable",
            seed,
            machine_p95 * HEALTHY_WORK_RATIO,
            rollback_p999_ms,
            combat_p95_work_ms=combat_p95,
            combat_rollback_p999_ms=combat_p95 * HEALTHY_COMBAT_TAIL_RATIO,
            combat_rollback_samples=OBSERVED_COMBAT_ROLLBACK_SAMPLES,
        )

    even_machines = synthetic_browser_cpu_matrix(1.0, machine_p95_ms=(3.0, 3.0, 3.0))
    exact_ceiling_boundary = browser_cpu_acceptance(
        replace_control(
            even_machines,
            uniform_shard_playable(2003, 3.0, MAX_BROWSER_ROLLBACK_P999_MS),
        ),
        "firefox",
    )
    if exact_ceiling_boundary["pass"] or not any(
        "complete_fixture seed 2003 playable_rollback_p999_ms=39.700000 "
        "does not meet <39.700000 (39.7 ms base ceiling at a 1.000000 runner scale "
        "over a 16.500000 ms peer median p95 work)" in reason
        for reason in exact_ceiling_boundary["reasons"]
    ):
        raise RuntimeError(
            "exact browser rollback ceiling passed strict gate: "
            f"{exact_ceiling_boundary['reasons']}"
        )
    if any(
        "seed 2003 rollback_p999_over_playable_p95" in reason
        for reason in exact_ceiling_boundary["reasons"]
    ):
        raise RuntimeError("the normalized ratio fired below its own threshold")

    # Then the scaled value, at a runner scale the fixture makes exact. Machines of 3.0, 3.0 and
    # 3.75 ms put seed 2003's playable p95 work at 20.625 ms against a 16.5 ms peer median: a
    # runner scale of exactly 1.25, so the enforced ceiling is 39.7 * 1.25 = 49.625 ms. The
    # normalized ratio at that tail is 2.406 and the work ratio is 5.5, so again neither of the
    # other gates can be what fires.
    uneven_machines = synthetic_browser_cpu_matrix(1.0, machine_p95_ms=(3.0, 3.0, 3.75))
    scaled_ceiling_boundary = browser_cpu_acceptance(
        replace_control(
            uneven_machines,
            uniform_shard_playable(2003, 3.75, MAX_BROWSER_ROLLBACK_P999_MS * 1.25),
        ),
        "firefox",
    )
    if scaled_ceiling_boundary["pass"] or not any(
        "complete_fixture seed 2003 playable_rollback_p999_ms=49.625000 "
        "does not meet <49.625000 (39.7 ms base ceiling at a 1.250000 runner scale "
        "over a 16.500000 ms peer median p95 work)" in reason
        for reason in scaled_ceiling_boundary["reasons"]
    ):
        raise RuntimeError(
            "exact scaled browser rollback ceiling passed strict gate: "
            f"{scaled_ceiling_boundary['reasons']}"
        )
    if any(
        "seed 2003 rollback_p999_over_playable_p95" in reason
        or "seed 2003 p95_work_ratio" in reason
        for reason in scaled_ceiling_boundary["reasons"]
    ):
        raise RuntimeError("a ratio gate fired below its own threshold")

    # #230 in miniature: the same shard, one microsecond of tail lower, passes -- and it passes
    # 25% above the base ceiling, purely because its own runner measured 25% slower than the
    # campaign's other two. Under the fixed ceiling this was a red build.
    scaled_ceiling_headroom = browser_cpu_acceptance(
        replace_control(
            uneven_machines,
            uniform_shard_playable(2003, 3.75, MAX_BROWSER_ROLLBACK_P999_MS * 1.25 - 0.001),
        ),
        "firefox",
    )
    if not scaled_ceiling_headroom["pass"]:
        raise RuntimeError(
            "a slow runner under its own scaled ceiling still failed: "
            f"{scaled_ceiling_headroom['reasons']}"
        )
    scaled_headroom_pair = next(
        pair
        for pair in scaled_ceiling_headroom["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2003
    )
    if (
        scaled_headroom_pair["absolute_diagnostics"]["playable_rollback_p999_ms"]
        <= MAX_BROWSER_ROLLBACK_P999_MS
    ):
        raise RuntimeError("the scaled-ceiling headroom case never cleared the base ceiling")
    if scaled_headroom_pair["rollback_p999_gate"]["absolute_ceiling"] != {
        "applied": True,
        "base_ms": MAX_BROWSER_ROLLBACK_P999_MS,
        "effective_ms": 49.625,
        "max_runner_scale": MAX_BROWSER_RUNNER_SCALE,
        "peer_count": MIN_BROWSER_CEILING_PEER_COUNT,
        "peer_reference_p95_work_ms": 16.5,
        "reference": BROWSER_CEILING_RUNNER_REFERENCE,
        "runner_scale": 1.25,
        "runner_scale_clamped": False,
        "status": ROLLBACK_P999_GATE_APPLIED,
    }:
        raise RuntimeError(
            "the scaled ceiling was not published: "
            f"{scaled_headroom_pair['rollback_p999_gate']['absolute_ceiling']}"
        )
    # The correction only ever loosens. Seeds 2001 and 2002 are faster than their own peer
    # medians here, and both are still measured against the unmodified 39.7 ms, so nothing that
    # passed the fixed ceiling can fail the scaled one.
    for faster_seed in (2001, 2002):
        faster_pair = next(
            pair
            for pair in scaled_ceiling_headroom["pairs"]
            if pair["scenario"] == "complete_fixture" and pair["seed"] == faster_seed
        )
        faster_ceiling = faster_pair["rollback_p999_gate"]["absolute_ceiling"]
        if (
            faster_ceiling["runner_scale"] != 1.0
            or faster_ceiling["effective_ms"] != MAX_BROWSER_ROLLBACK_P999_MS
            or faster_ceiling["peer_reference_p95_work_ms"]
            <= faster_pair["absolute_diagnostics"]["playable_p95_work_ms"]
        ):
            raise RuntimeError(
                f"seed {faster_seed} ran faster than its peers and had its ceiling tightened: "
                f"{faster_ceiling}"
            )

    # The clamp. A shard that reads absurdly slow against its peers cannot buy an unbounded
    # exemption: the correction stops at MAX_BROWSER_RUNNER_SCALE, the pair is still measured
    # against a ceiling, and the artifact says the clamp bound.
    clamped_ceiling = browser_cpu_acceptance(
        replace_control(
            synthetic_browser_cpu_matrix(1.0, machine_p95_ms=(3.0, 3.0, 9.0)),
            uniform_shard_playable(
                2003,
                9.0,
                MAX_BROWSER_ROLLBACK_P999_MS * MAX_BROWSER_RUNNER_SCALE,
            ),
        ),
        "firefox",
    )
    clamped_pair = next(
        pair
        for pair in clamped_ceiling["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2003
    )
    clamped_block = clamped_pair["rollback_p999_gate"]["absolute_ceiling"]
    if (
        clamped_block["runner_scale"] != MAX_BROWSER_RUNNER_SCALE
        or not clamped_block["runner_scale_clamped"]
        or clamped_block["effective_ms"]
        != round(MAX_BROWSER_ROLLBACK_P999_MS * MAX_BROWSER_RUNNER_SCALE, 9)
    ):
        raise RuntimeError(f"the runner scale was not clamped: {clamped_block}")
    if clamped_ceiling["pass"] or not any(
        "complete_fixture seed 2003 playable_rollback_p999_ms=63.520000" in reason
        for reason in clamped_ceiling["reasons"]
    ):
        raise RuntimeError(
            "a runner scale past the clamp escaped the ceiling entirely: "
            f"{clamped_ceiling['reasons']}"
        )

    def combat_pair(acceptance: dict[str, Any], seed: int) -> dict[str, Any]:
        return next(
            pair
            for pair in acceptance["pairs"]
            if pair["scenario"] == "combat" and pair["seed"] == seed
        )

    def small_sample_combat_run(rollback_samples: int) -> dict[str, Any]:
        # Seed 2002 clean controls are 2.25 ms (complete_fixture) and 2.7 ms (combat) at
        # scale 1.0. A 14.58 * 3.6 = 52.5 ms combat p99.9 is past both the normalized ratio
        # threshold and the absolute ceiling, so the floor has to hold back two gates.
        return synthetic_browser_cpu_run(
            "playable",
            2002,
            2.25 * 5.5,
            2.25 * 5.5 * 1.85,
            combat_p95_work_ms=2.7 * 5.4,
            combat_rollback_p999_ms=(
                2.7 * 5.4 * (MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95 + 1.0)
            ),
            combat_rollback_samples=rollback_samples,
        )

    # 8 is the observed browser combat sample count; the floor minus one is the boundary.
    for below_floor_samples in (8, MIN_ROLLBACK_P999_SAMPLE_COUNT - 1):
        below_floor = browser_cpu_acceptance(
            replace_control(
                complete_controls,
                small_sample_combat_run(below_floor_samples),
            ),
            "firefox",
        )
        below_floor_pair = combat_pair(below_floor, 2002)
        if not below_floor["pass"] or any(
            "combat seed 2002 rollback_p999_over_playable_p95" in reason
            or "combat seed 2002 playable_rollback_p999_ms" in reason
            for reason in below_floor["reasons"]
        ):
            raise RuntimeError(
                f"combat rollback p99.9 ratio at {below_floor_samples} samples was gated"
            )
        below_floor_ceiling = below_floor_pair["rollback_p999_gate"]["absolute_ceiling"]
        # The runner scale is still measured and still published for a pair the sample floor
        # holds back, so the artifact shows what the ceiling would have been compared against.
        # combat seed 2002 sits exactly at its own peer median here.
        if below_floor_ceiling != {
            "applied": False,
            "base_ms": MAX_BROWSER_ROLLBACK_P999_MS,
            "effective_ms": MAX_BROWSER_ROLLBACK_P999_MS,
            "max_runner_scale": MAX_BROWSER_RUNNER_SCALE,
            "peer_count": MIN_BROWSER_CEILING_PEER_COUNT,
            "peer_reference_p95_work_ms": 14.58,
            "reference": BROWSER_CEILING_RUNNER_REFERENCE,
            "runner_scale": 1.0,
            "runner_scale_clamped": False,
            "status": ROLLBACK_P999_GATE_DIAGNOSTIC,
        }:
            raise RuntimeError(
                f"the small-sample ceiling block was not recorded: {below_floor_ceiling}"
            )
        if {
            key: value
            for key, value in below_floor_pair["rollback_p999_gate"].items()
            if key != "absolute_ceiling"
        } != {
            "applied": False,
            "composite_ratio": below_floor_pair["ratios"][
                "rollback_p999_over_clean_p95"
            ],
            "minimum_sample_count": MIN_ROLLBACK_P999_SAMPLE_COUNT,
            "normalized_ratio": below_floor_pair["ratios"][
                "rollback_p999_over_playable_p95"
            ],
            "normalizer": "playable_p95_work_ms",
            "playable_rollback_p999_ms": below_floor_pair["absolute_diagnostics"][
                "playable_rollback_p999_ms"
            ],
            "sample_count": below_floor_samples,
            "status": ROLLBACK_P999_GATE_DIAGNOSTIC,
        }:
            raise RuntimeError("small-sample combat pair was not recorded diagnostically")
        if (
            below_floor_pair["ratios"]["rollback_p999_over_playable_p95"]
            < MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95
            or below_floor_pair["absolute_diagnostics"]["playable_rollback_p999_ms"]
            < MAX_BROWSER_ROLLBACK_P999_MS
            or below_floor_pair["absolute_diagnostics"][
                "playable_rollback_sample_count"
            ]
            != below_floor_samples
        ):
            raise RuntimeError(
                "small-sample combat diagnostics lost the over-threshold ratio"
            )
        if not all(
            pair["rollback_p999_gate"]["status"] == ROLLBACK_P999_GATE_APPLIED
            for pair in below_floor["pairs"]
            if pair["scenario"] == "complete_fixture"
        ):
            raise RuntimeError("the sample-count floor exempted a full-sample pair")

    at_floor = browser_cpu_acceptance(
        replace_control(
            complete_controls,
            small_sample_combat_run(MIN_ROLLBACK_P999_SAMPLE_COUNT),
        ),
        "firefox",
    )
    # combat is not a calibrated scenario, so clearing the sample floor must fail closed rather
    # than borrow the complete_fixture thresholds. This is the tripwire that stops 2.6 and 39.7 ms
    # being reused verbatim if #179 ever redesigns the combat fixture past 1,000 rollbacks.
    at_floor_pair = combat_pair(at_floor, 2002)
    if at_floor["pass"] or not any(
        "combat seed 2002 cleared the rollback p99.9 sample floor, but the tail thresholds "
        "are calibrated for ['complete_fixture'] only" in reason
        for reason in at_floor["reasons"]
    ):
        raise RuntimeError(
            "an uncalibrated scenario at the sample floor did not fail closed: "
            f"{at_floor['reasons']}"
        )
    if any(
        "combat seed 2002 rollback_p999_over_playable_p95" in reason
        or "combat seed 2002 playable_rollback_p999_ms" in reason
        for reason in at_floor["reasons"]
    ):
        raise RuntimeError("an uncalibrated scenario was compared against borrowed thresholds")
    if at_floor_pair["rollback_p999_gate"]["status"] != ROLLBACK_P999_GATE_UNCALIBRATED:
        raise RuntimeError("the uncalibrated pair was not recorded as uncalibrated")
    if at_floor_pair["rollback_p999_gate"]["applied"]:
        raise RuntimeError("an uncalibrated pair reported its gate as applied")

    # The calibrated scenario at exactly the sample floor still gates normally.
    calibrated_at_floor = browser_cpu_acceptance(
        replace_control(
            complete_controls,
            synthetic_browser_cpu_run(
                "playable",
                2002,
                12.5,
                12.5 * MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95,
                combat_p95_work_ms=2.7 * 5.4,
                combat_rollback_p999_ms=2.7 * 5.4 * 1.7,
                rollback_samples=MIN_ROLLBACK_P999_SAMPLE_COUNT,
                combat_rollback_samples=OBSERVED_COMBAT_ROLLBACK_SAMPLES,
            ),
        ),
        "firefox",
    )
    calibrated_at_floor_pair = next(
        pair
        for pair in calibrated_at_floor["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2002
    )
    if calibrated_at_floor["pass"] or not any(
        "complete_fixture seed 2002 rollback_p999_over_playable_p95=2.600000000" in reason
        for reason in calibrated_at_floor["reasons"]
    ):
        raise RuntimeError("the calibrated scenario at the sample floor passed the gate")
    calibrated_at_floor_ceiling = calibrated_at_floor_pair["rollback_p999_gate"][
        "absolute_ceiling"
    ]
    if (
        not calibrated_at_floor_ceiling["applied"]
        or calibrated_at_floor_ceiling["status"] != ROLLBACK_P999_GATE_APPLIED
        or calibrated_at_floor_ceiling["peer_count"] != MIN_BROWSER_CEILING_PEER_COUNT
    ):
        raise RuntimeError(
            "the calibrated pair at the sample floor did not apply its ceiling: "
            f"{calibrated_at_floor_ceiling}"
        )
    if {
        key: value
        for key, value in calibrated_at_floor_pair["rollback_p999_gate"].items()
        if key != "absolute_ceiling"
    } != {
        "applied": True,
        "composite_ratio": calibrated_at_floor_pair["ratios"][
            "rollback_p999_over_clean_p95"
        ],
        "minimum_sample_count": MIN_ROLLBACK_P999_SAMPLE_COUNT,
        "normalized_ratio": calibrated_at_floor_pair["ratios"][
            "rollback_p999_over_playable_p95"
        ],
        "normalizer": "playable_p95_work_ms",
        "playable_rollback_p999_ms": calibrated_at_floor_pair["absolute_diagnostics"][
            "playable_rollback_p999_ms"
        ],
        "sample_count": MIN_ROLLBACK_P999_SAMPLE_COUNT,
        "status": ROLLBACK_P999_GATE_APPLIED,
    }:
        raise RuntimeError("the pair at the sample floor was not recorded as gated")

    def rewrite_metric_markers(
        run: dict[str, Any],
        old: str,
        new: str,
    ) -> dict[str, Any]:
        rewritten = []
        for row in run["runtime_metrics"]["rows"]:
            raw = row["marker"].replace(old, new)
            parsed = parse_runtime_metric(raw)
            rewritten.append(
                {"fields": parsed.fields, "kind": parsed.kind, "marker": raw}
            )
        payload = ("\n".join(row["marker"] for row in rewritten) + "\n").encode()
        return {
            **run,
            "runtime_metrics": {
                "marker_sha256": sha256_bytes(payload),
                "rows": rewritten,
            },
        }

    intact_sample_count = f"rollback_sample_count={MIN_ROLLBACK_P999_SAMPLE_COUNT}"
    for intact, broken_sample_count, description in (
        (intact_sample_count, "rollback_sample_count=not-a-number", "malformed"),
        (intact_sample_count, "rollback_sample_count=-1", "negative"),
        (f"|{intact_sample_count}", "", "absent"),
    ):
        broken = browser_cpu_acceptance(
            [
                complete_controls[0],
                rewrite_metric_markers(
                    complete_controls[1],
                    intact,
                    broken_sample_count,
                ),
                *complete_controls[2:],
            ],
            "firefox",
        )
        if broken["pass"] or not any(
            "rollback_sample_count" in reason for reason in broken["reasons"]
        ):
            raise RuntimeError(
                f"{description} rollback_sample_count did not fail the browser CPU gate closed"
            )
        if any(
            pair["scenario"] == "complete_fixture" and pair["seed"] == 2001
            for pair in broken["pairs"]
        ):
            raise RuntimeError(
                f"{description} rollback_sample_count was exempted instead of rejected"
            )

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
    # The tail normalizer is a measurement, so it can go missing. finite_non_negative_float
    # accepts 0.0 upstream, which would make the normalized ratio 0/0 -- an exemption, not a
    # failure. Every collapsed normalizer must fail the pair instead.
    for normalizer_p95, normalizer_p999, description in (
        (0.0, 0.0, "zero"),
        (0.0, 24.0, "zero with a live tail"),
    ):
        zero_normalizer = browser_cpu_acceptance(
            replace_control(
                complete_controls,
                synthetic_browser_cpu_run(
                    "playable",
                    2001,
                    normalizer_p95,
                    normalizer_p999,
                    combat_p95_work_ms=2.4 * 5.4,
                    combat_rollback_p999_ms=2.4 * 5.4 * 1.7,
                ),
            ),
            "firefox",
        )
        if zero_normalizer["pass"] or not any(
            "complete_fixture seed 2001 playable p95 work normalizer must be finite and >0"
            in reason
            for reason in zero_normalizer["reasons"]
        ):
            raise RuntimeError(
                f"a {description} playable p95 normalizer passed normalization"
            )
        if any(
            pair["scenario"] == "complete_fixture" and pair["seed"] == 2001
            for pair in zero_normalizer["pairs"]
        ):
            raise RuntimeError("a collapsed normalizer still published a gated pair")
        # That collapsed measurement is also somebody else's runner reference. A peer that
        # reports 0.0 ms of p95 work cannot say how fast anyone's machine was, so the pairs
        # that would have divided by it fail closed instead of falling back to an unscaled
        # ceiling, an unbounded scale, or a division by zero.
        for orphaned_seed in (2002, 2003):
            if not any(
                f"complete_fixture seed {orphaned_seed} has no usable runner reference"
                in reason
                for reason in zero_normalizer["reasons"]
            ):
                raise RuntimeError(
                    f"a {description} peer p95 work silently exempted seed {orphaned_seed}"
                )
            orphaned = next(
                pair
                for pair in zero_normalizer["pairs"]
                if pair["scenario"] == "complete_fixture" and pair["seed"] == orphaned_seed
            )
            orphaned_ceiling = orphaned["rollback_p999_gate"]["absolute_ceiling"]
            if (
                orphaned_ceiling["applied"]
                or orphaned_ceiling["status"] != ROLLBACK_CEILING_GATE_ERROR
                or orphaned_ceiling["effective_ms"] is not None
                or orphaned_ceiling["runner_scale"] is not None
                or orphaned["pass"]
            ):
                raise RuntimeError(
                    f"seed {orphaned_seed} published a ceiling without a reference: "
                    f"{orphaned_ceiling}"
                )

    # The runner reference, unit-tested at every boundary it has. A peer set below
    # MIN_BROWSER_CEILING_PEER_COUNT in scope is a deferral to the aggregate; a peer set that is
    # in scope but short, absent, or carrying a value that cannot describe a machine is an error.
    # Neither is ever an exemption: only ROLLBACK_P999_GATE_APPLIED enforces, and only it
    # publishes an effective ceiling.
    for peers, expected_peer_count, expected_status in (
        ([], 0, ROLLBACK_CEILING_GATE_DEFERRED),
        ([16.5], 1, ROLLBACK_CEILING_GATE_DEFERRED),
        ([], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_CEILING_GATE_ERROR),
        ([16.5], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_CEILING_GATE_ERROR),
        ([16.5, 0.0], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_CEILING_GATE_ERROR),
        ([16.5, -1.0], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_CEILING_GATE_ERROR),
        ([16.5, float("inf")], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_CEILING_GATE_ERROR),
        ([16.5, float("nan")], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_CEILING_GATE_ERROR),
        ([16.5, 16.5], MIN_BROWSER_CEILING_PEER_COUNT, ROLLBACK_P999_GATE_APPLIED),
    ):
        reference_block = browser_ceiling_gate(20.625, peers, expected_peer_count)
        if reference_block["status"] != expected_status:
            raise RuntimeError(
                f"runner reference {peers!r} at {expected_peer_count} expected peers "
                f"reported {reference_block['status']!r}, expected {expected_status!r}"
            )
        enforced = expected_status == ROLLBACK_P999_GATE_APPLIED
        if reference_block["applied"] is not enforced:
            raise RuntimeError(f"runner reference {peers!r} misreported enforcement")
        if not enforced and reference_block["effective_ms"] is not None:
            raise RuntimeError(f"runner reference {peers!r} published a ceiling anyway")
        if enforced and reference_block["effective_ms"] != 49.625:
            raise RuntimeError(f"runner reference {peers!r} scaled the ceiling wrongly")

    def recorded_browser_cpu_matrix(
        measurements: tuple[tuple[float, float, float], ...],
    ) -> list[dict[str, Any]]:
        """Replay recorded (clean p95, playable p95, playable p99.9) triples, seed by seed.

        The combat cases are held at a fixed healthy shape so the assertions below are about
        the complete_fixture measurements and nothing else.
        """

        runs = []
        for seed, (clean_p95, playable_p95, playable_p999) in zip(
            NETWORK_SEEDS,
            measurements,
        ):
            combat_clean_p95 = clean_p95 * 1.2
            combat_playable_p95 = combat_clean_p95 * 4.0
            runs.append(
                synthetic_browser_cpu_run(
                    "clean",
                    seed,
                    clean_p95,
                    0.0,
                    combat_p95_work_ms=combat_clean_p95,
                    combat_rollback_p999_ms=0.0,
                )
            )
            runs.append(
                synthetic_browser_cpu_run(
                    "playable",
                    seed,
                    playable_p95,
                    playable_p999,
                    combat_p95_work_ms=combat_playable_p95,
                    combat_rollback_p999_ms=combat_playable_p95 * 1.5,
                    combat_rollback_samples=OBSERVED_COMBAT_ROLLBACK_SAMPLES,
                )
            )
        return runs

    # Measurements lifted verbatim from the artifacts analysed in #188.
    for description, measurements in (
        # The first sharded campaign, chrome. Seed 2003 read a composite ratio of 11.807 against
        # the contract-6 threshold of 11.7 and failed a healthy build. Seed 2001 is the pair
        # whose clean control disagreed with its own playable partner about the machine.
        (
            "first sharded chrome campaign",
            (
                (4.185, 12.490, 25.245),
                (2.355, 12.645, 25.910),
                (1.835, 9.590, 21.665),
            ),
        ),
        # The fastest runner in the 42-pair unsharded population (run 30179056065, chrome).
        # Fast machines are where the composite ratio drifts highest.
        (
            "fastest recorded unsharded runner",
            (
                (1.950, 10.320, 21.170),
                (1.970, 10.675, 22.065),
                (1.955, 10.665, 22.035),
            ),
        ),
        # The slowest runner in that population (run 30176706801, firefox).
        (
            "slowest recorded unsharded runner",
            (
                (3.600, 18.360, 30.400),
                (3.560, 18.480, 31.400),
                (3.600, 18.360, 33.580),
            ),
        ),
    ):
        replayed = browser_cpu_acceptance(
            recorded_browser_cpu_matrix(measurements),
            "firefox",
        )
        if not replayed["pass"]:
            raise RuntimeError(
                f"recorded healthy measurements from the {description} failed the gate: "
                f"{replayed['reasons']}"
            )
    sharded_replay = browser_cpu_acceptance(
        recorded_browser_cpu_matrix(
            (
                (4.185, 12.490, 25.245),
                (2.355, 12.645, 25.910),
                (1.835, 9.590, 21.665),
            )
        ),
        "firefox",
    )
    sharded_seed_2003 = next(
        pair
        for pair in sharded_replay["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2003
    )
    if round(sharded_seed_2003["ratios"]["rollback_p999_over_clean_p95"], 3) != 11.807:
        raise RuntimeError("the retired composite ratio is no longer published for comparison")
    if (
        sharded_seed_2003["ratios"]["rollback_p999_over_playable_p95"]
        >= MAX_BROWSER_ROLLBACK_P999_OVER_PLAYABLE_P95
    ):
        raise RuntimeError("the run that motivated #188 still fails the normalized gate")
    # The two genuine breaches of run 30181277777, replayed at a full sample count so the gate
    # is actually applied. Seed 2001 carries the chrome combat 2002 measurements (43.545 ms) and
    # seed 2002 the firefox combat 2002 measurements (34.100 ms). The firefox pair sits under the
    # absolute ceiling, so only the normalized ratio can catch it -- a rule that needed the
    # ceiling for both would be an absolute gate wearing a ratio's name. Seed 2003 is a healthy
    # pair from the same run and must stay clean.
    breach_replay = browser_cpu_acceptance(
        recorded_browser_cpu_matrix(
            (
                (3.015, 14.660, 43.545),
                (2.740, 12.720, 34.100),
                (3.000, 16.020, 29.500),
            )
        ),
        "firefox",
    )
    breach_expectations = (
        ("complete_fixture seed 2001 rollback_p999_over_playable_p95", True),
        ("complete_fixture seed 2001 playable_rollback_p999_ms", True),
        ("complete_fixture seed 2002 rollback_p999_over_playable_p95", True),
        ("complete_fixture seed 2002 playable_rollback_p999_ms", False),
        ("complete_fixture seed 2003", False),
    )
    for fragment, expected in breach_expectations:
        if any(fragment in reason for reason in breach_replay["reasons"]) != expected:
            raise RuntimeError(
                f"replayed genuine breach verdict changed for {fragment!r}: "
                f"{breach_replay['reasons']}"
            )
    if breach_replay["pass"]:
        raise RuntimeError("replayed genuine browser rollback breaches passed the gate")

    # #230, replayed verbatim. Run 30238674582, complete_fixture, chrome: the campaign that
    # turned main red on a healthy build because one of the three runners was about 38% slow.
    FALSE_RED_CHROME = (
        (3.140, 15.735, 29.720),
        (3.470, 16.775, 29.740),
        (4.120, 21.840, 48.400),
    )
    false_red = browser_cpu_acceptance(
        recorded_browser_cpu_matrix(FALSE_RED_CHROME),
        "firefox",
    )
    if not false_red["pass"]:
        raise RuntimeError(
            f"the #230 false red still fails the gate: {false_red['reasons']}"
        )
    false_red_pair = next(
        pair
        for pair in false_red["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2003
    )
    false_red_ceiling = false_red_pair["rollback_p999_gate"]["absolute_ceiling"]
    # This is the derivation of MAX_BROWSER_RUNNER_SCALE, pinned to the measurements it came
    # from: 21.840 ms of p95 work against a 16.2550 ms peer median is the worst runner slowdown
    # ever recorded, and the cap is that carried through the calibration margin. Change either
    # measurement and the constant no longer matches its own justification.
    if false_red_ceiling["peer_reference_p95_work_ms"] != 16.255 or round(
        false_red_ceiling["runner_scale"],
        9,
    ) != round(BROWSER_RUNNER_SCALE_CALIBRATION_MAX, 9):
        raise RuntimeError(
            "the runner-scale calibration drifted from the pair it was derived from: "
            f"{false_red_ceiling}"
        )
    if not (
        MAX_BROWSER_ROLLBACK_P999_MS
        < false_red_pair["absolute_diagnostics"]["playable_rollback_p999_ms"]
        < false_red_ceiling["effective_ms"]
    ):
        raise RuntimeError(
            "the #230 tail no longer sits between the base and the scaled ceiling: "
            f"{false_red_ceiling}"
        )
    # And the reason the clean control could not have been the reference instead (option 1 of
    # #230). The healthy firefox seed 2002 pair of run 30233355883 -- 3.680 ms clean, 19.300 ms
    # playable p95, 32.180 ms tail -- and the healthy firefox seed 2002 pair of run 30231753972
    # produce composite ratios ABOVE the false red's 11.748, so a ceiling scaled by the clean
    # control cannot separate a slow runner from a healthy one at all. The composite is still
    # published, so this stays checkable.
    healthy_composite = browser_cpu_acceptance(
        recorded_browser_cpu_matrix(
            (
                (3.680, 17.940, 29.180),
                (2.660, 13.580, 33.760),
                (3.660, 18.300, 29.400),
            )
        ),
        "firefox",
    )
    if not healthy_composite["pass"]:
        raise RuntimeError(
            "a recorded healthy campaign failed the scaled ceiling: "
            f"{healthy_composite['reasons']}"
        )
    healthy_composite_ratio = next(
        pair["ratios"]["rollback_p999_over_clean_p95"]
        for pair in healthy_composite["pairs"]
        if pair["scenario"] == "complete_fixture" and pair["seed"] == 2002
    )
    if healthy_composite_ratio <= false_red_pair["ratios"]["rollback_p999_over_clean_p95"]:
        raise RuntimeError(
            "the clean control now separates the #230 false red from healthy runs; "
            "re-read the option-1 rejection in MAX_BROWSER_ROLLBACK_P999_MS"
        )

    def scale_measurements(
        measurements: tuple[tuple[float, float, float], ...],
        playable_scale: float = 1.0,
        tail_scale: float = 1.0,
    ) -> tuple[tuple[float, float, float], ...]:
        return tuple(
            (clean_p95, playable_p95 * playable_scale, playable_p999 * tail_scale)
            for clean_p95, playable_p95, playable_p999 in measurements
        )

    # A genuine tail regression, in the exact shape the ceiling exists to catch and the shape
    # the normalized ratio cannot see: seed 2001's tail inflated from 29.720 ms to 40.000 ms
    # with its p95 work left flat. The normalized ratio reads 2.542 against 2.6 and passes, the
    # work ratio is untouched, and the ceiling fails the pair on its own.
    tail_regression = browser_cpu_acceptance(
        recorded_browser_cpu_matrix(
            ((3.140, 15.735, 40.000), *FALSE_RED_CHROME[1:]),
        ),
        "firefox",
    )
    if tail_regression["pass"] or not any(
        "complete_fixture seed 2001 playable_rollback_p999_ms=40.000000 "
        "does not meet <39.700000" in reason
        for reason in tail_regression["reasons"]
    ):
        raise RuntimeError(
            f"a flat-work tail regression escaped the ceiling: {tail_regression['reasons']}"
        )
    if any(
        "rollback_p999_over_playable_p95" in reason or "p95_work_ratio" in reason
        for reason in tail_regression["reasons"]
    ):
        raise RuntimeError(
            "another gate claimed credit for catching a flat-work tail regression"
        )
    # Deferring the ceiling to the aggregate delays the verdict; it does not soften it. The same
    # regression scored inside its own shard job passes, because that job cannot know how fast
    # its runner was, and the campaign still goes red when the rollback gate merges the shards.
    tail_regression_runs = recorded_browser_cpu_matrix(
        ((3.140, 15.735, 40.000), *FALSE_RED_CHROME[1:]),
    )
    tail_regression_shard = browser_cpu_acceptance(
        tail_regression_runs[:2],
        "firefox",
        (2001,),
    )
    if not tail_regression_shard["pass"]:
        raise RuntimeError(
            "a shard job enforced a ceiling it had no runner reference for: "
            f"{tail_regression_shard['reasons']}"
        )

    # A uniform proportional slowdown of the playable build, stated plainly. Every shard moves
    # together, so every peer median moves with it and every runner scale is bit-identical to
    # the healthy campaign's: the correction absorbs runner noise and nothing else, and the
    # ceiling keeps the whole of its sensitivity to this class. 1.10x passes and 1.15x fails,
    # which is where the fixed ceiling sat too, and the normalized ratio never moves either way.
    def runner_scales(acceptance: dict[str, Any]) -> list[float]:
        return [
            pair["rollback_p999_gate"]["absolute_ceiling"]["runner_scale"]
            for pair in acceptance["pairs"]
            if pair["scenario"] == "complete_fixture"
        ]

    for uniform_scale, expected_pass in ((1.10, True), (1.15, False)):
        uniform = browser_cpu_acceptance(
            recorded_browser_cpu_matrix(
                scale_measurements(
                    FALSE_RED_CHROME,
                    playable_scale=uniform_scale,
                    tail_scale=uniform_scale,
                )
            ),
            "firefox",
        )
        if uniform["pass"] is not expected_pass:
            raise RuntimeError(
                f"a {uniform_scale}x uniform playable slowdown was not {expected_pass}: "
                f"{uniform['reasons']}"
            )
        if runner_scales(uniform) != runner_scales(false_red):
            raise RuntimeError(
                "a uniform playable slowdown moved the runner scale, so the correction is "
                "absorbing regressions and not just runner noise"
            )
        if normalized_ratios(uniform) != normalized_ratios(false_red):
            raise RuntimeError("a uniform playable slowdown moved the normalized ratio")
        if not expected_pass and not any(
            "complete_fixture seed 2003 playable_rollback_p999_ms" in reason
            for reason in uniform["reasons"]
        ):
            raise RuntimeError(
                f"the ceiling did not backstop a {uniform_scale}x uniform slowdown: "
                f"{uniform['reasons']}"
            )

    # rollback_ci.py restates the shard sets instead of importing them to keep the
    # impact-filter job's import graph minimal: that job deliberately installs no
    # browser evidence dependencies. Selenium would in fact load today, because
    # browser_determinism defers its selenium imports into function bodies, but
    # relying on that couples the scope job to an incidental laziness one edit could
    # remove. Since #182 the direction is fixed anyway: this module imports
    # rollback_ci for attribution_from_environment, so importing back would be a
    # circular import that fails on a partially initialized module. Assert the two
    # agree from this side, where both imports are available.
    import rollback_ci

    if (
        rollback_ci.NETWORK_SEED_SHARDS != SEED_SHARDS
        or rollback_ci.NATIVE_SHARDS != NATIVE_SHARDS
        or rollback_ci.BROWSER_RUNTIMES != BROWSER_RUNTIMES
    ):
        raise RuntimeError("rollback_ci.py shard sets drifted from the pinned campaign")

    shard_identities = expected_shard_evidence()
    expected_shard_names = sorted(shard_identities)
    # Per browser: one artifact per matrix seed shard, plus the whole soak and the
    # short stress job.
    if len(expected_shard_names) != len(NATIVE_SHARDS) + len(BROWSER_RUNTIMES) * (
        len(BROWSER_MATRIX_SHARDS) + 2
    ):
        raise RuntimeError("pinned rollback shard set changed size")
    if rollback_ci.EXPECTED_ARTIFACTS != frozenset(
        {*expected_shard_names, rollback_ci.AGGREGATE_ARTIFACT}
    ):
        raise RuntimeError(
            "the reuse contract's artifact set drifted from the pinned shard manifest"
        )
    if require_complete_shards(expected_shard_names) != expected_shard_names:
        raise RuntimeError("the complete rollback shard set was rejected")
    for dropped in expected_shard_names:
        try:
            require_complete_shards(
                [name for name in expected_shard_names if name != dropped]
            )
        except RuntimeError as error:
            if f"missing rollback shard evidence: {dropped}" not in str(error):
                raise RuntimeError(
                    f"vanished rollback shard {dropped} was not named by the gate"
                ) from error
        else:
            raise RuntimeError(f"vanished rollback shard {dropped} passed the gate")
    for hostile_names, fragment in (
        ([*expected_shard_names, "omp2-rollback-native-2004"], "unpinned"),
        ([*expected_shard_names, expected_shard_names[0]], "duplicate"),
        ([], "missing"),
    ):
        try:
            require_complete_shards(hostile_names)
        except RuntimeError as error:
            if fragment not in str(error):
                raise RuntimeError(
                    f"{fragment} rollback shard reason self-test failed"
                ) from error
        else:
            raise RuntimeError(f"{fragment} rollback shard evidence passed the gate")

    with tempfile.TemporaryDirectory(prefix="omp2-rollback-shard-") as directory:
        shard_root = Path(directory)
        shard_name = "omp2-rollback-chrome-matrix-2001"
        shard_identity = shard_identities[shard_name]
        shard_revision = "a" * 40
        shard_payload: dict[str, Any] = {
            "browser": {
                "campaign": "matrix",
                "runtimes": {"chrome": {"runs": []}},
                "shard": "2001",
            },
            "campaign": "matrix",
            "gate_contract": int(GATE_CONTRACT),
            "mode": "browser",
            "pass": True,
            "schema": 1,
            "shard": "2001",
            "source": {"dirty": False, "revision": shard_revision},
        }
        shard_path = shard_root / shard_name / f"{shard_name}.json"
        write_json(shard_path, shard_payload)
        load_shard_evidence(shard_root, shard_name, shard_identity, shard_revision)

        def expect_shard_rejection(
            mutated: dict[str, Any],
            fragment: str,
            label: str,
        ) -> None:
            write_json(shard_path, mutated)
            try:
                load_shard_evidence(
                    shard_root,
                    shard_name,
                    shard_identity,
                    shard_revision,
                )
            except RuntimeError as error:
                if fragment not in str(error):
                    raise RuntimeError(
                        f"{label} rollback shard reason self-test failed"
                    ) from error
            else:
                raise RuntimeError(f"{label} rollback shard evidence passed the gate")

        browser_section = shard_payload["browser"]
        for label, mutated_payload, fragment in (
            ("failed", {**shard_payload, "pass": False}, "did not pass"),
            (
                "stale contract",
                {**shard_payload, "gate_contract": int(GATE_CONTRACT) - 1},
                "another gate contract",
            ),
            (
                "cross-revision",
                {
                    **shard_payload,
                    "source": {"dirty": False, "revision": "b" * 40},
                },
                "was produced at revision",
            ),
            (
                "dirty",
                {
                    **shard_payload,
                    "source": {"dirty": True, "revision": shard_revision},
                },
                "dirty checkout",
            ),
            (
                "cross-campaign",
                {**shard_payload, "campaign": "soak"},
                "reports campaign=",
            ),
            (
                "cross-seed",
                {**shard_payload, "browser": {**browser_section, "shard": "2002"}},
                "reports shard=",
            ),
            (
                "cross-runtime",
                {
                    **shard_payload,
                    "browser": {
                        **browser_section,
                        "runtimes": {"firefox": {"runs": []}},
                    },
                },
                "did not record exactly the chrome runtime",
            ),
        ):
            expect_shard_rejection(mutated_payload, fragment, label)
        shard_path.unlink()
        try:
            load_shard_evidence(
                shard_root,
                shard_name,
                shard_identity,
                shard_revision,
            )
        except RuntimeError as error:
            if "uploaded no evidence" not in str(error):
                raise RuntimeError(
                    "skipped rollback shard reason self-test failed"
                ) from error
        else:
            raise RuntimeError("skipped rollback shard evidence passed the gate")
        try:
            aggregate_shards({"source": {"revision": shard_revision}}, shard_root)
        except RuntimeError as error:
            if "missing rollback shard evidence" not in str(error):
                raise RuntimeError("aggregate shard gate self-test failed") from error
        else:
            raise RuntimeError("incomplete rollback shard evidence passed the aggregate")

    def native_case_marker(case: dict[str, str]) -> str:
        combat = case["scenario"] == "combat"
        profile = case["profile"]
        mode = cpu_gate_mode("native", profile, False)
        gate = "1" if mode == "absolute" else "not_applied"
        applied = "1" if mode == "absolute" else "0"
        digest = "0000000000000004" if combat else "0000000000000003"
        # These two must track validate_case_integrity's expected_tape_version and
        # expected_snapshot_version, which a gameplay change that alters the
        # snapshot format bumps. If this fixture starts reporting the wrong
        # version, update it there and here together.
        tape_version = "2" if combat else "1"
        snapshot_version = "12" if combat else "11"
        return (
            f"{MARKER_PREFIX}|case|schema=1|case={case['case']}|"
            f"scenario={case['scenario']}|profile={profile}|"
            f"network_seed={case['network_seed']}|success=1|lab_success=1|"
            "expected_failure=0|status=converged|late_tick=none|hidden_progress=0|"
            f"scenario_pass=1|tape_version={tape_version}|"
            f"snapshot_version={snapshot_version}|"
            f"tape_digest={'1111111111111111' if combat else HISTORICAL_SOCCER_TAPE_DIGEST}|"
            "initial_hash=0000000000000001|reference_hash=0000000000000002|"
            "client_hash=0000000000000002|rollbacks=8|max_depth=8|"
            f"resimulated={0 if profile == 'clean' else 20}|peak_snapshots=31|"
            "peak_snapshot_bytes=611274|peak_history_bytes=700000|"
            f"event_reference_digest={digest}|event_confirmed_digest={digest}|"
            f"event_confirmed_combat={14 if combat else 0}|event_residue=0|"
            f"sample=none|gate_contract={GATE_CONTRACT}|cpu_gate={gate}|"
            f"cpu_gate_applied={applied}|cpu_gate_mode={mode}|snapshot_gate=1|"
            "history_gate=1|game_gate=1"
        )

    def publish_complete_shard_tree(root: Path, revision: str) -> None:
        """Fabricate one passing artifact for every pinned shard."""

        def envelope(
            mode: str,
            campaign: str,
            shard: str | None,
            section: dict[str, Any],
        ) -> dict[str, Any]:
            return {
                "campaign": campaign,
                "gate_contract": int(GATE_CONTRACT),
                mode: section,
                "mode": mode,
                "pass": True,
                "schema": 1,
                "shard": shard,
                "source": {"dirty": False, "revision": revision},
            }

        def publish(name: str, payload: dict[str, Any]) -> None:
            write_json(root / name / f"{name}.json", payload)

        for seed_shard in SEED_SHARDS:
            seed_markers = [
                native_case_marker(case)
                for case in expected_case_plan("native", (seed_shard,))
            ]
            publish(
                f"omp2-rollback-native-{seed_shard}",
                envelope(
                    "native",
                    "all",
                    seed_shard,
                    {
                        "fresh_runs": [
                            {"markers": seed_markers},
                            {"markers": seed_markers},
                        ],
                        "fresh_runs_agree": True,
                        "shard": seed_shard,
                    },
                ),
            )
        publish(
            f"omp2-rollback-native-{TAIL_SHARD}",
            envelope(
                "native",
                "all",
                TAIL_SHARD,
                {
                    "late_window": {"case_count": 2, "suite": "late-window"},
                    "shard": TAIL_SHARD,
                    "soak": {"soak_memory": {"pass": True}},
                },
            ),
        )
        for runtime_name in BROWSER_RUNTIMES:
            # Same healthy shape as synthetic_browser_cpu_matrix, including the
            # observed browser combat sample count: combat sits below the p99.9
            # sample floor in reality, and contract 7 fails closed for any
            # uncalibrated scenario that clears it.
            for seed_index, seed_shard in enumerate(BROWSER_MATRIX_SHARDS):
                seed_value = int(seed_shard)
                machine_p95 = 2.0 + seed_index * 0.25
                combat_machine_p95 = machine_p95 * 1.2
                playable_p95 = machine_p95 * HEALTHY_WORK_RATIO
                combat_playable_p95 = combat_machine_p95 * HEALTHY_COMBAT_WORK_RATIO
                seed_runs = [
                    synthetic_browser_cpu_run(
                        "clean",
                        seed_value,
                        machine_p95,
                        0.0,
                        combat_p95_work_ms=combat_machine_p95,
                        combat_rollback_p999_ms=0.0,
                        combat_rollback_samples=OBSERVED_COMBAT_ROLLBACK_SAMPLES,
                        browser_name=runtime_name,
                    ),
                    synthetic_browser_cpu_run(
                        "playable",
                        seed_value,
                        playable_p95,
                        playable_p95 * HEALTHY_TAIL_RATIO,
                        combat_p95_work_ms=combat_playable_p95,
                        combat_rollback_p999_ms=(
                            combat_playable_p95 * HEALTHY_COMBAT_TAIL_RATIO
                        ),
                        combat_rollback_samples=OBSERVED_COMBAT_ROLLBACK_SAMPLES,
                        browser_name=runtime_name,
                    ),
                ]
                publish(
                    f"omp2-rollback-{runtime_name}-matrix-{seed_shard}",
                    envelope(
                        "browser",
                        "matrix",
                        seed_shard,
                        {
                            "runtimes": {
                                runtime_name: {
                                    "cpu_acceptance": browser_cpu_acceptance(
                                        seed_runs,
                                        runtime_name,
                                        (seed_value,),
                                    ),
                                    "runs": seed_runs,
                                }
                            },
                            "shard": seed_shard,
                        },
                    ),
                )
            publish(
                f"omp2-rollback-{runtime_name}-soak",
                envelope(
                    "browser",
                    "soak",
                    None,
                    {
                        "runtimes": {
                            runtime_name: {
                                "runs": [
                                    {"soak_memory": {"pass": True}, "suite": "soak"}
                                ]
                            }
                        },
                        "shard": None,
                    },
                ),
            )
            publish(
                f"omp2-rollback-{runtime_name}-stress",
                envelope(
                    "browser",
                    "stress",
                    None,
                    {
                        "runtimes": {
                            runtime_name: {
                                "runs": [
                                    {
                                        "arguments": [STRESS_PROFILE, stress_shard],
                                        "suite": "browser-stress",
                                    }
                                    for stress_shard in SEED_SHARDS
                                ]
                            }
                        },
                        "shard": None,
                    },
                ),
            )

    with tempfile.TemporaryDirectory(prefix="omp2-rollback-aggregate-") as directory:
        aggregate_root = Path(directory)
        aggregate_revision = "c" * 40
        publish_complete_shard_tree(aggregate_root, aggregate_revision)
        aggregate_evidence: dict[str, Any] = {
            "source": {"revision": aggregate_revision}
        }
        aggregate_shards(aggregate_evidence, aggregate_root)
        aggregate = aggregate_evidence["aggregate"]
        pinned_native_cases = len(expected_case_plan("native", ()))
        if aggregate["native"]["case_count"] != pinned_native_cases:
            raise RuntimeError(
                "the rollback aggregate did not reassemble the pinned native plan"
            )
        if aggregate["native"]["cases_per_shard"] != {
            seed_shard: len(expected_case_plan("native", (seed_shard,)))
            for seed_shard in SEED_SHARDS
        }:
            raise RuntimeError("the rollback aggregate mis-split its native seed shards")
        if aggregate["native"]["soak_memory"]["pass"] is not True:
            raise RuntimeError("the rollback aggregate lost the native soak gate")
        if len(aggregate["shards"]) != len(expected_shard_evidence()):
            raise RuntimeError("the rollback aggregate omitted a shard digest")
        expected_pairs = len(BROWSER_CPU_SCENARIOS) * len(NETWORK_SEEDS)
        for runtime_name in BROWSER_RUNTIMES:
            merged = aggregate["browser"]["runtimes"][runtime_name]
            merged_acceptance = merged["cpu_acceptance"]
            if (
                merged_acceptance["pass"] is not True
                or merged_acceptance["scope"] != "aggregate"
                or merged_acceptance["seeds"] != list(NETWORK_SEEDS)
                or len(merged_acceptance["pairs"]) != expected_pairs
            ):
                raise RuntimeError(
                    f"{runtime_name} aggregate acceptance did not evaluate all "
                    f"{expected_pairs} pairs across the seed shards"
                )
            if merged["soak_memory"]["pass"] is not True:
                raise RuntimeError(f"{runtime_name} aggregate lost its soak memory gate")
        vanished = f"omp2-rollback-{BROWSER_RUNTIMES[0]}-matrix-{SEED_SHARDS[-1]}"
        shutil.rmtree(aggregate_root / vanished)
        try:
            aggregate_shards(
                {"source": {"revision": aggregate_revision}},
                aggregate_root,
            )
        except RuntimeError as error:
            if f"missing rollback shard evidence: {vanished}" not in str(error):
                raise RuntimeError(
                    "the rollback aggregate did not name its vanished shard"
                ) from error
        else:
            raise RuntimeError("a vanished shard passed the complete rollback aggregate")

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
        f"gate_contract={GATE_CONTRACT}|cpu_gate=1|cpu_gate_applied=1|"
        "cpu_gate_mode=absolute|"
        "snapshot_gate=1|"
        "history_gate=1|game_gate=1|rollbacks=6903|"
        "tape_version=1|snapshot_version=11|"
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
        integrity_case.raw.replace(
            f"gate_contract={GATE_CONTRACT}",
            "gate_contract=4",
        ),
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
                    f"gate_contract={GATE_CONTRACT}",
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
        "tape_versions=1,2|snapshot_versions=11,12|tick_rate=60"
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
        passing_timing.raw.replace(f"gate_contract={GATE_CONTRACT}", "gate_contract=4"),
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
        runtime_provenance.raw.replace(f"gate_contract={GATE_CONTRACT}", "gate_contract=4")
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
        or transient_peak["measurement"] != "terminal_window_vs_baseline_window"
        or transient_peak["window"] != 1
        or transient_peak["peak_growth_percent"] != 20.0
        or transient_peak["terminal_bytes"] != 1090
    ):
        raise RuntimeError("transient memory peak self-test failed")
    # Both series below are the same healthy build, recorded by the native tail shard in
    # runs 30192643196 and 30196601837. Their mean heaps differ by 0.04% (21.831 MB vs
    # 21.823 MB) yet the single-sample rule scored them 0.000% and 11.572%, failing the
    # second. Both must pass, or the gate is still reading sample ordering.
    healthy_low_warmup = growth_gate(
        {
            "warmup": 20111247,
            "120": 22777187,
            "360": 21653003,
            "600": 22135047,
            "final": 22438539,
        },
        "recorded-healthy-low-warmup",
    )
    healthy_high_warmup = growth_gate(
        {
            "warmup": 22506091,
            "120": 21978147,
            "360": 20091631,
            "600": 22731639,
            "final": 21849187,
        },
        "recorded-healthy-high-warmup",
    )
    for recorded in (healthy_low_warmup, healthy_high_warmup):
        if not recorded["pass"] or recorded["window"] != SOAK_GROWTH_WINDOW:
            raise RuntimeError(
                f"recorded healthy soak series failed: {recorded['label']} "
                f"scored {recorded['growth_percent']}%"
            )
    if healthy_low_warmup["growth_percent"] >= MAX_MEMORY_GROWTH_RATIO * 100:
        raise RuntimeError("recorded healthy series is not comfortably inside the limit")
    # A sustained leak must still be rejected. Averaging the ends costs sensitivity, so
    # pin where detection now begins rather than letting it drift silently.
    leaking = growth_gate(
        {
            "warmup": 20_000_000,
            "120": 21_000_000,
            "360": 22_000_000,
            "600": 23_000_000,
            "final": 24_000_000,
        },
        "sustained-leak",
    )
    if leaking["pass"]:
        raise RuntimeError("a sustained 20% memory leak passed the growth gate")
    # Pin the detection floor explicitly so the sensitivity cost of averaging the ends
    # cannot drift unnoticed. A 15% sustained leak is caught; a 10% one is not, because
    # a 13% noise band cannot resolve a 10% signal. Tightening this needs more
    # checkpoints, not a lower limit.
    caught = growth_gate(
        {
            "warmup": 20_000_000,
            "120": 20_750_000,
            "360": 21_500_000,
            "600": 22_250_000,
            "final": 23_000_000,
        },
        "sustained-15pct",
    )
    if caught["pass"]:
        raise RuntimeError("a sustained 15% memory leak passed the growth gate")
    undetected = growth_gate(
        {
            "warmup": 20_000_000,
            "120": 20_500_000,
            "360": 21_000_000,
            "600": 21_500_000,
            "final": 22_000_000,
        },
        "sustained-10pct",
    )
    if not undetected["pass"]:
        raise RuntimeError(
            "the pinned detection floor moved: a sustained 10% leak is now caught, "
            "so this assertion and the documented sensitivity are stale"
        )
    try:
        growth_gate({}, "empty")
    except RuntimeError:
        pass
    else:
        raise RuntimeError("empty checkpoint series passed the growth gate")
    try:
        growth_gate({"warmup": 0, "120": 0, "360": 1, "600": 2, "final": 3}, "zero")
    except RuntimeError:
        pass
    else:
        raise RuntimeError("zero baseline passed the growth gate")
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
        choices=("native", "browser", "full", "aggregate"),
        default="native",
    )
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--campaign", choices=CAMPAIGNS, default="all")
    parser.add_argument(
        "--shard",
        choices=NATIVE_SHARDS,
        help=(
            "restrict this run to one pinned network seed, or to the "
            "seed-independent native tail (late-window pair plus persistent soak)"
        ),
    )
    parser.add_argument("--evidence-root", type=Path)
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
    if args.campaign in BROWSER_ONLY_CAMPAIGNS and args.mode != "browser":
        raise SystemExit(f"--campaign {args.campaign} requires --mode browser")
    if args.mode == "aggregate" and args.evidence_root is None:
        raise SystemExit("--evidence-root is required for aggregate mode")
    if args.mode == "aggregate" and args.shard is not None:
        raise SystemExit("the rollback aggregate spans every shard and cannot be sharded")
    if args.mode == "full" and args.shard is not None:
        raise SystemExit("--shard selects a single-runtime campaign, not full mode")

    output = (args.output or default_output()).resolve()
    raw_root = output.parent / (output.stem + "-raw")
    source = source_provenance()
    evidence: dict[str, Any] = {
        "attribution": attribution_from_environment(os.environ),
        "generated_at": utc_now(),
        "campaign": args.campaign,
        "gate_contract": int(GATE_CONTRACT),
        "mode": args.mode,
        "pass": False,
        "schema": 1,
        "shard": args.shard,
        "source": source,
        "system": system_provenance(),
    }
    signal.signal(signal.SIGINT, raise_on_interruption)
    signal.signal(signal.SIGTERM, raise_on_interruption)
    try:
        if source["dirty"] and not args.allow_dirty:
            raise RuntimeError("rollback validation refuses a dirty source checkout")
        if args.mode != "aggregate":
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
                args.shard,
            )
        if args.mode in {"browser", "full"}:
            browser_matrix(
                evidence,
                args.artifact.resolve(),
                args.browsers or list(BROWSER_RUNTIMES),
                raw_root,
                args.timeout_seconds,
                args.allow_dirty,
                args.campaign,
                args.shard,
            )
        if args.mode == "aggregate":
            aggregate_shards(evidence, args.evidence_root)
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
