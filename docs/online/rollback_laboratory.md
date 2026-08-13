# OMP-2 authoritative-reference rollback laboratory

> **Module names below are pre-port `require` paths.** The contract this document
> describes is current; the way it names files is not. Read `sim.foo` as
> `gc_sim::foo` (`rust/crates/gc-sim/src/foo.rs`), `game.online.foo` as
> `gc_netcode::foo`, `core.foo` as `gc_core::foo`, `data.foo` as `gc_data::foo`,
> and any `game/**` or `spec/**` path as its `ts/packages/**` counterpart. A
> `love .` command, a `love.*` API or a love.js measurement is **pre-port
> evidence** — commit `2c0d449` (#467) deleted that tree.

`sim.rollback_lab` is the transport-free convergence runner for OMP-2. It runs
one no-delay reference match beside one impaired rollback client and proves
that both consume the same checked-in authoritative input stream. It has no
renderer, display, socket, browser, or wall-clock dependency.

## Playable development mode

`sim.rollback_playable_lab` is the incremental counterpart used by the match
screen's explicit `MatchScreenOptions.rollback_lab` development option. It does
not add a product route or transport. `profile = "product"` rejects the option.
Normal `Match.new()` and `RealMatch` construction continue to use the legacy
single-player input path.

The screen keeps its one existing `sim.fixed_clock`. Render input is sampled by
`game.match_input_adapter`; only a consumed fixed tick calls `next_tick`, then
`sim.slot_input.to_sample` routes that row to the configured immutable slot.
The controller generates the other seven rows with seeded bots that read only
the independent reference state. It steps one complete reference
`InputFrame`, delivers the local row immediately, sends remote rows through the
selected network profile, reconciles one delivery batch, and catches the
displayed client up to the reference boundary. The screen restores only the
controller's copied current snapshot.

`game.render.correction_smoothing` keeps correction offsets outside
`MatchState`. A small correction begins at the preceding displayed player/ball
pose and immediately spends that render frame's `dt`, then sheds the remaining
offset linearly over 100 ms. Consecutive correction frames therefore keep
advancing instead of freezing until a gap. A correction at or above 160 world
units hard-snaps to authority. Repeated corrections compose from the current
displayed pose, while renderer-owned gait speed and lean follow that smoothed
trajectory rather than the corrected simulation jump. HUD score/time and every
simulation query still read the corrected state immediately. Goal/kickoff and
replay transitions, full time, restart, laboratory teardown, and
synchronization failure clear offsets rather than easing across scene
boundaries.

A synchronization terminal stops input and stays visible in the lab overlay,
but it is not presented as the match's `full_time` phase.

Corrected outputs replace the complete speculative stable-event tail before
new outputs are appended. Confirmation runs after corrections and again after
catch-up, which frees the oldest event slot at the exact 30-tick supported
edge. Each render update aggregates every output, event diff, confirmed step,
and correction rather than retaining only its last fixed tick.

Presentation consumes those copied batches under one policy:

- short-lived action particles may appear speculatively only while keyed by
  stable event ID; revoke and replace deltas remove the old keyed particles;
- a correction clears the renderer-owned loose-ball trail before sampling the
  corrected displayed state;
- audio, statistics, goal celebration/replay, kickoff, full time, and result
  completion consume newly confirmed steps only and deduplicate stable IDs;
- a screen-owned confirmed-lifecycle ledger gates all audio, replay, banner,
  full-time, and result side effects, so a duplicate record cannot restart a
  cue or presentation beat;
- replay frames are keyed by simulation boundary. A correction truncates the
  obsolete interval and records corrected retained snapshots at the same keys;
- a confirmed goal starts its replay at the corrected pre-goal boundary, even
  if newer live boundaries already exist;
- renderer-owned celebration/replay continues after the rollback simulation
  reaches a terminal status. Full-time result navigation waits until that
  sequence finishes or the player explicitly skips it;
- the legacy non-rollback match keeps its state/event adapter, so product and
  offline behavior remain unchanged.

Renderer-owned presentation state never feeds simulation or reference
authority. A confirmed event remains published exactly once even when newer
unconfirmed ticks later roll back. The opening kickoff remains the trusted
match-start beat: the legacy reset path plays it immediately, before any
rollback-confirmed post-goal lifecycle events exist.

Reference full time ends match-input production. Later fixed updates advance
transport only: they resend the retained final remote rows, poll, reconcile,
and confirm until final authority is complete and the network queue is empty.
Settlement is bounded to 256 transport ticks by default and reports
`drain_incomplete` instead of looping synchronously or inventing post-finish
frames.

The cached screen overlay reports the selected profile, reference/client/
transport ticks, confirmation, prediction, rollback depth, resimulation,
active smoothing count and maximum correction magnitude, snapshot retention,
network pending/high-water counters, and latest confirmed convergence. Drawing
reads that copied model only. A live `R` in laboratory mode reconstructs the
reference, bot RNG, rollback and event histories, network queue/counters,
settlement and convergence state, fixed clock, input adapter, and
renderer-owned correction/replay/view/effect/audio state while preserving the
selected laboratory configuration.

The runner accepts only a validated `InputTape`. The tape already contains
materialized eight-slot `InputFrame` rows and a canonical initial snapshot.
Authoritative decisions therefore belong to the tape/reference side and can
never read predicted or corrected client state. `sim.determinism_evidence`
publishes `fixture_tape()` as the narrow public seam for the frozen OMP-1
complete-match fixture; the laboratory does not reach into campaign internals
or invoke the bots that originally produced the recording.

## Execution order

For every tape frame, the laboratory:

1. steps the reference with the original frame and retains its next boundary
   in a bounded snapshot ring;
2. inserts configured local client rows immediately and sends configured
   remote rows through `sim.network_conditions`;
3. polls the current transport tick, feeds every delivery's redundant history
   oldest first, processes the whole batch, and reconciles once;
4. advances the client only to the reference's current boundary;
5. compares every newly confirmed client output boundary with the retained
   reference boundary.

Before forwarding redundant packet history, the lab suppresses only rows at or
below the session's monotonic `confirmed_tick`. All eight slots for those
ticks were already proven authoritative, so a later copy remains a duplicate
even after bounded storage prunes it. A first-seen gap can never be suppressed:
confirmation cannot cross that gap. Every unconfirmed row is forwarded, an
`outside_window` result is an explicit `late_input_unrecoverable` failure, and
the rest of that delivery batch is still processed so the report preserves
the earliest causal late tick. This lets a constant 30-tick stream reconcile
at the supported limit while a constant 31-tick stream fails on causal tick
zero.

After the final input, the runner asks the network simulator to recover the
last row for every remote slot. Drain deliveries are grouped by their actual
arrival tick, with one reconciliation per group. The client catches up only to
the final reference boundary. This recovers a lost final row without
inventing more match time.

A run succeeds only when all of these are true:

- drain completed and the transport queue is empty;
- every row through the final input tick is confirmed;
- confirmed output reaches the reference's final output;
- client and reference final boundaries and hashes match;
- every initial/confirmed boundary was compared;
- no late-window or unconfirmed-below-floor failure occurred.

If confirmation falls behind the monotonic input floor, the result records
`unconfirmed_authority` even if later rows or the final drain arrive. A
completed final-row request cannot hide an older loss.

## Public API and result

`rollback_lab.run(tape, options)` returns a timing-free logical result.
Options select a named or injected network profile, network-only seed, eight
local/remote source rows, rollback window, bounded drain, optional corruption,
and an optional measurement observer.

`rollback_lab.logical_marker(result)` emits a fixed-order
`GC_ROLLBACK_LAB|result|...` line. It includes fixture/profile/seeds, source
pattern, outcome and hashes, confirmation, comparisons, prediction,
correction, rollback and resimulation totals, a sorted depth histogram,
current/peak snapshot count and bytes, bounded input/network diagnostics,
network impairment counters, and drain/late-window status.

`rollback_lab.summary(result)` provides the corresponding human-readable
report.

The OMP-2 exit campaign, acceptance gates, browser matrix, and OMP-3 decisions
are recorded in
[`omp2_rollback_validation.md`](omp2_rollback_validation.md). The campaign
runner that produced that evidence, `scripts/check_rollback.sh`, was deleted
with the LÖVE tree in
[#467](https://github.com/osobytes/goliseo/pull/467) and has no replacement
yet; restoring the stress, matrix and soak evidence is owned by
[#472](https://github.com/osobytes/goliseo/issues/472). Until then the
rollback coverage that actually runs on every `./scripts/check.sh` is the
`gc-sim` test suite: `tests/rollback_lab.rs`,
`tests/rollback_playable_lab.rs`, `tests/rollback_session.rs`,
`tests/combat_load_fixtures.rs` (the four pinned combat load campaigns
against their retained-storage budgets), and `tests/snapshot_headroom.rs`
(the two-band retained-storage reading described in
[`omp2_rollback_validation.md`](omp2_rollback_validation.md) under "The
warning that arrives first").

Intentional corruption changes one client-only input sample. The reference
continues to consume the original tape. The failed result names the causal
input tick, expected and actual boundary hashes, and the first differing
canonical snapshot path.

Free-string marker values are byte-length-prefixed and hexadecimal escaped, so
fixture or custom-profile text containing `|` or `=` cannot forge fields.
Profile numbers use the lossless canonical number encoding shared with match
snapshots rather than rounded decimal formatting. A tape digest covers the
canonical initial snapshot, every exact encoded input frame, all declared
boundary hashes, and the complete fixture/build/source/content/tuning/config
identity. An injected profile without an explicit name is reported as
`custom`.

Network conditions and snapshot history maintain their own high-water
diagnostics at mutation time, before polling, pruning, replay replacement, or
tail truncation can hide transient use. The lab copies those peaks into its
logical result. Drain deliveries are consumed in arrival groups and then
discarded; the returned result retains only the delivery-free drain summary.

## Timing isolation

Pure lab/session state never reads a clock and the logical result contains no
duration. `rollback_session.new` accepts an optional injected
`measure(label, operation)` observer. Simulation retains ownership of the
operation and its return: the observer must invoke it exactly once, cannot
replace its result, and fails loudly if it skips or repeats it. The normal
default calls the operation directly. The headless runner owns monotonic
`love.timer.getTime()`, uses that observer for capture, restore,
resimulation, and inclusive total rollback phases, and prints a separate
`GC_ROLLBACK_LAB|timing|...` wall-time observation. Timings may vary between
runs and must never be used in marker equality or simulation decisions.

## Headless report

Run the complete frozen fixture with the fixed OMP-0 parity profile:

```sh
love . --rollback-lab omp0_parity 7302
```

Run it twice in fresh processes and compare only the logical marker:

```sh
love . --rollback-lab omp0_parity 7302 | sed -n '/^GC_ROLLBACK_LAB|result|/p'
love . --rollback-lab omp0_parity 7302 | sed -n '/^GC_ROLLBACK_LAB|result|/p'
```

The two result lines must be identical. The separate timing lines are
observations and are expected to differ.

To prove divergence diagnostics fail loudly, append `corrupt`:

```sh
love . --rollback-lab clean 7302 corrupt
```

That command intentionally exits non-zero after reporting the causal mismatch.
