# OMP-3 network diagnostics and desync capture

> **Module names below are pre-port `require` paths.** The contract this document
> describes is current; the way it names files is not. Read `sim.foo` as
> `gc_sim::foo` (`rust/crates/gc-sim/src/foo.rs`), `game.online.foo` as
> `gc_netcode::foo`, `core.foo` as `gc_core::foo`, `data.foo` as `gc_data::foo`,
> and any `game/**` or `spec/**` path as its `ts/packages/**` counterpart. A
> `love .` command, a `love.*` API or a love.js measurement is **pre-port
> evidence** — commit `2c0d449` (#467) deleted that tree.

Four modules, all in `game/online/`, all additive:

| Module | Job |
| --- | --- |
| `diagnostics_schema` | Versioned shapes, strict validation, canonical bytes, digests. |
| `net_diagnostics` | The bounded in-memory recorder and its opt-in export. |
| `diagnostic_transport` | A `StarTransportAdapter` decorator that observes the wire. |
| `desync_package` | The deterministic capture that drives an offline reproduction. |

None of them touch the match driver, the coordinator, the session protocol, or a
transport implementation. The driver takes its transport by injection and already
publishes everything else on `diagnostics()`, `checkpoints()`, and the batch it
returns from `advance`, so a diagnostic can be assembled entirely from the public
surface. That is not an accident of convenience: an instrument that can perturb
the thing it measures is not an instrument, and the driver is settled code.

`sim/`, `data/`, and `core/` stay free of all of this. The only thing borrowed
downward is `core.fnv1a64`.

## 1. The distinction the schema enforces

Canonical simulation evidence and wall-clock runtime observation are different
kinds of fact, and the most useful thing this schema does is make them
un-confusable *at the shape level* rather than by convention.

Every field carries a **domain**, and the domain decides what vocabulary the
field name may use:

| Domain | Holds | Name may not contain |
| --- | --- | --- |
| `identity` | Build, protocol, manifest, fixture identity; collector configuration. | wall-clock words |
| `canonical` | Ticks, boundaries, hashes, rollback depth, resimulation, delivery order. | wall-clock words |
| `runtime` | RTT, jitter, queue depth, `bufferedAmount`, ICE transitions, drops, teardown. | simulation words |
| `anchor` | The single binding between the two clocks. | must contain **both**, plus `mapping_error_ms` |

"Wall-clock words" are `_ms`, `rtt`, `jitter`, `monotonic`, `elapsed`,
`timestamp`, `wall`, `second`, `millis`, `realtime`, `frame_time`. "Simulation
words" are `tick`, `boundary`, `hash`, `checkpoint`, `confirmed`, `rollback`,
`resimulat`, `snapshot`.

`diagnostics_schema.record` asserts this when the shape is *built*, so a future
field added to the wrong section fails at module load, not at review. A spec
walks the whole export shape and re-checks it, so the guarantee is about the
artifact rather than about the handful of fields anyone thought to test.

An anchor is the one exception and it earns it. Placing an input tick on a
monotonic clock is genuinely useful ("the stall you felt at 12.4 s was around
input tick 744"), so `anchors` exists, and every entry must state
`mapping_error_ms` — the interval within which the mapping is honest. A frame
sampled once per step cannot place a tick more precisely than half a frame, and
the artifact says so instead of implying a precision it does not have.

### Canonical is not one thing

Inside `canonical` there are two distinguishable kinds of evidence, and the doc
is the place that distinction lives because the vocabulary guard cannot express
it:

- `canonical.simulation` is **simulation truth**. Two peers that agree here agree
  about the match. Boundary hashes, the confirmed tick, the retained floor,
  rollback depth, resimulated ticks.
- `canonical.delivery` is **observed delivery order, recorded in tick space**.
  When a packet was sent and when it arrived is a fact about the network, not
  about the simulation — but it is measured in transport ticks, not
  milliseconds, and replaying the same delivery order reproduces the same
  divergence. It is an *input* to a reproduction. It is never evidence about
  what the simulation should have computed.

Runtime timing is never promoted into either. There is no code path that reads a
millisecond and writes a tick.

## 2. What is collected

### Identity (`session`)

Session and peer id (pseudonymous), role, match mode, combat status, manifest and
assignment digests, countdown id, build/source/content/tuning/match-config/
fixture/arena/combat-rules/gameplay-AI ids, protocol/input/snapshot/tape/combat
schema versions, seed, tick rate, duration, max goals, and an optional network
profile digest. Together these pin the exact build, protocol, manifest, fixture,
and network profile a reproduction has to match.

### Canonical

- **Policy and clocks**: input delay ticks, hash interval, max rollback ticks,
  first/present/confirmed-input/confirmed-output/retained-floor ticks, step count.
- **Rollback**: rollback count, correction count, predicted slot samples,
  resimulated ticks, max rollback depth, and `worst_correction`
  (causal tick, through tick, depth, resimulated ticks).
- **Hashes**: every published boundary checkpoint with its hash *and* the live
  slot of each human at that boundary, plus `mismatches` — the boundaries where a
  peer's published hash disagreed, with an optional first differing path.
- **Events**: `added`, `unchanged`, `replaced`, `revoked`, and
  `resimulated_tick_count`, derived from the outputs the batch already carries. A
  rollback re-emits every corrected tick, so a tick seen twice *is* a resimulated
  tick, and comparing the tick's event digest across emissions says whether the
  reconciliation actually changed anything a consumer cares about.
- **Delivery**: per-packet `sample_step`, `send_transport_tick`,
  `arrival_transport_tick`, `authority_input_tick`, `apply_input_tick`, sequence,
  payload bytes, and disposition, plus totals for authored/published/sent/
  arrived/deferred/duplicate/rejected, applied rows, and reconciliations.
  `published` is the driver's own count of packets handed to the transport and
  `sent` is what an observer saw leave; they are kept apart rather than summed,
  and a disagreement between them is itself a finding.
- **Control**: accepted session control messages in order, by kind, sender,
  sequence, and message id.

### Runtime

Star state and cumulative counters; per peer and per channel state, `ice_state`,
queue depth, `bufferedAmount`, sent/received, drops, sequence gaps, backpressure,
malformed counts, redacted last error text; per-peer RTT/jitter min/max/last with the
monotonic window they were taken over; ordered transport and lifecycle events;
signalling records; and teardown completeness.

`runtime.star` and `runtime.peers` are the **latest** snapshot: transport
diagnostics are counters the star already accumulates, so replacing beats summing.
That is right for counters and wrong for *depths*, which are instantaneous levels
rather than totals. The last snapshot a run takes is nearly always a quiescent
one — after the final pump, or after teardown drained the queues — so a resource
gate written against `peers[*].control.outbound_depth` reads zero however hard
the transport was pushed, and cannot fail.

`runtime.pressure` exists for that: a running peak folded across **every**
transport snapshot, with `samples` saying how many. It carries
`peak_outbound_depth`, `peak_inbound_depth`, `peak_buffered_amount`,
`peak_event_depth`, and `peak_peer_count`, plus the highest cumulative
`backpressure`, `peer_backpressure`, `overflow`, `dropped_outbound`, and
`dropped_inbound` seen. The latch counters matter independently of the depths: a
channel that reached its `bufferedAmount` ceiling and drained again between two
observations leaves no depth behind, only a latch. Anything gating on transport
pressure should read this record, not the peers array — see
[the fault harness](fault_harness.md#resource-measurement-and-teardown).

### Anchors

`input_tick`, `monotonic_ms`, `mapping_error_ms`.

## 3. Privacy boundaries

**Never stored, anywhere, in any form:**

- SDP blobs and everything in them — ICE ufrag/pwd, fingerprints, and candidate
  lines carrying host and server-reflexive addresses.
- Raw IP addresses.
- Clipboard contents.
- Direct identifiers (names, emails, handles, account ids).
- Any playtest or participant payload. The movement telemetry this project
  collects from friends and family lives in `sim/research_*` and has no seam into
  these modules at all — no shared module, no shared serializer, no shared
  version.

**Signalling specifically.** `diagnostic_transport` wraps `request_offer`,
`accept_offer`, `accept_answer`, and `take_signal` and hands each blob to
`net_diagnostics.record_signal`, which keeps three things: the peer id, the
direction, and the byte length. The content field is the constant
`diagnostics_schema.REDACTED`. The blob is not truncated into the record, not
digested, and not hashed — a digest of an SDP is still derived from ICE
credentials, and "we only kept a hash of it" is the kind of half-measure this
schema is meant to avoid. Byte length is kept because a bloated candidate list is
a real thing to debug and a length reveals nothing.

**Free text is redacted, not just shortened.** A star's `last_error`, a peer's
`last_error`, and a runtime event's `detail` are the one place uncontrolled text
enters the system: they carry `String(error)` on a WebRTC or DOM exception and
the browser bridge's own prose, and neither this schema nor the transport
contract constrains a byte of it. Once STUN/TURN is configured, such a string can
contain a candidate line, a relay URL, or a peer address verbatim.

Every one of those fields passes through `diagnostics_schema.redact_free_text`,
which replaces the string **whole** with `[redacted]` if it matches any sensitive
shape — a dotted quad, two or more colons (every IPv6 form), `ice-`, `candidate`,
`fingerprint`, `sdp`, `stun:`/`turn:`/`turns:`, `://`, `@`, or an SDP body line at
a line start. Replaced whole rather than scrubbed in place: partial scrubbing of a
grammar nobody controls is the same half-measure as digesting an SDP instead of
dropping it. This deliberately over-rejects — a timestamp like `12:34:56` is
address-shaped by this rule — because losing a timestamp from a diagnostic detail
is cheap and leaking an address is not. Benign text (`outbound queue reached its
limit of 64 messages`) survives untouched.

This matters beyond the export: `desync_package` embeds `runtime.events` verbatim
into the artifact designed to be attached to GitHub issues, so the redaction has
to happen on the way in. A spec asserts it does, for the export and the package.

**Ids are validated, not trusted.** Anything typed `id` must match
`^[%w][%w_%-%.]*$`. The charset excludes `@`, `:`, `/`, and `\`, so
`player@example.com`, `https://…`, `/home/oscar/save.json`, `c:/users/oscar`, and
`[fe80::1]` all fail on a *character*, not on a substring blacklist that a
cleverly shaped string could slip past. `..` and `@` are additionally rejected as
substrings, and a dotted quad (`192.168.1.14`) is rejected by an explicit pattern
because it is the one shape that satisfies the charset while being exactly the
thing that must never be stored.

Values that fail validation are **counted, not coerced**: every `record_*`
function returns `nil, err` and increments `collection.rejected_values`. A
rejected value never lands in the ring in a mangled form.

## 4. Bounds, truncation, and retention

Collection is always bounded. Defaults:

| Ring | Cap | Keeps |
| --- | --- | --- |
| `checkpoints` | 64 | newest |
| `packets` | 256 | newest |
| `control` | 64 | newest |
| `events` | 128 | newest |
| `signals` | 32 | newest |
| `mismatches` | 16 | **oldest** |
| `anchors` | 32 | newest |
| latency peers | 8 | first seen, then rejects |

`mismatches` keeps the *oldest* on purpose: the first divergence is the causal
one and everything after it is an echo. Everything else keeps the newest, because
a failure is explained by what just happened.

Truncation is never silent. `collection.retention.<ring>` is `complete` or
`truncated`, `collection.dropped.<ring>` says how many entries were lost, and
`net_diagnostics.summary` prints a `truncated` line when any ring has dropped
anything. A shorter array that looks complete is worse than no artifact.

Memory: the rings are fixed-size circular buffers allocated on first write; the
per-tick event digest table is pruned to the rollback session's retained floor
every step. Nothing in the recorder grows with match length.

The one part that costs real memory is `diagnostic_transport`'s optional wire
ring, off unless `retain_wires` is set. At the protocol's 1 KiB envelope bound,
the fixture's 192 wires is a 192 KiB ceiling.

## 5. Export procedure

Export is **opt-in and off by default**. Holding diagnostics in memory is cheap
and local; producing something a person might paste somewhere is a decision.

```lua
local net_diagnostics = require("game.online.net_diagnostics")

local recorder = net_diagnostics.new({
    role = "host",
    peer_id = peer_id,
    manifest = manifest,
    freeze = freeze,
    input_delay_ticks = input_protocol.FAIRNESS_DELAY_TICKS,
    hash_interval_ticks = match_driver.DEFAULT_HASH_INTERVAL_TICKS,
})

-- Each driver step:
local batch = match_driver.advance(driver, sample)
net_diagnostics.record_step(recorder, match_driver.diagnostics(driver), batch)
for _, addressed in ipairs(batch.control) do
    net_diagnostics.record_control(recorder, addressed)
end

-- Locally, any time -- no opt-in needed, nothing leaves the machine:
for _, line in ipairs(net_diagnostics.summary(recorder)) do
    print(line)
end

-- To produce an artifact:
net_diagnostics.opt_in_export(recorder)
local artifact = assert(net_diagnostics.export(recorder))     -- validated table
local bytes = assert(net_diagnostics.encode(recorder))        -- canonical bytes
local digest = assert(net_diagnostics.digest(recorder))       -- fnv1a64 hex
```

`export` validates against the schema and returns `nil, err` if anything fails —
including if a caller managed to record an id that is not pseudonymous. A
diagnostic that would leak does not get produced.

### Which digest to quote

`net_diagnostics.digest` covers the whole artifact, runtime observation included,
so it is **not** stable across runs and must never be quoted as if it were.

`net_diagnostics.canonical_digest` covers `identity` + `canonical` only. That is
the digest to quote when comparing two runs, two peers, or a report against a
rerun.

## 6. Deterministic and repeatable: what is actually claimed

This is worth being precise about, because the easy mistake is to claim
byte-identity for everything and quietly freeze a clock to make it true.

| Section | Claim | How it is tested |
| --- | --- | --- |
| `identity`, `canonical` | **Byte-identical** for the same fixture and the same delivery schedule. | Two runs, `canonical_bytes` compared directly; and one run against another with a different injected clock and RTT bias, which must not move a single canonical byte. |
| `runtime` | **Invariants only.** Sample counts follow the schedule; `rtt_ms_min <= rtt_ms_max`; `monotonic_ms_first <= monotonic_ms_last`; counters are non-negative and monotonic. | Invariant assertions, plus an explicit assertion that the *full* export digest **differs** across two clocks. |
| `anchors` | Each entry declares a non-zero mapping error. | Asserted per anchor. |

The fixture harness injects a fixed clock, which makes the runtime half
reproducible *inside a test*. That proves the recorder is deterministic given its
inputs. It proves nothing about a real machine, and the specs deliberately do not
generalise from it — the byte-identity assertion is made only against
`canonical_bytes`, which contains no millisecond at all.

## 7. Desync capture

`desync_package.build` produces the smallest artifact that lets someone else
reproduce a divergence: identity, the canonical input wires over a bounded
window, the boundary the peers last agreed on plus its hash, the boundary they
disagreed on plus both hashes, the first differing path if the capture holds both
sides, and the ordered control and runtime events.

**No snapshot bytes.** A canonical `MatchSnapshot` is hundreds of kilobytes;
embedding one turns a capture into a file nobody attaches. The agreed boundary is
carried as a tick and a hash, which is what a reproducer needs to *check* it
rebuilt the right state. A hash is evidence; a snapshot is a copy of the match.

### `reproducible_from` is a checked claim

| Value | Means |
| --- | --- |
| `fixture_boundary_zero` | The wires start at the session's first input tick. Rebuild boundary zero from the pinned fixture and you have everything. |
| `tape_reference` | The wires do not reach boundary zero, but a recorded input tape does, and its id, digest, and version are carried. |
| `retained_window` | Neither. Reproduces from the named boundary onward, *if* the reproducer can independently produce the state there. |

`build` derives this by decoding the wires and reading their actual coverage — it
is not passed in and cannot be asserted by the caller. Trim the opening wires and
the claim weakens by itself.

### Offline reproduction

```lua
local rows = assert(desync_package.rows(package, manifest, sender_id))
local session = rollback_session.new(fixture_boundary_zero, sources)
assert(rollback_session.add_authoritative_batch(session, rows))
-- step to package.divergence.agreed_boundary_tick, then compare
-- match_snapshot.hash(...) against package.divergence.agreed_boundary_hash
```

`rows` de-duplicates on `(tick, slot)` — the redundancy window re-sends each row
up to seven times and authorship is a frozen partition, so de-duplicating is
lossless — and returns canonical `(tick, slot)` order. The spec
`rust/crates/gc-netcode/tests/desync_package.rs` does exactly the above against a fresh
`rollback_session` that never saw the session, and requires the reproduced hash to
equal the captured one.

## 8. Attaching artifacts to GitHub safely

A diagnostic export and a desync package are designed to be attachable, but the
procedure matters:

1. **Export deliberately.** `opt_in_export` is a decision. If you did not mean to
   produce an artifact, do not.
2. **Attach the encoded artifact, not a screenshot of a session.** A screenshot
   can catch a lobby code, a window title, or a browser URL bar. The encoded
   artifact contains only what the schema permits.
3. **Do not paste raw signalling.** If you are debugging connection setup, the
   `runtime.signals` records already say a blob of *n* bytes went that way. The
   blob itself carries ICE credentials and addresses and must not be attached,
   pasted, or filed — not in an issue, not in a gist, not in a comment thread.
4. **Quote `canonical_digest` when comparing runs**, and say so. Quoting the full
   digest invites someone to conclude a difference is meaningful when it is only
   a different millisecond.
5. **Say what the package claims.** Include the `reproducible_from` line from
   `desync_package.summary` so a reader knows whether they can reproduce it or
   only inspect it.
6. **Truncation is information.** If `collection.retention` says `truncated`,
   leave it in. A reviewer needs to know the window closed.

The artifacts contain no participant data because no participant data is ever
collected by these modules. Movement telemetry from friends-and-family playtests
is a separate system with separate storage, and the two share no code.

## 9. Versioning

`diagnostics_schema.SERIALIZATION_VERSION` and `net_diagnostics.SCHEMA_VERSION`
are both 1, and `desync_package.VERSION` is 1. Bumping the serialization version
changes every canonical preimage and therefore every digest; it is a coordinated
breaking change and existing artifacts are not comparable across it.
`digest_algorithm` is `fnv1a64/v1` and is carried in the artifact, so a future
SHA-256 digest becomes a new value rather than a silent reinterpretation of
stored ones.

## 10. What this does not do

No telemetry backend, no analytics profile, no automatic upload, no crash-report
vendor, no IP geolocation, no device fingerprinting. No desync *fixing*, no
reconnect, no resync, no adaptive tuning, no public status UI. And no path by
which a runtime log becomes canonical simulation evidence.
