# OMP-1 determinism evidence

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

Status: **native pass on the authoritative snapshot-v11 fixture**. The accepted
snapshot-v4 Chrome/Firefox evidence remains historical until CI records the v11
browser run; the snapshot-v1 browser artifact is also preserved below.

This report closes the OMP-1 evidence line. It proves that one complete,
recorded eight-slot fixture has a stable state boundary after every fixed
tick, that selected restore/replay windows converge, and that the existing
offline product flow remains covered. It does not implement prediction,
rollback, transport, rooms, or network presentation.

## Authoritative fixture

The checked-in `data.omp1_determinism` table is the immutable tape artifact.
It contains 7,201 canonical `InputFrame` wires (all eight stable outfield rows
on every frame), the matching 7,202 start-of-tick snapshot hashes, identity,
event counts, and restore windows. The source bot policy and its RNG are not
used during verification; they only materialize a replacement recording when
the explicit refresh command is invoked.

**"Immutable" is true of the recorded input, not of the hashes derived from
it** (#503). The Rust fixture `rust/crates/gc-data/src/omp1_determinism.json`
splits in two: `frame_wires`, `identity`, `source_seeds` and `windows` can
never be recaptured — the source bots that authored them are deleted — while
`boundary_hashes`, `boundary_count`, `expected_final_hash` and
`expected_sequence_digest` are what the current simulation computes by
replaying those frames, and a deliberate gameplay change moves all of them.
The re-record command is `record_omp1_derived_baseline` in
`rust/crates/gc-sim/tests/determinism_evidence.rs` (`#[ignore]`d; it never
runs in a gate). Re-recording is a decision, not a fix, and it weakens what a
pass means — see `gc_data::omp1_determinism`'s module doc for the rule and
the trade.

| Identity field | Frozen value |
| --- | --- |
| Fixture | `omp1-nebula-orion-eight-streams-v2` |
| Tape / input / snapshot versions | `1 / 2 / 11` |
| Build | `omp1-determinism-v1` |
| Source | `issue-39-canonical-recording-v1` |
| Content | `nebula-orion-showcase-content-v1` |
| Configuration | `field=960x540;duration=120;max_goals=3;tick_rate=60` |
| Tuning | Exact default blob (the canonical serialization is empty) |
| Match seed | `19` |
| Recorded source seeds | `1997, 2094, 2191, 2288, 2385, 2482, 2579, 2676` |
| Ownership | Nebula and Orion five-player rosters; four fixed outfield slots per side |

The nominal 120-second match consumes input ticks `0..7200`, then finishes at
boundary `7201`. The extra terminal tick comes from the existing repeated
floating-point countdown rather than a change to the 60 Hz authority. OMP-2
must preserve this recorded boundary or deliberately replace the countdown
with an integer tick budget and version the fixture.

**What a campaign gates on is narrower than what it checks** (#505, #512). Two
repository-owner decisions, recorded on those issues, split the campaign's
assertions by what they prove:

| Gated — the run goes red | Reported — printed, never red |
| --- | --- |
| every boundary hash, `expected_final_hash`, `expected_sequence_digest`, two independent replays agreeing, every restore window replaying to its pinned hashes | `DeterminismCoverage`: which of a tackle, a catch, a header and a full time **occurred** |
| the recording reaches full time and consumes exactly `frame_count` frames | `expected_score` |
| | `event_counts` |
| | each window's `event_tick`, including whether the window still contains the event it is scoped around |

**After #512, OMP-1 gates on exactly one thing: the boundary-hash chain.** #505
drew the line at *"these behaviors occurred"* against *"exactly 147 tackles
occurred and the score was 1-0"*, keeping the first as a gate. #512 withdrew
that carve-out on measurement: this fixture's `frame_wires` are **frozen button
presses**, so `MOVE_ACCEL` 1100 → 1105 — 0.45% — puts every player somewhere
slightly different, the recorded presses stop producing the header, and the
campaign failed `fixture did not cover aerial`. With frozen inputs, *which*
behaviors occurred is a claim about one recorded scenario exactly as much as
*how many* is. All of it is incidental to the determinism guarantee the fixture
exists to provide, and gating on any of it foreclosed every queued gameplay
rework (#488, #489, #490, #491).

**The gap that leaves.** Nothing in this repository now gates "the simulation
still produces football". That is real and is not being papered over: the
replacement is **#518**, a live-AI behavior fixture whose bots are driven by
*current* code and therefore adapt to a tuning change, so it can carry the
claim OMP-1 structurally cannot. It was filed with #512's decision precisely so
the gap would not be left as an intention.

A moved claim is reported as **drift** — the recorded value and the current
one, side by side — through three channels: `scripts/check.sh`'s determinism
gate (`coverage=` and `drift=` on the `GC_DETERMINISM` line, the latter
escalated to a `BEHAVIORAL DRIFT` block when non-empty),
`ts/packages/wasm/src/determinism.spec.ts`'s log line, and the
`record_omp1_derived_baseline` recorder's warning block. A lost headline
behavior appears as `coverage.aerial:covered->absent`. Drift is not
self-evidently fine: read it the way a drifted boundary hash is read —
intended, or a finding? If intended, record it in the PR that causes it, with
the previous value and the new one.

## Hash and repeated-run result

Every boundary is encoded with canonical snapshot version 11 and hashed with
the browser-safe FNV-1a-64 implementation. Verification performs these three
checks:

1. Two independently constructed matches agree at every boundary.
2. Each observed boundary agrees with its literal checked-in hash.
3. FNV-1a-64 over the ordered newline-delimited boundary hashes agrees with
   the pinned sequence digest.

The authoritative values are:

```text
boundaries=7202
final_hash=bfbb106aea5480f8
sequence_digest=0bfd0ed355f87322
final_snapshot_bytes=21820
```

The complete match produced — **reported, not gated** since #505 (and, for
`coverage`, since #512); these are the values current drift is measured
against:

```text
coverage=tackle,aerial,keeper,full_time
score=1-0 outcome=home
catch=1 claim=3 header=2 pass=4 reception=1 shot=2 tackle=147 touch=180
window event ticks: tackle=24 keeper=1692 aerial=1788 full_time=7200
```

These values were last refreshed for
[#450](https://github.com/osobytes/goliseo/issues/450), which ends a keeper's
dive at the moment it takes possession instead of letting `dive_timer` run out
underneath the catch. This fixture is direct evidence of how narrow that is:
exactly **26 of the 7,202 boundary hashes move**, `1693..1718`, and nothing
else. `final_hash` does not move, nor does the `1-0` score, the event counts,
`frame_wires`, the identity, the schema versions or the restore windows.
Compared field by field, the only differences across the whole recording are
three fields on one player — the away keeper's `dive_timer`, `dive_target` and
`keeper_get_up_timer` — which is the dive ending 15 ticks earlier and its
get-up window starting 15 ticks earlier. No position, ball, score or RNG value
differs on any boundary, so the fixture's *play* is unchanged and only the
keeper's own dive bookkeeping moved. (The AI-driven reference match in
`crates/gc-sim/tests/fixtures/session_ai_driven_lua_reference.txt` is the
scenario where the same change does alter play; it is not this fixture.)

Before that, the values were refreshed for the possessed-ball touchline fix.
That fix clamps an owned ball to the arena, and this fixture was direct
evidence the old code let one out: boundaries `0..2026` were bit-identical
across that refresh and only `2027` onward moved, so the clamp is a genuine
no-op until the first tick on which the ball actually leaves the pitch.

Because a stranded ball no longer kills the attack it was part of, the fixture
now runs to `1-0` rather than `0-0`; one `claim` and one `pass` become seven
more `touch` events and a second `shot`, which is play continuing instead of
stalling. The frozen
input wires are unchanged byte-for-byte, as is every identity and schema field,
so the authoritative input contract did not move -- only the state the same
inputs now produce. `coverage` still reports `tackle,aerial,keeper,full_time`.
Note that it does so even though the match now scores: the `goal_kickoff`
predicate is written against an *away* goal (`score.away > 0`), and this one is
the home side's, so the goal/kickoff window remains uncovered by the frozen
match and is still carried by the bounded synthetic tape described in
`snapshot_replay.md`. The fixture's outcome is also no longer a draw, so no
OMP-1 evidence exercises a drawn full time.

`sim.determinism_evidence` reports the causal tick and expected/actual hash on
the first mismatch. A normal verification cannot regenerate its expectation.
The deliberate refresh command is separate:

```sh
love . --determinism-refresh
```

Refreshing the recording is a snapshot/state-evidence contract change. It
replays every existing effective sample and cannot invoke bot policy to replace
the authoritative input contract. Review the identity, wires, hashes, event
counts, score, and restore windows before committing it.

The input-v2 migration is deliberately fixture-specific. It changes the frozen
fixture and ownership identity from v1 to v2 and rewrites only each canonical
wire's leading version field. All movement axes and existing held/edge masks
remain unchanged. General runtime decode, replay, and ownership validation
reject v1: frame and ownership decoding return `unsupported_version`, while
replay rejects a v1 tape as `identity_mismatch` at `identity.input_version`.
This evidence seam is not a compatibility decoder. Snapshot-only refreshes
continue to change only `snapshot_version`. In either case, refresh consumes
every migrated frozen frame in order to regenerate snapshot hashes. Bot
materialization is not part of the refresh path.

The snapshot-v11 migration retains the canonical 725,882-byte block of 7,201
effective input wires byte-for-byte
(`SHA-256 380908c9ae2ab1a04b1dfd1196d1395a7ea047160c31cff41dcbb2758c08a7f7`).
It adds two team-level `MatchState` fields: `transition_windows`, each side's
authored counter-press and counter-attack seconds, and `transition`, the
versioned possession memory (last and currently holding established team, the
capped hold streak, the turnover winner, and seconds since that turnover) that
`brain.phase` decays into each team's counterpress/counterattack phase.
Fixed-slot players remain excluded from match AI, so the frozen score, event
counts, and effective inputs do not change; every outfielder in this fixture
owns an input slot, so no transition phase steers anyone in it. The new state
grew the final snapshot from 21,659 to 21,937 bytes at that migration; the
possessed-ball refresh above later moved it to 21,820 by changing the state the
same inputs reach, not the schema. Only schema identity,
canonical bytes, boundary hashes, sequence digest, and snapshot size are
refreshed.

The snapshot-v10 migration also retained all effective input wires
byte-for-byte. It added one keeper field, `keeper_get_up_timer`: the
simulation-owned window that arms when a dive lunge ends and decays under the
fixed timestep, so the post-dive get-up pose reads a real recovery timer instead
of the keeper's `recover` positioning state. The new per-player scalar grew the
final snapshot from 21,389 to 21,659 bytes.

The snapshot-v9 migration also retained all effective input wires byte-for-byte.
It added each team's authored formation identity and OutfieldDecision v2's
optional run expiry, and changed those decision records to compact positional
encoding. Fixed-slot players remained excluded from match AI, so the score,
event counts, and effective inputs did not change; the positional encoding
reduced the final snapshot from 22,488 to 21,389 bytes despite the new
formation fields.

The snapshot-v8 migration also retained all effective input wires byte-for-byte.
It added one compact, versioned, team-owned outfield press state per side:
presser identity, contain-or-commit mode, and the single stable commit reason.
Fixed-slot players remained excluded from match AI, so the score, event counts,
and effective inputs did not change; only schema identity, canonical bytes,
boundary hashes, sequence digest, and snapshot size were refreshed.

The snapshot-v7 migration retained all 7,201 effective input wires
byte-for-byte. It adds the derived scan/composure values and serializable
outfield cadence/intent state to every player boundary. Fixed-slot players are
excluded from match AI, so score, event counts, and effective inputs do not
change; only schema identity, canonical bytes, boundary hashes, sequence
digest, and snapshot size are refreshed.

The input-v2 migration retained all 7,201 effective input rows from the
snapshot-v5/input-v1 fixture; only wire version headers and input/ownership
identity changed. Because ownership version is canonical snapshot state, the
boundary hashes and sequence digest were regenerated even though gameplay
inputs and the 0-0 outcome did not change.

The snapshot-v5 migration retained all 7,201 snapshot-v4 input wires
byte-for-byte
(`SHA-256 a717c094e69229e7149e6d184a8a3dcc7a12476a0c07109eff1552de01bf2292`).
The migration source was the exact fixture on `main`; only the schema identity
and regenerated evidence changed. The new explicit keeper set/context behavior
intentionally changes the frozen outcome from `0-1` to `0-0`; event counts,
restore windows, boundary hashes, the sequence digest, and snapshot size drift
accordingly. The tape contains no selected chip, so this outcome change is not
caused by hidden chip accuracy or altered input wires.

The draw removes the old full-match goal/kickoff window. That loss is disclosed
rather than manufacturing a replacement result or weakening the keeper state
contract. Snapshot-v5 coverage instead builds a bounded synthetic input tape
from a pre-goal canonical snapshot, crosses the goal line, performs the kickoff
reset, and advances through a post-kickoff boundary. `sim.input_tape` and
`sim.replay` validate every boundary hash and compare an independently restored
tape, while the initial snapshot exercises all new keeper fields.

The final neutral-positioning refinement retained the same snapshot-v5 schema
and all 7,201 input wires, but deliberately regenerated boundary evidence. Base
depth now varies from the physical one-radius inset at 12 px to an 18 px cap as
the attack approaches; a bounded 40 px near-post bias makes the far-corner
concession explicit without preserving the legacy lateral band.
The frozen outcome was 0-0 at that point, and the hashes, event counts and
final snapshot size described that audited behavior rather than either earlier
fixed-depth snapshot-v5 candidate. The possessed-ball refresh recorded at the
top of this file has since moved the outcome to 1-0, so the authoritative block
above is no longer this paragraph's evidence.

## Restore/replay windows

The complete pass captures start-of-window snapshots. Each window is later
restored independently, advanced with the same frozen wires, and compared
against every pinned boundary:

| Scenario | Start boundary | Last boundary | Required transition |
| --- | ---: | ---: | --- |
| Tackle | 23 | 26 | `tackle` at causal tick 24 |
| Keeper | 1690 | 1695 | `catch` at causal tick 1692 |
| Aerial | 1786 | 1791 | `header` at causal tick 1788 |
| Full time | 7198 | 7201 | `finished`, zero time at causal tick 7200 |

This covers routine play in the uninterrupted complete run except for the
explicitly disclosed goal-window drift above. The harness uses the same canonical
identity, effective-frame, snapshot, and boundary-hash shapes as
`sim.input_tape` and `sim.replay`, while exposing a bounded incremental step
API so love.js yields to the browser between batches.

## Commands and measurements

The native gate launches two fresh LÖVE processes, compares their complete
result markers, and then reports the existing snapshot microbenchmark:

```sh
./scripts/check_determinism.sh
```

On the development machine (Zorin OS 18.1, Linux x86_64, native LÖVE 11.5),
100 operations at boundary tick 120 measured during the final v5 native gate:

```text
snapshot_measure version=5 tick=120 bytes=18292 iterations=100 hash=5e32bb31e3cdb281
snapshot_measure encode_us_each=242.810
snapshot_measure hash_with_encode_us_each=1506.240
snapshot_measure restore_us_each=96.650
```

These are observations, not thresholds. The two final fresh native runs
completed in 27.507 s and 26.930 s and emitted identical result markers.
Browser evidence records wall-clock duration per fresh process because
WebAssembly timings are not interchangeable with native `os.clock`
measurements.

For the actual love.js runtime matrix:

```sh
./scripts/web_build.sh /tmp/omp1-web
python3 scripts/browser_determinism.py \
    --artifact /tmp/omp1-web \
    --output /tmp/omp1-browser-determinism.json
```

The runner requires a boolean clean-source marker, validates every served byte
against the manifest, pins the love.js repository/commit/archive, requires one
result marker and no loader/runtime errors, and verifies bounded process-group
cleanup. It launches two fresh profiles per required browser and fails, rather
than skips, if Chrome or Firefox is missing.

### CI shape: one shard per browser

The determinism assertion is *within* a browser — two fresh processes of the
same runtime must produce the same marker — so CI runs one shard per browser
(`chrome`, `firefox`) instead of both in one job. Wall clock becomes the slower
browser rather than the sum of both. Inside a shard the two fresh processes run
concurrently (`--run-concurrency 2`); they still own separate browser process
groups, separate profiles, and separate WebDriver logs, and the comparison is
hash equality, which no amount of CPU contention can alter.

Each shard uploads `omp1-determinism-<browser>`, and `OMP-1 browser determinism
gate` aggregates them with `--mode aggregate`. The rolled-up matrix result is
checked first but is only a necessary condition: GitHub collapses a matrix job
to a single `result`, which cannot distinguish a shard that ran from one that
was never scheduled. The sufficient condition is the pinned manifest in
`scripts/browser_determinism.py`:

1. `require_complete_shards` compares the artifacts that arrived against
   `expected_shard_evidence()` and fails, naming the shard, on any **missing**,
   **unpinned**, or **duplicate** entry. A shard that vanished, was skipped, or
   was cancelled uploads nothing, so its pinned name is missing and the gate
   fails closed. An unaccounted-for artifact fails closed too. The duplicate
   arm is contract, not defence: `actions/upload-artifact` enforces unique names
   per run and a directory listing cannot repeat an entry, so the self-test
   asserts it by calling the function directly rather than through the gate.
2. `load_shard_evidence` requires each shard's JSON to declare the pinned
   evidence schema, its own `pass: true`, its own browser, the pinned love.js
   commit, a clean checkout at the gate's exact revision, at least two records
   from **distinct browser process groups**, one browser version, and markers
   equal to the pinned determinism, protocol, and input-protocol goldens.
   Evidence from another commit, runtime, or process cannot stand in for a
   missing shard.
3. `require_cross_runtime_agreement` then compares the records to each other:
   every record from every shard must carry byte-identical marker fields, so
   Chrome and Firefox disagreeing fails. Step 2's golden pin already implies
   this, and that redundancy is deliberate — one check asks whether each runtime
   matches what we recorded, the other whether the runtimes match each other.
   They share no data and no code, so relaxing or breaking either one cannot
   quietly leave cross-runtime agreement unenforced.

`parse_marker` additionally rejects any marker field outside the pinned set, so
a shard cannot vary a value that nothing else checks. Every branch above is
covered by `python3 scripts/browser_determinism.py --self-test`, including
dropping each pinned shard in turn.

Steps 2 and 3 are each proven **individually** necessary, because a check that
only ever fires behind another one is indistinguishable from dead code. The
self-test isolates them with fixtures the other check cannot reject:

- Shards whose hashes diverge, aggregated with step 2's golden pin switched off
  via `aggregate_shards`' `marker_check` seam. Only step 3 can reject that, so
  deleting step 3's comparison makes the self-test fail.
- Shards that all agree on a hash the golden does not pin. Step 3 is blind to
  that by construction, so deleting step 2's comparison makes the self-test fail.

`marker_check` is a test seam and nothing more; CI always aggregates with the
pinned golden in place.

## Runtime verification

The authoritative snapshot-v11 fixture passes the two-fresh-process native
command above. CI builds a clean love.js artifact and runs the same current
fixture in real Chrome and Firefox; that workflow, rather than a hand-edited
evidence file, supplies the current browser integration proof.

### Historical snapshot-v1 browser evidence

| Runtime | Executions | Wall time | Historical result |
| --- | ---: | ---: | --- |
| Linux Chrome 151.0.7922.34 / pinned love.js 11.5 | Two fresh browser profiles | 207.956 s, 196.828 s | Pass on snapshot v1 |
| Linux Firefox 152.0.6 / pinned love.js 11.5 | Two fresh browser profiles | 217.953 s, 214.245 s | Pass on snapshot v1 |

Those four historical browser executions produced final hash
`b379a3a3ab5d7682` and sequence digest `0ff53075e3e626e0`. They are not presented
as proof for the current v11 hashes.

The clean browser artifact was built from source commit `16fad22`, with package
SHA-256 `2ec87dfa91770ea6b6772444c490808bf4ef7eaf2eca9693a3e7fbca27187f4f`.
Chrome exited normally. Firefox 152 reached the valid result in both runs but
its normal quit exceeded 30 seconds; the runner's isolated-process-group
fallback sent `TERM`, observed geckodriver exit code 0, verified the complete
group disappeared, and left no Firefox/geckodriver orphan. This is a teardown
limitation, not a simulation mismatch or silent skip.

The immutable historical machine-readable record, including exact durations,
driver versions, teardown outcomes, and raw-log hashes, is
[`evidence/omp1_browser_linux_2026-07-20.json`](evidence/omp1_browser_linux_2026-07-20.json).

## Offline-product compatibility

The deterministic gate is additive and runs before the normal product
bootstrap only when its explicit flag is present. Native evidence disables
window/audio modules; browser evidence retains the ordinary love.js window and
yields through `love.update`.

The full headless suite continues to cover the title → team sheet → match →
result → rematch loop, repeated rematches, result exits, the real
match adapter, and browser compatibility flow. The required compatibility
commands are:

```sh
love . --test
./scripts/web_smoke.sh
```

No offline input mapping, screen route, match request/result contract, or
browser artifact packaging path is replaced by this evidence work.

## Remaining OMP-2 risks

- The checked-in browser artifact is historical snapshot-v1 evidence. Current
  snapshot-v11 Chrome/Firefox proof runs in CI and is not a substitute for
  Windows, macOS, or cross-architecture floating-point evidence.
- The full-time boundary currently depends on floating countdown semantics
  and consumes 7,201 inputs for a nominal 7,200-tick duration.
- Canonical snapshots intentionally include all declared simulation state and
  are about 21 KiB here. OMP-2 needs memory/bandwidth policy before keeping
  rollback history.
- The 850 KiB fixture favors auditability and exact per-tick regression
  diagnosis over repository size. A future compressed format must preserve
  canonical decoded bytes and versioning.
- The now-total nearest-player comparator uses descending player index for an
  exact-distance tie to preserve the existing native outcome, and quantization
  now canonicalizes negative zero. Other new rankings and numeric boundaries
  still need explicit total ordering and cross-runtime evidence.
- This suite proves deterministic replay only. It says nothing about late
  input policy, prediction quality, resimulation cost, network packet shape,
  state repair, or transport behavior.
