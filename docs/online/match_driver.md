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

That is still true, and the driver still has no pre-check. What #241 added is a
different question asked at a different time: not "is this row too old?" but "can
this peer still confirm at all?" — see [confirmation
liveness](#confirmation-liveness-is-checked-not-inferred). The two interact in one
direction worth stating plainly. A row is only offered to the history when it is
*above* this peer's confirmation, so a row below the floor implies
`confirmed + 1 < floor`, which the liveness check terminates on at the end of the
previous step, before any arrival is applied. **The history's `outside_window`
rejection is therefore no longer reachable from an arrival**: the reactive path is
subsumed by a proactive one that is strictly earlier and strictly more precise,
and reports the same `late_input` failure into the coordinator. `late_input`
survives as a driver terminal for the other thing it always meant — the window
overflowing during reconciliation — and the floor rule itself stays pinned where
it lives, in `spec/sim/rollback_input_history_spec.lua`.

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

### The host fans out authority it holds, not traffic it just received

That bound is `SLOT_COUNT * RETAINED_ROWS` for a reason: one host batch is meant
to carry a seven-tick window for **every** slot. That window is the only
redundancy a guest's confirmation has, because the host→guest leg has no
retransmission at all — a row a guest never receives is a row it never gets.

Selecting only the bundles that arrived on *this* transport tick spends far less
of it than the bound implies. A slot whose author's bundle was lost or delayed on
the guest→host leg contributes **nothing** to that batch, even though the host
already holds that slot's rows as authority. The guest→host leg's losses
therefore multiply into the host→guest leg instead of being absorbed by the one
peer that has the rows. Measured on the #241 capture, a row the design fans out
seven times was fanned out five, and the guest that stranded received a batch
*inside* its loss burst whose tick span covered the missing row but which carried
no row for that slot at all.

So the host keeps the newest bundle it has accepted per canonical slot, and after
selection tops the batch up from it for every slot the selection does not already
cover. Those rows are authority the host has already validated and canonicalized;
authorship is a frozen partition, so a re-sent row is byte-identical to the one it
repeats and can never open a divergence.

Two things bound it. The top-up runs **after** selection and only inside the
remaining row budget, so it can never displace a real arrival or push one into the
carried queue — in steady state, when all eight slots delivered, there is no room
and nothing changes. And a retained bundle whose oldest row has fallen below the
host's own retained floor is dropped rather than relayed: authority that old can
no longer be placed in any peer's rollback window, and re-sending it would convert
a silent gap into a spurious `late_input` on a healthy peer.

This repairs the *leak*, not the *bound*. It cannot help when the batch is already
saturated, which is exactly what the `stress` profile does: the harness's
[open `4v4.stress` finding](fault_harness.md#still-open-4v4stress-strands-on-batch-capacity-not-on-a-leak)
records the measurement and the three allocation schemes that were tried against
it and rejected.

## Full time settles before it completes

Reaching full time is not the same as agreeing on it. A sample is authority
`DELAY` ticks after it is taken, so up to `DELAY` ticks of the match can still be
unconfirmed at the moment the final tick is simulated. A driver that terminated
there would report a final hash taken partly over *predicted* authority, and
under a burst straddling full time two peers would stop at different confirmation
depths and disagree about a match in which nothing actually diverged.

So full time opens a **settle phase** instead of terminating. The simulation
stops for good — nothing is authored, nothing is stepped, the retained window
stops sliding — and the driver keeps polling, keeps applying one arrival batch
per step through one reconciliation, and keeps re-publishing the last authored
redundancy window until `confirmed_output_tick + 1` reaches the final boundary.
Only then does it terminate `completed`.

Re-publishing that window is the point of the phase's outbound half. There is no
new sample to author, but the final rows are otherwise carried by fewer packets
than any earlier row — the redundancy window has no ticks left to ride on — which
is exactly why the tail is the part that fails to confirm. Settling gives it the
same protection every other tick got. The host's re-sends go back through its own
collector on the ordinary fairness-delay due date: settling is not a licence to
bypass canonical sequencing.

### The host is the star's relay, so it leaves last

A guest's settle re-sends carry rows it already holds. The rows it is **missing**
belong to other peers and can only reach it inside a canonical host batch, so the
instant the host stops, the star stops. And the host is structurally the *first*
peer to confirm the final boundary — it is the sequencer — so "confirmed,
therefore done" made it leave first, by construction, every time.

That is what stranded `guest_5` in #241: the host completed three steps into a
sixty-step settle window while three of its guests were still settling, and the
row one of them was missing was thereafter unobtainable. It kept re-publishing its
own window into a dead star for the remaining fifty-two steps.

So the host stays while somebody is still asking. A settling guest re-publishes
its window on **every** settle step, which makes inbound input traffic the signal
that the relay is still wanted and silence the signal that it is not: the host
completes once its own final boundary is confirmed **and**
`SETTLE_RELAY_QUIET_STEPS` (4, the fairness delay plus one) consecutive settle
steps have brought no input traffic at all. A clean match costs the host those
four steps — 67 ms — and a match with a straggler keeps the relay alive exactly as
long as the straggler keeps asking.

It introduces no new wait. The phase's two existing deadlines still end it, and
they no longer decide the status: a peer whose own final boundary is confirmed
completes when the phase expires, relaying or not. `settle_timeout` therefore
keeps meaning exactly one thing — the phase expired with *this peer's own* final
boundary still unconfirmed.

Under clean delivery the guests still cost nothing: confirmation already runs
ahead of the present, so a guest settles one step after full time, when the
fan-out carrying the final row arrives.

**It is bounded twice, and waits on nothing else.** It ends after
`SETTLE_TIMEOUT_TICKS` (60) further driver steps, or after
`SETTLE_TIMEOUT_SECONDS` (2) of monotonic wall clock for a caller whose frames
have stopped arriving at 60 Hz, whichever comes first, with the typed terminal
`settle_timeout`. Both deadlines are fixed the moment full time is reached and
re-checked exactly once per `advance`. The tick bound is a liveness choice, not a
correctness one: nothing is simulated while settling, so waiting can never push
the tail out of the retained window.

Those bounds are now **measured** rather than reasoned. The #169
[fault harness](fault_harness.md) drives the documented profiles from
`data/network_profiles.lua` through `sim.network_conditions`. The same campaign
found the two #241 stalls this document now describes — a tail stall the host's
early exit made unrecoverable, and a mid-match stall confirmation liveness now
reports — and neither of them was a bound that needed raising. Across the whole
declared matrix the worst settle a peer actually completed from is well inside the
60-step bound.

It is deliberately **not** gated on hash agreement. The driver cannot see another
peer's hash, so waiting for one would be an unbounded wait on a fact that never
arrives — and it would be the wrong instrument anyway. A disagreement reported
through `observe_checkpoint` while settling terminates `hash_mismatch` exactly as
it does while playing; settling never swallows a divergence, it only refuses to
report a result over authority it has not confirmed. A genuinely divergent peer
therefore still settles, and still carries its own divergent final hash into
`coordinator.apply_result_ack`, which ends the session as `hash_mismatch`.

Boundary hashes stop at full time, including on the step that reaches it.
Settling peers no longer terminate on the same step, so a checkpoint published
while one peer settles could reach another that has already left `running` for
the result — and a hash report is only legal in a running session. It costs no
detection: the acknowledged final hash is a function of every confirmed tick, so
a boundary that disagreed anywhere cannot agree there.

`match_driver.settled` is the settled boundary as a predicate. Presentation keys
the final whistle off it rather than off the tick the simulation stopped on, so
the visible whistle and the confirmed result agree.

## Terminal statuses

The driver ends once, with a reason, and makes no hidden progress afterwards: a
further `advance` polls nothing, sends nothing, applies nothing, and simulates
nothing.

| Status | Reported to the coordinator as | Raised when |
| --- | --- | --- |
| `completed` | — | Full time was reached *and* the final boundary confirmed. |
| `settle_timeout` | `input_channel` | Full time was reached, but the final boundary was still unconfirmed when the settle phase expired. |
| `confirmation_stalled` | `late_input` | An unconfirmed tick fell below the retained floor, so confirmation can never advance past it again. |
| `late_input` | `late_input` | The rollback window overflowed while reconciling. |
| `hash_mismatch` | `desync` | Boundary hashes disagreed at `MAX_HASH_MISMATCHES` consecutive checkpoints. |
| `ownership_violation` | `input_channel` | A peer authored a slot outside its frozen owned set. |
| `authority_conflict` | `input_channel` | Two bundles claimed one `(slot, tick)` with different bytes. |
| `input_channel_failure` | `input_channel` | Queue overflow, backpressure, a malformed bundle, or an unsendable batch. |
| `transport_lost` | — | The star or a frozen link is no longer connected. |

`late_input` and `hash_mismatch` remain causally distinct here even though #161's
wire folds both onto `desync`; the local reason stays exact. `settle_timeout` is
distinct from both for the same reason and a sharper one: a tail that never
arrived is a delivery failure, and reporting a healthy match's missing final rows
as a desync is precisely the mislabelling the settle phase exists to end.

### Confirmation liveness is checked, not inferred

`confirmation_stalled` is the fourth member of that set and the reason it exists
is that confirmation could previously stop advancing *permanently* with nothing
raised at all.

`rollback_input_history` confirms tick `N` once all eight of its rows are
authoritative, and it prunes below the retained floor as the present boundary
slides. Those two are independent, and nothing stopped the floor from passing a
tick that never got its eighth row. When it did, that tick's authority was deleted
outright, and because confirmation only ever advances from `confirmed_tick + 1`,
it could never cross the hole again — not later in the match, not ever. The peer
went on simulating on authority it would never confirm.

Nothing detected it. The `late_input` path is **reactive**: it fires when a row
*arrives* below the floor. A row that simply never arrives trips nothing. So in
#241 a `4v4.playable` guest lost the seven host batches carrying one row, kept
predicting, lost the tick to the floor 30 steps later, and ran another 66 ticks
before surfacing — 60 settle steps after full time — as `settle_timeout` with a
divergent final hash, reported by a mechanism that exists for a lost peer.

`confirmed_tick + 1 < oldest_retained_tick` is exactly that state, it is
permanent, and it is one comparison at the end of every `advance`, checked before
settling so a peer whose confirmation is already dead reports the reason that is
true rather than waiting out a phase that would describe a different fault. The
three stay sharply separable, which is what #169's harness and #171's evidence
gate need: `settle_timeout` is a tail that did not arrive in time,
`hash_mismatch` is peers disagreeing about a tick they both confirmed, and
`confirmation_stalled` is a peer that can no longer confirm at all.

It is a report, not a repair. A peer in this state needs authority it can no
longer be sent, and recovering it needs the snapshot resync deferred to OMP-5.
What the fan-out and settle-relay fixes above do is stop the state from being
reached; this is the check that says so out loud when it is.

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
