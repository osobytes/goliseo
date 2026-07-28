# OMP-3 automated multi-client fault harness

The milestone cannot rely on ad hoc eight-window manual testing. This harness
launches one host plus up to seven guests, drives the complete direct-host
lifecycle from handshake to acknowledged result and teardown, impairs delivery
with the documented network profiles, injects the declared fault taxonomy, and
compares every client at every confirmed checkpoint and at the final state.

It has three tiers, and each one proves a different thing:

| Tier | Command | Proves |
| --- | --- | --- |
| Bounded CI subset | `love . --test` (`spec/game/online_fault_harness_spec.lua`) | The harness's own mechanisms, on short deterministic rows. |
| Headless matrix | `love . --fault-harness full` | The declared matrix converges, or names exactly where it does not. |
| Separate-process campaign | `python3 -B scripts/fault_harness.py --selection full` | The agreement is independent of this build's per-process hash seed. |

## What it is made of

Four modules, and none of them reimplements something that already exists:

| Module | Owns |
| --- | --- |
| `game.online.fault_transport` | A `StarTransportAdapter` decorator that impairs *arrivals* using `sim.network_conditions` and `data/network_profiles.lua`. |
| `game.transport.fake_relay` | The optional second wire shape. `--topology relay` runs every row over an in-process relay room instead of the direct-host star; see [`relay_topology_probe.md`](relay_topology_probe.md). |
| `game.online.fault_harness` | N isolated clients, the lifecycle, and the cross-client comparison. |
| `game.online.fault_scenarios` | The declared matrix, and the interpreter that turns a row into a report. |
| `game.online.fault_campaign` | The headless entry point and its deterministic marker stream. |

The session state machine is [`coordinator`](session_coordinator.md), the policy
over a running match is `game.screens.online_match_model`, control framing is
`lobby_link`, the match is [`match_driver`](match_driver.md), presentation is
`match_presentation`, and measurement is
[`net_diagnostics`](net_diagnostics.md). The harness is wiring plus comparison.

### Isolation

Every client holds its own coordinator state, star endpoint, impairment state,
diagnostic recorder, rollback session, presentation timeline, and session model.
Boundary zero is **not** shared: each client derives its own through
`match_session.request`, which is what makes an agreed `MatchSnapshot` hash
evidence rather than a tautology.

They are still *logical* clients inside one operating-system process. That
distinction is load bearing and is never elided in this document or in the
harness's own claims.

## The profiles are the real ones

Every impairment row drives `sim.network_conditions` against the checked-in
`data/network_profiles.lua`, at the documented four rolls per packet in the
documented order, from a private network seed that never touches the match seed.
The decorator does not re-derive delay, jitter, loss, duplication, or bursts; it
asks that module for a verdict and carries the **real** #161/#162 envelope to the
arrival tick the module chose.

Impairment is applied on the way *in*, not on the way out. `send` and `broadcast`
are forwarded verbatim, so `overflow`, `backpressure`, `not_connected`,
`role_forbidden`, and `malformed` are still the real star's synchronous verdicts
rather than something this decorator reimplements. It is also the more faithful
model: a real sender does transmit the packets the network then drops.

Two faults are **declared rather than rolled**, and are counted separately:

- `withhold(from, through)` drops every input arrival in a window;
- `hold(from, through)` buffers them and releases the backlog afterwards.

A probabilistic burst lands where its RNG puts it. Pinning "a burst that
straddles full time" or "a backlog released past the 30-tick retained floor"
needs a window chosen on purpose, so those rows say so in their counters
(`withheld=`, `held=`) instead of pretending a profile produced them.

The **control channel is never impaired by a profile.** A WebRTC control channel
is reliable and ordered; silently losing session mail would model a transport
this project does not ship. Control-channel faults are explicit
(`duplicate_control_every` re-delivers every Nth envelope, which is duplication a
reliable channel can still produce through a retransmit).

## The matrix

`game.online.fault_scenarios.SCENARIOS` is the single list every tier quotes.
Rows cover:

- **All three modes.** 1v1 (2 clients × 4 owned slots), 2v2 (4 × 2), 4v4 (8 × 1),
  plus a short 4v4 lobby so declared bot fills exist at all.
- **All four profiles.** `clean`, `omp0_parity`, `playable`, `stress`.
- **Recovery independence.** A reversed arrival release order must produce
  byte-identical confirmed boundaries.
- **The terminal taxonomy.** Peer disconnect, host departure, ownership
  violation, authority conflict, malformed traffic, manifest mismatch,
  over-window input, and persistent hash divergence, each declaring the terminal
  status it expects.

The mode and profile bullets are crossed with each other. The taxonomy bullet is
**not**: each fault row runs in one representative mode (2v2, visible in its
scenario id) rather than in all three. The terminal paths it exercises are
mode-independent by construction — ownership is a frozen partition, the retained
floor is a per-peer window, and a lost link is a lost link — but that is an
argument, not a measurement, and this matrix does not make it into one.

Every row that is bounded says so. `--fault-harness smoke` prints how many of the
declared rows it ran and which selector runs the rest, and every row carrying a
`known_gap` is printed in **every** run whether or not it was selected.

## Live-slot comparison, and why 4v4 cannot provide it

Convergence compares the boundary hash *and* the live slot of every human at each
confirmed checkpoint. In 4v4 the owned sets are singletons, so every branch of
`coordinator.next_live_slot` returns the slot already live and switching is
inert: that row is reported `SKIP` with the reason, not counted as coverage. 1v1
and 2v2 are the rows that can exhibit a live-slot divergence, and they are the
rows the separate-process campaign leans on.

## Separate processes, and exactly what that buys

`#166` proved the live-slot ranking deterministic by inspection: a strict total
order (squared distance, then canonical slot index) with no `pairs()` anywhere in
the ranking path. That is the right mitigation for a single-process change, but
it is an absence-of-evidence proof.

This build **randomizes `pairs()` iteration order per process**, so two clients
inside one process share a hash seed and can agree by accident. A same-process
harness structurally cannot catch a hash-order-induced divergence.

`scripts/fault_harness.py` runs the same scenario in N genuinely separate
operating-system processes and makes three checks:

1. **Cross-process determinism.** Every process must emit an identical marker
   stream. Any `pairs()`-order dependence in the hashed path breaks this.
2. **Cross-process, cross-client agreement.** Client A's confirmed checkpoints as
   computed in process P are compared against client B's as computed in process
   Q, for every P ≠ Q, in both the boundary hash and the live slot per human.
   Those two clients demonstrably do not share a hash seed. This is the
   comparison a single-process harness cannot make.
3. **Hash-seed diversity.** Each process prints its own observed `pairs()` order
   over a fixed table. If every process reported the same order, check 2 is
   vacuous for that run, and the controller **fails and says so** rather than
   passing quietly. Note the exact guarantee: it requires **at least two
   distinct** orders across the N processes, not that every process differs from
   every other. It therefore detects a total collapse of per-process
   randomization, which is the failure that would make check 2 meaningless; it
   does not certify that any particular pair of compared clients had different
   seeds. In practice all three processes have differed on every run.

This is a genuine execution proof for the divergence class in question. It is
*not* the same thing as running each client in its own process with a real
inter-process transport; every process here simulates the whole session and the
comparison is across the per-client results those independently seeded processes
produced.

## Resource measurement and teardown

Measurement goes through #168's recorder and diagnostic tap, so the harness does
not invent numbers the driver does not publish. Declared gates
(`fault_harness.GATES`):

| Gate | Value | Source |
| --- | --- | --- |
| `max_rollback_depth` | `rollback_input_history.ROLLBACK_WINDOW_TICKS` (30) | The retained floor. |
| `max_channel_depth` | `transport_contract.MAX_QUEUE_LIMIT` (256) | The star's own bound. |
| `max_overflow` | 0 | A healthy row must never have a send refused. |
| `max_residual_queue` | 0 | Teardown must drain. |
| `max_orphan_peers` | 0 | Teardown must close every link. |

Exceeding one is a blocking finding, not a warning. Per-client packets, bytes,
rollback count, correction count, worst rollback depth, deferred/duplicate/
rejected rows, and applied rows are printed as markers so a report can quote
measured numbers rather than analytical ones.

**Queue depth is read from `runtime.pressure`, never from `runtime.peers`.** A
depth is an instantaneous level, and the last transport snapshot a run takes is
almost always a quiescent one — after the final pump, after teardown drained
everything. A gate written against the last snapshot reads zero however hard the
transport was pushed, so `net_diagnostics` now folds a genuine running peak
across every snapshot, alongside the cumulative backpressure and overflow latches
that a depth sample can miss entirely (a channel that hit its `bufferedAmount`
ceiling and drained again between two observations leaves no depth behind, only a
latch).

Two findings exist purely to keep those gates honest:

- `resources.channel_depth_observed` fails if the peak the depth gate is checking
  was never non-zero. A blocking gate that structurally cannot fail is worse than
  no gate, because this document invites a reader to trust it.
- `faults.backpressure_observed` fails if a row that clamped the send buffer
  never actually latched backpressure — a scenario that silently turned itself
  off. Reverting `2v2.backpressure`'s clamp to the adapter default makes this row
  go red, which is how it was checked.

## Commands

```sh
# Bounded CI subset (also runs inside ./scripts/check.sh via love . --test)
love . --test

# One row
love . --fault-harness 2v2.playable

# One row, explicit impairment seed and duration
love . --fault-harness 4v4.playable 9001 240

# The bounded subset, or the whole matrix
love . --fault-harness smoke
love . --fault-harness full

# The same matrix over the in-process relay instead of the direct-host star
love . --fault-harness full --topology relay

# The separate-process campaign (three processes by default)
python3 -B scripts/fault_harness.py --selection smoke
python3 -B scripts/fault_harness.py --selection full --processes 4 --seed 9001
python3 -B scripts/fault_harness.py --selection full --topology relay

# The controller's own logic, with no LOVE process (this is what CI runs)
python3 -B scripts/fault_harness.py --self-test
```

The marker stream is stable: `marker`, `note`, `known-gap`, `finding`,
`scenario-result`, `RESULT`. `hash-order-probe` is deliberately excluded from the
comparable stream, because it is *expected* to differ and it is the evidence that
the processes differed.

## What this harness cannot prove

- **Nothing about real connectivity.** Simulated profiles are simulated. They say
  nothing about NAT traversal, ICE, STUN/TURN, or real-device frame scheduling.
  #170 covers real transport. Every impairment row is labelled with its profile
  name for exactly this reason.
- **Nothing about browsers.** This is the headless tier. The browser
  multi-context bridge is exercised by `scripts/browser_matrix.py` and the
  WebRTC proof suite; the `browser.multi_context` row here is `SKIP` with that
  reason rather than absent.
- **Logical clients are not physical clients.** Eight logical clients in one
  process is not eight machines, and the separate-process campaign is a
  hash-seed-independence proof, not a hardware one.
- **The combat correction phases.** Non-live owned slots and declared bot fills
  run `sim.slot_input`'s pre-#112 deterministic bot, which never reaches wind-up,
  guard, contact, projectile flight, stagger, ball spill, or immunity expiry.
  Those rows exist and are `SKIP`-with-reason, because a passing row would pin an
  absence. The *mechanism* — the combat companion surviving correction and
  resimulation — is covered by the driver's own specs.
- **The accepted default combat disposition (#114).** The manifest still carries
  a placeholder.

## The stranded-peer findings this harness found (#241): two fixed, one open

`4v4.playable` and `4v4.stress` used to strand one or two guests on roughly half
the seeds tried, in two visibly different shapes. From the captured
`4v4.playable` run at the default network seed:

| Peer | Host batches lost | Confirmed boundary at full time | Full-time boundary |
| --- | ---: | ---: | ---: |
| `guest_5` | 5 (2 independent, 3 burst) | 118 | 121 |
| `guest_6` | 7 (1 independent, 6 burst) | 55 | 121 |

They were two defects, not one, and the number of lost batches predicted neither
— which is what an earlier draft of this page got wrong, and why it is worth
recording how each was actually pinned rather than only what the fix was.

**`guest_6`, the mid-match stall.** Instrumenting the guest's confirmation showed
it lose exactly one row — tick 56, slot 4 — and never recover it. The host emitted
that row in **five** canonical batches, not the seven the redundancy window is
supposed to give it, because a host batch carries only the guest bundles that
arrived on that transport tick and slot 4's author did not deliver on every tick.
`guest_6` lost host batches at transport ticks 61, 62, 63, 65, 66 and 67 — and
*received* the one at 64, whose row span covered tick 56 but which carried no
slot-4 row. Every one of the host's five emissions fell inside the burst. Thirty
steps later the retained floor slid past tick 56, `prune_before` deleted it, and
because confirmation only advances from `confirmed_tick + 1` it froze at 55 for
the rest of the match with nothing raised. Both halves are fixed in the driver:
the host now tops its batch up from authority it already holds, and
`confirmation_stalled` reports the frozen state at the step it becomes permanent.

**`guest_5` and `guest_3`, the tail stalls.** `guest_5` ended missing exactly one
row — tick 119, slot 5 — with the retained floor at 91, so the row was well inside
its window and still placeable. The host reached full time at step 120 and
completed at step **122**, the earliest of any peer because as sequencer its
confirmation runs ahead. After a terminal status a driver polls, applies and
broadcasts nothing, so the star's only relay was gone three steps into a
sixty-step settle window. Slot 5's author kept re-publishing its tail until it too
completed at step 127, with nothing left to fan those bundles out. `guest_5` spent
the remaining fifty-two settle steps re-publishing its own window into a dead
star. The host now stays for as long as a guest is still asking.

What the harness established on its own, and which held up:

- never observed under `clean` or `omp0_parity`, which have no jitter, no
  duplication, and no bursts; observed under `playable` and `stress`, which have
  all three. Reordering, not raw loss rate, is what distinguishes them;
- it is not the host's row bound: the recorder reports `deferred=0` and
  `rejected=0` on the host in a stranding run;
- every confirmed checkpoint that *was* compared still agreed. The divergence was
  in the tail and in the final hash, not in the confirmed timeline.

Neither fix widened the redundancy window, raised the settle bound, or changed
this matrix's scenarios, profiles or seeds. `4v4.playable` is the evidence: the
seeds that stranded `guest_5` and `guest_6` now pass unchanged.

### Still open (#243): `4v4.stress` strands on batch capacity, not on a leak

`4v4.stress` still strands a guest, and the row keeps a `known_gap` pointing at
[#243](https://github.com/osobytes/goliseo/issues/243) — but the cause is now
measured rather than suspected, and it is **not** the same defect.

Under `stress` the host's canonical batch is saturated at exactly
`MAX_HOST_ROWS` on every tick from the twelfth onwards: all eight slots deliver
their seven-row window and there is no leftover budget at all. The fan-out repair
above therefore has nothing to spend, and a row the host learns *late* cannot be
re-sent without displacing a fresh row. Captured at the default seed: the host
learned `(tick 9, slot 5)` seven transport ticks late, fanned it out at transport
ticks 16, 20 and 21 — three times rather than seven — and `guest_7` received the
batches at 15, 17, 18, 19 and 23 and missed exactly those three.

#245 added a second piece of evidence for that reading: the row fails
**identically** under `--topology relay` — the same eight `confirmation_stalled`
statuses, the same eight final hashes, the same stalled confirmation ticks — even
though the relay cuts the host's per-tick upload from 5,291.5 B to 755.9 B. A
capacity limit of the canonical batch is not a property of the wire, so moving
the fan-out to a relay does not touch it. See
[`relay_topology_probe.md`](relay_topology_probe.md).

That is a capacity limit of the 56-row batch, not a leak in how it is filled.
Three things were tried against it and are recorded so they are not re-derived:
giving each accepted bundle a relay quota counted from when the host learned it
(inert — there is no leftover budget under saturation); repaying that quota
per-slot in one pass (worse — one slot's history starves another slot's present);
and reserving one bundle's worth of the batch for repayment (worse — it turned
`4v4.playable` red). Closing it needs a way for a guest to *ask* for the row it
missed, which is a protocol addition rather than a wider open-loop window, and
that is what **#243** tracks. This row stays red until it lands.

What did change for this row: the stranded peer now reports
`confirmation_stalled` at the step its confirmation dies rather than
`settle_timeout` a whole match later. A terminal netcode failure ends the session
for every peer — as it does for every other terminal this driver raises — so the
row now fails with eight `confirmation_stalled` clients rather than seven
completions and one late mislabelled straggler. That is a louder and earlier
signal for the same underlying loss, not a new defect.
