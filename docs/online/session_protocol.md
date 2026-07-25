# OMP-3 session protocol and immutable match identity

`game.online.protocol` is the transport-neutral control-plane contract for the
first direct-host 4v4 combat-soccer fixture. It defines what peers agree on
before readiness and how later coordinators describe lifecycle events. It does
not create WebRTC peers, sequence input bundles, advance rollback, or own a
lobby.

## Channels and trust boundary

Each host/guest link keeps the OMP-0 topology:

- one reliable, ordered control channel carries this protocol; and
- one unordered, loss-tolerant channel carries the tick-numbered input bundles
  owned by #162.

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
`peer_id` values. A peer can own at most one of the eight canonical OMP-1
outfield slots. Missing humans are explicit deterministic bot producers with
unique `producer_id` and integer `bot_seed`; they are not synthetic peers.
Producer ids are unique across the combined peer and bot namespaces, so a bot
cannot impersonate or collide with a peer producer. Keepers occur first in each
five-player roster, own no slot, have no combat loadout, and remain protected
AI. Roster, manifest-slot, and producer-assignment player ids share
InputFrame's 64-byte player-id limit.

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
5. initial seed, 60 Hz tick rate, duration, and goal limit;
6. ordered home/away team ids and five-player rosters;
7. every roster player's position and every outfielder's mechanical
   loadout/family mapping; and
8. all eight player assignments in OMP-1 `home_1..home_4`,
   `away_1..away_4` order.

The currently supported versions are protocol 1, InputFrame 2, combat-capable
MatchSnapshot 9, combat input tape 2, and combat companion schema 1. The
canonical manifest digest is a reproducibility/correlation id, not a
cryptographic signature.

`manifest_difference` returns the first differing path, so a one-field
mismatch fails before countdown with a name such as
`manifest.teams.2.roster.3.family_id`. The manifest is immutable after
proposal; any change requires a new session.

Runtime compatibility is deliberately separate. The handshake carries runtime
identity version, runtime family/revision, presentation-content identity, and
a sorted capability list. `runtime_difference` compares these values without
putting them in the deterministic manifest, snapshot, tape, or simulation
hash. A presentation/theme swap can therefore change runtime compatibility
while leaving manifest identity and outcomes unchanged.

## Message set and lifecycle validation

All message bodies have closed field allowlists:

| Message | Purpose |
| --- | --- |
| `handshake` | Declare host/guest role and runtime compatibility. |
| `manifest_proposal` / `manifest_accept` | Propose the complete manifest and accept its canonical digest. |
| `peer_assignment` / `slot_assignment` | Name a stable peer and publish all peer/bot slot producers. |
| `ready` | Assert or revoke readiness for the accepted manifest. |
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
unknown messages, malformed/noncanonical data, oversized data, deterministic
identity mismatch, runtime mismatch, conflicting duplicate, or invalid phase
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
