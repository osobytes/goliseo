# OMP-3 online match driver

`game.online.match_driver` is the game-layer controller that turns real transport
arrivals into simulation. It polls a `StarTransportAdapter`, authors the rows its
peer owns, sequences host authority through [#162's input
bundles](input_packets.md), feeds arrivals into the OMP-2
[rollback session](rollback_policy.md), advances the fixed 60 Hz clock, keeps the
live-slot timeline, publishes boundary hashes, and ends with one typed terminal
status.

It is the only module that holds both a transport and a rollback session.
`sim/`, `data/`, and `core/` stay free of WebRTC, browser, and LÖVE
dependencies: everything transport-shaped enters through the injected adapter,
and the fake in-process star drives every test here without a browser.

It does not run the lobby, draw anything, migrate a host, resync a snapshot, or
change bot policy.

## Three clocks

| Clock | Meaning |
| --- | --- |
| driver step `T` | One `advance` call; one 60 Hz frame. |
| transport tick | `T + DELAY` on the wire. |
| input tick | `first_input_tick + T`, the authority tick simulated during step `T`. |

`DELAY` is `input_protocol.FAIRNESS_DELAY_TICKS`, three ticks. **A sample taken
during driver step `T` is authority for input tick `first_input_tick + T +
DELAY`.** That is the documented input delay, and the host pays it exactly like a
guest: the host's own bundles are queued in its collector and
`canonical_host_batch` refuses to read them until three transport ticks have
passed, so the host cannot bypass canonical sequencing. The wire clock is offset
by `DELAY` for one reason only — so the pre-start rows the host must have before
its first simulated tick can still spend the full fairness delay in the
collector.

The first `DELAY` input ticks carry neutral human rows on every peer. Nobody had
sampled anything before the start boundary, so there is nothing else they could
honestly carry.

## Authorship is a frozen partition

Ownership validation is **set membership**, not equality:

- a peer authors **every slot in its frozen owned set** — the human sample on its
  control slot, deterministic AI rows on the rest;
- the host additionally authors every declared bot fill;
- nobody may author anything else, and a bundle naming a slot outside the
  sender's frozen owned set ends the match as `ownership_violation`.

Because authorship is a partition of the eight canonical slots, two peers can
never author the same `(slot, input tick)`. Conflicting authority is therefore a
detectable fault rather than a routine race.

A full lobby covers all eight slots in every supported mode — 1v1 seats two
humans on four slots each, 2v2 four humans on two each, 4v4 eight humans on one
each — so **a declared bot fill only exists when fewer humans are seated than the
mode allows**. Keepers stay AI-only and slotless in every mode.

### Deterministic AI rows

Every slot a peer authors that is not its control slot gets a deterministic AI
row from `sim.slot_input`, seeded from the freeze: a declared fill uses its
frozen `bot_seed`, and a human's non-live owned slot uses a seed derived from the
frozen match seed and the canonical slot index. The bot stream advances once per
authored tick per slot regardless of which slot is live, so the stream is a
function of the tick count alone and never of the live-slot history.

These rows are indistinguishable from declared bot fills in the input stream,
which is exactly how solo play already treats the players you are not
controlling. They are authored once, by their single owner, and are authority
from then on — a later rollback does not re-derive them, any more than it
re-derives a human's keypress.

## The live slot

`live(N)` is the slot a human is controlling at input tick `N`:

```text
live(first)  = freeze.live
live(N + 1)  = coordinator.next_live_slot(owned, live(N), transition(N))
```

`transition(N)` is built by `game.online.live_slot` from the boundary `N + 1`
simulation state, plus the canonical `switch` edge of the row the human's bits
are actually on. Nothing local — presentation, frame rate, or when a key was
physically pressed — reaches the decision, so every peer evaluates the same
transition at the same tick. A rollback that replaces ticks `[a, b]` replaces
`live(a + 1 .. b + 1)` with them, so the timeline is corrected by exactly the
same mechanism as the simulation.

`control_slot(N) = live(N - DELAY)`, clamped to the opening live slot before the
delay has elapsed. Because a sample is authority `DELAY` ticks after it is taken,
that is the row a human's bits land on — and it is itself a pure function of the
same timeline, so every peer computes it, not only the author. The `switch` edge
is read from `control_slot(N)`'s row for the same reason: reading `live(N)`'s row
would read an AI row whenever the two differ, and switching would silently stall.

### Why the ranking is the risky part

`coordinator.next_live_slot` is provably deterministic *given* a deterministic
`ranked` ordering — it only ever returns the winner, a member of `ranked`, or the
incoming live slot, each gated by owned-set membership. It consumes that ordering
rather than computing it. `game.online.live_slot` computes it, and two properties
are load bearing:

1. **The ranking is total.** Slots are compared by squared distance to the ball
   and then by ascending canonical slot index. Canonical indexes are distinct
   integers, so no two entries compare equal in both keys. A strict total order
   makes `table.sort`'s result unique regardless of its pivot choices or of
   insertion order, so two slots at *exactly* equal distance resolve identically
   on every peer. A tiebreak that fell through to table order would be a desync.
2. **No `pairs()` anywhere in the ranking path.** Every table is walked with a
   numeric `for index = 1, SLOT_COUNT` loop or `ipairs`. Lua hash order is the
   classic cross-peer divergence source, and two peers inside one process share a
   hash seed and would agree by accident — so the absence is asserted against the
   source itself, not inferred from a passing run.

Squared distance is used instead of `Vec2:dist`: it is the same monotone ordering
with one fewer rounded operation, so the comparison cannot depend on how a square
root rounds.

This failure mode **cannot appear in 4v4 at all** — singleton owned sets make
every branch of `next_live_slot` return the slot already live — so coverage
asserts live-slot identity per confirmed checkpoint in 1v1 and 2v2 specifically,
alongside an explicit exact-tie fixture.

## One arrival batch, one reconciliation

Each driver step:

1. drains transport events, mapping terminal ones onto a typed status;
2. polls **every** envelope available on that transport tick, keeping the input
   ones and handing control-channel traffic back on the batch — one transport
   carries both channels and the reliable control channel belongs to the session
   coordinator, so draining and discarding it would eat the coordinator's mail;
3. host: validates each sender against its frozen owned set, adds its own due
   collector bundles, and canonicalizes one `canonical_host_batch`;
   guest: decodes every host batch, unions the rows, and sorts them into
   canonical `(tick, slot)` order;
4. applies that union through `rollback_session.apply_authoritative_batch`,
   which preflights atomically and reconciles **exactly once**;
5. authors this step's rows, records them in the redundancy window, and
   publishes them. A guest also inserts its own rows locally, but through
   `add_authoritative_batch` with no reconciliation pass: its own rows are
   always for a tick it has not simulated yet, and the redundant re-sends behind
   them are byte-identical duplicates, so they cannot open a divergence. The
   guarantee is checked rather than assumed — if a local insert ever did report
   one, the reconciliation runs;
6. steps the fixed clock to the next boundary and extends the live timeline;
7. hashes any confirmed checkpoint that came due.

Poll and callback order is recorded, never used as authority. Stepping the peers
in reverse order produces byte-identical confirmed boundaries.

### The retained floor has one owner

`sim.rollback_input_history` owns the 30-tick floor and rejects
`tick < oldest_retained_tick` with `outside_window`; the driver maps that onto a
`late_input` terminal and does **not** duplicate the check. An earlier revision
pre-checked the floor in the driver, which turned out to be provably redundant:
the driver reads the same `oldest_retained_tick`, and every row that survives the
"already confirmed, skip it" filter and is below the floor is necessarily the
lowest tick in the batch — valid rows are all at or above the floor — so a
pre-check and the history's own rejection terminate on the same batch and
attribute the same causal tick. One owner is better than two agreeing owners.

### The bounded batch and carry-over

One host batch carries at most `MAX_HOST_ROWS` = 56 distinct `(tick, slot)` rows,
which is exactly eight slots times the seven-row redundancy window. Steady state
fits exactly. A delivery burst wider than that window does not, so the excess is
**carried to the next transport tick** rather than dropped — dropping would
strand authority the peers still need to confirm, and emitting two batches on one
tick would break the one-batch-one-reconciliation contract. Selection is sorted
by `(host-local first, transport tick, sender, sequence)` and then taken greedily,
so it is independent of poll order. The host's own collector path is never
deferred: its slots are the ones its own materialization requires to be
authoritative, so starving them would stall the host on its own input. The
carried queue is bounded; exceeding it is `input_channel_failure`.

## Terminal statuses

The driver ends once, with a reason, and makes no hidden progress afterwards: a
further `advance` polls nothing, sends nothing, applies nothing, and simulates
nothing.

| Status | Reported to the coordinator as | Raised when |
| --- | --- | --- |
| `completed` | — | Full time was reached. |
| `late_input` | `late_input` | Unconfirmed authority older than the retained 30-tick floor, or the window overflowed while reconciling. |
| `hash_mismatch` | `desync` | Boundary hashes disagreed at `MAX_HASH_MISMATCHES` consecutive checkpoints. |
| `ownership_violation` | `input_channel` | A peer authored a slot outside its frozen owned set. |
| `authority_conflict` | `input_channel` | Two bundles claimed one `(slot, tick)` with different bytes. |
| `input_channel_failure` | `input_channel` | Queue overflow, backpressure, a malformed bundle, or an unsendable batch. |
| `transport_lost` | — | The star or a frozen link is no longer connected. |

`late_input` and `hash_mismatch` remain causally distinct here even though #161's
wire folds both onto `desync`; the local reason stays exact.

Boundary hashes are published every `DEFAULT_HASH_INTERVAL_TICKS` (30) confirmed
boundaries, starting at `first_input_tick`. The boundary comes from the
session's `confirmed_output_tick`, **never** the raw `confirmed_tick`: a sample
is authority up to `DELAY` ticks before it is consumed, so raw confirmation can
name a boundary that was never captured, and a checkpoint landing there aborts a
perfectly healthy match. `confirmed_output_tick + 1 <= present_boundary` is
exactly the guarantee a snapshot lookup needs, and it holds *by construction*
rather than by observation: `rollback_session.diagnostics` computes
`confirmed_output_tick = math.min(confirmed_tick, present_boundary - 1)`, so the
bound is unconditional for any raw `confirmed_tick`, however far ahead it runs. `apply_rows` still uses the raw
confirmation, which is correct there: it is asking about input-authority
completeness, not snapshot availability. Each checkpoint carries the live slot
of every human at that boundary alongside the hash, because the timeline is
pruned with the rollback window and a checkpoint is exactly the boundary at which
peers must agree on both. A single disagreement is tolerated and cleared by the
next agreement, matching the coordinator's own hash policy. Snapshot resync
stays deferred to OMP-5.

## Fixtures and what is still contingent

`game.online.match_driver_fixture` pins a slot-mode boundary zero, a frozen
session rebuilt from the same public pieces `coordinator.begin_countdown` uses,
and a connected in-process star. Producer ids double as transport link ids —
`canonical_host_batch` requires a peer producer to arrive on its own selected
link — so they satisfy `game.transport.contract.PEER_ID_PATTERN`, which the
session coordinator's own test ids deliberately do not.

The driver's *mechanical* behaviour is verified against those fixtures. Two
acceptance criteria remain contingent on open work and are not claimed here:

- **#112, combat-aware gameplay AI.** Non-live owned slots and declared fills
  currently materialize `sim.slot_input`'s existing deterministic bot, which is
  not combat aware. 1v1 leans on this hardest: three of every human's four owned
  slots are AI-driven at any instant. The plumbing — seeding, single authorship,
  indistinguishability from a declared fill — is what is proven now.
- **#114, the accepted default combat disposition.** The manifest identity the
  fixture pins is a fixture, not an accepted policy. When #114 lands, the
  identity changes in the fixture and nowhere else.

The combat companion is a third, separate line, and it splits in two.
`fixture.initial_snapshot(duration, true)` opts boundary zero into the combat
snapshot schema (`match_snapshot.COMBAT_VERSION`), and the
`carries the combat companion through correction and resimulation` spec drives
that fixture through bursty delivery so restore and resimulation carry
`CombatMatchState` alongside `MatchState` on every peer. The **mechanism** is
therefore proven: the companion survives a correction.

The **behaviour** is not. The rows are still produced by the pre-#112 bot, which
never drives the companion into wind-up, guard, contact, projectile flight,
stagger, ball spill, or immunity expiry, so none of the combat correction cases
the issue names are individually pinned. That is a distinct gap from "the AI is
not combat-aware yet" — it is what the combat-aware AI would be needed to
*test*, not merely what it would change — and those scenarios are worth pinning
only once #112 can produce them.
