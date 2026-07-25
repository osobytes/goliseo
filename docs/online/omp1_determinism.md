# OMP-1 determinism evidence

Status: **native pass on the authoritative snapshot-v9 fixture**. The accepted
snapshot-v4 Chrome/Firefox evidence remains historical until CI records the v9
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

| Identity field | Frozen value |
| --- | --- |
| Fixture | `omp1-nebula-orion-eight-streams-v2` |
| Tape / input / snapshot versions | `1 / 2 / 9` |
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

## Hash and repeated-run result

Every boundary is encoded with canonical snapshot version 9 and hashed with
the browser-safe FNV-1a-64 implementation. Verification performs these three
checks:

1. Two independently constructed matches agree at every boundary.
2. Each observed boundary agrees with its literal checked-in hash.
3. FNV-1a-64 over the ordered newline-delimited boundary hashes agrees with
   the pinned sequence digest.

The authoritative values are:

```text
boundaries=7202
final_hash=db0c203af21d6cfd
sequence_digest=e1040cdab6b46393
score=0-0
outcome=draw
final_snapshot_bytes=21389
```

The complete match produced:

```text
catch=1 claim=4 header=2 pass=5 reception=1 shot=1 tackle=147 touch=173
```

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

The snapshot-v9 migration retains the canonical 725,882-byte block of 7,201
effective input wires byte-for-byte
(`SHA-256 380908c9ae2ab1a04b1dfd1196d1395a7ea047160c31cff41dcbb2758c08a7f7`).
It adds each team's authored formation identity and OutfieldDecision v2's
optional run expiry, and changes those decision records to compact positional
encoding. Fixed-slot players remain excluded from match AI, so the frozen
score, event counts, and effective inputs do not change. The positional
encoding reduces the final snapshot from 22,488 to 21,389 bytes despite the
new formation fields. Only schema identity, canonical bytes, boundary hashes,
sequence digest, and snapshot size are refreshed.

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
The frozen outcome is 0-0. The hashes, event counts, and final snapshot size
above describe that final audited behavior rather than either earlier
fixed-depth snapshot-v5 candidate.

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

## Runtime verification

The authoritative snapshot-v9 fixture passes the two-fresh-process native
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
as proof for the current v9 hashes.

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

The full headless suite continues to cover the title → squad → formation →
tactic → result → rematch loop, repeated rematches, result exits, the real
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
  snapshot-v9 Chrome/Firefox proof runs in CI and is not a substitute for
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
