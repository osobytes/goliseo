# OMP-3 input bundles and canonical host batches

`game.online.input_protocol` is the pure data-plane contract for the direct-host
combat-soccer fixture. Guests use it to send one assigned slot to the host. The
host validates every producer, canonicalizes all envelopes polled on one
transport tick, and rebroadcasts one ordered authority batch. It does not create
WebRTC peers, advance a match, infer missing input, or make the host a simulation
authority.

## Identity and clocks

Input packets use protocol version 1 and carry `InputFrame` sample version 2.
Changing the sample bytes, row layout, six-row redundancy, three-tick fairness
delay, ordering, identity, or bounds requires an explicit version decision.

The established session link supplies the full `session_id` and sender/producer
id as decode context. The wire carries the accepted 16-hex `manifest_id` and a
16-hex packet id derived from:

```text
guest: GCIG;1; + length(session) + session + length(sender) + sender + length(sequence) + sequence
host:  GCIH;1; + length(session) + session + length(sender) + sender + length(sequence) + sequence
```

Guest and host domains cannot collide. A packet replayed on another
session/sender context fails its packet id before application. These FNV-1a
identities are deterministic correlation and duplicate ids, not authentication
or anti-cheat. #161's established peer link and frozen assignment remain the
trust boundary.

Transport time and input authority time stay separate:

- `transport_tick` is when the sender placed the envelope on its input channel.
- The outer input envelope's `seq` and `tick` must equal packet `sequence` and
  `transport_tick`.
- Every authority row carries its own simulation input `tick`.
- A host batch uses the host transport tick on which its complete arrival set
  was collected. Its rows may name several input ticks.

No input tick is derived from a transport tick.

## Canonical sample and row encoding

`sim.input_frame.encode_sample` maps one complete version-2 sample to four
explicit bytes:

| Byte | Meaning |
| --- | --- |
| 1 | `move_x + 127`, range 0 through 254 |
| 2 | `move_y + 127`, range 0 through 254 |
| 3 | complete held mask, including equipment hold |
| 4 | complete edge mask, including equipment press/release |

Its public ASCII form is eight lowercase hex characters. Packet records carry
the same four bytes directly; combat edges are never omitted, repeated, or
inferred.

Each packet record is nine bytes:

```text
u32 big-endian input tick | u8 canonical slot index | four sample bytes
```

The complete record block is base64 encoded canonically. Rows must be strictly
ordered by `(input tick, canonical slot index)`, with no duplicate key inside a
wire packet.

The ASCII wire has thirteen semicolon-separated fields:

```text
GCIP ; packet-version ; G-or-H ; input-version ; manifest-id ;
sequence ; packet-id ; transport-tick ; first-input-tick ;
confirmed-span ; input-delay-ticks ; row-count ; base64-record-block
```

All integers use canonical unsigned decimal: zero is `0`; other values have no
leading zeroes. The decoder re-encodes the complete packet and requires exact
byte equality.

## Confirmation feedback

`confirmed-span` is the count of contiguous input ticks the **sender** has
confirmed, measured from `first_input_tick`. Zero means it has confirmed
nothing. `input_protocol.confirmed_tick` recovers the tick:
`first_input_tick + confirmed_span - 1`.

It is a span rather than a tick for one reason: a sender that has confirmed
nothing sits at `first_input_tick - 1`, which is `-1` for a session that starts
at tick zero, and this wire has no signed encoding. The span makes that state
`0` and keeps every integer on the wire canonically unsigned.

Both roles fill it, over the traffic that already exists — no new message kind,
no round trip, and nothing to wait on:

- a **guest** reports where its own confirmation has reached, which is what lets
  the host stop fanning out blind. Without it the host gives identical
  redundancy to a guest that is fully caught up and to one that is stuck, and a
  row that ages out of the redundancy window is unrecoverable. See
  `docs/online/match_driver.md` for what the host does with it.
- the **host** reports the same thing in its canonical batches, which is what
  lets a settling guest know whether the tail it authored ever reached the
  sequencer. A guest's last authored rows exist nowhere else until the host has
  them.

The field is additive and costs at most eleven bytes of header. It moved every
pinned conformance golden, because it changes the bytes of every packet.

## Guest redundancy

A guest packet contains one and only one frozen source slot. If its current row
is input tick `N`, it contains every row from:

```text
max(first_input_tick, N - 6) through N
```

in oldest-first order. At steady state this is current plus exactly six prior
rows. During the first six session ticks it contains only rows that were
actually sampled after the synchronized start boundary. A gap, extra older row,
mixed slot, missing current row, or noncanonical order fails before host
collection.

Six prior rows recover up to six consecutive lost emissions when a following
packet arrives. A seventh loss can let the oldest missing row fall out; the
codec never creates that authority. The existing 30-tick rollback floor remains
the separate late-input limit.

## Host collection and ownership

`canonical_host_batch` receives every guest-shaped envelope available on one
host transport tick, including host-local human and deterministic bot
producers. It:

1. validates the accepted manifest and complete frozen slot assignment;
2. validates every packet, its context identity, and its exact outer envelope;
3. requires peer producers to arrive on their own selected link;
4. permits bot producers only on the host link;
5. requires host-link producers to spend at least the versioned three transport
   ticks in the same collector path;
6. classifies repeated sender sequences before looking at authority;
7. unions rows by `(slot, input tick)`;
8. rejects any conflicting sample instead of selecting the first poll result;
9. sorts the union by `(input tick, slot)`; and
10. emits one host packet for that complete transport-tick boundary.

The host copies valid sample bytes. It cannot change a guest row, claim another
producer's slot, inject its own human row directly, or bypass the fixed
three-tick fairness delay through this API. The host remains a sequencer. It
does not fill missing slots, predict input, select match outcomes, or overwrite
simulation state.

Bot producer ids remain distinct from peer ids. Their packet sender is the
frozen bot producer, while their transport peer is the host that owns their
delayed collector path.

## Duplicates, conflicts, and atomic application

There are two identities:

- `(kind, session, sender, sequence)` identifies a packet. Repeating that
  identity is idempotent only when the canonical bytes are identical. Reuse
  with different bytes is `packet_conflict`.
- `(slot, input tick)` identifies authority. Repeating it is idempotent only
  when all four sample bytes are identical. Different bytes are
  `authority_conflict`/`conflicting_authoritative`.

Sequence gaps and lower sequences arriving after higher ones are valid on the
unordered input channel. Coordinators may record them but must not use
arrival order as authority.

`sim.rollback_input_history.add_authoritative_batch` preflights the entire
canonical row array, including malformed rows, retained-floor failures,
within-batch duplicates/conflicts, and conflicts with retained history. Only
then does it insert new rows. A rejected row therefore cannot leave earlier
rows partially applied.

`sim.rollback_session.apply_authoritative_batch` inserts that complete batch
and invokes the existing reconciliation path exactly once. Prediction,
restore/resimulation, and presentation confirmation retain their OMP-2
semantics.

## Size, queues, and backpressure

Both guest and host packets have an explicit 1,024-byte payload limit, matching
`game.transport.contract`. The input codec does not reuse the reliable control
protocol's separate 8,192-byte bound.

The checked-in maximum fixture uses:

- eight slots;
- nine ticks;
- all 72 explicit rows;
- maximum tick, sequence, session-id, and sender-id contexts;
- a `first_input_tick` and `confirmed_span` that are *simultaneously* ten
  digits, which is not the same thing as each being at its maximum — the span is
  bounded by `MAX_TICK - first_input_tick + 1`, so pushing the first tick to its
  ceiling would collapse the span to one digit and understate the header; and
- explicit valid axis, held, and edge bytes.

Full context ids affect the fixed-size packet id rather than being repeated on
wire. The resulting packet is **958 bytes**, leaving **66 payload bytes** below
the 1,024-byte limit. Each row costs 9 raw bytes and exactly 12 canonical base64
bytes, so the hard ceiling is **77 rows at 1,018 bytes** and a 78th row is
refused `wire_too_large`. The 72-row bound therefore sits five rows under the
wall rather than on it, and `MIN_WIRE_MARGIN_BYTES` pins that slack so the next
additive header field has to notice it. Any oversized message returns
`wire_too_large`; there is no implicit fragmentation or partial host batch.

The bound is a **measurement**, not a design intent. It was `SLOT_COUNT *
RETAINED_ROWS` — "seven ticks of history for eight slots" — until #243 measured
the byte budget it was supposed to be sized against and found it spending 755 of
1,024 bytes. See `docs/online/match_driver.md` for what the extra rows buy.

At 60 Hz the host fans one batch to seven guests, or 420 sends per second. A
single shared 64-message queue represents only about 152 ms of fan-out; even a
per-peer 64-message queue would retain roughly 1.07 seconds of stale packets.
Downstream transport must use per-peer `bufferedAmount`/queue diagnostics and
must not grow an authority queue without bound.

`supersede_for_backpressure` permits replacing an unsent packet only when the
newer packet contains every older `(slot, tick, sample)` byte-identically.
Ordinary next-tick guest bundles normally fail this check because the oldest
row falls out. The caller must then report/drop through its explicit overflow
policy rather than pretending redundancy preserved the row.

The 958-byte application payload is below the proof bridge's 1,024-byte payload
cap, but percent-escaped outer envelopes, SCTP/DTLS/IP overhead, browser
scheduling, and actual MTU fragmentation remain #164/#170 measurements. A
passing codec size test is not a claim that a browser data channel never
fragments.

## Frozen cross-runtime evidence

`game.online.input_protocol_conformance` pins:

- one complete literal guest wire and digest;
- one complete literal host wire and digest;
- schema, InputFrame, redundancy, and fairness versions;
- the exact 958-byte maximum fixture and its 66-byte margin; and
- the snapshot and combat schema versions the literals were generated against.

The `manifest-id` field of every wire is a hash over the session manifest, which
carries the snapshot and combat schema versions, so bumping either version
invalidates every literal above. The verifier checks the pinned versions first
and reports which version the goldens were built for, instead of failing with an
opaque byte mismatch. Regenerate the literals in the same change that bumps the
version.

Native tests call the verifier directly. `love . --determinism` emits one
`GC_INPUT_PROTOCOL|golden|...` marker before running the OMP-1 evidence.
`scripts/browser_determinism.py` requires that exact marker in every pinned
Chrome and Firefox run, so native Lua and love.js execute the same literal
decoder/re-encoder rather than trusting two implementations that drift
together.
