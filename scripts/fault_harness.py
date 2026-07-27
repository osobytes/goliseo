#!/usr/bin/env python3
"""Separate-process campaign controller for the OMP-3 fault harness (#169).

The Lua harness (`love . --fault-harness ...`) runs one host plus up to seven
*logical* clients inside a single process and prints a deterministic marker
stream. That is enough to prove the clients agree with each other. It is not
enough to prove the agreement is independent of this build's per-process
`pairs()` hash seed: two clients inside one process share that seed and can
agree by accident, and a 4v4 matrix cannot exhibit the failure at all because
singleton owned sets make live-slot switching inert.

So this controller runs the same scenario in N genuinely separate operating
system processes and compares:

  1. **Cross-process determinism.** Every process must emit an identical marker
     stream (the hash-order probe line excepted). A `pairs()`-order dependence
     anywhere in the hashed path breaks this.
  2. **Cross-process, cross-client agreement.** Client A's confirmed checkpoints
     as computed in process P must equal client B's as computed in process Q,
     for every P != Q, in both the MatchSnapshot hash *and* the live slot per
     human. Those two clients demonstrably do not share a hash seed.
  3. **Hash-seed diversity.** If every process reported the same probe order,
     claim 2 is vacuous for that run. This is reported explicitly rather than
     passed quietly. The check requires **at least two distinct** orders across
     the N processes, not full pairwise distinctness: it detects a total collapse
     of per-process randomization, which is the failure that would make claim 2
     meaningless, but it does not certify that any particular compared pair had
     different seeds.

What it does not prove: anything about NAT traversal, ICE, TURN, real device
scheduling, or browser contexts. Simulated impairment is simulated; #170 covers
real transport.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass, field

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_PROCESSES = 3
DEFAULT_SEED = 4703


@dataclass
class Checkpoint:
    tick: int
    hash: str
    live: str


@dataclass
class RunMarkers:
    """One process's parsed marker stream."""

    label: str
    probe: str = ""
    lines: list[str] = field(default_factory=list)
    # scenario id -> client peer id -> tick -> Checkpoint
    checkpoints: dict[str, dict[str, dict[int, Checkpoint]]] = field(default_factory=dict)
    failures: list[str] = field(default_factory=list)
    result_ok: bool = False
    raw: str = ""

    def digest(self) -> str:
        return hashlib.sha256("\n".join(self.lines).encode("utf-8")).hexdigest()[:16]


def parse_markers(label: str, text: str) -> RunMarkers:
    run = RunMarkers(label=label, raw=text)
    scenario = "?"
    for line in text.splitlines():
        line = line.rstrip()
        if line.startswith("hash-order-probe "):
            run.probe = line[len("hash-order-probe ") :]
            # Deliberately excluded from the comparable stream: it is *expected*
            # to differ, and it is the evidence that the processes differed.
            continue
        if line.startswith("scenario "):
            scenario = line.split(" ", 1)[1]
        if line.startswith("marker checkpoint "):
            _, _, peer_id, tick, digest, live = line.split(" ", 5)
            run.checkpoints.setdefault(scenario, {}).setdefault(peer_id, {})[int(tick)] = (
                Checkpoint(tick=int(tick), hash=digest, live=live)
            )
        if line.startswith("finding ") and " FAIL " in line:
            run.failures.append(line)
        if line.startswith("RESULT "):
            run.result_ok = line.startswith("RESULT ok")
        run.lines.append(line)
    return run


def run_process(selector: str, seed: int, index: int, duration: int | None) -> RunMarkers:
    command = ["love", ".", "--fault-harness", selector, str(seed)]
    if duration is not None:
        command.append(str(duration))
    completed = subprocess.run(
        command,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    run = parse_markers(f"process-{index}", completed.stdout)
    if completed.returncode != 0 and not run.failures:
        run.failures.append(
            f"process {index} exited {completed.returncode}: {completed.stderr.strip()[:400]}"
        )
    return run


# ---------------------------------------------------------------------------
# Comparisons
# ---------------------------------------------------------------------------


def compare_streams(runs: list[RunMarkers]) -> list[str]:
    """Every process must produce the same marker stream."""
    problems: list[str] = []
    reference = runs[0]
    for run in runs[1:]:
        if run.lines == reference.lines:
            continue
        for index, (left, right) in enumerate(zip(reference.lines, run.lines)):
            if left != right:
                problems.append(
                    f"{reference.label} and {run.label} diverge at marker {index}:\n"
                    f"  {reference.label}: {left}\n"
                    f"  {run.label}: {right}"
                )
                break
        else:
            problems.append(
                f"{reference.label} emitted {len(reference.lines)} markers, "
                f"{run.label} emitted {len(run.lines)}"
            )
    return problems


def compare_clients_across_processes(runs: list[RunMarkers]) -> tuple[list[str], int]:
    """Client A in process P versus client B in process Q, for every P != Q.

    This is the comparison a same-process harness structurally cannot make: the
    two clients being compared were computed under different hash seeds.
    """
    problems: list[str] = []
    compared = 0
    scenarios = sorted({scenario for run in runs for scenario in run.checkpoints})
    for scenario in scenarios:
        for left_index, left in enumerate(runs):
            for right_index, right in enumerate(runs):
                if right_index <= left_index:
                    continue
                left_clients = left.checkpoints.get(scenario, {})
                right_clients = right.checkpoints.get(scenario, {})
                for left_peer, left_points in sorted(left_clients.items()):
                    for right_peer, right_points in sorted(right_clients.items()):
                        for tick, point in sorted(left_points.items()):
                            other = right_points.get(tick)
                            if other is None:
                                continue
                            compared += 1
                            if other.hash != point.hash:
                                problems.append(
                                    f"{scenario}: {left.label}/{left_peer} and "
                                    f"{right.label}/{right_peer} disagree on the boundary "
                                    f"hash at tick {tick}"
                                )
                            if other.live != point.live:
                                problems.append(
                                    f"{scenario}: {left.label}/{left_peer} and "
                                    f"{right.label}/{right_peer} disagree on the live slot "
                                    f"at tick {tick} ({point.live} vs {other.live})"
                                )
    return problems, compared


def probe_diversity(runs: list[RunMarkers]) -> tuple[int, str]:
    """Distinct probe orders observed. Two is enough to refute a total collapse.

    Deliberately not pairwise distinctness: with N processes drawing seeds
    independently, insisting every pair differ would make the check flaky for a
    reason that says nothing about the code under test.
    """
    probes = {run.probe for run in runs if run.probe}
    return len(probes), ", ".join(sorted(probes))


# ---------------------------------------------------------------------------
# Campaign
# ---------------------------------------------------------------------------


def campaign(selector: str, seed: int, processes: int, duration: int | None) -> bool:
    print(f"==> fault harness campaign: selection={selector} seed={seed} processes={processes}")
    runs = [run_process(selector, seed, index + 1, duration) for index in range(processes)]

    ok = True
    for run in runs:
        print(f"    {run.label}: markers={len(run.lines)} digest={run.digest()} probe={run.probe}")
        if not run.result_ok:
            ok = False
            print(f"    ! {run.label} reported a failing run")
        for failure in run.failures:
            print(f"      {failure}")

    stream_problems = compare_streams(runs)
    if stream_problems:
        ok = False
        print("    ! marker streams are not identical across processes")
        for problem in stream_problems:
            print(f"      {problem}")
    else:
        print("    cross-process determinism: identical marker streams")

    client_problems, compared = compare_clients_across_processes(runs)
    if client_problems:
        ok = False
        print("    ! cross-process client comparison failed")
        for problem in client_problems[:20]:
            print(f"      {problem}")
        if len(client_problems) > 20:
            print(f"      ... and {len(client_problems) - 20} more (bounded output)")
    else:
        print(f"    cross-process client comparison: {compared} checkpoint pairs agreed")

    distinct, probes = probe_diversity(runs)
    if distinct >= 2:
        print(f"    hash-seed diversity: {distinct} distinct pairs() orders observed")
    else:
        ok = False
        print(
            "    ! hash-seed diversity was NOT observed: every process reported the same "
            f"pairs() order ({probes}). The cross-process claim is not established for this "
            "run — re-run, and if it persists this build no longer randomizes per process."
        )

    print(f"==> {'CAMPAIGN OK' if ok else 'CAMPAIGN FAILED'}")
    return ok


# ---------------------------------------------------------------------------
# Self-test: the controller's own logic, with no LOVE process
# ---------------------------------------------------------------------------

SELF_TEST_STREAM = """FAULT-HARNESS v1
selection smoke rows=1
hash-order-probe alpha,bravo
scenario 2v2.playable
marker checkpoint host 0 aaaa0000 guest_1=away_1,host=home_1
marker checkpoint host 30 bbbb1111 guest_1=away_2,host=home_1
marker checkpoint guest_1 0 aaaa0000 guest_1=away_1,host=home_1
marker checkpoint guest_1 30 bbbb1111 guest_1=away_2,host=home_1
finding 2v2.playable converge.checkpoint_hash ok compared 2
RESULT ok rows=1 failures=0
"""


def self_test() -> bool:
    ok = True

    def check(condition: bool, message: str) -> None:
        nonlocal ok
        if not condition:
            ok = False
            print(f"    ! {message}")

    clean = parse_markers("a", SELF_TEST_STREAM)
    twin = parse_markers("b", SELF_TEST_STREAM.replace("alpha,bravo", "bravo,alpha"))
    check(clean.result_ok, "a passing stream should parse as passing")
    check(clean.probe == "alpha,bravo", "the probe line should be captured")
    check(not compare_streams([clean, twin]), "a differing probe alone must not be a divergence")
    problems, compared = compare_clients_across_processes([clean, twin])
    check(compared == 8, f"expected 8 cross-process checkpoint pairs, got {compared}")
    check(not problems, "identical streams must agree across processes")
    check(probe_diversity([clean, twin])[0] == 2, "two distinct probes should be counted")
    check(probe_diversity([clean, clean])[0] == 1, "identical probes should be counted once")

    # A hash-order divergence: one process computes a different live slot.
    diverged = parse_markers(
        "c", SELF_TEST_STREAM.replace("guest_1=away_2,host=home_1", "guest_1=away_1,host=home_1")
    )
    problems, _ = compare_clients_across_processes([clean, diverged])
    check(any("live slot" in problem for problem in problems), "a live-slot divergence must fail")
    check(bool(compare_streams([clean, diverged])), "a marker divergence must be reported")

    # A boundary hash divergence.
    hashed = parse_markers("d", SELF_TEST_STREAM.replace("bbbb1111", "cccc2222"))
    problems, _ = compare_clients_across_processes([clean, hashed])
    check(any("boundary hash" in problem for problem in problems), "a hash divergence must fail")

    # A failing run must be surfaced.
    failing = parse_markers(
        "e",
        SELF_TEST_STREAM.replace(
            "finding 2v2.playable converge.checkpoint_hash ok compared 2",
            "finding 2v2.playable converge.checkpoint_hash FAIL compared 2, 1 disagreed",
        ).replace("RESULT ok", "RESULT fail"),
    )
    check(not failing.result_ok, "a failing stream should parse as failing")
    check(len(failing.failures) == 1, "a FAIL finding should be collected")

    print("==> fault harness controller self-test " + ("OK" if ok else "FAILED"))
    return ok


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="check this controller's logic")
    parser.add_argument("--selection", default="smoke", help="smoke, full, or a scenario id")
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED, help="impairment seed")
    parser.add_argument("--processes", type=int, default=DEFAULT_PROCESSES)
    parser.add_argument("--duration", type=int, default=None, help="override duration ticks")
    args = parser.parse_args()

    if args.self_test:
        return 0 if self_test() else 1
    return 0 if campaign(args.selection, args.seed, args.processes, args.duration) else 1


if __name__ == "__main__":
    sys.exit(main())
