# Online threat model: what GOLISEO's netcode resists, and what it cannot

Status: **assessment, not an accepted decision.** Written 2026-08-05 against
`feat/wasm-sim-host` (8e8e45a). Nothing here has been fixed; this document
exists so the OMP-4 topology choice is made with the cheat surface visible.

The findings below cite the Lua tree. They are **not** v1-only: `game/online/**`
— rollback scheduling, input encode/decode, state hashing and the protocol — was
ported to the `gc-netcode` crate ([`v2/README.md`](../../v2/README.md), the
port-mapping table), and the mechanisms this document attacks survived the port
unchanged. Spot-checked on 2026-08-10 against the port:

| Claim below | State in `gc-netcode` |
| --- | --- |
| Unknown own hash reads as a pass (§1) | `match_driver.rs`, `observe_checkpoint` — `None => return true` |
| Hash is bare string equality, no nonce | `match_driver.rs`, `mine == hash`; no nonce or commit-reveal anywhere in the crate |
| Only *consecutive* mismatches count | `match_driver.rs` resets the counter to 0 on any match; `MAX_HASH_MISMATCHES = 3` |
| Coordinator evicts its own hash after 8 boundaries | `coordinator.rs`, `HASH_WINDOW = 8` |
| Arrival validation never checks `kind` (§2) | `match_driver.rs`, `collect_arrivals` tests `slot_index` against the owned set and nothing else |
| No anti-lookahead beyond the fixed delay (§3) | `input_protocol.rs`, `FAIRNESS_DELAY_TICKS = 3`, unchanged |

The remaining §2 and §4 items were not re-verified line by line against the
port. The recommended order at the end applies to `gc-netcode`, not to the Lua
tree, for anything built from here.

Every existing online document says some version of "OMP-3 provides mismatch
detection and useful termination, not anti-cheat"
([`session_protocol.md`](session_protocol.md), line 28;
[`session_coordinator.md`](session_coordinator.md), line 519;
[`input_packets.md`](input_packets.md), line 57). That is honest and correct.
This document is the first one to ask what it would take to change that answer,
and what the answer can never be.

## The one-paragraph verdict

The architecture is, by accident of good design rather than by intent,
**already immune to the entire family of cheats that plague commercial
multiplayer games** — the state-tampering family. Nothing but inputs crosses
the wire, player stats live only in local content, and no peer ever accepts
simulation state from another peer. There is no "set my speed to 3x" packet to
forge, because there is no packet that carries speed. What remains is a much
narrower and more interesting set of problems: two cheat families that lockstep
determinism is *structurally incapable* of catching (input automation and
lookahead), one anti-tamper mechanism that does not survive a modified client
(boundary-hash reporting is forgeable by echo), and three availability bugs that
let any single participant end a match with one or two crafted packets. The
first two need design answers; the last two need code.

## What the shape of the system buys, for free

These are not defenses anybody wrote. They fall out of choosing deterministic
lockstep, and they are worth naming because they are the expensive half of
anti-cheat in most games.

**Only inputs cross the wire.** Every peer runs the whole simulation locally
from a boundary-zero snapshot each derives independently from the frozen
manifest ([`match_session.lua:162`](../../game/online/match_session.lua)). The
snapshot is never transmitted. The classic client-authoritative exploits —
teleport, speed multipliers, infinite stamina, forged positions — have no wire
representation to attack.

**Player stats are structurally absent from the protocol.** The roster message
carries `player_id`, `position`, `loadout_id`, `family_id` and nothing else
([`protocol.lua:92`](../../game/online/protocol.lua)). Pace, strength,
technique, stamina and mental live in `data/players.lua`, are looked up locally
by id, and the wire-carried record is discarded once its id validates
([`match_manifest.lua:248`](../../game/online/match_manifest.lua)). A cheat
client cannot inflate a stat because there is no field to inflate.

**Content identity is pinned by hash to local data.** `content_id`, `tuning_id`,
`arena_id`, `combat_rules_id`, `gameplay_ai_policy_id` are compared field by
field against an expectation each guest derives from its own installation
([`coordinator.lua:294`](../../game/online/coordinator.lua)). An edited roster
or an edited tuning value aborts the session before kickoff rather than
silently favouring the editor.

**Input ownership is checked against transport-attested identity, not payload
claims.** Inbound messages are tagged with the id of the data channel they
physically arrived on, never with a field read from the payload
([`contract.lua:108`](../../game/transport/contract.lua)), and a row must match
both the frozen manifest's producer and the transport origin
([`input_protocol.lua:1015`](../../game/online/input_protocol.lua)). Guests
cannot impersonate guests.

**Ownership is frozen once and is unreachable afterward.** Every message that
could move a slot is phase-gated to pre-freeze
([`coordinator.lua:1387`](../../game/online/coordinator.lua)), and the driver
copies the partition once at construction. This is not merely checked, it is
structurally unreachable mid-match.

**A peer's own input cannot be forged back at it.** Each peer applies its
authored row locally before the relayed echo can return, and a later arrival for
an already-recorded `(tick, slot)` with different bytes is rejected
([`rollback_input_history.lua:344`](../../sim/rollback_input_history.lua)).

**The wire refuses out-of-range input.** Stick axes are bounded to `[-127,127]`
integers on every decode path; the ASCII grammar cannot even parse `5.0`. The
"send an oversized movement vector" cheat is impossible here, not merely
mitigated.

**No state-injection surface exists at all.** `desync_package.lua` deliberately
carries hashes rather than snapshot bytes, and there is no reconnect, rejoin or
catch-up path anywhere in the tree. This category is absent, not guarded.

## What is broken now

Ordered by what I would fix first. All four are verified in code, not inferred.

### 1. Boundary-hash reports are forgeable by echo (critical)

This is the load-bearing anti-tamper mechanism, and it does not hold against a
modified client.

The hash is a plain string compared by equality against the receiver's own value
([`match_driver.lua:1489`](../../game/online/match_driver.lua),
[`coordinator.lua:2832`](../../game/online/coordinator.lua)). Nothing binds a
reported hash to the reporter's actual computation: there is no nonce, no
signature, no commit-then-reveal ordering. `publish_checkpoints` — the function
that computes the honest value — is ordinary client Lua a modified build
replaces. **A cheating client can therefore run any simulation it likes and
simply retransmit the hash its opponent reported for that tick**, passing every
checkpoint forever at zero cost.

Two adjacent holes widen it. Both layers treat "I have not computed my own hash
for that tick yet" as a pass, with no deferred re-check
([`match_driver.lua:1490`](../../game/online/match_driver.lua)), and the
coordinator evicts its own hash after `HASH_WINDOW = 8` boundaries, so a report
late by more than four seconds is silently unchecked. `MAX_HASH_MISMATCHES = 3`
counts only *consecutive* disagreements, so intermittent divergence stays under
the bar indefinitely.

The same trust gap reaches the result: `apply_result_ack` skips verification
entirely when the receiver's own result is still nil, after which a guest adopts
the reported score wholesale
([`coordinator.lua:2874`](../../game/online/coordinator.lua)). A peer that
finishes "first" declares the final score unverified.

### 2. Three ways for one participant to kill a match (high, griefing)

Each is a single modified client sending one or two well-formed packets. None
requires breaking a cryptographic assumption, because there is not one.

**A guest-sent `kind="host"` packet.** `decode` accepts the kind field as a
valid enum from any sender
([`input_protocol.lua:648`](../../game/online/input_protocol.lua));
`collect_arrivals` validates only that `rows[1].slot_index` is inside the
sender's owned set ([`match_driver.lua:908`](../../game/online/match_driver.lua))
and never checks kind. `canonical_host_batch` then rejects it, returning nil,
and `host_sequence_authority` treats nil as fatal and terminates *the host's own
driver* ([`match_driver.lua:1328`](../../game/online/match_driver.lua)). One
packet, eight players, match over.

**A self-conflicting resend.** The six-row redundancy window lets a guest resend
the same `(tick, slot)` with different bytes. `canonical_host_batch` builds its
conflict table fresh per transport tick and has no cross-tick memory, so the
durable check one layer down fires `conflicting_authoritative`, which
`match_driver` escalates to a whole-driver `terminate`
([`match_driver.lua:763`](../../game/online/match_driver.lua)) — on the host,
before the broadcast. Two packets.

**An unbounded-future tick.** `row.tick` is bounded only against `MAX_TICK`
(2³¹−1), never against session progress. A row stamped near `MAX_TICK` is
inserted permanently and can never be pruned, since pruning only evicts below
the advancing floor — and the host rebroadcasts it into every peer's history.
Unbounded memory growth on all eight clients at packet-send rate.

The common root cause is worth stating plainly: **the driver has no
peer-scoped remediation.** Every fault class, however attributable, escalates to
match-wide termination. A peer can only conflict with itself on a slot only it
owns, so the offender is always known — the correct response is to drop that
producer, not the match.

### 3. Lookahead cheating (high, intrinsic to lockstep but currently unbounded)

The only anti-lookahead mechanism is `FAIRNESS_DELAY_TICKS = 3`, and its one
enforcement gate stops the *host* from shortening its own delay
([`input_protocol.lua:1040`](../../game/online/input_protocol.lua)). Nothing
stops any peer from *lengthening* its delay deliberately.

Two facts combine. `match_driver.advance` takes no `dt` and never compares its
step cadence to wall time — the injected clock is read only for the settle
deadline ([`match_driver.lua:1597`](../../game/online/match_driver.lua)). And
inbound authority is applied *before* the local row is authored, in the same
call ([`match_driver.lua:2158`](../../game/online/match_driver.lua)).

So a client that starves its own `advance` while its transport keeps buffering
opponents' packets can resume in a catch-up burst and choose its input for tick
N with the opponents' tick-N rows already folded into local history. It commits
with knowledge instead of blind. There is no commit-reveal scheme anywhere in
the tree. Depth is bounded only by the receive queue (`DEFAULT_QUEUE_LIMIT = 64`,
roughly a second) before the cheater's own link tears down — and a second of
perfect foreknowledge of a shot or tackle is decisive in a game this fast.

### 4. Speedhack, rollback storms, rage-quit (medium)

`fixed_clock` walks up to `MAX_TICKS_PER_UPDATE = 8` ticks per call from real
`dt`, so an inflated timer buys up to ~8x local tick throughput — every
tick-timed mechanic (dash cooldown, shot charge, stamina) completes faster in
wall-clock terms. It is self-limiting, because the cheater outruns real network
data and trips its own `late_input` terminal within about a second, but it banks
an advantage first and nothing detects the anomaly itself.

Nothing bounds how *often* a peer triggers deep resimulation. Riding the oldest
edge of the 30-tick window forces up to 30 full `sim.match.step` recomputations
on the host and every guest, repeatable every ~500 ms indefinitely, never
crossing into `late_input_unrecoverable`. Diagnostics count corrections but
gate nothing.

A result is committed only on `completed`; every other terminal records nothing
for anyone ([`match_flow.md`](match_flow.md), lines 344-347). A losing player
can void the match by killing the link, and the protocol cannot distinguish that
from honest network failure. Harmless today — no ladder exists — and a free
rage-quit shield the moment one does.

## What cannot be fixed by protocol design

Two families remain no matter how well the wire is hardened, because they are
consequences of lockstep itself.

**Input automation.** Every peer must hold full authoritative state to simulate,
and ownership validation can only ask "did this slot's owner author this row",
never "was this row produced by a human". A bot emitting legal samples at 60 Hz
is indistinguishable at the protocol layer, permanently.

This is not theoretical here: **the repository already ships the bot.**
`sim/brain.lua`, `sim/outfield_decision.lua` and `sim/combat_policy.lua` are
zero-latency deterministic decision engines over full match state, and
`sim/env_action.lua:197` is literally a policy-to-`InputSample` adapter. The seam
a cheater replaces is one line
([`online_match.lua:65`](../../game/screens/online_match.lua)). The `.love`
archive is a plain zip of unobfuscated Lua source, so this needs a text editor,
not reverse engineering.

What it would win, in the game's own numbers: an unarmed attack telegraphs for
**6 ticks (100 ms)**, guard commits in 6, light melee 12, ranged 18
([`action_families.lua`](../../data/action_families.lua)); the standing-tackle
poke window is 13 ticks and the shot windup 9. Human visual reaction is roughly
9–15 ticks. A bot reading state reacts in one. It would never be hit by a
telegraphed attack — and since the shipped combat AI has no defensive
dodge-timing logic at all, such a bot would out-defend the game's own reference
AI, not merely a human.

**Information advantage.** The client legitimately holds opponent charge level,
exact stamina, every cooldown timer, and the keeper's committed `dive_target`
before the dive renders. `env_observation.lua:96` documents precisely which of
these the renderer withholds. Withholding is a paint-routine convention, not a
secrecy boundary; the data must be present for the simulation to run.

Neither is reachable by tightening the protocol. Both need either design changes
(reaction floors on defensive actions, so frame-perfect is no better than
human-perfect) or behavioural detection — which requires somewhere trustworthy
to run it.

## The path to "virtually unhackable", and its one precondition

The honest answer to "can we sidestep all of it without anti-cheat software" is:
**almost, and the missing piece is not anti-cheat software — it is a trusted
observer.** Pure P2P cannot arbitrate, because every check reduces to one
modified client's word against another's.

The good news is that this codebase is unusually close to being able to afford
one, and the OMP-4 relay decision is still open
([`relay_topology_decision.md`](relay_topology_decision.md) leaves
sequencer-versus-sequencer-less explicitly undecided).

**Deterministic replay verification is nearly free here.** I measured 40
complete 120-second matches — 7,200 ticks, eight players — replayed headless in
25.6 s on one core: **0.64 s per match, about 190x faster than real time.** The
machinery already exists: `sim/replay.lua` does pure input-tape replay with
first-divergence diagnostics, and `sim/input_tape.lua` tapes already carry build,
content, tuning, seed and ownership identity. A relay that retains the input
tape can re-simulate the entire match authoritatively for well under a cent, and
that single capability collapses most of this document:

- Forged hashes stop mattering — the verifier computes the truth itself.
- Modified `tuning.lua` defaults and modified simulation code become detectable,
  closing the `build_id` self-assertion gap.
- Match results become attestable rather than "two clients agreed", which is the
  precondition for ever shipping ranked play.
- Behavioural detection of automation gets somewhere to run: the same tape
  yields reaction-time distributions and block/dodge conversion rates, and
  `sim/metrics.lua` and `sim/research_*.lua` already compute most of it.

Two things the relay must get right, both already flagged by the team: it must
frame every forwarded message with its true origin, or ownership validation
degrades to self-declared `sender_id`
([`relay_topology_probe.md`](relay_topology_probe.md), lines 199-218); and a
sequencer-less shape removes the per-peer backpressure isolation the star has.

**Lookahead needs commit-reveal, and only that.** Each peer sends
`hash(sample ‖ nonce)` for tick N before any peer reveals plaintext for N, then
reveals. A peer that stalls to gain foreknowledge finds it has already committed.
This works without server authority and is the one fix that closes an intrinsic
lockstep cheat purely in protocol. It costs one small extra message per tick.

**Wall-clock pacing closes lookahead's cheap variant and blunts speedhack.**
Compare `_step` progression against the already-injected monotonic clock and
terminate a peer whose local tick rate drifts too far from real time in either
direction. One check, using a seam that already exists for the settle deadline.

### Recommended order

1. **Peer-scoped fault isolation.** Drop the offending producer instead of the
   match, and add the missing `kind` check and a forward tick bound. Fixes all
   three one-packet kills. Small, self-contained, no design debate.
2. **Wall-clock pacing check.** One comparison; blunts lookahead and speedhack.
3. **Commit-reveal for input samples.** Closes lookahead properly.
4. **Decide OMP-4 with verification in mind.** The relay is the cheapest trusted
   observer this project will ever get; a sequencer-less shape that keeps
   per-origin framing and retains tapes buys authoritative results.
5. **Replay verification service**, once a relay exists. 0.64 s per match.
6. **Reaction floors on defensive combat actions**, if automation ever shows up
   in practice. This is a game-design lever, not a security one, and it is the
   only thing that meaningfully limits a bot.

Hardening the hash exchange (nonces, deferred comparison, never accepting an
unverified `result_ack`) is worth doing at step 1 or 2, but note that it raises
the cost of cheating rather than closing it — only an independent verifier
makes hash agreement mean anything.
