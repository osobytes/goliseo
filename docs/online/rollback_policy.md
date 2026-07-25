# OMP-2 rollback input and confirmation policy

`sim.rollback_input_history` is the pure source of truth for the eight input
rows a rollback client has received and the complete `InputFrame` it actually
used on each simulation tick. It is an in-process policy layer: it has no
transport, LÖVE, presentation, snapshot, or wall-clock dependency.

The canonical sample shape, slot order, integer quantization, and fixed
ownership remain the OMP-1 contracts in [`input_frame.md`](input_frame.md) and
[`slot_match.md`](slot_match.md). The simulation cadence remains 60 Hz as
defined in [`fixed_tick.md`](fixed_tick.md).

## Tick and record terminology

- **Input tick `N`** is the causal `InputFrame.tick == N` consumed by
  `sim.match.step`.
- **Boundary `N`** is the start-of-tick snapshot before input tick `N`. It
  describes the state after input `N - 1`, as defined in
  [`snapshot_replay.md`](snapshot_replay.md).
- An **authoritative sample** is the immutable local or remote sample accepted
  for one tick and slot.
- A **predicted sample** is the row materialized because remote authority for
  that tick and slot has not arrived.
- An **effective frame** is the complete eight-row frame materialized
  immediately before the simulation consumes that tick. Its per-slot record
  preserves both the fixed `local` / `remote` source and the
  `authoritative` / `predicted` status used at that time.

Source and status are independent. A remote sample can be authoritative once
it arrives. A local row must always be authoritative before materialization;
missing local input is a producer bug and fails loudly instead of being hidden
as prediction. Only a missing remote row is eligible for prediction.

## Authoritative arrival rules

`add_authoritative(history, tick, slot, sample)` accepts local samples from the
local input adapter and remote samples from the eventual packet/laboratory
adapter through the same deterministic path. Arrivals may be out of order.
The history deep-copies the sample before retaining it, so later caller
mutation cannot rewrite authority.

An identical duplicate is idempotent. A second, different authoritative
sample for the same tick and slot returns the recoverable
`conflicting_authoritative` failure and changes no sample, count, confirmed
tick, or divergence state. The client must treat that conflict as invalid
upstream data; authority is never last-writer-wins.

## Prediction rule

For each missing remote row on tick `N`, search that slot's authoritative
history at or before `N` and choose the greatest tick found:

1. Copy `move_x`, `move_y`, and `held` from that sample.
2. Set `edges` to zero unconditionally.
3. If no prior authoritative sample exists, use the fully neutral sample:
   zero axes, zero held bits, and zero edges.

Future out-of-order arrivals are never used to predict an earlier tick.
Predictions chain from the latest authoritative sample, not from another
prediction. Discrete shoot, pass, switch, dash, dodge, equipment-press, and
equipment-release edges therefore fire only when their authoritative tick is
simulated; a lost or delayed edge can never become sticky or repeat across
predicted ticks. Equipment held intent repeats with the rest of the held mask.

## Confirmation

The **confirmed tick** is the greatest contiguous input tick starting at zero
for which all eight slots are authoritative. Its initial sentinel is `-1`,
meaning even tick zero is not yet fully authoritative. A complete future tick
cannot cross a gap. When that gap becomes complete, confirmation advances
through every already-complete following tick. It never moves backward.

This definition deliberately requires all local and remote rows. It is an
input-history boundary only; confirmed presentation events are introduced by
the later stable-event contract.

## Divergence and resimulation handoff

An authoritative arrival is a **divergence** only if its sample differs from
the effective row already materialized and used for that same tick and slot.
An identical prediction produces no divergence, and an arrival for a tick that
has not been materialized is simply authority. Across any batch of differing
arrivals, the history retains the smallest affected tick.

The caller follows this order:

1. Insert all arrivals available for the update.
2. Peek at `earliest_divergence(history)` for diagnostics.
3. Consume that tick with `consume_earliest_divergence(history)` immediately
   before restoring its start-of-tick boundary.
4. Resimulate forward, calling `materialize` again for each replayed tick. Each
   call replaces that tick's former effective record with the corrected one.

Materializing at or after an unconsumed divergence fails loudly. This prevents
normal forward simulation or accidental diagnostics from overwriting evidence
needed to choose the restore boundary. Consuming clears only the reported
batch boundary; authoritative samples remain immutable, and a later correction
can establish a new earliest divergence.

Returned frames, effective records, and authoritative records are deep copies.
Mutating one cannot change retained history or a later read.

## Initial rollback window

OMP-2 uses a maximum rollback depth of **30 input ticks**, exactly **500 ms at
60 Hz**. `sim.rollback_input_history` publishes those two constants, and
`sim.rollback_snapshot_history` uses the tick count as its default maximum.

Snapshot history retains the current start-of-tick boundary plus the preceding
30 boundaries: exactly 31 ring positions. `store` derives the boundary key
from `snapshot.state.input_tick`; callers cannot provide a second tick that
could disagree. Advancing the present boundary deterministically evicts every
stored tick below `present - 30`. Lookup identifies the present boundary, an
older retained boundary, a gap inside the supported range, or a tick outside
the window. Stored snapshots and lookup results are independent copies. A
version-6 entry owns its `MatchState` and `CombatMatchState` together;
`restore_simulation` returns both without changing the legacy soccer-only
`restore` return order.

The history canonically encodes each insertion to maintain exact retained-byte
accounting. Hashes remain lazy and cached: ordinary storage does not hash or
perform another canonical encoding solely for diagnostics, and replacement
invalidates the old cached hash. The final full-time boundary is stored like
any other start-of-tick boundary even though the finished state must not
consume another `InputFrame`.

After snapshot eviction, the session calls
`prune_before(input_history, oldest_retained_tick)`. Input history removes
authoritative, effective, and diagnostic records below that public floor but
keeps one copied authoritative predecessor per slot. Predictions at the oldest
retained boundary therefore preserve held/axis continuity without retaining an
unbounded timeline. Per-slot sorted tick indexes and binary predecessor lookup
make materialization depend on bounded retained history rather than scanning
every historical tick. Diagnostics expose the floor, newest retained tick,
record/sample counts, anchor count, confirmed tick, and pending divergence.

The floor only advances. A valid authoritative arrival below it returns the
recoverable `outside_window` result without mutation. Pruning refuses with
`pending_divergence` if it would discard an unconsumed correction; the caller
must first transfer that divergence to the rollback session. Confirmation
never moves backward, including when already-confirmed records are pruned.

Resimulation can finish before the former predicted present, including by
reaching full time earlier. If corrected simulation ends at boundary `N`, that
boundary is retained but `InputFrame N` was not consumed. The session must
discard snapshot boundaries strictly after `N` with
`snapshot_history.truncate_after(..., N)` and discard effective input/record
ticks greater than or equal to `N` with `input_history.truncate_from(..., N)`.
The input operation preserves authoritative arrivals, prediction indexes,
anchors, and monotonic confirmation. It clears a pending earliest divergence
only when that divergence starts inside the wholly discarded tail. Authority
for a discarded effective tick therefore remains ordinary upstream data and
cannot create a false correction until some later timeline materializes that
tick again.

The snapshot retention floor is also monotonic. Tail truncation moves the
present boundary backward but never makes already-evicted history restorable.
It rejects malformed, missing, or outside-window final boundaries without
mutation. A normal #70 correction follows this storage order:

1. Consume the earliest divergence and restore its retained snapshot.
2. Resimulate, replacing snapshots and effective frames along the corrected
   timeline.
3. Store the corrected final/present boundary `N`.
4. Call snapshot `truncate_after(N)`, then input `truncate_from(N)` if the
   corrected timeline did not return to the former present.
5. Advance input pruning from the snapshot diagnostics
   `oldest_supported_tick`.

A correction whose earliest divergence requires a boundary more than 30 ticks
behind the current simulation tick is unrecoverable in this first policy. It
must produce the explicit late-input failure/desynchronized state defined by
restore/resimulation work; it must not clamp the tick, invent a snapshot,
overwrite current state, or silently ignore the correction. The 30-tick bound
is an initial laboratory decision to make memory, CPU, and failure behavior
measurable, not a production internet latency promise.

## Public operations

- `new(sources)` copies exactly eight canonical `local` / `remote` source rows.
- `add_authoritative(...)` validates, copies, de-duplicates, and detects a
  correction against used input.
- `materialize(history, tick)` produces a complete copied frame plus its
  copied source/status record.
- `authoritative_record(...)` and `record(...)` return copied diagnostics.
- `confirmed_tick(...)` returns the contiguous all-eight-authoritative tick.
- `earliest_divergence(...)` peeks at the current correction batch;
  `consume_earliest_divergence(...)` transfers that boundary to the rollback
  session before resimulation.
- `prune_before(...)` advances the retained floor after snapshot eviction;
  `diagnostics(...)` reports the bounded range and record/sample counts.
- `truncate_from(history, boundary_tick)` removes effective frames and records
  at or after a corrected final boundary without discarding authority.

`sim.rollback_snapshot_history` provides:

- `new(max_rollback_ticks?)` for the fixed-capacity boundary ring.
- `store(history, snapshot)` for insertion, replacement, and deterministic
  eviction keyed by the snapshot's own `input_tick`.
- `lookup(history, tick)` with `present`, `retained`, `missing`, and
  `outside_window` results.
- `boundary_hash(history, tick)` for lazy cached diagnostics.
- `truncate_after(history, boundary_tick)` keeps the named corrected boundary,
  makes it present, and removes only later snapshots.
- `diagnostics(history)` for capacity, retained count/range, and canonical byte
  totals, including count/byte high-water marks captured before tail
  truncation can hide the obsolete predicted timeline.

## Session restore and resimulation

`sim.rollback_session` is the pure coordinator that owns the mutable slot-mode
`MatchState` and optional combat companion; callers receive only canonical
snapshot and output copies. Its
constructor accepts a tick-zero `MatchSnapshot`, the eight input sources, and
an optional rollback-window override. It creates both bounded histories and
stores boundary zero before simulation begins.

Callers insert every authoritative row available for an update with
`add_authoritative`, then call `reconcile` once. The session observes whether
each accepted row differs from the already-consumed effective row before the
history changes, so a batch can count every correction while still restoring
only once from its earliest causal input tick. Equal predictions and
authoritative input for an unconsumed tick are not corrections.

`step` materializes exactly the current input tick, calls `sim.match.step` with
`fixed_clock.TICK_SECONDS`, stores the resulting start-of-next-tick boundary,
and records one immutable output. Each output names its input tick and start/end
boundaries, the copied `RollbackInputTickRecord`, copied match events, final
score/time/finished view, and whether that step reached full time. The output
and input histories are pruned to the snapshot floor after every advance.

`reconcile` first checks terminal status and whether a divergence exists. A
no-op/equal batch and repeated terminal call therefore do not capture or hash
the present. A changed rollback consumes the earliest divergence once, restores
its exact `present` or `retained` boundary, and rematerializes corrected frames
toward the old present. All later authority in the same batch is therefore
applied during that one replay. Corrected outputs are returned in causal tick
order and also replace the session's per-tick output index for confirmed-event
publication.

Per-correction predicted-versus-corrected hashes and the first structural
difference are explicit detailed diagnostics: `reconcile(session, true)`
collects them, while the production default omits that redundant work.
Authoritative boundary comparison and final convergence still always use the
canonical snapshots, hashes, and first-difference report below. A missing
boundary inside the supported range is an invariant failure. An outside-window
arrival enters terminal `late_input_unrecoverable` status without consuming
another retained divergence or changing the live match; diagnostics always
attribute a mixed batch to that actual late input tick. `step` cannot make
hidden progress from the terminal state.

If corrected play finishes before the old present at boundary `N`, the session
stores the final output/boundary, truncates snapshots strictly after `N`, then
truncates effective input records and outputs at ticks `>= N`. Authoritative
tail rows remain upstream facts. Later authority for a discarded tick cannot
be mistaken for a correction until that tick is simulated on a future active
timeline. The snapshot and input floors remain monotonic across this rewind.

`current_snapshot`, `snapshot`, and `output` return independent copies.
`compare` reports actual/expected boundary numbers and hashes, an optional
causal tick, and `match_snapshot.first_difference`. Diagnostics expose both the
monotonic all-input `confirmed_tick` and `confirmed_output_tick`, which caps
confirmation to the logical simulated-output ceiling when corrected full time
drops an authoritative tail. Older confirmed outputs can still be absent from
the queryable output index after ordinary bounded-history pruning.

Prediction counters are cumulative execution costs: replaying a predicted tick
increments them again. `predicted_slot_samples` counts individual predicted
rows, `predicted_ticks` counts executions with at least one predicted row, and
`resimulated_ticks` counts replayed match steps. `rollback_count` counts
successful restores, while `correction_count` counts accepted authoritative
rows that differed from consumed effective rows. Latest/maximum rollback depth
is `old_present_boundary - causal_tick`; late-window failures are counted
separately.

## Stable events and confirmed publication

`sim.rollback_events` keeps presentation identity outside `MatchState` and the
canonical snapshot/replay schemas. A wrapped match event uses the domain
`match/<kind>`; lifecycle domains are `lifecycle/goal`,
`lifecycle/kickoff`, and `lifecycle/full_time`. Its fixed identity encoding
contains the causal input tick, domain length and bytes, and a four-digit
per-domain ordinal. Interleaving another event kind therefore cannot renumber
shots, tackles, or keeper events at the same tick. Actor, position, style,
outcome, team, and post-goal score remain payload rather than identity.

Combat events are authoritative snapshot data and use
`combat/<kind>/<source_sequence>` domains. The restored monotonic source
sequence keeps an action's ID stable when unrelated same-tick events are
inserted or removed; the causal tick still changes the ID when corrected input
moves the action. Rollback outputs and event steps include combat batches only
for combat-capable snapshots, preserving the soccer-only retained payload.
Match, combat, and lifecycle events are diffed in that order.

When corrected play retains an identity but changes its payload, the event
timeline reports one explicit replacement. A changed kind/domain reports a
revocation plus an addition. Additions and replacements follow corrected
tick/event order; revocations follow the stale order. Reapplying an identical
timeline produces no diff. Numeric payload equality uses the canonical
snapshot number encoding, so signed zero and every other finite-number edge
follow state-hash semantics rather than Lua's `==`. All retained event/state
views and all returned diffs/confirmed steps are defensive copies.

Goal, kickoff, and full-time events are derived from canonical pre-step and
post-step snapshots:

- one score increment produces a goal with the scoring team and post-goal
  score;
- an active post-goal state also produces a kickoff for the conceding team;
- the first transition to `finished` produces full time;
- a max-goal step produces goal plus full time but no kickoff, while timer
  expiry produces full time alone.

Opening kickoff is before input tick zero and deliberately has no event.
Lifecycle audio, goal replay, statistics, result flow, and other irreversible
effects wait for confirmation. Consumers may render immediate speculative
feedback only when they can reverse additions, revocations, and replacements.
Confirmed wrappers keep the same IDs that they had speculatively.

The event timeline does not duplicate the session's full snapshot ring.
Supplied post-step snapshots are validated and inspected during `apply`, then
discarded. Each unconfirmed or returned confirmed step retains only the score,
time remaining, finished flag, owner player ID/team, and wrapped events needed
by downstream observers. The stable player-index-to-ID/team mapping is copied
once from boundary zero. At the 30-tick default rollback depth, this keeps the
event seam to at most 30 compact step records rather than another set of
match snapshots.

Confirmation is also the event-retention bound. If an unconfirmed oldest step
would grow the timeline beyond its configured window, `apply` changes the
timeline to terminal `unconfirmed_window_exceeded`, returns that typed error,
and retains the bounded existing steps without accepting part of the new
update. Diagnostics expose status, configured window, confirmed cursor and
boundary, retained step/event counts, and oldest/latest ticks. Callers must
treat this as a laboratory synchronization failure, not silently discard
unconfirmed presentation history.

The call order is part of the contract. A normal update calls
`rollback_events.apply` with the new output and its canonical post-step
snapshot, then calls `confirm` with
`rollback_session.diagnostics(...).confirmed_output_tick`. After arrivals,
call `rollback_session.reconcile`, apply its corrected outputs and snapshot
lookups as one replacement of the complete stale output range, and only then
advance confirmation. A corrected list may end before the stale range when
full time moves earlier; corrected lists are never empty, and an active shorter
timeline is rejected. Apply and confirmation validate their complete requested
ranges before replacing/deleting any retained step, so a caught invariant
failure leaves the cursor and speculative tail unchanged. Correction or
confirmation of an already-confirmed tick, and missing/noncontiguous steps,
fail loudly.

Public operations are:

- `new(initial_snapshot, max_unconfirmed_ticks?)`, defaulting to the session's
  30-tick rollback window;
- `apply(timeline, replaced_from_tick, replaced_through_tick, steps)`, returning
  a diff or typed `unconfirmed_window_exceeded`;
- `confirm(timeline, confirmed_output_tick)`, returning newly confirmed compact
  steps exactly once;
- `diagnostics(timeline)`, returning the bounded status and retention counters.
