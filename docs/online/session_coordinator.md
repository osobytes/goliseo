# OMP-3 direct-host peer and match lifecycle coordinator

`game.online.coordinator` is the pure control-plane state machine that turns the
[session protocol](session_protocol.md) vocabulary into a session: it admits
peers, negotiates the immutable manifest, assigns the eight canonical OMP-1
outfield slots, holds the readiness barrier, freezes the match at countdown,
names one simulation start boundary, acknowledges the simulation's own result,
and ends every session with a stable reason.

It creates no WebRTC peers, opens no data channels, draws no lobby, encodes no
input packets, and advances no rollback. It has no clock of its own: time enters
as explicit `tick` events, so a whole session replays deterministically without
LÖVE, a display, or a network.

## Roles and identities

One coordinator instance models one peer. A host instance admits itself at
construction, so only a guest ever observes the `new` phase. Every instance
keeps the same shape: `peers[1]` is always the local peer, and every other entry
carries the transport `link_id` it speaks over. A guest therefore tracks exactly
two peers — itself and the host — and knows nothing about its fellow guests.

Link identity is authoritative. A guest claims a `peer_id` in its handshake; the
host binds that id to the arriving link and confirms it with `peer_assignment`.
Afterwards a message whose `peer_id` disagrees with its link is a protocol
violation, so no peer can speak for another.

## Lifecycle phases

The coordinator reuses #161's `SessionLifecyclePhase` rather than inventing a
parallel set:

| Phase | Meaning |
| --- | --- |
| `new` | A guest has not sent its handshake yet. |
| `handshake` | Admission is open; guests may still join. |
| `manifest` | The immutable manifest is proposed; ownership is unpublished. |
| `assigned` | All eight slots have a declared source; peers may ready up. |
| `ready` | Every tracked peer is ready; the host may start a countdown. |
| `countdown` | Manifest, ownership, and start boundary are frozen. |
| `running` | The simulation owns the match; the coordinator acknowledges it. |
| `result` | Full time was reached; the result is being acknowledged. |
| `terminal` | The session ended, with one stable reason. |

`terminal` carries an explicit `CoordinatorTerminal` record — reason, protocol
code, origin (`local`, `remote`, or `timeout`), the peer involved, and a
non-localised detail string — so "aborted" and "closed" are distinguishable
without a separate phase.

## Admission versus termination

Failures before a link becomes a peer refuse that link alone: the host sends
`abort` on it, closes it, and leaves the roster, manifest, readiness, and phase
of everyone else untouched. Capacity (more than eight humans), a duplicate peer
id, a second claimed host, an incompatible runtime, a foreign session id, a
malformed wire, and any join attempt after the manifest is proposed all resolve
this way. There is no join-in-progress and no reconnect.

Once a link is an admitted peer, the same classes of failure end the session,
because a peer that has already influenced lifecycle state cannot be
un-influenced. The one exception is voluntary or transport departure before the
countdown, which is a roster change rather than a failure: the departing peer is
removed, readiness is cleared, and ownership that named it is voided so the host
must republish before anyone can be ready again. The lifecycle phase itself is
not rewound past `assigned`.

## Manifest and ownership

The host proposes the complete #161 manifest exactly once; the digest is
computed by `protocol.manifest_id`, and any later proposal with a different
digest is rejected — the manifest is immutable, and a change needs a new
session. A guest compares the proposal against a `CoordinatorManifestExpectation`
(build, source, content, tuning, match-configuration, fixture, arena,
combat-rules, gameplay-AI-policy identity, and combat disposition) in a fixed
order and reports the first differing path. Session-scoped values such as the
seed are the host's to choose and are not pre-known.

`plan_assignments` seats humans in canonical `home_1..home_4`, `away_1..away_4`
order and fills the remaining slots with bots whose `producer_id` is
`bot.<slot>` and whose `bot_seed` is derived from the manifest seed. It performs
no team balancing — that is deliberately out of scope. At every point where
ownership is published or frozen, `slot_sources` proves that all eight canonical
slots have exactly one declared source, that producer ids are unique across the
peer and bot namespaces, that every admitted peer owns exactly one slot, and
that no combat-protected keeper is seated.

## Readiness

A peer may only become ready after accepting the exact manifest digest and
owning a slot; a `ready` that precedes acceptance is a protocol violation.
Readiness is revocable until the countdown freezes, and any ownership change
clears it.

Ownership changes race with in-flight readiness, and the `ready` body carries
only the immutable `manifest_id` — never an assignment identity — so the host
cannot tell "ready for the previous ownership" from "ready for the current one"
by inspection. Owning *a* slot is not sufficient evidence either: a swap can
leave a peer owning exactly one slot in both generations.

The coordinator closes this without extending the wire, by making the peer
prove it observed the generation:

1. publishing ownership marks every remote peer *unconfirmed* and clears its
   readiness;
2. a guest that accepts a *changed* assignment always answers `ready = false`,
   whether or not it was ready — that falling edge is the acknowledgement;
3. the host treats a negative readiness as the confirmation, and refuses to
   count a positive readiness from an unconfirmed peer.

Per-link FIFO puts the acknowledgement after anything already in flight for the
previous generation, so a stale `ready = true` is always refused and the
following confirmation always lands. A byte-identical republication is
idempotent: it is not a new generation and preserves readiness.

The refusal of stale or unconfirmed readiness is a *rejection*, not a
termination — the peer did nothing wrong, it simply answered for a
configuration that no longer exists, and it will answer again.

## Countdown, freeze, and the start boundary

`begin_countdown` requires the `ready` phase and freezes a `CoordinatorFreeze`:
manifest digest, countdown id, first input tick, seed, tick rate, duration, goal
limit, content/tuning/combat-rules/gameplay-AI identity, combat disposition, a
deep copy of the assignments, and the slot-to-source table. After the freeze,
assignment and readiness changes are rejected.

The countdown is measured in coordinator `tick` events, not wall-clock time.
Wall clock is never simulation authority: the single canonical boundary is
`first_input_tick`, a simulation tick index that every peer receives in the same
`countdown` and `start` bodies. When the countdown drains, the host publishes
`start`; every guest echoes the identical body back as its acknowledgement,
enters `running`, and emits a `start_match` action carrying the freeze. The host
enters `running` when every guest has acknowledged. A guest that never
acknowledges within `START_ACK_TIMEOUT_TICKS` ends the session with
`start_ack_timeout`.

## Match lifecycle

The simulation owns score, ticks, and outcome; the coordinator only
acknowledges. It enforces ordering rather than content: a running match opens
with `kickoff`, `playing` follows `kickoff`, `goal_stoppage` and `full_time`
follow `playing`, `kickoff` follows `goal_stoppage`, ticks and scores never move
backwards, and a `goal_stoppage` must follow an actual increase in the score.
`finish` is refused unless full time was reached and its scores match what the
simulation last reported, so the control plane cannot restate a result.

Boundary hashes are symmetric: both roles publish `hash_report`, and a peer
whose hash disagrees at a tick the receiver also hashed increments a consecutive
mismatch counter. `MAX_HASH_MISMATCHES` consecutive disagreements end the
session as `hash_mismatch`; a single disagreement is tolerated and cleared by
the next agreement. Full time is the canonical `running` -> `result` boundary
for a guest, so the host's `result` body and `result_ack` arrive in a legal
phase. The session completes only when every peer has acknowledged the identical
result; a differing acknowledgement is a desync.

## Terminal reasons

Local reasons are specific; the wire stays inside #161's closed rejection codes.

| Reason | Code | Raised when |
| --- | --- | --- |
| `completed` | — | Every peer acknowledged the same result. |
| `local_abort` | `host_abort` | This peer deliberately ended the session. |
| `peer_abort` | (carried) | A peer sent `abort`. |
| `guest_left` | `peer_disconnect` | A guest departed after the freeze. |
| `host_left` | `peer_disconnect` | The host link was lost or announced. |
| `removed` | `peer_disconnect` | The host disconnected this guest. |
| `transport_lost` | `peer_disconnect` | A frozen link failed. |
| `protocol_violation` | `malformed_message` / `invalid_phase` | Malformed, out-of-phase, misdirected, spoofed, or conflicting traffic. |
| `manifest_mismatch` | `manifest_mismatch` | Deterministic identity disagreed. |
| `invalid_assignment` | `invalid_assignment` | Published ownership was unusable. |
| `start_ack_timeout` | `peer_disconnect` | A peer never reached the start boundary. |
| `input_channel_failure` | `peer_disconnect` | #162's input channel failed terminally. |
| `late_input` | `desync` | The rollback window overflowed terminally. |
| `hash_mismatch` | `desync` | Persistent boundary-hash or result disagreement. |

`input_channel_failure` and `late_input` are reported *into* the coordinator as
`netcode_failure` events. The coordinator does not detect them: sequencing,
delay, and resimulation belong to #162 and #166. It only guarantees that such a
failure produces one stable reason and no hidden progress.

Two of these mappings are lossy and should be revisited when #164 and #166 land.
`late_input` and `hash_mismatch` are causally different classes — an overflowed
rollback window versus divergent simulation — that share the `desync` wire code
because #161 has no closer one. The local reason stays exact; only the byte on
the wire is coarse. Likewise, every decode or validation failure other than an
unsupported version folds into `malformed_message`.

## Duplicates and out-of-order control traffic

Sequences are sender-local and strictly increasing but not contiguous, because
some messages (`peer_assignment`, an admission refusal) address a single link.
A sequence above the last one seen is new. A sequence at or below it is a
duplicate: byte-identical bytes are an accepted no-op, and the same identity
with different bytes is terminal, exactly as #161 requires. Duplicates are
classified before phase validation, so a retransmitted terminal message cannot
revive a session.

Byte comparison needs the original wire, and only the last `DUPLICATE_WINDOW`
messages per peer are retained. That window bounds *conflict detection*, never
session survival. Nothing in #161 bounds how late a reliable transport may
retransmit, so a genuine retransmission can always age out of any finite
window — ending the session there would kill a healthy match over an ordinary
loss-and-retry. An unprovable duplicate therefore fails **open**: it is dropped
without being applied, reported as an accepted no-op carrying
`STALE_DUPLICATE_REASON`, and nothing advances. Dropping is strictly safer than
applying, and the case the window might have caught is refused either way,
because the message is never applied. Semantically idempotent repeats occupy
window slots like any other message, which is correct rather than merely
tolerable: a retransmission of one of *them* must stay classifiable too.

Semantic repeats that are not wire duplicates are idempotent too: re-admitting,
re-accepting, re-readying, re-starting, and re-acknowledging a result all leave
the session unchanged.

## Purity and evidence

`coordinator.step(state, event)` is pure and copy-on-write. It never mutates the
state it is given, and a rejected event returns the *identical* state table, so
"no hidden progress" is a testable property rather than a claim. Effects leave
as data: `send` (with the ordered link targets), `close`, `start_match` (with
the freeze), and `terminate` (with the terminal record).

`game.online.coordinator_driver` supplies the fake clock and fake reliable
control links used by the tests. It runs up to eight real coordinator instances
that exchange real canonical `GCOP` wires, supports per-link latency, link
loss, replayed packets, and `inject` — a message the sender's own coordinator
would never emit, which is how a misbehaving peer is modelled without adding a
test-only entry point to the coordinator itself.
`game.online.coordinator_conformance` pins the canonical eight-human,
bot-filled, and solo sessions.

Two dimensions are covered as systematic cross-products rather than chosen
scenarios, so a phase or message kind added later cannot escape coverage:

- **phase × control message kind**, in both directions, asserted against
  `protocol.validate_phase` as the oracle. Legal cells must not be refused as
  out of phase; illegal cells must terminate with `invalid_phase`. The oracle
  mirrors the one documented remap (a republished `slot_assignment` during
  `ready`) and nothing else.
- **phase × local event kind**, asserting the reducer is total: every
  combination returns a disposition and an action list, and every rejection
  returns the identical state with a code.

Beyond the matrices, the specs cover ownership invariants, the readiness
generation rule, aged-out retransmission, adversarial hosts and guests, and
every abort path. "Exhaustive" applies to the two matrices above; the remaining
coverage is scenario-based, and content-level outcomes within a legal cell (for
example which specific mismatch a body triggers) are asserted case by case
rather than exhaustively.

## Assumptions

A guest verifies that published ownership seats *it* in exactly one slot, but it
cannot verify that the whole roster maps one-to-one onto genuinely admitted
peers — it never sees the roster. That check lives on the host. This is a
property of the single-trusted-host topology #161 defines, not a gap: OMP-3
provides mismatch detection and useful termination, not anti-cheat. A host that
lies about ownership can already choose the manifest.
