# OMP-3 online match driver

> **Partly pre-port (LÖVE/Lua).** The contract this document describes is
> current, but it still names the Lua tree that commit `2c0d449` (#467) deleted.
> Read `sim/foo.lua` as `rust/crates/gc-sim/src/foo.rs`, `data/foo.lua` as
> `rust/crates/gc-data/src/foo.rs`, `sim.foo` as `gc_sim::foo`, and `game/**` /
> `spec/**` as `ts/packages/**`. Any `love .` command, `love.*` API, or
> `file.lua:LINE` citation is **pre-port evidence**, not something you can run
> or open. The live tree is described by `ARCHITECTURE.md`.

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

One host batch carries at most `MAX_HOST_ROWS` = 72 distinct `(tick, slot)` rows:
eight slots times a nine-tick window. Steady state — eight bundles at a full
seven-row window each — is 56 rows and fits with headroom. A delivery burst wider
than that does not, so the excess is
**carried to the next transport tick** rather than dropped — dropping would
strand authority the peers still need to confirm, and emitting two batches on one
tick would break the one-batch-one-reconciliation contract. Selection is sorted
by `(host-local first, transport tick, sender, sequence)` and then taken greedily,
so it is independent of poll order. The host's own collector path is never
deferred: its slots are the ones its own materialization requires to be
authoritative, so starving them would stall the host on its own input. The
carried queue is bounded; exceeding it is `input_channel_failure`.

### Where the 72 comes from

It used to be `SLOT_COUNT * RETAINED_ROWS` = 56, read as "the guest's own retained
window, eight times". That was a design intent, and #243 measured that it was
never the transport limit it was assumed to be: a maximally-full 56-row batch
encoded to 755 of the 1,024 available bytes.

The sizing is now the measurement. Each row costs exactly 12 base64 bytes on top
of a 92-byte worst-case header, so **77 rows is the hard ceiling** at 1,018 bytes
and a 78th is refused `wire_too_large`. Nine ticks — 72 rows, 958 bytes — is the
largest whole-slot-window sizing under that ceiling, and it leaves 66 spare bytes
that `input_protocol_conformance` pins against `MIN_WIRE_MARGIN_BYTES` rather than
leaving for the next person to rediscover. Ten ticks would need 80 rows and does
not fit.

The 16 rows above steady state are not spare capacity for its own sake. They are
what the targeted repair below spends, and raising the bound **without** the
repair was measured and does not close `4v4.stress`: on a 48-seed sweep it moved
the failures from 18 to 12 and flipped one previously-green seed red. Capacity
alone moves the threshold; it does not remove the failure mode.

### The host fans out authority it holds, not traffic it just received

One host batch is meant to carry a seven-tick window for **every** slot. That
window is the only redundancy a guest's confirmation has, because the host→guest
leg has no retransmission at all — a row a guest never receives is a row it never
gets.

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

This repairs the *leak*, not the *bound*. It is open-loop: it re-sends a slot's
newest window on the chance that some guest missed it, and it has nothing to say
about a row that has already aged out of that window.

### The host repairs what a guest says it is missing

Every bundle a guest sends now carries `confirmed_span` — where its own
confirmation has actually reached. See
[input packets](input_packets.md#confirmation-feedback) for the field. It is
additive, costs at most eleven header bytes, and introduces no round trip: the
guest states a fact it already knows and never waits on an answer.

What it buys is that the host stops fanning out blind. Confirmation advances only
from `confirmed_tick + 1`, so a guest stuck at `C` is blocked on exactly tick
`C + 1`; every later tick it received is already sitting in its history,
authoritative and unusable, and confirms in one step once the hole is filled. The
host takes the **minimum** reported confirmation across every author it has heard
from and re-sends a contiguous span of ticks from there, read straight out of its
own retained authority.

Three conditions gate it, and each is load-bearing:

| Condition | Why |
| --- | --- |
| `tick <= host_confirmed` | The host must hold all eight rows of a tick, or it would be inventing some. A tick short even one slot ends the span rather than being sent partial — a partial tick cannot unblock confirmation. |
| `tick >= retained floor` | Below the host's own floor the rows are gone, and a guest that far behind is already `confirmation_stalled` on its own side. |
| `host_confirmed - needed > HISTORY_ROWS` | Inside the redundancy window the open-loop fan-out is still re-sending that tick every transport tick. Only once the window has moved past it has blind redundancy provably failed. This is what keeps the mechanism **off** in a healthy match. |

It is a span (`REPAIR_SPAN_TICKS` = 4) rather than the single blocking tick, and
that is a measurement rather than a preference. A guest's report reaches the host
about three transport ticks after its confirmation moves and the repair takes
several more to land, so single-tick repair discovers the next tick roughly six
steps later — while the guest's retained floor climbs one tick *per* step. One
tick recovered per six steps against a floor eating one per step is a race the
guest loses. Captured on the default seed: single-tick repair walked the frontier
9 → 10 and was still waiting for the report when the floor passed 11, and the
guest that stranded was missing exactly two rows.

**The allocation order is the policy.** Arrivals first, because they carry
authority nobody else holds. Repair second. The open-loop top-up last. Repair
outranking the top-up is the whole point — once a guest has *said* where it is
stuck, spending the same rows on a guess is strictly worse than spending them on
the answer, and with the top-up taking the headroom first the repair was left 6 to
16 rows and `4v4.stress` stayed red. Repair never outranks arrivals, because
displacing one defers it a transport tick and delays fresh authority for every
peer to help one; reserving repair budget ahead of selection was measured too, and
is worse at every span tried.

This is what closed the harness's `4v4.stress` row. See
[the fault harness](fault_harness.md) for the seed sweep.

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

So the host stays while somebody is still asking. Since #243 it can *know* that
rather than infer it: every settling guest reports its own confirmation in the
bundle it re-publishes each step, so the host completes once its own final
boundary is confirmed **and** every author it has heard from has confirmed the
final boundary too. That is a bound, not a heuristic.

It introduces no new wait. The phase's two existing deadlines still end it, and
they no longer decide the status: a peer whose own final boundary is confirmed
completes when the phase expires, relaying or not. `settle_timeout` therefore
keeps meaning exactly one thing — the phase expired with *this peer's own* final
boundary still unconfirmed.

#### Silence never pre-empts a peer's own report (#255)

There used to be a second way out of the host branch: `SETTLE_RELAY_QUIET_STEPS`,
four consecutive settle steps (the fairness delay plus one) with no inbound input
traffic at all, tested *before* any report was read. It is gone. **Evidence beats
inference: the report loop is now the only thing that decides whether the tail is
delivered.**

The decision is worth recording, because the obvious reading of the quiet count —
"the fallback for a peer that never speaks at all" — was wrong twice over, and a
future reader who re-derives it will otherwise re-add it. `tail_delivered`'s loop
treats a `nil` entry in `_peer_confirmed` as *not known to be behind*, so a peer
that has never reported is already covered without any escape hatch. And
`remember_confirmation` runs on every host tick of the whole match, so any guest
that sent even one packet before settle began already has a non-`nil` entry — a
peer with zero reports ever is close to unreachable in practice.

So the only situation the quiet count could still change was the opposite one: a
peer that **did** report itself behind and then went quiet, where four silent
steps tripped the short-circuit ahead of that peer's own report. That is silence
overriding evidence rather than standing in for evidence that never came, and it
made it an override, not a fallback. Reordering the two checks would have made it
unreachable, so retiring it was the honest form of the fix rather than a cleanup
alongside it. Two smaller reasons point the same way: the counter was match-wide
rather than per-peer, so "a peer went quiet" was only literally what it measured
in 1v1 or once every other peer had completed; and it was the one heuristic in a
phase whose other bounds are hard.

**A clean match paid nothing for this; a lossy one pays the host's whistle.**
Measured on the fault harness's clean rows before and after, every peer in 1v1,
2v2 and 4v4 settles in 2 steps either way — since #243 the reports arrive on the
bundles already in flight, so the quiet window was never the binding constraint on
a healthy match. On impaired rows the host's wait grew, in some seeds to the whole
settle window; the before-and-after figures and what they revealed are in
[What the settle bounds cost, measured](#what-the-settle-bounds-cost-measured)
below, and they are the most useful thing this change produced. Guests are
unaffected on every row.

What changed in behaviour is the one case the quiet count used to decide: a peer
that reported behind and then vanished is now bounded by `SETTLE_TIMEOUT_TICKS` /
`SETTLE_TIMEOUT_SECONDS` instead of by four steps. It waits longer before
reporting the same typed terminal, and nothing became unbounded.
`spec/game/online_match_driver_spec.lua` pins exactly that case.

### A guest leaves last too, for the mirror-image reason

The host is not the only peer holding something nobody else has. A guest's
authored rows for the final ticks exist **nowhere else** until the host has them,
and the host is the only peer that can fan them out to the other six. A guest that
confirms its own boundary and leaves therefore takes authority with it.

That is `4v4.stress` seed 4738, which #243's repair exposed by closing the
confirmation stall that had been failing the row earlier: `guest_7` completed
while input ticks 115 and 116 of its slot had never reached the host, and the
remaining seven peers — the host included — spent the whole settle window missing
exactly those two rows and then timed out. It is #241's tail stall in the opposite
direction.

The host's batches carry the host's own `confirmed_span`, so a guest can see that
the sequencer is still behind the final boundary and keep re-publishing until it
is not. Silence is not consent: a guest that has never heard a host batch has no
evidence its tail landed and must keep publishing.

**Which** window it re-publishes matters as much as whether it does. One bundle is
a fixed seven rows ending at the tick it names, so re-sending the newest one — the
obvious choice — carries nothing useful once the host is more than a window
behind. On seed 4738 `guest_7` re-published `[118, 124]` sixty times while the
rows nobody held were 115 and 116. The window is therefore anchored just above the
host's reported confirmation, capped at the newest authored tick, so a caught-up
host still gets the ordinary newest window and a healthy settle is unchanged.
Authored samples are retained down to the rollback floor rather than to the seven
ticks one bundle carries, so an older window can still be built at all.

Neither wait is unbounded. The settle deadline expires both, and a peer that has
confirmed its own boundary completes at expiry rather than failing, so waiting
here can delay a whistle but can never invent a `settle_timeout`.

Under clean delivery the guests still cost almost nothing: confirmation already
runs ahead of the present, so a guest settles two steps after full time — one for
the fan-out carrying the final row, one for the batch reporting the host's
confirmation back. `spec/game/online_match_driver_spec.lua` pins both bounds so
they cannot quietly grow.

**It is bounded twice, and waits on nothing else.** It ends after
`SETTLE_TIMEOUT_TICKS` (60) further driver steps, or after
`SETTLE_TIMEOUT_SECONDS` (2) of monotonic wall clock for a caller whose frames
have stopped arriving at 60 Hz, whichever comes first, with the typed terminal
`settle_timeout`. Both deadlines are fixed the moment full time is reached and
re-checked exactly once per `advance`. The tick bound is a liveness choice, not a
correctness one: nothing is simulated while settling, so waiting can never push
the tail out of the retained window.

#### What the settle bounds cost, measured

Those bounds are now **measured** rather than reasoned. The #169
[fault harness](fault_harness.md) drives the documented profiles from
`data/network_profiles.lua` through `sim.network_conditions`. The same campaign
found the two #241 stalls this document now describes — a tail stall the host's
early exit made unrecoverable, and a mid-match stall confirmation liveness now
reports — and neither of them was a bound that needed raising.

**The 60-step bound is genuinely exercised, and since #255 it is reached.** A
guest's worst completed settle across the declared matrix is 23 steps, and every
guest figure is unchanged by #255. The *host* is the peer that moved, because it
is the one waiting on other peers' reports. Measured over `4v4` seeds 4703–4750,
48 seeds per row, before and after #255:

| Row | host mean, before → after | host worst, before → after | host completed at the 60-step bound |
| --- | ---: | ---: | ---: |
| `4v4.clean` | 2.00 → 2.00 | 2 → 2 | 0/48 → 0/48 |
| `4v4.omp0_parity` | 9.10 → 9.10 | 10 → 10 | 0/48 → 0/48 |
| `4v4.playable` | 12.52 → 15.21 | 18 → **60** | 0/48 → 3/48 |
| `4v4.stress` | 24.48 → 32.42 | 32 → **60** | 0/48 → **12/48** |

A quarter of `stress` seeds now settle for the full second rather than well
inside it. Every one of those 192 runs still ends `completed` on an agreed final
hash — the bound is where the *whistle* lands, not a failure — but the earlier
claim that the worst real settle sits comfortably inside the bound is false as of
#255 and is corrected here rather than left standing.

**What the measurement revealed is worth more than the number.** The exposure the
quiet count was covering is not silence, it is *stale evidence*. Under loss a
guest's last confirmation report can be the one that is dropped, so the host goes
on holding a report saying that peer is behind after the peer has already
confirmed, completed and gone quiet. Nothing further will ever arrive to update
it, so the host relays to the deadline. That is why removing the quiet count is
free on a clean match and costs a full settle window on a lossy one, and it is
the fact a future reader touching this phase needs: shortening this wait again
means giving a peer a way to *say* it is finished — a departure report — not
re-deriving a way to infer it from silence.

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

The driver's *mechanical* behaviour is verified against those fixtures. Of the
two acceptance criteria that used to be contingent on open work, one still is
and is not claimed here; the other has closed:

- **#114, the accepted default combat disposition.** The manifest identity the
  fixture pins is a fixture, not an accepted policy. When #114 lands, the
  identity changes in the fixture and nowhere else.
- ~~**#112, combat-aware gameplay AI.**~~ **Closed.** Non-live owned slots and
  declared bot fills materialize `gameplay_ai/combat/v1` (`sim.combat_policy`)
  through `sim.slot_input`'s `bot_combat_signals`, reached from this driver at
  `materialize_authored`. It is the same policy, observation schema, option
  ordering, and reason vocabulary the gameplay match AI runs. 1v1 leaned on this
  hardest — three of every human's four owned slots are AI-driven at any instant
  — and those slots now use and counter all four families. (`sim.match`'s
  `_ai_combat_inputs` is the *offline* path and is structurally dead online: in
  slot mode `is_human_player` is true for every slotted player, so its AI branch
  never fires. `sim.bot` is a third population, the human-proxy evidence driver,
  and stays deliberately combat-incapable behind its own policy id.)

## Combat corrections

The combat companion splits into a mechanism and a behaviour, and both are now
pinned.

The **mechanism**: `fixture.initial_snapshot(duration, true)` opts boundary zero
into the combat snapshot schema (`match_snapshot.COMBAT_VERSION`), and the
`carries the combat companion through correction and resimulation` spec drives
that fixture through bursty delivery so restore and resimulation carry
`CombatMatchState` alongside `MatchState` on every peer.

The **behaviour**: the seven combat correction phases #166 names — wind-up,
guard, contact, projectile flight, stagger/knockback, ball spill, and immunity
expiry — are each pinned as their own online rollback scenario in
`spec/game/online_match_driver_spec.lua`
(`converges a correction taken during <phase>`). Kickoff, possession change and
full time were already covered by whole-match runs under impaired delivery; these
are the combat half of the same matrix.

The fixtures live in `spec/support/online_combat_phases.lua`, which owns the
per-phase boundary zero, the live slot's input program, and the predicate that
recognises the phase on a resimulated tick. What each scenario asserts is not
that combat happened somewhere during a bursty run, but that a correction
arriving **while a peer is in that phase** converges: the tick the rollback
resimulated really was a tick of that phase, the peers that resimulated it landed
on one identical combat-bearing snapshot hash, every confirmed boundary hash and
live slot still agrees, and no peer terminated on invalid or duplicate authority.
The schema version is read from `match_snapshot.COMBAT_VERSION` and deliberately
not written down here — it has moved several times and a copied number goes stale
silently.

A scenario counts a phase only on a tick the correction actually re-derived.
`batch.outputs` mixes the reconciliation's `corrected_outputs` with the ordinary
forward tick `step_to` appends on the same call, and nothing in
`RollbackTickOutput` labels them, so the spec separates them by boundary: a
corrected tick is strictly below the present as it stood before the call, because
reconciliation restores to the divergence and resimulates back to the same
present; a forward tick is that present. The separation was validated against
instrumented ground truth from `rollback_session.reconcile` — 4,334 outputs across
the seven scenarios, zero disagreements — rather than argued for, and the spec
re-checks at runtime that every reported rollback yields at least one corrected
tick.

Five of the seven reach their phase through `gameplay_ai/combat/v1` itself: the
scenario arranges only the equipped families and the opening pose, and the
decision, the commit and the resolution are the shipped policy's. **Guard is the
exception.** The policy raises a guard only while it can attribute a telegraphed
hostile path to a purpose target (`combat_feasibility` guard feasibility), which
needs the threat re-armed inside the deciding player's scan cadence;
`spec/sim/combat_ai_match_spec.lua` arranges that by re-pinning the hostile every
tick, and a driver-level scenario cannot, because boundary zero is the only thing
it controls. So the guard scenario raises its guard from a live slot's own held
equipment, carried on the canonical input stream like any other authority. No
scenario force-sets a combat runtime field: every boundary zero opens `ready`,
which `opens every combat phase scenario from a ready combat state` pins
directly.

That exception is itself evidence rather than an assertion.
`combat_phases.GUARD_PROBE` holds four driver-level geometries — guard against
light melee, against an unarmed scrum, and against ranged fire in both a spread
and a single-lane pose — and
`finds no driver-level geometry where the policy guards often enough` runs all
four with no human input at all, counting the same thing a phase scenario counts.
Ranged is in there on purpose: a latched release projects a public path out to
the projectile's whole 60-tick lifetime, against roughly 17 ticks for light
melee's wind-up plus active window, so it is the threat most likely to still be
readable when a slow scanner's decision tick comes round
(`combat_intent.should_decide` runs on a 9-tick period at the fast refresh and 27
at the slow one).

The result is about **rate, not possibility**, and the distinction matters. The
policy *does* guard online — `vs_unarmed_scrum` produces exactly one commit in
240 steps, and the other three geometries produce none — but one commit is not a
scenario, and it is not even a stable one: it disappears when the burst period
changes, because a bot fill authors from the state it currently predicts, so
delivery timing moves the AI's own decisions. The bar is
`combat_phases.GUARD_POLICY_ROUTE_MINIMUM`, set to the margin the thinnest
scenario that actually shipped runs on, so promoting guard could never mean
adopting a weaker scenario than the ones already trusted. When a geometry clears
it the test fails and `guard` moves to the `policy` route. Whether the guard
family is *usable enough* by AI-driven slots is a calibration question for
#149/#114, not a netcode one.
