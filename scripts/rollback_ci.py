#!/usr/bin/env python3
"""Scope OMP-2 CI and discover complete reusable workflow evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Sequence, TypeVar


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_PATH = ".github/workflows/ci.yml"
GATE_CONTRACT = 5
FINGERPRINT_FORMAT = 2
MAX_ARTIFACT_BYTES = 10 * 1024 * 1024
HEX_DIGEST = re.compile(r"^[0-9a-f]{64}$")
ARTIFACT_DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
GIT_REVISION = re.compile(r"^[0-9a-f]{40}$")
RUN_ID = re.compile(r"^[1-9][0-9]*$")

DIRECT_RELEVANT_PATHS = (
    WORKFLOW_PATH,
    "conf.lua",
    "main.lua",
    "docs/online/evidence/omp2_rollback_linux_2026-07-24.json",
    "scripts/browser_determinism.py",
    "scripts/browser_matrix.py",
    "scripts/browser_matrix-requirements.txt",
    "scripts/browser_storage_host.js",
    "scripts/check_rollback.sh",
    "scripts/rollback_ci.py",
    "scripts/rollback_validation.py",
    "scripts/web_build.py",
    "scripts/web_build.sh",
    "scripts/web_report.py",
    "scripts/web_serve.py",
    "scripts/webrtc_proof_host.js",
    "scripts/webrtc_proof_runner.js",
    "scripts/webrtc_proof_suite.js",
)

LUA_VALIDATION_ROOTS = (
    "sim.rollback_validation",
    "game.rollback_validation",
)

LUA_MODULE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$")
REGULAR_FILE_MODES = frozenset(("100644", "100755"))

EXPECTED_JOB_STEPS = {
    "OMP-2 rollback impact filter": ("Detect rollback-impacting changes",),
    "OMP-2 rollback native": (
        "Run isolated native matrix, late-window, and persistent soak",
        "Upload native rollback evidence",
    ),
    "OMP-2 rollback browser matrix (chrome)": (
        "Run complete and stress browser validation",
        "Upload browser rollback evidence",
    ),
    "OMP-2 rollback browser matrix (firefox)": (
        "Run complete and stress browser validation",
        "Upload browser rollback evidence",
    ),
    "OMP-2 rollback browser soak (chrome)": (
        "Run persistent browser memory soak",
        "Upload browser rollback evidence",
    ),
    "OMP-2 rollback browser soak (firefox)": (
        "Run persistent browser memory soak",
        "Upload browser rollback evidence",
    ),
    "OMP-2 rollback gate": ("Require scoped rollback evidence",),
}

LONG_JOB_NAMES = frozenset(EXPECTED_JOB_STEPS) - {
    "OMP-2 rollback impact filter",
    "OMP-2 rollback gate",
}

EXPECTED_ARTIFACTS = frozenset(
    {
        "omp2-rollback-native",
        "omp2-rollback-chrome-matrix",
        "omp2-rollback-firefox-matrix",
        "omp2-rollback-chrome-soak",
        "omp2-rollback-firefox-soak",
    }
)


class DiscoveryError(RuntimeError):
    """The discovery mechanism itself could not establish a safe answer."""


class CandidateRejected(RuntimeError):
    """One prior workflow run is not complete reusable evidence."""


@dataclass(frozen=True)
class ScopeDecision:
    run: bool
    fingerprint: str
    reason: str
    changed_paths: tuple[str, ...]


@dataclass(frozen=True)
class DiscoveryContext:
    fingerprint: str
    repository: str
    repository_id: int
    pr_number: int
    head_branch: str
    head_repository_id: int
    base_repository_id: int
    base_ref: str
    base_sha: str
    current_run_id: str


@dataclass(frozen=True)
class GitTreeEntry:
    mode: str
    kind: str
    object_id: str


T = TypeVar("T")


def run_git(repo: Path, arguments: Sequence[str]) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *arguments],
        cwd=repo,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def require_git(repo: Path, arguments: Sequence[str]) -> bytes:
    completed = run_git(repo, arguments)
    if completed.returncode != 0:
        message = completed.stderr.decode("utf-8", errors="replace").strip()
        raise DiscoveryError(message or f"git {' '.join(arguments)} failed")
    return completed.stdout


def revision_tree(repo: Path, revision: str) -> dict[str, GitTreeEntry]:
    raw = require_git(repo, ["ls-tree", "-r", "-z", "--full-tree", revision])
    entries: dict[str, GitTreeEntry] = {}
    for raw_record in raw.split(b"\0"):
        if not raw_record:
            continue
        try:
            metadata, raw_path = raw_record.split(b"\t", 1)
            mode, kind, object_id = metadata.decode("ascii").split(" ")
            path = raw_path.decode("utf-8")
        except (UnicodeDecodeError, ValueError) as error:
            raise DiscoveryError(
                f"{revision} contains an unsafe Git tree record"
            ) from error
        if path in entries:
            raise DiscoveryError(f"{revision} contains duplicate path {path}")
        if kind not in ("blob", "commit") or not re.fullmatch(r"[0-7]{6}", mode):
            raise DiscoveryError(f"{revision}:{path} has an unsafe Git entry")
        if not re.fullmatch(r"[0-9a-f]{40,64}", object_id):
            raise DiscoveryError(f"{revision}:{path} has a malformed object id")
        entries[path] = GitTreeEntry(mode, kind, object_id)
    return entries


def revision_blob(repo: Path, revision: str, path: str) -> bytes:
    return require_git(repo, ["cat-file", "blob", f"{revision}:{path}"])


def require_regular_file(
    tree: dict[str, GitTreeEntry], path: str, revision: str
) -> GitTreeEntry:
    entry = tree.get(path)
    if entry is None:
        raise DiscoveryError(f"{revision} is missing required rollback path {path}")
    if entry.kind != "blob" or entry.mode not in REGULAR_FILE_MODES:
        raise DiscoveryError(
            f"{revision}:{path} is not a regular tracked rollback file"
        )
    return entry


def long_bracket_end(source: str, start: int) -> tuple[str, int] | None:
    if source[start] != "[":
        return None
    cursor = start + 1
    while cursor < len(source) and source[cursor] == "=":
        cursor += 1
    if cursor >= len(source) or source[cursor] != "[":
        return None
    return "]" + source[start + 1 : cursor] + "]", cursor + 1


def lua_tokens(source: bytes, path: str) -> list[tuple[str, str | None]]:
    try:
        text = source.decode("utf-8")
    except UnicodeDecodeError as error:
        raise DiscoveryError(f"{path} is not valid UTF-8 Lua source") from error

    tokens: list[tuple[str, str | None]] = []
    cursor = 0
    while cursor < len(text):
        character = text[cursor]
        if character.isspace():
            cursor += 1
            continue
        if text.startswith("--", cursor):
            long_comment = (
                long_bracket_end(text, cursor + 2)
                if cursor + 2 < len(text)
                else None
            )
            if long_comment is None:
                newline = text.find("\n", cursor + 2)
                cursor = len(text) if newline < 0 else newline + 1
                continue
            terminator, content_start = long_comment
            end = text.find(terminator, content_start)
            if end < 0:
                raise DiscoveryError(f"{path} has an unterminated long comment")
            cursor = end + len(terminator)
            continue
        if character in ("'", '"'):
            quote = character
            cursor += 1
            value: list[str] = []
            escaped = False
            while cursor < len(text):
                character = text[cursor]
                if character == quote:
                    cursor += 1
                    tokens.append(
                        ("escaped_string" if escaped else "string", "".join(value))
                    )
                    break
                if character in ("\n", "\r"):
                    raise DiscoveryError(f"{path} has an unterminated quoted string")
                if character == "\\":
                    escaped = True
                    cursor += 1
                    if cursor >= len(text):
                        raise DiscoveryError(f"{path} has an unterminated escape")
                    value.append(text[cursor])
                    cursor += 1
                    continue
                value.append(character)
                cursor += 1
            else:
                raise DiscoveryError(f"{path} has an unterminated quoted string")
            continue
        long_string = long_bracket_end(text, cursor)
        if long_string is not None:
            terminator, content_start = long_string
            end = text.find(terminator, content_start)
            if end < 0:
                raise DiscoveryError(f"{path} has an unterminated long string")
            tokens.append(("long_string", None))
            cursor = end + len(terminator)
            continue
        if character.isalpha() or character == "_":
            start = cursor
            cursor += 1
            while cursor < len(text) and (
                text[cursor].isalnum() or text[cursor] == "_"
            ):
                cursor += 1
            tokens.append(("identifier", text[start:cursor]))
            continue
        tokens.append(("symbol", character))
        cursor += 1
    return tokens


def static_lua_requires(source: bytes, path: str) -> tuple[str, ...]:
    tokens = lua_tokens(source, path)
    required: list[str] = []
    for index, token in enumerate(tokens):
        if token != ("identifier", "require"):
            continue
        if index > 0 and tokens[index - 1] in (
            ("symbol", "."),
            ("symbol", ":"),
        ):
            raise DiscoveryError(
                f"{path} uses member-style require syntax; "
                "cannot prove rollback reachability"
            )
        argument = index + 1
        parenthesized = argument < len(tokens) and tokens[argument] == (
            "symbol",
            "(",
        )
        if parenthesized:
            argument += 1
        if argument >= len(tokens) or tokens[argument][0] != "string":
            raise DiscoveryError(
                f"{path} uses a non-literal or escaped require; "
                "cannot prove rollback reachability"
            )
        module = tokens[argument][1]
        if module is None or not LUA_MODULE.fullmatch(module):
            raise DiscoveryError(f"{path} requires unsafe module name {module!r}")
        if parenthesized:
            close = argument + 1
            if close >= len(tokens) or tokens[close] != ("symbol", ")"):
                raise DiscoveryError(
                    f"{path} has a require call that cannot be parsed safely"
                )
        required.append(module)
    return tuple(required)


def resolve_lua_module(
    tree: dict[str, GitTreeEntry], module: str, revision: str
) -> str:
    prefix = module.replace(".", "/")
    candidates = tuple(
        path for path in (f"{prefix}.lua", f"{prefix}/init.lua") if path in tree
    )
    if len(candidates) != 1:
        detail = "missing" if not candidates else "ambiguous"
        raise DiscoveryError(
            f"{revision} has {detail} reachable Lua module {module!r}"
        )
    path = candidates[0]
    require_regular_file(tree, path, revision)
    return path


def reachable_lua_paths(
    repo: Path, revision: str, tree: dict[str, GitTreeEntry]
) -> frozenset[str]:
    pending = list(LUA_VALIDATION_ROOTS)
    visited_modules: set[str] = set()
    visited_paths: set[str] = set()
    while pending:
        module = pending.pop()
        if module in visited_modules:
            continue
        visited_modules.add(module)
        path = resolve_lua_module(tree, module, revision)
        if path in visited_paths:
            raise DiscoveryError(
                f"{revision} maps multiple module names to reachable path {path}"
            )
        visited_paths.add(path)
        pending.extend(static_lua_requires(revision_blob(repo, revision, path), path))
    return frozenset(visited_paths)


def relevant_manifest(
    repo: Path, revision: str = "HEAD"
) -> dict[str, GitTreeEntry]:
    tree = revision_tree(repo, revision)
    selected: set[str] = set()
    for path in DIRECT_RELEVANT_PATHS:
        if path not in tree:
            raise DiscoveryError(f"{revision} is missing required rollback root {path}")
        require_regular_file(tree, path, revision)
        selected.add(path)
    selected.update(reachable_lua_paths(repo, revision, tree))
    return {path: tree[path] for path in sorted(selected)}


def fingerprint_manifest(manifest: dict[str, GitTreeEntry]) -> str:
    digest = hashlib.sha256()
    digest.update(
        (
            f"galactic-cup/omp2-relevant-content/v{FINGERPRINT_FORMAT}/"
            f"contract-{GATE_CONTRACT}\0"
        ).encode()
    )
    for path in DIRECT_RELEVANT_PATHS:
        digest.update(b"direct\0")
        digest.update(path.encode("utf-8"))
        digest.update(b"\0")
    for module in LUA_VALIDATION_ROOTS:
        digest.update(b"lua-root\0")
        digest.update(module.encode("utf-8"))
        digest.update(b"\0")
    for path, entry in manifest.items():
        digest.update(entry.mode.encode("ascii"))
        digest.update(b" ")
        digest.update(entry.kind.encode("ascii"))
        digest.update(b" ")
        digest.update(entry.object_id.encode("ascii"))
        digest.update(b"\t")
        digest.update(path.encode("utf-8"))
        digest.update(b"\0")
    return digest.hexdigest()


def relevant_fingerprint(repo: Path, revision: str = "HEAD") -> str:
    return fingerprint_manifest(relevant_manifest(repo, revision))


def valid_comparison_base(repo: Path, revision: str) -> bool:
    if not revision or set(revision) == {"0"}:
        return False
    return run_git(repo, ["cat-file", "-e", f"{revision}^{{commit}}"]).returncode == 0


def changed_manifest_paths(
    before: dict[str, GitTreeEntry], after: dict[str, GitTreeEntry]
) -> tuple[str, ...]:
    return tuple(
        path
        for path in sorted(set(before) | set(after))
        if before.get(path) != after.get(path)
    )


def decide_scope(
    repo: Path,
    event_name: str,
    event_before: str = "",
    pr_base_sha: str = "",
) -> ScopeDecision:
    fingerprint = "unavailable"
    fingerprint_error = ""
    current_manifest: dict[str, GitTreeEntry] | None = None
    try:
        current_manifest = relevant_manifest(repo)
        fingerprint = fingerprint_manifest(current_manifest)
    except DiscoveryError as error:
        fingerprint_error = str(error)

    if event_name == "workflow_dispatch":
        return ScopeDecision(
            True,
            fingerprint,
            "manual dispatch always runs the complete campaign",
            (),
        )
    if event_name == "pull_request":
        base = pr_base_sha
        separator = "..."
    elif event_name == "push":
        base = event_before
        separator = ".."
    else:
        return ScopeDecision(
            True,
            fingerprint,
            f"unsupported event {event_name!r}; fail-open full campaign",
            (),
        )
    if not valid_comparison_base(repo, base):
        return ScopeDecision(
            True,
            fingerprint,
            "comparison commit unavailable; fail-open full campaign",
            (),
        )

    revision_range = f"{base}{separator}HEAD"
    if fingerprint_error or current_manifest is None:
        return ScopeDecision(
            True,
            fingerprint,
            f"fingerprint failed ({fingerprint_error}); fail-open full campaign",
            (),
        )
    try:
        base_manifest = relevant_manifest(repo, base)
        base_fingerprint = fingerprint_manifest(base_manifest)
    except DiscoveryError as error:
        return ScopeDecision(
            True,
            fingerprint,
            f"base fingerprint failed ({error}); fail-open full campaign",
            (),
        )
    if base_fingerprint == fingerprint:
        return ScopeDecision(
            False,
            fingerprint,
            f"rollback-reachable content is unchanged in {revision_range}",
            (),
        )
    changed_paths = changed_manifest_paths(base_manifest, current_manifest)
    return ScopeDecision(
        True,
        fingerprint,
        f"rollback-reachable content changed in {revision_range}",
        changed_paths,
    )


def write_github_output(path: Path | None, values: dict[str, str]) -> None:
    if path is None:
        return
    with path.open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            if "\n" in key or "\n" in value:
                raise ValueError("GitHub outputs must be single-line values")
            handle.write(f"{key}={value}\n")


def duplicate_safe_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate field {key!r}")
        result[key] = value
    return result


def api_json(url: str, token: str) -> dict[str, Any]:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "User-Agent": "galactic-cup-rollback-ci",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read()
    except (OSError, urllib.error.HTTPError, urllib.error.URLError) as error:
        raise DiscoveryError(f"GitHub evidence request failed: {error}") from error
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=duplicate_safe_object)
    except Exception as error:
        raise DiscoveryError(f"GitHub evidence response is malformed: {error}") from error
    if not isinstance(value, dict):
        raise DiscoveryError("GitHub evidence response root is not an object")
    return value


def api_collection(
    api_url: str,
    repository: str,
    endpoint: str,
    field: str,
    token: str,
) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    page = 1
    while True:
        separator = "&" if "?" in endpoint else "?"
        payload = api_json(
            f"{api_url}/repos/{repository}/{endpoint}{separator}per_page=100&page={page}",
            token,
        )
        values = payload.get(field)
        total = payload.get("total_count")
        if not isinstance(values, list) or type(total) is not int or total < 0:
            raise DiscoveryError(f"GitHub {field} response has invalid pagination")
        if not all(isinstance(value, dict) for value in values):
            raise DiscoveryError(f"GitHub {field} response contains malformed records")
        records.extend(values)
        if len(records) >= total:
            if len(records) != total:
                raise DiscoveryError(f"GitHub {field} response count drifted")
            return records
        if not values or page >= 20:
            raise DiscoveryError(f"GitHub {field} response is incomplete")
        page += 1


def validate_context(context: DiscoveryContext) -> None:
    if not HEX_DIGEST.fullmatch(context.fingerprint):
        raise DiscoveryError("current fingerprint is malformed")
    if not context.repository or "/" not in context.repository:
        raise DiscoveryError("current repository is malformed")
    for name, value in (
        ("repository_id", context.repository_id),
        ("pr_number", context.pr_number),
        ("head_repository_id", context.head_repository_id),
        ("base_repository_id", context.base_repository_id),
    ):
        if type(value) is not int or value < 1:
            raise DiscoveryError(f"current {name} is malformed")
    if not context.head_branch or not context.base_ref:
        raise DiscoveryError("current branch identity is incomplete")
    if not GIT_REVISION.fullmatch(context.base_sha):
        raise DiscoveryError("current base revision is malformed")
    if not RUN_ID.fullmatch(context.current_run_id):
        raise DiscoveryError("current run id is malformed")


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise CandidateRejected(f"producer {label} is malformed")
    return value


def require_exact(value: Any, expected: Any, label: str) -> None:
    if value != expected or type(value) is not type(expected):
        raise CandidateRejected(f"producer has wrong {label}")


def validate_run_linkage(run: dict[str, Any], context: DiscoveryContext) -> str:
    run_id = run.get("id")
    if type(run_id) is not int or run_id < 1:
        raise CandidateRejected("producer run id is malformed")
    if str(run_id) == context.current_run_id:
        raise CandidateRejected("current run cannot reuse itself")
    revision = run.get("head_sha")
    if not isinstance(revision, str) or not GIT_REVISION.fullmatch(revision):
        raise CandidateRejected("producer revision is malformed")
    expected = {
        "event": "pull_request",
        "head_branch": context.head_branch,
        "path": WORKFLOW_PATH,
        "run_attempt": 1,
        "status": "completed",
        "conclusion": "success",
    }
    for key, value in expected.items():
        require_exact(run.get(key), value, key)

    repository = require_object(run.get("repository"), "repository")
    require_exact(repository.get("id"), context.repository_id, "repository id")
    require_exact(repository.get("full_name"), context.repository, "repository name")
    head_repository = require_object(run.get("head_repository"), "head repository")
    require_exact(
        head_repository.get("id"),
        context.head_repository_id,
        "head repository id",
    )

    pull_requests = run.get("pull_requests")
    if not isinstance(pull_requests, list):
        raise CandidateRejected("producer pull-request linkage is malformed")
    matches = []
    for pull_request in pull_requests:
        if not isinstance(pull_request, dict):
            continue
        if pull_request.get("number") == context.pr_number:
            matches.append(pull_request)
    if len(matches) != 1 or len(pull_requests) != 1:
        raise CandidateRejected("producer does not link exactly to the active pull request")
    pull_request = matches[0]
    base = require_object(pull_request.get("base"), "pull-request base")
    base_repo = require_object(base.get("repo"), "pull-request base repository")
    require_exact(base.get("ref"), context.base_ref, "pull-request base ref")
    require_exact(base.get("sha"), context.base_sha, "pull-request base revision")
    require_exact(
        base_repo.get("id"),
        context.base_repository_id,
        "pull-request base repository id",
    )
    head = require_object(pull_request.get("head"), "pull-request head")
    head_repo = require_object(head.get("repo"), "pull-request head repository")
    require_exact(head.get("ref"), context.head_branch, "pull-request head ref")
    require_exact(
        head_repo.get("id"),
        context.head_repository_id,
        "pull-request head repository id",
    )
    return revision


def unique_named_records(
    records: list[dict[str, Any]], expected_names: frozenset[str]
) -> dict[str, dict[str, Any]]:
    selected: dict[str, dict[str, Any]] = {}
    for record in records:
        name = record.get("name")
        if not isinstance(name, str) or name not in expected_names:
            continue
        if name in selected:
            raise CandidateRejected(f"producer evidence has duplicate {name}")
        selected[name] = record
    missing = expected_names - frozenset(selected)
    if missing:
        raise CandidateRejected(
            f"producer evidence is missing {', '.join(sorted(missing))}"
        )
    return selected


def validate_jobs(jobs: list[dict[str, Any]]) -> None:
    selected = unique_named_records(jobs, frozenset(EXPECTED_JOB_STEPS))
    for job_name, job in selected.items():
        if job.get("status") != "completed" or job.get("conclusion") != "success":
            raise CandidateRejected(f"producer job {job_name} did not succeed")
        labels = job.get("labels")
        if not isinstance(labels, list) or "ubuntu-24.04" not in labels:
            raise CandidateRejected(f"producer job {job_name} used the wrong platform")
        steps = job.get("steps")
        if not isinstance(steps, list) or not all(
            isinstance(step, dict) for step in steps
        ):
            raise CandidateRejected(f"producer job {job_name} has malformed steps")
        step_records = unique_named_records(
            steps, frozenset(EXPECTED_JOB_STEPS[job_name])
        )
        for step_name, step in step_records.items():
            if step.get("status") != "completed" or step.get("conclusion") != "success":
                raise CandidateRejected(
                    f"producer step {job_name} / {step_name} did not succeed"
                )


def parse_api_time(value: Any) -> datetime:
    if not isinstance(value, str):
        raise CandidateRejected("artifact expiration is malformed")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise CandidateRejected("artifact expiration is malformed") from error
    if parsed.tzinfo is None:
        raise CandidateRejected("artifact expiration has no timezone")
    return parsed


def validate_artifacts(
    artifacts: list[dict[str, Any]], now: datetime | None = None
) -> None:
    now = now or datetime.now(timezone.utc)
    selected = unique_named_records(artifacts, EXPECTED_ARTIFACTS)
    for name, artifact in selected.items():
        if artifact.get("expired") is not False:
            raise CandidateRejected(f"producer artifact {name} is expired")
        if parse_api_time(artifact.get("expires_at")) <= now:
            raise CandidateRejected(f"producer artifact {name} has passed its expiration")
        size = artifact.get("size_in_bytes")
        if type(size) is not int or not 0 < size <= MAX_ARTIFACT_BYTES:
            raise CandidateRejected(f"producer artifact {name} has invalid size")
        digest = artifact.get("digest")
        if not isinstance(digest, str) or not ARTIFACT_DIGEST.fullmatch(digest):
            raise CandidateRejected(f"producer artifact {name} has no SHA-256 digest")


def fetch_candidate_evidence(
    api_url: str,
    context: DiscoveryContext,
    run_id: int,
    token: str,
) -> tuple[dict[str, Any], list[dict[str, Any]], list[dict[str, Any]]]:
    run = api_json(
        f"{api_url}/repos/{context.repository}/actions/runs/{run_id}", token
    )
    jobs = api_collection(
        api_url,
        context.repository,
        f"actions/runs/{run_id}/attempts/1/jobs",
        "jobs",
        token,
    )
    artifacts = api_collection(
        api_url,
        context.repository,
        f"actions/runs/{run_id}/artifacts",
        "artifacts",
        token,
    )
    return run, jobs, artifacts


def select_candidate(
    candidates: list[dict[str, Any]],
    validator: Callable[[dict[str, Any]], T],
) -> T | None:
    for candidate in candidates:
        try:
            return validator(candidate)
        except CandidateRejected as error:
            run_id = candidate.get("id", "unknown")
            print(f"rejected prior run {run_id}: {error}")
    return None


def discover_reusable_run(
    repo: Path,
    api_url: str,
    context: DiscoveryContext,
    token: str,
) -> tuple[bool, str]:
    validate_context(context)
    branch = urllib.parse.quote(context.head_branch, safe="")
    workflow = urllib.parse.quote(Path(WORKFLOW_PATH).name, safe="")
    candidates = api_collection(
        api_url,
        context.repository,
        f"actions/workflows/{workflow}/runs?event=pull_request&status=success&branch={branch}",
        "workflow_runs",
        token,
    )

    def validate(candidate: dict[str, Any]) -> str:
        candidate_id = candidate.get("id")
        if type(candidate_id) is not int or candidate_id < 1:
            raise CandidateRejected("producer run id is malformed")
        run, jobs, artifacts = fetch_candidate_evidence(
            api_url, context, candidate_id, token
        )
        revision = validate_run_linkage(run, context)
        ancestor = run_git(repo, ["merge-base", "--is-ancestor", revision, "HEAD"])
        if ancestor.returncode == 1:
            raise CandidateRejected("producer revision is not an ancestor")
        if ancestor.returncode != 0:
            raise DiscoveryError("producer ancestry check failed")
        if relevant_fingerprint(repo, revision) != context.fingerprint:
            raise CandidateRejected("producer relevant-content fingerprint differs")
        validate_jobs(jobs)
        validate_artifacts(artifacts)
        return (
            f"reusing complete first-attempt rollback campaign from run "
            f"{candidate_id} at {revision}"
        )

    result = select_candidate(candidates, validate)
    if result is None:
        return False, "no complete prior workflow run is reusable; running all five shards"
    return True, result


def fail_open_discovery(operation: Callable[[], tuple[bool, str]]) -> tuple[bool, str]:
    try:
        return operation()
    except Exception as error:
        return False, f"evidence discovery failed open: {error}; running all five shards"


def git(repo: Path, *arguments: str) -> None:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=repo,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if completed.returncode != 0:
        raise AssertionError(completed.stderr.strip() or completed.stdout.strip())


def commit_fixture(repo: Path, message: str) -> str:
    git(repo, "add", ".")
    git(
        repo,
        "-c",
        "user.name=Rollback CI Self-Test",
        "-c",
        "user.email=rollback-ci@example.invalid",
        "commit",
        "-m",
        message,
    )
    return require_git(repo, ["rev-parse", "HEAD"]).decode().strip()


def write_fixture(repo: Path, relative: str, contents: str) -> None:
    path = repo / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents, encoding="utf-8")


def fixture_context(fingerprint: str, base_sha: str) -> DiscoveryContext:
    return DiscoveryContext(
        fingerprint,
        "osobytes/galactic-cup",
        1001,
        124,
        "codex/issue-123",
        1001,
        1001,
        "main",
        base_sha,
        "99999",
    )


def fixture_run(
    context: DiscoveryContext, run_id: int, revision: str
) -> dict[str, Any]:
    return {
        "id": run_id,
        "event": "pull_request",
        "head_branch": context.head_branch,
        "head_sha": revision,
        "path": WORKFLOW_PATH,
        "run_attempt": 1,
        "status": "completed",
        "conclusion": "success",
        "repository": {
            "id": context.repository_id,
            "full_name": context.repository,
        },
        "head_repository": {"id": context.head_repository_id},
        "pull_requests": [
            {
                "number": context.pr_number,
                "base": {
                    "ref": context.base_ref,
                    "sha": context.base_sha,
                    "repo": {"id": context.base_repository_id},
                },
                "head": {
                    "ref": context.head_branch,
                    "sha": revision,
                    "repo": {"id": context.head_repository_id},
                },
            }
        ],
    }


def fixture_jobs() -> list[dict[str, Any]]:
    return [
        {
            "name": name,
            "status": "completed",
            "conclusion": "success",
            "labels": ["ubuntu-24.04"],
            "steps": [
                {"name": step, "status": "completed", "conclusion": "success"}
                for step in steps
            ],
        }
        for name, steps in EXPECTED_JOB_STEPS.items()
    ]


def fixture_artifacts() -> list[dict[str, Any]]:
    return [
        {
            "name": name,
            "expired": False,
            "expires_at": "2099-01-01T00:00:00Z",
            "size_in_bytes": 1024,
            "digest": f"sha256:{hashlib.sha256(name.encode()).hexdigest()}",
        }
        for name in sorted(EXPECTED_ARTIFACTS)
    ]


def validate_workflow_wiring() -> None:
    workflow = (ROOT / WORKFLOW_PATH).read_text(encoding="utf-8")
    assert "actions/cache/" not in workflow
    scope = workflow.index("\n    rollback_scope:")
    discovery = workflow.index("scripts/rollback_ci.py discover", scope)
    decision = workflow.index("- name: Select full campaign or aggregate reuse", scope)
    assert scope < discovery < decision
    assert workflow.count("if: needs.rollback_scope.outputs.run == 'true'") == 3
    gate = workflow.index("\n    rollback_gate:")
    assert "pointer-write" not in workflow[gate:]


def initialize_scope_fixture(repo: Path) -> str:
    git(repo, "init", "-q")
    for path in DIRECT_RELEVANT_PATHS:
        if path.endswith(".lua"):
            contents = "return {}\n"
        elif path.endswith(".json"):
            contents = "{}\n"
        else:
            contents = f"fixture root: {path}\n"
        write_fixture(repo, path, contents)
    write_fixture(
        repo,
        "sim/rollback_validation.lua",
        'local runtime = require("sim.runtime")\nreturn runtime\n',
    )
    write_fixture(
        repo,
        "game/rollback_validation.lua",
        'local runtime = require("sim.runtime")\nreturn runtime\n',
    )
    write_fixture(repo, "sim/runtime.lua", "return { value = 1 }\n")
    write_fixture(repo, "docs/readme.md", "base\n")
    return commit_fixture(repo, "base")


def run_self_test() -> None:
    validate_workflow_wiring()
    parser_fixture = b"""
-- require("sim.line_comment")
--[=[
require("sim.long_comment")
]=]
local message = 'require("sim.quoted_string")'
local template = [==[require("sim.long_string")]==]
local runtime = require("sim.runtime")
return runtime, message, template
"""
    assert static_lua_requires(parser_fixture, "parser_fixture.lua") == ("sim.runtime",)
    for label, source in (
        ("dotted literal", b'_G.require("sim.runtime")\n'),
        ("colon nonliteral", b"loader:require(module_name)\n"),
    ):
        try:
            static_lua_requires(source, "unsafe_member_require.lua")
        except DiscoveryError as error:
            assert "member-style require syntax" in str(error)
        else:
            raise AssertionError(f"{label} require syntax was accepted")
    with tempfile.TemporaryDirectory(prefix="rollback-ci-self-test-") as directory:
        repo = Path(directory)
        base = initialize_scope_fixture(repo)
        base_fingerprint = relevant_fingerprint(repo, base)

        write_fixture(repo, "docs/readme.md", "unrelated\n")
        write_fixture(repo, "sim/brain.lua", "return { unrelated = true }\n")
        write_fixture(
            repo,
            "spec/sim/rollback_validation_spec.lua",
            'local brain = require("sim.brain")\nreturn brain\n',
        )
        commit_fixture(repo, "unreferenced module and spec")
        unrelated = decide_scope(repo, "pull_request", pr_base_sha=base)
        assert not unrelated.run and unrelated.fingerprint == base_fingerprint

        write_fixture(repo, "sim/runtime.lua", "return { value = 2 }\n")
        producer_revision = commit_fixture(repo, "relevant")
        relevant = decide_scope(repo, "pull_request", pr_base_sha=base)
        assert relevant.run and HEX_DIGEST.fullmatch(relevant.fingerprint)
        assert relevant.changed_paths == ("sim/runtime.lua",)

        write_fixture(repo, "docs/readme.md", "follow-up\n")
        follow_up_revision = commit_fixture(repo, "docs follow-up")
        follow_up = decide_scope(repo, "pull_request", pr_base_sha=base)
        assert follow_up.run and follow_up.fingerprint == relevant.fingerprint

        write_fixture(
            repo,
            "sim/rollback_validation.lua",
            'local runtime = require("sim.runtime")\n'
            'local extension = require("sim.rollback_extension")\n'
            "return { runtime = runtime, extension = extension }\n",
        )
        write_fixture(repo, "sim/rollback_extension.lua", "return { enabled = true }\n")
        wired_revision = commit_fixture(repo, "wire new rollback dependency")
        wired = decide_scope(repo, "pull_request", pr_base_sha=follow_up_revision)
        assert wired.run and wired.fingerprint != relevant.fingerprint
        assert wired.changed_paths == (
            "sim/rollback_extension.lua",
            "sim/rollback_validation.lua",
        )

        write_fixture(
            repo, "sim/rollback_extension.lua", "return { enabled = false }\n"
        )
        commit_fixture(repo, "change new rollback dependency")
        changed = decide_scope(repo, "pull_request", pr_base_sha=wired_revision)
        assert changed.run and changed.changed_paths == ("sim/rollback_extension.lua",)
        assert decide_scope(repo, "workflow_dispatch").run
        assert decide_scope(repo, "push", event_before=base).run
        assert decide_scope(repo, "pull_request", pr_base_sha="0" * 40).run

        context = fixture_context(relevant.fingerprint, base)
        fresh_run = fixture_run(context, 100, producer_revision)
        validate_run_linkage(fresh_run, context)
        mutable_linkage = json.loads(json.dumps(fresh_run))
        mutable_linkage["pull_requests"][0]["head"]["sha"] = follow_up_revision
        assert validate_run_linkage(mutable_linkage, context) == producer_revision
        jobs = fixture_jobs()
        validate_jobs(jobs)
        validate_artifacts(fixture_artifacts())

        for mutation in (
            ("cross PR", lambda run: run["pull_requests"][0].update(number=999)),
            ("cross base", lambda run: run["pull_requests"][0]["base"].update(sha="1" * 40)),
            ("cross fork", lambda run: run["head_repository"].update(id=2002)),
        ):
            hostile = json.loads(json.dumps(fresh_run))
            mutation[1](hostile)
            try:
                validate_run_linkage(hostile, context)
            except CandidateRejected:
                pass
            else:
                raise AssertionError(f"{mutation[0]} producer was accepted")

        for conclusion in ("failure", "cancelled"):
            rejected = dict(fresh_run, conclusion=conclusion)
            try:
                validate_run_linkage(rejected, context)
            except CandidateRejected:
                pass
            else:
                raise AssertionError(f"{conclusion} producer was accepted")
        bool_attempt = dict(fresh_run, run_attempt=True)
        try:
            validate_run_linkage(bool_attempt, context)
        except CandidateRejected:
            pass
        else:
            raise AssertionError("boolean run attempt was accepted")

        reused_jobs = json.loads(json.dumps(jobs))
        for job in reused_jobs:
            if job["name"] in LONG_JOB_NAMES:
                job["conclusion"] = "skipped"
                job["steps"] = []
        evidence = {200: reused_jobs, 100: jobs}

        def validate_candidate_fixture(candidate: dict[str, Any]) -> int:
            candidate_jobs = evidence[candidate["id"]]
            validate_jobs(candidate_jobs)
            return candidate["id"]

        selected = select_candidate(
            [{"id": 200}, {"id": 100}], validate_candidate_fixture
        )
        assert selected == 100

        missing_artifact = fixture_artifacts()[:-1]
        try:
            validate_artifacts(missing_artifact)
        except CandidateRejected:
            pass
        else:
            raise AssertionError("partial artifacts were accepted")

        reusable, message = fail_open_discovery(
            lambda: (_ for _ in ()).throw(RecursionError("deep hostile JSON"))
        )
        assert not reusable and "failed open" in message

        git(repo, "checkout", "-q", "-b", "changed-direct-root", producer_revision)
        write_fixture(repo, WORKFLOW_PATH, "name: changed fixture\n")
        commit_fixture(repo, "change workflow root")
        direct = decide_scope(repo, "pull_request", pr_base_sha=producer_revision)
        assert direct.run and direct.changed_paths == (WORKFLOW_PATH,)

        git(repo, "checkout", "-q", "-b", "unsafe-symlink", producer_revision)
        validation_path = repo / "scripts" / "rollback_validation.py"
        validation_path.unlink()
        write_fixture(repo, "scripts/rollback_impl.py", "implementation = 1\n")
        validation_path.symlink_to("rollback_impl.py")
        symlink_revision = commit_fixture(repo, "symlink direct rollback root")
        write_fixture(repo, "scripts/rollback_impl.py", "implementation = 2\n")
        commit_fixture(repo, "change symlink target only")
        symlink = decide_scope(repo, "pull_request", pr_base_sha=symlink_revision)
        assert symlink.run and symlink.fingerprint == "unavailable"
        assert "not a regular tracked rollback file" in symlink.reason

        git(repo, "checkout", "-q", "-b", "unsafe-dynamic", producer_revision)
        write_fixture(
            repo,
            "sim/rollback_validation.lua",
            'local module = "sim.runtime"\nreturn require(module)\n',
        )
        commit_fixture(repo, "dynamic require")
        dynamic = decide_scope(repo, "pull_request", pr_base_sha=producer_revision)
        assert dynamic.run and dynamic.fingerprint == "unavailable"
        assert "non-literal" in dynamic.reason

        git(repo, "checkout", "-q", "-b", "unsafe-missing", producer_revision)
        git(repo, "rm", "sim/runtime.lua")
        commit_fixture(repo, "missing reachable module")
        missing = decide_scope(repo, "pull_request", pr_base_sha=producer_revision)
        assert missing.run and missing.fingerprint == "unavailable"
        assert "missing reachable Lua module" in missing.reason

        git(repo, "checkout", "-q", "-b", "unsafe-ambiguous", producer_revision)
        write_fixture(repo, "sim/runtime/init.lua", "return { duplicate = true }\n")
        commit_fixture(repo, "ambiguous reachable module")
        ambiguous = decide_scope(repo, "pull_request", pr_base_sha=producer_revision)
        assert ambiguous.run and ambiguous.fingerprint == "unavailable"
        assert "ambiguous reachable Lua module" in ambiguous.reason

        git(repo, "checkout", "-q", "-b", "unsafe-root", producer_revision)
        git(repo, "rm", "conf.lua")
        commit_fixture(repo, "missing direct root")
        missing_root = decide_scope(
            repo, "pull_request", pr_base_sha=producer_revision
        )
        assert missing_root.run and missing_root.fingerprint == "unavailable"
        assert "missing required rollback root" in missing_root.reason

    print("rollback CI self-test passed")


def scope_command(arguments: argparse.Namespace) -> int:
    decision = decide_scope(
        arguments.repo.resolve(),
        arguments.event_name,
        event_before=arguments.event_before,
        pr_base_sha=arguments.pr_base_sha,
    )
    print(
        f"rollback_scope={'true' if decision.run else 'false'} "
        f"fingerprint={decision.fingerprint}"
    )
    print(decision.reason)
    for path in decision.changed_paths:
        print(path)
    write_github_output(
        arguments.github_output,
        {
            "run": "true" if decision.run else "false",
            "fingerprint": decision.fingerprint,
        },
    )
    return 0


def discover_command(arguments: argparse.Namespace) -> int:
    if arguments.impact != "true":
        reusable, message = False, "no rollback impact; prior evidence discovery is unnecessary"
    else:
        context = DiscoveryContext(
            arguments.fingerprint,
            arguments.repository,
            arguments.repository_id,
            arguments.pr_number,
            arguments.head_branch,
            arguments.head_repository_id,
            arguments.base_repository_id,
            arguments.base_ref,
            arguments.base_sha,
            arguments.current_run_id,
        )
        reusable, message = fail_open_discovery(
            lambda: discover_reusable_run(
                arguments.repo.resolve(),
                arguments.api_url,
                context,
                arguments.token,
            )
        )
    print(message)
    write_github_output(
        arguments.github_output,
        {"reuse": "true" if reusable else "false"},
    )
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    scope = subparsers.add_parser("scope", help="decide cumulative rollback impact")
    scope.add_argument("--repo", type=Path, default=ROOT)
    scope.add_argument("--event-name", required=True)
    scope.add_argument("--event-before", default="")
    scope.add_argument("--pr-base-sha", default="")
    scope.add_argument("--github-output", type=Path)
    scope.set_defaults(handler=scope_command)

    discover = subparsers.add_parser(
        "discover", help="discover a prior complete first-attempt workflow run"
    )
    discover.add_argument("--impact", choices=("true", "false"), required=True)
    discover.add_argument("--fingerprint", required=True)
    discover.add_argument("--repository", required=True)
    discover.add_argument("--repository-id", type=int, required=True)
    discover.add_argument("--pr-number", type=int, required=True)
    discover.add_argument("--head-branch", required=True)
    discover.add_argument("--head-repository-id", type=int, required=True)
    discover.add_argument("--base-repository-id", type=int, required=True)
    discover.add_argument("--base-ref", required=True)
    discover.add_argument("--base-sha", required=True)
    discover.add_argument("--current-run-id", required=True)
    discover.add_argument("--token", required=True)
    discover.add_argument("--api-url", default="https://api.github.com")
    discover.add_argument("--repo", type=Path, default=ROOT)
    discover.add_argument("--github-output", type=Path)
    discover.set_defaults(handler=discover_command)

    self_test = subparsers.add_parser("self-test", help="run deterministic fixtures")
    self_test.set_defaults(handler=lambda _arguments: run_self_test() or 0)
    return parser.parse_args()


def main() -> int:
    arguments = parse_args()
    return arguments.handler(arguments)


if __name__ == "__main__":
    raise SystemExit(main())
