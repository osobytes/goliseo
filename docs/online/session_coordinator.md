# OMP-3 direct-host peer and match lifecycle coordinator

> **Module names below are pre-port `require` paths.** The contract this document
> describes is current; the way it names files is not. Read `sim.foo` as
> `gc_sim::foo` (`rust/crates/gc-sim/src/foo.rs`), `game.online.foo` as
> `gc_netcode::foo`, `core.foo` as `gc_core::foo`, `data.foo` as `gc_data::foo`,
> and any `game/**` or `spec/**` path as its `ts/packages/**` counterpart. A
> `love .` command, a `love.*` API or a love.js measurement is **pre-port
> evidence** — commit `2c0d449` (#467) deleted that tree.

`game.online.coordinator` is the pure control-plane state machine that turns the
[session protocol](session_protocol.md) vocabulary into a session: it admits
peers, negotiates the immutable manifest, assigns the eight canonical OMP-1
outfield slots, holds the readiness barrier, freezes the match at countdown,
names one simulation start boundary, acknowledges the simulation's own result,
and ends every session with a stable reason.

It creates no WebRTC peers, opens no data channels, draws no lobby, encodes no
input packets, and advances no rollback. It has no clock of its own: time enters
as explicit `tick` events, so a whole session replays deterministically without
a renderer, a display, or a network.

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
| `manifest` | The immutable manifest, including the match mode, is proposed; ownership is unpublished. |
| `assigned` | All eight slots have a declared source and every owned set matches the mode; peers may ready up. |
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

The manifest also carries the [match mode](session_protocol.md#match-modes-and-slot-ownership),
which fixes how many of the eight canonical slots each human owns: four in
`1v1`, two in `2v2`, one in `4v4`. Because admission closes when the manifest is
proposed and the mode is immutable afterwards, a proposal is refused with
`capacity` when more humans are already admitted than the mode seats, rather
than leaving the lobby to deadlock at assignment time. Admission itself still
bounds at `MAX_PEERS` — the mode is not known while guests are still joining —
so a too-large lobby is caught at proposal, not at handshake.

`plan_assignments` seats humans in contiguous canonical blocks of
`slots_per_human` over `home_1..home_4`, `away_1..away_4`: the Nth human owns
slots `(N-1)*k+1 .. N*k`. Four slots per team divide evenly by every supported
`k`, so a block never straddles the halfway line, and at `k = 1` the result is
byte-identical to the one-slot-per-peer seating OMP-3 already shipped. Remaining
slots are filled with bots whose `producer_id` is `bot.<slot>` and whose
`bot_seed` is derived from the manifest seed. It performs no team balancing —
that is deliberately out of scope.

At every point where ownership is published or frozen, `slot_sources` proves
that all eight canonical slots have exactly one declared source, that a bot
producer drives exactly one slot, that producer ids never cross the peer and bot
namespaces, that one human's owned slots all lie on one team, that every owned
set has exactly the size the frozen mode requires, and that no combat-protected
keeper is seated. `validate_local_assignments` adds the host-only half the
guests cannot check: that those owned sets map onto genuinely admitted peers and
leave nobody unseated.

## Owned sets, the live slot, and switching

A human owns a set of slots and controls exactly one of them at a time — the
*live* slot. `owned_slots(state, producer_id)` returns that set in canonical
order; the freeze records it per human as `owned`, together with `live`, the
slot live at the first tick. The opening live slot is the first owned slot in
canonical order, derived from the frozen assignments, so every peer computes the
same one without exchanging anything further.

`next_live_slot(owned, live, transition)` is the transition rule, and it is the
shipped single-player rule from [`docs/controls.md`](../controls.md) intersected
with the owned set:

1. if the slot that won the ball this tick is owned, it becomes live;
2. otherwise, on the `switch` edge while the live slot is not the carrier, the
   first owned slot in the simulation's deterministic distance-to-ball ranking
   becomes live;
3. otherwise the live slot is unchanged.

`switch` is already a bit in the canonical input frame, read from the input row
of the currently live slot; `carrier`, `winner`, and `ranked` come from the
deterministic simulation. Nothing local — presentation, frame timing, or when a
key was physically pressed — enters the decision, so every peer evaluates the
same transition at the same tick.

In 4v4 the owned set has exactly one member, so every branch returns the slot
already live and switching is inert. That is a consequence of the general rule;
there is no mode branch anywhere in it, and adding one would be the bug.

`slot_drivers(freeze, live)` says who materializes each slot's input row for a
tick: `human` only for a human's live slot, `ai` for every other owned slot and
for every declared bot fill. Non-live owned slots are therefore
indistinguishable from bots in the input stream, which is exactly how solo play
already treats the players you are not controlling.

**Keepers stay AI-only, unassignable, and slotless in every mode**, a deliberate
divergence from solo play that is spelled out in the
[protocol document](session_protocol.md#match-modes-and-slot-ownership).

## Readiness

A peer may only become ready after accepting the exact manifest digest and
owning a slot; a `ready` that precedes acceptance is a protocol violation.
Readiness is revocable until the countdown freezes, and any ownership change
clears it.

Ownership changes race with in-flight readiness. The manifest digest cannot
settle that race — it is immutable and shared by every generation — and neither
can slot ownership, since a swap can leave a peer owning an identically sized
owned set both before and after.

The generation is therefore named explicitly on the wire. Every publication
increments an epoch and mints an `assignment_id` (see
[ownership generations](session_protocol.md#ownership-generations)); the host
stores it, each guest stores whatever it last received, and every `ready` body
carries the generation it answers for. The host counts readiness only when that
identity equals the generation currently in force.

Two earlier designs for this failed review, and both failed the same way, which
is worth recording so it is not retried:

- **Ownership-based.** Checking that the peer owns *a* slot. Defeated by a swap
  that leaves the peer owning a same-sized set in both generations.
- **Ordering-based.** Treating a negative readiness as an acknowledgement and
  relying on per-link FIFO to sequence it after anything stale. Defeated by two
  republishes in flight against one readiness answer: the negative arrives after
  the host has already advanced, re-arms the *new* generation, and the stale
  positive behind it is then accepted for ownership the peer never saw.

Both inferred the generation from something other than the generation. Per-link
FIFO orders messages within a link; it says nothing about which publication a
peer had seen when it spoke. Only an explicit identity answers that.

Owned sets and the match mode both interact with this, and the reasoning is
recorded in the
[protocol document](session_protocol.md#ownership-generations): owned sets are
already inside the digest, because they *are* the assignments array, so
repartitioning a 2v2 pair necessarily mints a new generation; the mode is
deliberately outside it, because it is manifest-immutable and `manifest_id` is
checked before `assignment_id` on every message that carries both.

Consequences:

- a byte-identical republication is still a *new* generation, because the epoch
  differs — restoring earlier ownership cannot revive readiness for it;
- the freeze records the exact `assignment_id` it froze;
- readiness naming a superseded generation is *rejected*, not fatal. The peer
  did nothing wrong: it answered honestly for what it knew. It will answer again
  for the generation it now holds.

## Pair preferences

`prefer_pair` is how a peer asks for the outfield slots it wants to control. A
guest cannot decide — only the host holds every peer's ownership and every
peer's claim — so a guest records the request as pending and sends a
`pair_preference`. The host is a seated peer like any other and answers its own
request with the same rule.

`coordinator.evaluate_preference(state, peer_id, slots)` is that rule, and it is
pure. It refuses in a fixed order, so two hosts would report the same reason:

| Reason | Refused because |
| --- | --- |
| `after_freeze` | Ownership is frozen; there is no next generation to name. |
| `superseded` | The request names an ownership generation no longer in force. |
| `not_seated` | The peer owns nothing in the ownership in force. |
| `invalid_slot` | The set is malformed, or is not `slots_per_human` slots. |
| `wrong_team` | A requested slot is not on the team the peer is seated on. |
| `detached` | The set keeps none of the slots the peer already owns. |
| `already_taken` | A requested slot is inside another peer's claimed pair. |

Anything else is granted, and a request for the set the peer already owns is
`unchanged`: nothing is published and no readiness clears, which is what makes a
repeated preference harmless.

**Why the path is inert in `1v1` and `4v4`.** Not by naming them. A request must
be a same-team set of exactly `slots_per_human` slots that keeps at least one
slot the peer already owns. In `1v1` an owned set is the team's whole outfield
line, so the only set that satisfies it is the one already owned; in `4v4` an
owned set is a single slot, so a set that keeps one of the peer's slots *is*
that slot. Both modes answer `unchanged` to everything they can express — the
same shape of argument that makes switching inert in `4v4`.

**Claims, and why `already_taken` exists.** The seating a host plans is
provisional; a granted or `unchanged` preference is a peer's explicit choice and
is recorded as its claim. A grant may move a peer that has not claimed — it
receives the requester's vacated slots, so every owned set keeps the size the
mode fixed — but it can never move a peer that has. That is what settles two
guests racing for one pair: the first is granted and claims it, and the second
is refused `already_taken` instead of taking it straight back.
`publish_ownership` asserts that invariant on every publication.

A claim is recorded and dropped in `publish_ownership`, which sets every peer's
claim from the `retained` set it is handed; a peer left out of that set has none
afterwards. An explicit host republication hands it nothing and so drops all of
them, because the host has just overruled every guest's choice.

There is **one deliberate exception**, in both `unchanged` branches
(`handle_prefer_pair` and `apply_pair_preference`). An `unchanged` verdict fires
only when the requested set *is* the set the peer already owns, so the claim it
records describes the ownership already in force and no publication is owed.
Routing it through `publish_ownership` would mint a generation and clear every
peer's readiness in order to announce that nothing moved, which is precisely what
`unchanged` exists to avoid. The written value cannot violate the partition — it
is the peer's own owned set by construction — and a later reseat reads it like
any other claim. Every other write to `pair_choice` goes through
`publish_ownership`, which is what lets a reseat settle claims in one place.

**A roster change reseats around the claims it can keep.** A seating plan is
derived from the roster alone and knows nothing about pairs, so publishing one
as it stands after a departure would take a granted pair back with no message.
`coordinator.reseat_claims(state, plan)` stands between the two, and is pure. A
claim survives when the plan still has room for it exactly as it was: the right
number of slots for the mode, every one of them a slot the plan seats a human
on, all on one team, none already kept for another peer. What survives is held
fixed and the plan fills in around it — the peers with no surviving claim take
the remaining human slots in the plan's own order, `slots_per_human` at a time.
Each team's share of the human slots is a multiple of `slots_per_human` and
every claim sits inside one team, so what is left over is too and no owned set
can straddle the halfway line. No mode is named to make that true.

A claim the new roster cannot hold is dropped deliberately, and the peer that
made it is told — without anything being sent to tell it. The published
ownership either seats the pair that peer was granted or it does not, and each
end reads that answer off the generation it already holds: the host in
`publish_ownership`, a guest in `apply_slot_assignment`. The record becomes
`rejected` with the typed reason `reseated`, which is why that reason is minted
locally and never appears on the wire.

`reseated` is therefore **cause-neutral**, and its lobby text is too. A peer that
lost a pair sees only the ownership that took it, and cannot tell a roster change
from the host reasserting its seating order; a `SWAP` reaches this the same way a
departure does. Naming a cause the peer cannot observe would be inventing one.

A guest that is still waiting on a verdict when a publication lands is a separate
case: `apply_pair_preference_result` adopts the host's answer as sent, so a grant
whose pair a later publication has already seated away reads as `granted` until
that publication's `slot_assignment` arrives and `settle_preference` rewrites it.
The window is expected and closes itself.

A pre-countdown departure voids ownership but no longer voids the claims: until
the reseat that follows, no claim can be acted on anyway, because a preference
against an unpublished ownership is refused `not_seated`.

`state.preference` is the local peer's last request and the answer to it, kept
so a lobby can say what happened. It is a record and never an authority: the
ownership in force is `state.assignments`, and a peer that reads both will see a
granted pair it was later moved out of as exactly that.

**One generation mechanism.** Neither a granted preference nor a roster reseat
mints anything of its own. Both call `publish_ownership`, the same function the
host's own `assign_slots` calls, which bumps the epoch, mints
`protocol.assignment_id(assignments, epoch)`, clears readiness, and emits the
`slot_assignment`. There is exactly one place a generation is decided, however
ownership came to move.

## Countdown, freeze, and the start boundary

`begin_countdown` requires the `ready` phase and freezes a `CoordinatorFreeze`:
manifest digest, the exact ownership generation, countdown id, first input tick,
seed, tick rate, duration, goal limit, content/tuning/combat-rules/gameplay-AI
identity, combat disposition, the match mode, a deep copy of the assignments,
the slot-to-source table, and each human's owned set and opening live slot.
After the freeze, assignment and readiness changes are rejected — so a 2v2
human's chosen pair, which is just which two slots name that human, cannot
change mid-match.

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
| `build_mismatch` | `manifest_mismatch` | The proposed `build_id` or `source_id` is not this build's. |
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

`build_mismatch` is the same idea used deliberately. It shares the
`manifest_mismatch` wire code — the closed #161 vocabulary has no code for
builds, and inventing one would be a protocol change to say locally what
`manifest_mismatch` already says — but it is a separate *local* reason because
its fix is unlike every other identity failure: not "agree on content", but
"install the same build on both machines". It is raised only for `build_id` and
`source_id`, the two expectation fields derived from `ts/packages/app/src/build_info.ts` and
the control vocabulary; every other field stays `manifest_mismatch`. See
[the match flow document](match_flow.md) for why the vocabulary is part of
`build_id` at all.

## Departures

Before the freeze, a guest that aborts or sends traffic the session cannot
accept is **dropped**: the lobby stands and can still be filled. That is #163's
rule and it is unchanged. What is new is that the host records why.

`state.departure` is the host's own account of the last seat it lost —
`{ peer_id, reason, code, detail }`. `code` is the `disconnect` code actually
announced, so the wire is untouched; `reason` is a local
`CoordinatorTerminalReason`, because a drop and a termination answer the same
question and deserve the same vocabulary. `coordinator.DISCONNECT_REASONS` is
the mapping from one to the other, and it is public because a reader that shows
departures has to cover all of it.

One reason may be chosen rather than mapped, and **which drops may choose it is
decided at each call site, not inside `drop_guest`.** There are four:

| Site | Code | May name a build? |
| --- | --- | --- |
| `apply_abort` | always `protocol_error` | only when the abort carried `manifest_mismatch` |
| `apply_manifest_accept` | always `protocol_error` | yes — the trigger is already a specific manifest disagreement |
| `apply_disconnect` | **the peer's**, verbatim from the wire | never |
| `handle_link_lost` | the local transport's | never |

The narrowness is the point. The host is correlating, not proving: it can see
that a guest disagreed about session identity and that the guest declared a
different build, and it cannot see that the second caused the first. Gating on
`manifest_mismatch` keeps the correlation worth stating. A guest aborting over a
bad assignment or a phase race, on a mixed-build run where *everything*
correlates with the build, would otherwise be reported as a build problem and
send a tester to reinstall instead of to the bug.

`apply_disconnect` is excluded for a different reason: its code arrives verbatim
from the peer, so a guest could otherwise choose the sentence its own departure
is reported under. A remote value never selects a local attribution.
`handle_link_lost` is excluded because a link that went, went, whatever the peer
behind it was built from.

The comparison uses only what the handshake declared (see
[the protocol document](session_protocol.md#the-declared-build)). A host that
declares no build of its own never claims a build disagreement — which is why
every session built from `coordinator_fixture` still records
`protocol_violation` exactly as it did, and why no pinned coordinator transcript
moved for this. A peer that declares nothing against a host that does is a build
from before the field existed; that is a real difference and is named as one.

The record is cleared when another guest is admitted — a filled seat is no
longer news. The lobby renders it through `lobby_model.DEPARTURE_TEXT`, and a
terminated session outranks it. That text states the two observations and
asserts no cause between them, for the same reason the trigger is narrow.

None of this reaches the traffic that ends a session outright. A decode
failure, a validation failure, a session-id or sender-role violation, or an
illegal phase all route through `terminate_from`, not `drop_guest`: they end the
whole session as `protocol_violation` and render through `TERMINAL_TEXT`. A
departure is only ever the pre-freeze case where one guest goes and the lobby
stays.

Without this the host learned nothing at all. Build skew is detected locally by
the guest, so only the guest ever knew; in a two-device test the host was left
holding a lobby it could not fill and no sentence explaining why.

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
bot-filled, and solo 4v4 sessions, plus a full 1v1 and a full 2v2 session with
their owned sets and opening live slots.

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

A guest verifies that published ownership seats it, and — because it holds the
manifest, and therefore the mode — that every declared owned set is the right
size. It cannot verify that those owned sets map onto genuinely admitted peers,
because it never sees the roster. That check lives on the host. This is a
property of the single-trusted-host topology #161 defines, not a gap: OMP-3
provides mismatch detection and useful termination, not anti-cheat. A host that
lies about ownership can already choose the manifest.
