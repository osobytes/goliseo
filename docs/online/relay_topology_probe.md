# Measuring the OMP-4 relay topology in the fault harness (#245)

The relay topology decision moves input distribution from the player-host to a
dedicated relay server. This page is the measurement that was taken **before**
any server exists, using the #169 fault harness, #168's diagnostics and a third
in-process `StarTransportAdapter`.

It is a measurement, not an endorsement. Two of the decision's claims are
confirmed, one is confirmed but not for the stated reason, and two of the
predicted numbers are wrong by more than an order of magnitude in one direction
and by a factor of two in the other.

Nothing in the driver, the coordinator, or the protocol was changed to make any
of this pass. Where the session stack refuses the topology, that refusal is
recorded here rather than removed.

## What was built

| Piece | What it is |
| --- | --- |
| `game/transport/fake_relay.lua` | A third adapter beside `fake_star` and `browser_star`. Every member holds one link to a `FakeRelayRoom`; the room concatenates the opaque lines it received for each destination and hands down one frame. It decodes nothing and never returns a member's own line. |
| `--topology relay` | The whole declared 25-row matrix, and the separate-process campaign, run over the relay instead of the star. |
| `wire_counters()` | Per-copy uplink and downlink bytes on **both** adapters, split by channel, plus `downlink_framed_bytes` — what crossed the link once addressing and separators are included. |
| `spec/game/transport_relay_spec.lua` | The adapter's contract rows, and the sequencer-less probe rows below. |

### Why the recorder could not answer the bandwidth question

`DiagnosticTransport` records one packet per `broadcast` **call**, deliberately:
it measures what the driver published, not what the wire carried. On a star that
one call is one queued copy per guest link. So #168's per-client byte figures are
identical under both topologies, and a comparison quoting them would conclude the
two shapes cost the same. `wire_counters()` counts copies at the link, on both
adapters, which is why the figures below exist at all.

### Two topologies, and the third that cannot be run

There are two independent things the decision changes at once, and they have to
be named separately because they turn out to be separable:

1. **The hub.** Who carries the fan-out — a player, or a server.
2. **The sequencer.** Who canonicalises — one privileged peer, or nobody.

`--topology relay` changes (1) and leaves (2) alone: the host still builds the
canonical batch, but publishes it once to the room instead of seven times. This
runs the whole matrix.

Changing (2) as well is what the decision actually proposes, and it **cannot be
run at all** without changing the driver. The findings section says exactly
where it stops.

## Question 1 — does `4v4.stress` converge under a relay?

**No. It fails identically.**

The whole 25-row matrix produces byte-identical markers under both topologies.
Each stream is **1,733 lines** with the hash-order probe excluded; removing the
new `marker wire` lines and the one `selection` line carrying `topology=` leaves
**1,571 comparable lines, of which exactly nine differ**, and all nine are inside
`2v2.backpressure` (below). Every checkpoint hash, every client status, every
terminal and every scenario result agrees. `4v4.stress` fails the same way on
both:

```
marker client host    status=confirmation_stalled terminal=confirmation_stalled final=56142afa03699a10 confirmed_output=43
marker client guest_7 status=confirmation_stalled terminal=confirmation_stalled final=41c29f75cbdb3aba confirmed_output=9
```

— the same eight statuses, the same eight final hashes, the same stalled
confirmation ticks, on the star and on the relay.

**Reproducing the line counts.** The 1,733 figure is the marker stream of
`python3 -B scripts/fault_harness.py --selection full --topology {star,relay}` at
the default seed (4703) and default duration, which is what the controller prints
as `markers=1733`. Ad hoc `love . --fault-harness` invocations with a different
seed, duration or selector produce a different count; quote the command with the
number.

This does **not** falsify #243's diagnosis; it falsifies the inference drawn from
it. #243 is a capacity limit of the **56-row canonical batch**, which lives in
`input_protocol.MAX_HOST_ROWS` and in `match_driver`'s `fill_relay_window`. It is
not a property of the wire. Relaying the fan-out moves *where the bytes are
paid* and touches nothing about how many rows fit in a batch, so a relay wire
under an unchanged sequencer leaves #243 exactly where it was.

The decision's sentence — "there is no eight-slot aggregation into one bounded
packet, so there is nothing to saturate" — is only true of the **sequencer-less**
arrangement, where each client publishes only its own seven-row bundle. That
bundle is `MAX_GUEST_ROWS` and always fits, so the saturation genuinely cannot
occur. The claim is therefore about removing the sequencer, not about moving the
hub, and the harness cannot yet demonstrate it because the driver refuses that
arrangement (findings 1–3).

The honest summary: **the bandwidth win and the #243 fix are separable, and the
decision buys only the first of them at the wire.**

### The one row that behaves differently

`2v2.backpressure` clamps the send buffer to 2 KiB. Under the relay the clamp
bites harder, because the budget is per *link* and a relay member has one link
where a star host has one per guest:

| | star | relay |
| --- | ---: | ---: |
| backpressure latches | 6 | 9 |
| transport snapshots | 1,124 | 1,132 |

The row passes on both. It is worth recording because it is the only place the
topology changes behaviour rather than only cost: a relay client's uplink is a
single shared buffer, so per-peer backpressure isolation is something the star
has and the relay does not.

## Question 2 — is worst-node upload really 5,285 → 1,190 B/tick?

**The star figure is right almost to the byte. Both relay figures are wrong.**

Measured on `4v4.clean`, input channel only, over 129 driver steps:

| | predicted | measured | note |
| --- | ---: | ---: | --- |
| host-star, worst node (host) | 5,285 B/tick | **5,291.5 B/tick** | within 0.2% |
| host-star, a guest | ~170 B/tick | **168.6 B/tick** | |
| relay wire, sequencer kept (host) | — | **755.9 B/tick** | one copy of the canonical batch |
| relay, sequencer-less (every client) | 1,190 B/tick | **190.4 B/tick** | measured in `transport_relay_spec` |
| relay downlink, framing (envelopes only) | ~650 B/tick | **1,332.8 B/tick** | comparable with the star figure |
| relay downlink, framing (on the wire) | — | **1,433.8 B/tick** | addressing and separators included |

Two corrections:

**The predicted 1,190 B/tick relay upload is the mesh column.** 1,190 = 7 × 170,
which is what a client uploads when it sends its bundle to seven peers directly.
A relay client uploads **one** copy: 190.4 B/tick measured on the real protocol
bundle. The relay is 6.3× better than its own decision record claims.

**The predicted ~650 B/tick framing downlink is inverted.** A framing relay
cannot be cheaper than a canonicalising one, and the reason is the property that
makes it attractive: it does not parse, so it cannot merge. Each client receives
the other seven bundles whole — seven protocol headers, seven sender ids, seven
sequences — where a canonical batch carries the union of their rows under one
header. Measured: **1,332.8 B/tick of envelopes versus 755.9 B/tick canonical**,
so the framing relay costs **1.76× more downstream**, not 16% less.

### The 1,332.8 figure is a floor, not the wire cost

That number counts **encoded envelopes only**, deliberately, so that it is
comparable with the star's 755.9 B/tick, which is also an envelope figure. It
excludes the `origin|channel|` addressing on each forwarded line and the
separators between lines. Both are real costs a relay has to pay and a star does
not:

- finding 2 below establishes that a relay **must** name the origin of every line
  it forwards, or ownership validation degrades to a self-declared `sender_id`.
  The overhead is therefore a requirement, not an artefact of this
  implementation;
- a star pays nothing equivalent, because each guest link is its own data channel
  and the origin of an arrival *is* the channel it arrived on.

`wire_counters().downlink_framed_bytes` counts what actually crossed the link.
Measured on the same run: **1,433.8 B/tick** for a member receiving one `host`
line and six `guest_N` lines, and 1,436.8 B/tick for the member receiving seven
`guest_N` lines. Against the 755.9 B canonical batch that is **1.90×**, not
1.76×.

So the honest bracket is **1.76× to 1.90×**, and where it lands inside that
depends on how compactly a real relay encodes origin. This adapter uses the
transport contract's own `peer_id|channel|` text form, which is verbose — 11 to
14 bytes per line here — so 1.90× is the pessimistic end and a compact binary
origin tag would sit near the optimistic one. **It cannot be below 1.76×**, and
the direction of the correction is what matters: opaque framing is more expensive
downstream than canonicalising, and the gap is wider than the envelope figure
alone suggests.

The trade is real either way — 190 B up and 1,333 B down beats 5,292 B up on the
worst node by a wide margin — but the decision's table understates the relay's
upload win and inverts its downlink cost, and the "this is also *cheaper*"
paragraph does not survive measurement.

## Question 3 — what breaks when no peer is the sequencer?

Six findings, each pinned by a row in `spec/game/transport_relay_spec.lua`.
They are recorded, not fixed.

### Finding 1 — a client that receives a peer's bundle kills the match

`game/online/match_driver.lua`, `guest_apply_authority`. A guest decodes each
inbound input envelope and requires `packet.kind == "host"`. A framing relay can
deliver nothing else but peer bundles, so the first one is terminal:

```
terminal.status = "ownership_violation"
terminal.detail = "a guest received authority that was not a host batch"
```

The relay itself accepts the send without complaint — the wire imposes nothing,
which is exactly the point. **This is the single blocking item.** The decision's
migration note ("a relay adapter becomes a third implementation ... and the
driver above it does not change") is false: every non-host client must learn to
canonicalise locally before a relay can carry a match.

### Finding 2 — ownership validation survives only if the relay keeps origin

`game/online/input_protocol.lua`, `canonical_host_batch`. Ownership is checked
against `arrival.transport_peer_id`, the link the bundle arrived on, and not
against anything inside the packet:

```lua
if assignment.producer_id ~= packet.sender_id then
    return failure("ownership_mismatch", "input producer does not own the declared slot")
end
if assignment.producer_kind == "peer" and arrival.transport_peer_id ~= assignment.producer_id then
    return failure("ownership_mismatch", "transport peer cannot carry this input producer")
end
```

`fake_relay` preserves origin, because each line is written by the sending
adapter and the room only ever appends and concatenates. A relay described
loosely as "concatenating opaque blobs" would not — and the check would then
degrade from *transport-attested* ownership to self-declared `sender_id`, which
is not a check at all. **The relay protocol must frame per-origin. This is a
requirement on the server design, not an implementation detail.**

### Finding 3 — declared bot fills have no author without a host

`game/online/match_driver.lua`, `match_driver.new`:

```lua
local mine = owned_set[index] or (role == "host" and producer.producer_kind == "bot")
```

Only the host authors bot-filled slots. Measured on a 4-human 4v4 lobby: the host
owns 1 slot and authors 5; every guest owns 1 and authors 1. Remove the host and
four slots have no author at all. Any relay design needs a rule for who
publishes declared fills — or the fills have to become client-side deterministic
simulation that nobody publishes, which is a protocol change.

### Finding 4 — the coordinator is host-authoritative regardless of the wire

`game/online/coordinator.lua` refuses `propose_manifest`, `assign_slots`,
`begin_countdown` and match-phase publication from a non-host, with
`not_permitted`. This is unchanged by the topology and the decision says so
("the session coordinator stays client-side"). Recorded because it means the
relay does **not** remove the host: it removes the host's *input* privilege and
leaves its *session* privilege in place, so host departure still ends the match
and OMP-5 host migration is still required.

### Finding 5 — the settle phase's relay-quiet wait is complexity the relay deletes

`match_driver.SETTLE_RELAY_QUIET_STEPS` and `relay_drained` are host-only:

```lua
if driver._role ~= "host" then
    return true
end
```

The host stays in the settle phase for `DELAY_TICKS + 1` consecutive silent steps
after confirming its own boundary, because a departing player-host strands every
guest's tail (#241). Measured on a healthy 1v1: the host settles for strictly
more steps than the guest. A relay is not a player and cannot leave, so this
heuristic has no reason to exist under the new topology. **This is the one place
the decision makes the code simpler rather than harder**, and it is worth
claiming explicitly since the decision does not.

### Finding 6 — the fairness delay is universalised, not removed

`input_protocol.FAIRNESS_DELAY_TICKS = 3` is enforced only against host-local
input:

```lua
if arrival.transport_peer_id == options.host_peer_id
    and arrival.arrival_tick - packet.transport_tick < input_protocol.FAIRNESS_DELAY_TICKS
then
    return failure("fairness_delay", "host-local input bypassed the fixed fairness delay")
end
```

Under sequencer-less canonicalisation every client is "the host" for its own
input, so every client pays the delay against itself and receives every peer's
input already aged. The constant survives with the meaning the decision predicts
— a uniform input-delay knob — but note the corollary the decision does not draw:
the check above is written in terms of a *single* `host_peer_id`, so it needs
restating before it means anything in a topology with eight of them.

### A seventh observation: loss coupling, which is the real structural argument

The decision argues the relay removes saturation. The stronger argument it does
not make is about **loss coupling**. On the star, one lost host batch removes
authority for all eight slots for that transport tick; #241's mid-match stall was
exactly that, six consecutive batches lost. On a framing relay, one lost packet
removes one slot's seven-row window and nothing else.

The counterweight, also unmade: the host's fan-out repair (`remember_relay` /
`fill_relay_window`, added for #241) works because one peer holds everyone's
authority and can re-send a row it learned late. **No such peer exists under a
framing relay**, so a row lost by one client past its author's seven-tick window
can only be recovered by the retransmission protocol #243 already calls for.
Both #241's fix and #243's fix are host-star-shaped; neither transfers.

## Question 4 — does live-slot identity still agree per confirmed checkpoint?

**Yes, under both topologies, against genuinely distinct per-process hash seeds.**

`scripts/fault_harness.py --selection full --topology {star,relay}` runs the
matrix in three separate operating-system processes and compares client A's
confirmed checkpoints from process P against client B's from process Q, for every
P ≠ Q, in the boundary hash *and* the live slot per human. 1v1 and 2v2 are the
rows that can exhibit a live-slot divergence; 4v4's singleton owned sets make
switching inert and are reported `SKIP` with that reason.

The campaign's own hash-seed diversity check passed on both runs, so the
comparison is not vacuous.

## What this does not prove

- **Nothing about the sequencer-less topology's convergence.** Findings 1–3
  block it. Everything this page says about that shape is a byte measurement or
  a reading of the code, never a run.
- **Nothing about real connectivity.** In-process only. No NAT, no ICE, no
  server-side termination, no packet-rate limits.
- **Nothing about a real relay's latency.** The room forwards within one pump;
  a real relay adds a hop the harness does not model.
- **Nothing about eight machines.** Eight logical clients in one process, with
  the separate-process campaign proving hash-seed independence and nothing more.

## Verdict

Of the decision's claims that this exercise can reach:

| Claim | Verdict |
| --- | --- |
| The relay concentrates nothing on one player's uplink | **Confirmed.** 5,291.5 → 755.9 B/tick at the wire, → 190.4 B/tick sequencer-less. |
| Worst-node upload 5,285 → 1,190 B/tick | **Star figure confirmed; relay figure wrong.** The true sequencer-less figure is 190.4 B/tick; 1,190 is the mesh column. |
| Opaque framing is *cheaper* than canonicalising (~650 vs 755 B) | **Falsified.** 1,332.8 B/tick of envelopes and 1,433.8 B/tick on the wire; framing costs **1.76× to 1.90×** more downstream because it cannot merge rows and must name every origin. |
| #243 disappears structurally | **Not at the wire.** `4v4.stress` fails identically under a relay. The claim is about removing the sequencer, and the driver refuses that today. |
| The relay never needs to parse a game packet | **Confirmed**, with one requirement the decision omits: it must frame per-origin, or ownership validation degrades to self-declaration. |
| A relay adapter is a third implementation and the driver does not change | **Falsified.** `match_driver.guest_apply_authority` terminates on the first peer bundle. |
| Host departure still ends the session | **Confirmed.** The coordinator's authority is untouched by the wire. |
| The settle phase's relay-quiet heuristic becomes unnecessary | **Confirmed**, and it is the decision's clearest simplification. |

The decision's *direction* survives measurement. Its *arithmetic* and its
*migration cost* do not: the relay is cheaper on upload than claimed, more
expensive on download than claimed, and it does not fix #243 until every client
canonicalises for itself — which is a driver change the decision records as
"does not change".
