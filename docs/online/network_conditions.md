# OMP-2 deterministic network conditions

`sim.network_conditions` is a pure, in-process packet impairment layer for the
rollback laboratory. It makes network delivery repeatable from a checked-in
profile, a network-only seed, and an ordered sequence of calls. It has no wall
clock, LÖVE, browser, socket, JavaScript, or `MatchState` dependency.

Transport time and match input time are deliberately separate integers:

- `send_tick` and `arrival_tick` belong to the laboratory transport clock.
- `current.tick` belongs to the authoritative `InputSample` stream consumed by
  `sim.rollback_input_history`.
- `drain` advances only the transport clock. A final input can therefore be
  resent and recovered while the match remains at the same simulation tick.

Both clocks currently allow exact integers from zero through `2147483647`, but
they remain separate contracts. `MAX_TRANSPORT_TICK` owns the delivery bound.
A send or resend is rejected before retaining authority, consuming RNG, or
changing counters when the profile's maximum possible arrival would exceed
that bound. `drain` preflights its entire requested transport range the same
way, and `poll` rejects ticks above the bound. No accepted envelope can become
undrainable through integer overflow.

## Profiles

`data.network_profiles` contains the exact OMP-2 configurations. Rates are
probabilities per source packet; delay and jitter are whole transport ticks.
Delay is one-way.

| Profile | Base delay | Jitter | Independent loss | Duplicate | Burst start | Burst length |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `clean` | 0 | 0 | 0% | 0% | off | 0 |
| `omp0_parity` | 3 | 0 | 1% | 0% | off | 0 |
| `playable` | 3 | -2..+2 | 1% | 0.25% | 0.25% | 3 ticks |
| `stress` | 6 | -3..+3 | 3% | 1% | 1% | 3 ticks |

The constructor copies its profile, so a caller cannot change an active run by
mutating the data table later. The network seed initializes a private
`core.rng` state. It never reads or writes the match RNG.

## Packet and history contract

`send(conditions, send_tick, source_slot, input_tick, sample)` retains the
authoritative row for its source slot and schedules one packet. An envelope has
a globally increasing `sequence`, source slot, send and arrival ticks, a
`current` input record, and up to six unique earlier retained records in
`history`.

History is oldest first. `records(delivery)` returns copied history followed by
the copied current row, so a consumer can feed every result directly to:

```lua
for _, record in ipairs(network_conditions.records(delivery)) do
    local accepted, err, code = rollback_input_history.add_authoritative(
        history,
        record.tick,
        delivery.source_slot,
        record.sample
    )
    if accepted == nil then
        return nil, err, code
    end
end
```

Samples are copied when retained, when scheduled, and when returned. Later
mutation of producer samples or returned deliveries cannot rewrite the queued
or retained authority.

Each slot retains seven unique authoritative records: the current row plus the
six rows that may accompany it. Sending the same tick and sample is an
idempotent authoritative duplicate and schedules a new packet without adding a
history row. A different sample for a retained tick returns
`conflicting_authoritative`. An older unseen tick returns
`stale_authoritative`; an explicit resend outside the retained seven rows
returns `not_retained`. Authority is never last-writer-wins.

`resend(conditions, send_tick, source_slot, input_tick)` schedules an already
retained row as the packet's current record. It cannot append a duplicate input
tick.

## Deterministic impairments

Every `send` or `resend` consumes exactly four RNG values, in this order:

1. jitter;
2. independent loss;
3. duplication;
4. burst start.

All four values are consumed even when a prior outcome makes a later value
irrelevant, the profile disables an impairment, or the source slot is already
inside a burst. This fixed consumption policy prevents configuration branches
from making a run depend on incidental control flow.

The jitter roll selects a whole tick uniformly across the inclusive configured
range. Arrival is:

```text
max(send_tick, send_tick + base_delay_ticks + jitter_ticks)
```

Jitter therefore creates reordering naturally but can never deliver a packet
before it was sent.

Burst state is independent per source slot. A successful start roll drops the
starting packet and every packet from that slot through the configured
inclusive transport-tick range. Rolls observed during an active burst do not
extend it. If burst and independent-loss rolls both qualify, burst loss takes
precedence so their counters remain exclusive.

A duplicate is created only for a packet that survives loss. It retains the
source envelope's `sequence`, send tick, arrival tick, current input, and
history. The original uses `duplicate_ordinal == 0`; the copy uses ordinal one.
Both arrive together.

`poll(conditions, delivery_tick)` returns every due envelope ordered by:

```text
(arrival_tick, sequence, duplicate_ordinal)
```

This makes equal-arrival order stable across runtimes. The `reordered` counter
increments when an original envelope is eventually delivered after a higher
original sequence. Every sequence has exactly one original, so duplicate
copies do not need a lifetime identity set and cannot inflate that counter.

## Counters

`counters` returns a copy with:

- `sent`: source packets scheduled by `send` or `resend`, before loss;
- `delivered`: returned envelopes, including impairment-created duplicates;
- `independent_lost`: packets dropped only by independent loss;
- `burst_lost`: packets dropped by a new or active source-slot burst;
- `duplicated`: duplicate envelopes created after loss decisions;
- `reordered`: unique packet identities delivered behind a later sequence;
- `history_recovered`: first-seen authoritative samples obtained from a
  delivery's redundant history rather than its current row.

`pending` reports queued delivery envelopes. Counter and pending reads do not
advance either clock.

Delivered-sample bookkeeping is also bounded. Each sample is represented by a
collision-free mixed-radix integer packing of the two 255-value axes, 256-value
held mask, and 128-value edge mask. This is an exact identity, not a hash. A
ledger row remains only while that tick is among the slot's seven retained
authoritative records or is referenced by a pending envelope. This is enough
to de-duplicate redundant and reordered history, detect an impossible
conflict, and track a retained drain target without keeping match-lifetime
sample tables.

`diagnostics` returns current and mutation-time peak retained-authority,
delivered-ledger, pending-envelope, and pending-record-reference counts for
memory evidence. Peaks are captured before polling or ledger pruning can hide
transient work. `sample_key` exposes the exact packed identity for diagnostic
boundary tests; it is not a production wire encoding.

## Drain and resend

`drain(conditions, start_tick, max_ticks, requests)` accepts unique
`{ source_slot, input_tick }` requests. It sorts them by slot and input tick,
polls the transport clock, and resends each still-unseen target once per
transport tick. Once all requested rows are observed, it stops resending and
continues polling already-scheduled redundant packets until the queue is empty
or the tick budget expires.

The result includes all deliveries, the final transport tick, recovered and
requested counts, remaining pending envelopes, and `complete`. A false
`complete` is an explicit bounded-drain outcome, not silent sample loss. The
caller chooses the budget; `drain` never steps or corrects `MatchState`.

## Rollback laboratory handoff

The OMP-2 runner should consume this module as follows:

1. Construct one conditions state per client with a named profile and explicit
   network seed.
2. Generate authoritative `InputSample` rows independently of that network
   seed.
3. Call `send` for remote source slots, then `poll` at the current transport
   tick.
4. For each delivery, feed `records(delivery)` oldest to newest into
   `rollback_input_history.add_authoritative` using `delivery.source_slot`.
5. After the last match input tick, call `drain` for final remote rows without
   stepping the reference or client match.
6. Record copied counters and diagnostics, then assert `pending == 0` and a
   bounded delivered ledger before final convergence evidence.

This contract simulates delivery conditions only. Restore, resimulation,
confirmed events, presentation reconciliation, and real production transport
remain separate OMP-2/OMP-3 responsibilities.

OMP-3 preserves this exact current-plus-six policy in the production
[input packet contract](input_packets.md). The production codec adds session
identity, ownership, envelope validation, compact bounded bytes, host
canonicalization, atomic history insertion, and one reconciliation boundary;
it does not change the impairment evidence recorded here.
