# OMP-3 session protocol and immutable match identity

`game.online.protocol` is the transport-neutral control-plane contract for the
first direct-host 4v4 combat-soccer fixture. It defines what peers agree on
before readiness and how later coordinators describe lifecycle events. It does
not create WebRTC peers, sequence input bundles, advance rollback, or own a
lobby.

## Channels and trust boundary

Each host/guest link keeps the OMP-0 topology:

- one reliable, ordered control channel carries this protocol; and
- one unordered, loss-tolerant channel carries the tick-numbered
  [input bundles and canonical host batches](input_packets.md).

Control messages are pure Lua records. Their canonical `GCOP;1;...` encoding
is bounded to 8,192 bytes and never invokes simulation code while parsing.
The OMP-0 1,024-byte loopback envelope was a proof-level queue boundary, not a
complete manifest carrier; #164 must either expose the control data channel
directly or add bounded framing without weakening this protocol limit.

The host is a coordinator, not an authoritative simulation server. Peers trust
the host to assign identities and slots, sequence control traffic, and abort
the fixture. They do not trust any peer-supplied participant identity,
presentation object, runtime asset, target selection, result, or simulation
outcome. OMP-3 provides mismatch detection and useful termination, not
anti-cheat, authentication, confidentiality, accounts, or host migration.

Only bounded opaque ASCII identifiers, closed/localizable terminal reason
codes, canonical integers, and booleans cross this boundary. Abort and
disconnect messages carry no peer-authored prose. Direct identifiers,
participant-study data, survey responses, presentation objects, runtime asset
instances, credentials, SDP/ICE payloads, IP addresses, and free-form logging
details are not protocol fields. Network diagnostics remain separately
governed by #168.

## Stable identities

A manual connection begins with a caller-generated `session_id`. Within that
session the host is one stable peer and up to seven guests receive stable
`peer_id` values. A peer owns a *set* of the eight canonical OMP-1 outfield
slots whose size is fixed by the [match mode](#match-modes-and-slot-ownership).
Missing humans are explicit deterministic bot producers with unique
`producer_id` and integer `bot_seed`; they are not synthetic peers. A bot
producer drives exactly one slot, because its `bot_seed` is per-slot. Producer
ids never cross the peer and bot namespaces, so a bot cannot impersonate or
collide with a peer producer. Keepers occur first in each five-player roster,
own no slot, have no combat loadout, and remain protected AI. Roster,
manifest-slot, and producer-assignment player ids share InputFrame's 64-byte
player-id limit.

Every message has a sender-local monotonic `sequence`. Session and peer
components are independently bounded to 128 bytes. Their canonical message id
is `GCMI;1;` followed by byte-length-prefixed session id, peer id, and decimal
sequence. Its exact maximum is 284 bytes. Length prefixes make
`("a.b", "c")` distinct from `("a", "b.c")` even though dots are valid inside
components. The transcript digest length-prefixes canonical messages in
delivery order. Repeating the exact same message id and bytes is idempotent;
reusing an id with different bytes is a terminal `transcript_conflict`. A
later coordinator must reject the conflict before mutating session state.

## Deterministic manifest

Manifest version 1 contains, in comparison order:

1. session, protocol, InputFrame, MatchSnapshot, input-tape, and combat schema
   versions;
2. immutable build and source identities;
3. content, exact tuning, match-configuration, fixture, and arena identities;
4. combat-rules and gameplay-AI-policy identities plus combat disposition;
5. initial seed, 60 Hz tick rate, duration, goal limit, and match mode;
6. ordered home/away team ids and five-player rosters;
7. every roster player's position and every outfielder's mechanical
   loadout/family mapping; and
8. all eight player assignments in OMP-1 `home_1..home_4`,
   `away_1..away_4` order.

The currently supported versions are protocol 1, InputFrame 2, combat-capable
MatchSnapshot 10, combat input tape 2, and combat companion schema 1. The
canonical manifest digest is a reproducibility/correlation id, not a
cryptographic signature.

The duration in field 5 is `duration_ticks`, and it is **7,200 ticks — 120
seconds at 60 Hz** — the same length online and offline. That was decided in
[#251](https://github.com/osobytes/goliseo/issues/251) after the online default
was found running half of it; the reasoning, and why the frozen manifest rather
than `MAX_DURATION_TICKS` is what bounds a future extra-time feature, is
recorded in
[`match_flow.md`](match_flow.md#a-match-is-120-seconds-online-and-offline).

`manifest_difference` returns the first differing path, so a one-field
mismatch fails before countdown with a name such as
`manifest.teams.2.roster.3.family_id`. The manifest is immutable after
proposal; any change requires a new session.

## Match modes and slot ownership

The pitch is always 4v4: eight canonical outfield slots plus two protected
keepers, in every mode. What a mode changes is how many humans share those eight
slots, and therefore how many slots each human owns.

| Mode | Humans | Slots per human | Humans per team |
| --- | ---: | ---: | ---: |
| `1v1` | 2 | 4 (the team's whole outfield line) | 1 |
| `2v2` | 4 | 2 (a chosen pair) | 2 |
| `4v4` | 8 | 1 | 4 |

`humans * slots_per_human` is always eight, which is why `3v3` has no entry:
four outfield slots per team do not divide into three humans. It is refused by
the same closed table as any other unsupported size, with the typed
`unsupported_match_mode` code, at manifest validation — long before readiness.
There is no `if mode == ...` anywhere in the ownership or switching rules.

The mode lives in the deterministic manifest, so it is frozen with everything
else and cannot change mid-session. A guest does not pre-verify it: like the
seed, it is a session-scoped choice the host makes in the lobby, and a peer that
disagreed about it would already fail on `manifest_id`.

Ownership is a *function* from the eight canonical slots onto producers. Every
slot still names exactly one declared source; a human source may now cover
several. The invariants that keep the source unambiguous are:

- every canonical slot has exactly one declared producer, in OMP-1 order;
- a bot producer drives exactly one slot;
- a producer id never appears as both a peer and a bot;
- one human's owned slots all lie on one team, so an owned set is always a
  subset of a single outfield line; and
- every human's owned set has exactly `slots_per_human` members for the
  manifest's mode. Wire shape alone cannot check this last one, so
  `validate_slot_assignments` checks the first four and
  `validate_assignment_manifest` — which has the manifest, and therefore the
  mode — rejects a disagreeing size with the typed `invalid_ownership` code.

A human controls exactly one owned slot at a time, the *live* slot; the others
run the deterministic gameplay AI and are materialized as input rows exactly
like declared bot fills, so nothing in the input stream distinguishes them. The
live slot moves by the switch rule already documented in
[`docs/controls.md`](../controls.md) — winning the ball switches control to the
winner, and the `switch` edge without the ball hands control to the outfielder
nearest the ball — intersected with the owned set. `switch` is already a bit in
the canonical input frame, so the transition is part of the deterministic input
stream rather than a new mechanism. In 4v4 the owned set has one member, so
every branch of that rule returns the slot already live and switching is inert;
that is a consequence of the general rule, not a special case.

**Keepers remain AI-only, unassignable, and slotless in every mode.** This is a
deliberate divergence from solo play, where gathering the ball hands control to
your keeper. Online never does, in any mode, because keepers are also
combat-protected and giving one mode's human a keeper would change what the
eight canonical slots mean. It is intentional, not an oversight to be fixed
later.

Runtime compatibility is deliberately separate. The handshake carries runtime
identity version, runtime family/revision, presentation-content identity, and
a sorted capability list. `runtime_difference` compares these values without
putting them in the deterministic manifest, snapshot, tape, or simulation
hash. A presentation/theme swap can therefore change runtime compatibility
while leaving manifest identity and outcomes unchanged.

### The declared build

The handshake also carries an optional `build_id`: the peer's own
`match_manifest.build_id()`, the same value the lobby prints in its `BUILD` row.

It is **never compared for admission.** A host that refused a differing build
here would take the skew away from the guest that detects it — the guest
compares manifests locally and mints its own `build_mismatch`, and that reading
must not change. The declaration exists for the other end of the same failure:
when a guest refuses the manifest and the host drops it, the host can say
*why* instead of recording a generic protocol error. See
[the coordinator document](session_coordinator.md#departures).

It is optional for the same reason. A peer built before the field existed still
speaks a handshake this build accepts, so its disagreement lands where the guest
detects it rather than as a malformed message the host refuses outright. Absence
against a host that does declare a build is itself a build difference, and is
named as one.

## Message set and lifecycle validation

All message bodies have closed field allowlists:

| Message | Purpose |
| --- | --- |
| `handshake` | Declare host/guest role, runtime compatibility, and optionally this peer's build. |
| `manifest_proposal` / `manifest_accept` | Propose the complete manifest and accept its canonical digest. |
| `peer_assignment` / `slot_assignment` | Name a stable peer and publish all peer/bot slot producers. |
| `ready` | Assert or revoke readiness for one ownership generation. |
| `pair_preference` | Ask the host for the outfield slots this peer wants to control. |
| `pair_preference_result` | The host's typed verdict on one pair preference. |
| `countdown` / `start` | Bind a countdown id to the first input tick. |
| `match_phase` | Report kickoff, play, goal stoppage, full time, or result. |
| `hash_report` | Report a canonical 16-hex boundary hash at one tick. |
| `result_ack` | Acknowledge score, final tick, and final boundary hash. |
| `abort` | End with a closed typed protocol/session rejection code. |
| `disconnect` | End with a closed peer/transport/host/protocol reason. |

`validate_phase(message, current_phase)` is state-independent. Coordinators
must call it before applying a message; a rejection never supplies a next
state. This keeps invalid transitions from partially mutating lifecycle data.
During `running`, match-phase bodies may report kickoff, playing, goal
stoppage, or full time. During `result`, the only accepted match-phase body is
`result`; playing, kickoff, and countdown traffic cannot regress the lifecycle.
Ordering among successive running bodies remains stateful coordinator work for
#163.
Exact abort/disconnect duplicates are classified as idempotent before phase
validation; a new terminal-state message is rejected, so reliable-channel
delivery cannot revive or rewrite a completed session. Slot assignment must
also pass `validate_assignment_manifest` so its player ids agree with the
accepted manifest rather than merely having valid canonical slot shapes.

## Pair preferences

A `pair_preference` is the one thing a guest may say about ownership, and it is
a *request*: it names a manifest, the ownership generation it answers, and the
owned set the peer wants, and it changes nothing by itself. The host answers
every one with a `pair_preference_result` carrying a closed status —
`granted`, `unchanged`, or `rejected` with a closed typed reason — and a grant
republishes ownership through the ordinary `slot_assignment` path. There is no
route by which a guest writes ownership.

The requested `slots` array is validated for *shape* here and for *size* where
the mode is known, exactly as published owned sets are: the wire requires one to
eight canonical outfield slot ids in strictly ascending canonical order, so one
set has exactly one encoding and no duplicate can hide inside it, and the
coordinator requires the count the frozen mode fixes. Keeper protection needs no
rule: keepers hold no canonical slot, so this vocabulary cannot name one.

Both kinds are legal in `assigned`, `ready`, and `countdown`. The first two are
where configuration can still change. `countdown` is included deliberately: the
freeze lands there, and a preference already in flight when it does is an
ordinary race that deserves the `after_freeze` refusal the host gives it rather
than a terminated session. Past the countdown, every peer has seen `start`, and
a preference is as much a protocol violation as a late `ready`.

## Ownership generations

`slot_assignment` and `ready` both carry an `assignment_id`: the identity of one
ownership generation, minted by the host with `assignment_id(assignments,
epoch)` and echoed unchanged by peers. The epoch is part of the digest, so
republishing byte-identical ownership still produces a distinct generation.

This exists because readiness is otherwise unattributable. The manifest is
immutable and shared by every generation, and a reassignment can leave a peer
owning a same-sized owned set both before and after, so neither `manifest_id`
nor slot ownership distinguishes "ready for the ownership in force" from "ready
for ownership two republishes ago". Ordering cannot settle it either: any number of
republishes may be in flight against a single readiness answer. Naming the
generation on the wire is what makes the answer verifiable, and lets a
coordinator refuse a superseded one without ending the session.

Owned sets and the match mode both belong to this argument, and they enter the
generation from opposite directions:

- **Owned sets are already inside the digest.** They are not a separate field —
  an owned set *is* the set of slots naming that producer in the `assignments`
  array, which `assignment_id` hashes in full. Repartitioning a 2v2 pair changes
  those bytes, and the epoch changes too, so it necessarily mints a new
  generation. Readiness for the old partition cannot be credited to the new one.
- **The mode is deliberately not in the digest, because it cannot vary.** It is
  a manifest field, the manifest is immutable after proposal, and both
  `slot_assignment` and `ready` already carry `manifest_id`, which a receiver
  checks *before* `assignment_id`. A peer holding a different mode is therefore
  already refused on manifest identity, and widening `assignment_id`'s signature
  to re-carry a value that cannot change within a session would add a second
  source of truth without closing any interleaving. The mode-dependent check
  that does matter — that owned-set sizes match — is re-evaluated against the
  manifest every time ownership is validated, on the host and on every guest,
  rather than being inferred from the token.

Only the host can derive the value, because only the host holds the epoch;
peers treat it as an opaque bounded token and the protocol validates its shape
alone. This is a deliberate exception to the rule that identity fields are
recomputable by the receiver — the token names *which* publication, not *what*
was published, and the assignments themselves are already verified against the
manifest.

## Validation order and terminal failures

Both host and guest parse in this order:

1. total wire bound and `GCOP` header;
2. protocol version;
3. canonical scalar/table grammar and complete consumption;
4. closed message kind and fields;
5. bounded session, peer, sequence, and transcript identity;
6. message-specific shape and bounds;
7. deterministic manifest version/shape and canonical digest;
8. deterministic manifest comparison;
9. runtime/presentation compatibility comparison;
10. duplicate/transcript classification; then
11. current lifecycle phase.

Failure through step 6 cannot reach session logic. Unsupported versions,
unknown messages, an unsupported match mode, malformed/noncanonical data,
oversized data, deterministic identity mismatch, ownership that disagrees with
the frozen mode, runtime mismatch, conflicting duplicate, or invalid phase
terminate the handshake/session with the corresponding typed code. No runtime
coercion or unknown-field preservation is allowed. A byte-identical duplicate
is the sole no-op success.

## Frozen wire evidence

`game.online.protocol_conformance` pins the exact manifest and transcript
digests, one complete literal `manifest_accept` wire, and a fixed wire digest
for every message kind. The verifier decodes and re-encodes the literal vector
as well as comparing freshly encoded fixtures with those checked-in bytes and
digests. Tests therefore fail when encoder and decoder drift together; they do
not derive their expected values from the implementation under test.
The conformance fixture exercises protocol schema only; its illustrative
loadout/family mappings need not match the current executable content catalog.

The native test suite calls the same verifier directly. In addition,
`love . --determinism` runs it before the OMP-1 replay and emits a
`GC_PROTOCOL|golden|...` marker. The existing browser determinism job packages
`core/`, `game/`, and `sim/`, launches that exact command in both pinned Chrome
and Firefox love.js runtimes twice, requires exactly one marker with the pinned
manifest/transcript ids and message count, and treats a conformance assertion
as a `GC_DETERMINISM|failure|...` terminal error. Its independent Python marker
parser also rejects changed literals. This provides cross-runtime wire evidence
without adding transport, WebRTC, or #164 channel behavior.

## Versioning and #114 provisional values

Any field addition/removal, meaning change, comparison-order change, canonical
encoding change, message transition change, or bound change requires a
protocol/manifest version decision. Old or future versions are rejected; there
is no general migration at the network boundary.

**Decision on record:** the `assignment_id` field on `slot_assignment` and
`ready` was folded into protocol version 1 rather than minting version 2.
OMP-3 has never shipped and no peer outside this repository speaks version 1,
so there is nothing to migrate and no compatibility claim to break. The change
is additive: no existing field changed meaning, no bound moved, no rejection
code changed, and the manifest digest is untouched. The cost is confined to
regenerating the two affected wire digests, the transcript digest, and the
browser evidence parser's pinned transcript id, all of which changed visibly and
on purpose. Once a build ships that a player can connect with, this option
closes and any further field change takes a version bump.

**Decision on record:** the manifest's `match_mode` field, the relaxed
duplicate-producer rule, and the `unsupported_match_mode` /
`invalid_ownership` codes were likewise folded into protocol version 1, for the
same reason and under the same closing condition. The change is additive at the
field level, but it deliberately *is* a digest change: `match_mode` enters the
canonical manifest, so `manifest_id` moves and every pinned wire that embeds it
moves with it — the `manifest_proposal`, `manifest_accept`, `slot_assignment`,
`ready`, `countdown`, and `start` digests, the transcript digest, the
coordinator session transcripts, the input-packet literals, and the browser
evidence parser's pinned ids. Making the mode digest-invisible was never an
option: two peers must be unable to agree on a manifest while disagreeing about
how many slots a human owns. The `handshake`, `peer_assignment`, `match_phase`,
`hash_report`, `result_ack`, `abort`, and `disconnect` digests do not carry the
manifest id and are unchanged, as are the 4v4 ownership goldens and the
coordinator trace digest, which is the evidence that 4v4 behaviour itself did
not move.

**Decision on record:** the `pair_preference` and `pair_preference_result`
messages were folded into protocol version 1 on the same terms. No existing
field changed meaning, no bound moved, no existing rejection code changed, and
**the manifest is untouched, so `manifest_id` does not move** — nor does any
wire digest that embeds it. Two new message kinds cannot be digest-invisible,
though: the conformance fixture carries one message per kind, so its transcript
digest and message count move, and each new kind gets a pin of its own. The new
messages are appended to the fixture rather than inserted, because a message's
sequence number is part of its message id and therefore of its wire digest;
appending leaves all thirteen shipped digests byte-identical, which is the
evidence that no existing message moved.

**Decision on record:** [#268](https://github.com/osobytes/goliseo/issues/268)
took the goal limit to 99 — no limit — and needed **no protocol change at all**.
`max_goals` is untouched as a field: same name, same position in the compared
set, same `1..MAX_GOALS` bound, same rejection code. Only the *value* the
conformance fixture and the content-derived manifest carry moved, from 5. Removing
the field would have been a version decision; changing what it says is not, which
is precisely why the decision was implemented as a value rather than a deletion —
see [the record in `match_flow.md`](match_flow.md#a-match-has-no-goal-limit-online-or-offline).

The value is nevertheless inside the manifest digest, so this is a deliberate
digest change: `manifest_id` moves, and with it the `manifest_proposal`,
`manifest_accept`, `slot_assignment`, `ready`, `pair_preference`,
`pair_preference_result`, `countdown`, and `start` wire digests, the protocol
transcript digest, the four coordinator session transcripts, the two input-packet
literals, and the browser evidence parser's pinned ids. The `handshake`,
`peer_assignment`, `match_phase`, `hash_report`, `result_ack`, `abort`, and
`disconnect` digests carry no manifest id and are byte-identical, as are
`vocabulary_id`, the coordinator trace digest, every ownership golden, and
`maximal_wire_bytes` — which together are the evidence that nothing but the
manifest moved.

`combat_status = provisional_114` is the only valid pre-disposition status.
It lets protocol, lobby, and transport foundations use the accepted interaction
contract without claiming that the prototype is approved. The final OMP-3
manifest freeze requires #114 to record either:

- `proceed`, represented by `accepted_proceed`; or
- an explicitly bounded `revise` after its prerequisite fixes land,
  represented by `accepted_revision` plus new immutable combat-rules,
  tuning, content/fixture, and policy identities as applicable.

If #114 records `stop` or `inconclusive`, OMP-3 must be refreshed rather than
constructing an accepted manifest. Participant evidence, presentation ids, and
runtime objects never enter the replacement manifest.
