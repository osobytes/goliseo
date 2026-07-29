# OMP-3 online match flow

The lobby freezes a session; [the driver](match_driver.md) plays it. This
document covers the seam between them: how a peer turns the frozen manifest into
a playable match, how local input reaches exactly one slot, how the shipped
combat presentation is reused rather than reimplemented, and how the session ends
with one result every peer agrees on.

Four modules, one direction:

| Module | Owns |
| --- | --- |
| `game.online.match_manifest` | The content-derived manifest, and resolving one back into `data/` content. |
| `game.online.match_session` | The online match request: boundary zero, the owned set, the opening live slot. |
| `game.online.match_presentation` | The stable event timeline over the driver's outputs. |
| `game.screens.online_match_model` | Session policy: what a confirmed step publishes, when a result may be committed. |

`game.screens.online_match` is the impure shell that wires those to the star, the
renderer, and the input. `sim/`, `data/`, and `core/` stay free of transport,
browser, and LÖVE dependencies.

## The manifest has to be playable

`game.online.protocol_fixture` pins a manifest whose team and player ids exist
only in that fixture. It is fine for protocol conformance and useless for
playing: nothing can rebuild a match from it. The lobby always said as much —
"a content-derived builder replaces it when the online match flow lands, which is
why the source is injectable rather than hard-wired" — and this is that builder.

`match_manifest.template` derives every identity from shipped content:
`content_id` is a digest over both rosters, their fixed loadouts, and the arena;
`tuning_id` is a digest over `tuning.serialize()`, the same string the OMP-1
determinism campaign pins; the canonical slot table comes from
`sim.match.ownership_for_teams`, so the manifest cannot describe a seating the
simulation would not build. Two peers on the same build therefore compute the
same manifest byte for byte, and a guest's manifest comparison is a real
content check rather than a fixture equality.

`match_manifest.resolve` is the inverse, and it is strict: an unknown team,
arena, or player, a disagreeing position or loadout, or a slot table that
disagrees with the locally computed ownership all fail rather than producing a
quietly different match.

## A match is 120 seconds, online and offline

**Decision on record** (repository owner, 2026-07-28, [#251](https://github.com/osobytes/goliseo/issues/251)):
a match is **120 seconds — 7,200 ticks at 60 Hz — everywhere**.

It was not, until now. `match_manifest.DEFAULT_DURATION_TICKS` said `3600`, so an
ordinary online match ended at the whistle **twice as early** as an offline one,
with nothing in the lobby explaining why. Every other source already said 120
seconds: `sim.match` defaults `opts.duration or 120` (`sim/match.lua:958`), the
OMP-1 determinism fixture pins `duration_seconds = 120`
(`data/omp1_determinism.lua:34`), the protocol conformance fixture pins
`duration_ticks = 7200` (`game/online/protocol_fixture.lua:118`), and the
committed scope calls it "a two-minute match" (`docs/showcase_release.md`). The
online default was the only dissenter, and it was a divergence nobody chose — so
it was raised to `7200` rather than pulling the other four down.

The reasoning for 120 over 60: one mental model across online and offline, and
offline evidence that stays directly transferable. A shorter online match would
narrow the window for a desync and reach a result sooner, which is a real
argument — but it buys that by making every offline measurement need a caveat
about which path produced it, and it asks a player to learn two match lengths for
one game.

Two things a reader asking "can we add extra time later?" should not have to
rediscover:

- **The wire is not the constraint.** `protocol.MAX_DURATION_TICKS = 216000`
  (`game/online/protocol.lua:268`) is enforced at manifest validation
  (`game/online/protocol.lua:941`) — 60 minutes at 60 Hz. 7,200 ticks is about 3%
  of it, so the ceiling leaves ample room and this decision does not move toward
  it in any meaningful way.
- **The frozen manifest is the constraint.** `duration_ticks` is part of the
  immutable session manifest, and the driver's full-time and settle logic keys
  off it, so extra time can never be added by extending the value mid-match.
  It would need either a duration agreed upfront that already contains the
  maximum extra time, or a canonical extra-time event carried in the confirmed
  input stream — a deliberate wire addition, out of scope here and unaffected by
  the choice of 120 over 60.

The OMP-3 fault campaign does not read this default. `fault_harness` builds on
`match_manifest.template` and then overwrites `duration_ticks` with its own
`DEFAULT_DURATION_TICKS = 150` (`game/online/fault_harness.lua:145,227`), as the
session spec fixture does with its `90`
(`spec/fixtures/online_match_session.lua:30,42-43`). `match_driver_fixture` does
not reach this constant at all: its manifests come from
`protocol_fixture.manifest` (`game/online/match_driver_fixture.lua:106,150`),
and its own `DEFAULT_DURATION = 6` is a `sim.match.new` duration in **seconds**
for `initial_snapshot`, not a manifest `duration_ticks`. The campaign runtime is
therefore unchanged by this decision, which is why a two-minute online match
costs nothing to validate.

## `build_id` carries the control vocabulary

`build_info` is three constants — name, version, channel — and a working commit
moves none of them. On the development channel that made `build_id` blind to the
difference that matters most while testing: two peers on different commits
digested to the same string, passed the manifest comparison, and only found out
they spoke different control vocabularies when one sent a message the other had
never heard of. That surfaced as an announced `protocol_violation` partway
through the lobby, which reads as a transport bug rather than "you deployed two
different builds".

So `build_id` digests `protocol.vocabulary_id()` alongside the three constants.
That id is derived at load from the two tables `protocol.validate` and
`protocol.validate_phase` actually read, covering three things a peer has to
agree with — the message kinds, the fields of each body, and the lifecycle
phases each kind is legal in — sorted so it never depends on `pairs` order. Disagreeing about any of them is unrecoverable
(`unknown_message`, `malformed`, `invalid_phase` respectively), so a difference
is worth refusing, and refusing it at the manifest check is strictly earlier
than the first message that would have exposed it.

The scope is deliberate in both directions. It is not a digest over the source
tree: peers whose vocabularies agree stay compatible however else their commits
differ, so this adds no mismatch a session would otherwise have survived. And it
does not claim to catch every skew — two builds that differ only in simulation
code still share a `build_id`, and that divergence is caught later by the
boundary-hash machinery rather than here.

The manifest schema is untouched: `build_id` was already a compared manifest
field and already part of a guest's `CoordinatorManifestExpectation`, so nothing
new crosses the wire. A guest that disagrees terminates with the coordinator's
`build_mismatch` reason, which the lobby renders as "The peers are running
different builds. Install the same build on both."

## The request is a pure function of the freeze

`match_session.request` takes a role, a peer id, the frozen manifest, and the
`CoordinatorFreeze`, and returns everything needed to start — including
**boundary zero**, captured from `sim.match.new` with the frozen seed, duration,
goal limit, and canonical ownership, plus a `CombatMatchState` companion.

Boundary zero is the load-bearing part. Every peer seeds its rollback session
from it, so a difference is an immediate desync rather than a cosmetic
disagreement; the specs assert that every peer in a session hashes the same
snapshot.

The combat contracts are selected **explicitly**: the request refuses any
`snapshot_version` other than `match_snapshot.COMBAT_VERSION` and any
`tape_version` other than `input_tape.COMBAT_VERSION`. The protocol already
refuses the alternatives, but stating it here means the companion exists because
this flow asked for one, not because a default happened to line up.

## Switching is on. Keeper control is off.

A human owns a *set* of canonical outfield slots — four in 1v1, two in 2v2, one
in 4v4 — and controls one at a time.

**Switching is enabled and routed**, and it is the shipped single-player rule
from [`docs/controls.md`](../controls.md): switch hands control to the outfielder
nearest the ball, and winning the ball auto-switches to the winner. Online it is
that rule intersected with the frozen owned set by
`coordinator.next_live_slot`, evaluated by every peer from the canonical input
stream. Local input contributes exactly one thing to it: the `switch` bit in the
sample, which the driver puts on this peer's control slot.

**In 4v4 the owned set is a singleton, so every branch of the rule returns the
slot already live and switching is inert.** That is a consequence of the general
rule. There is no `if mode == "4v4"` anywhere in the path, and adding one would
be the bug — the mode's only appearance in this flow is the *size* the freeze
gives the owned set.

**Keeper control stays disabled in every mode.** Keepers are slotless and
AI-only, so a human's owned set can never contain one and no input path can
reach one. This is a **deliberate divergence from solo play**, where control
passes to your keeper when it gathers the ball, and it was reaffirmed by the
repository owner for OMP-3. It is recorded as `match_session.KEEPER_CONTROL =
false` and pinned by a spec so it is not later "fixed" as a bug.

Another peer's input remains impossible, unchanged: the driver validates
authorship as set membership and ends the match as `ownership_violation` for a
bundle naming a slot outside the sender's frozen owned set.

`state.controlled` follows the live slot for **presentation only** — the camera,
the HUD subject, and which contextual meaning a key has. Slot routing in
`sim.match` is `slot_players`, which never reads `controlled`, so following it
cannot widen what a peer authors.

## Rendering reuses the shipped path

`game.screens.match` gained one seam, `MatchRollbackSource`: six questions the
renderer asks whatever is simulating. The development rollback laboratory
implements it; the online driver implements it. Everything downstream —
speculative add/revoke/replace, confirmed exactly-once consumers for goals,
combat contacts, projectiles, ball spills, VFX, audio, camera, HUD, statistics,
replay, kickoff, full time, and result — is the shipped #113/#147 path, untouched.
No new feedback system was added.

Interpolation and correction smoothing stay presentation-only: a correction
begins at the preceding displayed pose and sheds only its render-owned offset,
and neither the renderer, the HUD, nor a predicted event can modify canonical
simulation or result truth.

`match_presentation` supplies the one thing the driver deliberately does not: a
`sim.rollback_events` timeline over its outputs. The driver emits corrected
outputs before fresh ones, so an output at or below the highest tick already
applied is the head of a correction run that replaces the complete speculative
tail, and anything above it is an ordinary append. `rollback_events` asserts both
shapes, so a mis-split fails loudly rather than silently publishing a cue twice.

## Focus loss, pause, and a lost controller

A peer that stops advancing stops publishing authority, and every other peer
stalls behind it. An online pause is a pause for everybody, and OMP-3 has no
host-arbitrated pause. So:

- **focus loss** produces a notice and nothing else; the driver keeps stepping,
  and `App:focus` deliberately does not push the pause screen on the online
  route;
- **a pause request** explains why it cannot pause, and a second press offers the
  only honest alternative — an explicit abort, which ends the session for
  everyone rather than silently freezing it. Any other input dismisses it;
- **a lost controller** notifies and nothing else: keyboard input still reaches
  the live slot, and stalling the session would punish every other peer for one
  peer's unplugged pad.

Offline behaviour is untouched: an offline match still pauses on focus loss and
on controller removal, and the showcase and combat-prototype paths keep their
own fixed-clock, legacy-input flow.

## Full time and the result

Full time is published from the simulation; the result is committed from the
session.

The host publishes `match_phase` from **confirmed** lifecycle records only —
`kickoff`, then `playing`, then a `goal_stoppage` after the score actually moved,
then `kickoff` again — so a predicted goal can never advance the session. At
full time both roles report `finish` with their own final tick, score, and
boundary hash: the host's becomes the session result, and a guest's is retained
locally so the host's `result_ack` is checked against a simulation this peer
actually ran rather than accepted on faith. A differing acknowledgement is a
desync, not a second scoreboard.

The peer records its result only from the coordinator's acknowledged `result`
after the session reaches `completed`. Until then the screen holds on "waiting
for every peer to acknowledge the result". Statistics stay local — they are
presentation, and OMP-3 does not put them on the wire — but the score, the
winner, and the final tick are the session's.

The driver stops polling once it is terminal, which is its "no hidden progress"
contract. Full time is exactly when the result acknowledgement crosses the wire,
so from that point the flow pumps the star directly for control traffic, into the
same reassembly buffers, and a link lost after the final tick still reaches the
coordinator.

### Routing out

- **completed** goes to the product result screen on its own `online_result`
  route, so the offline rematch — which replays the local session — cannot be
  reached from a session that no longer exists;
- **rematch** opens a *fresh lobby*. The manifest is immutable per session and
  the freeze is spent, so the honest rematch is a new session rather than a
  silent reuse of an incompatible manifest;
- **anything else** — abort, transport loss, hash mismatch, late input, an input
  channel failure — returns to the title with the terminal reason. Its lobby is
  unusable: nothing about it can be reused, and pretending otherwise would put a
  stale manifest and a dead transport behind the next match.

## What is still contingent

The flow is built against versioned fixtures, and two dependencies are open.
These criteria are **not** claimed satisfied:

- **#114, the accepted default combat disposition.** The manifest carries
  `combat_status = "provisional_114"` and a placeholder
  `combat_rules_id`/`gameplay_ai_policy_id`. When #114 accepts a disposition,
  `match_manifest` and the protocol fixture are the only places that change.
- **#112, combat-aware gameplay AI.** Non-live owned slots and declared bot fills
  materialize `sim.slot_input`'s existing deterministic bot, which is not combat
  aware. 1v1 leans on this hardest: three of every human's four owned slots are
  AI-driven at any instant. The plumbing is proven; the behaviour is not.
- Consequently the **seven combat correction phases** the issue names — wind-up,
  guard, contact, projectile flight, stagger, ball spill, immunity expiry — are
  not individually pinned. The pre-#112 bot never reaches them, so a spec for
  each would pin an absence. The *mechanism* is covered: the combat companion
  survives correction and resimulation on every peer.
- **"All four accepted families remain intentionally usable"** likewise cannot be
  demonstrated end to end online until the AI can use equipment; what is proven
  is that the fixed loadout and family reach the request, the HUD, and the
  renderer for every seated slot.

## Full time is settled, not merely reached

The driver no longer terminates the moment its *present* simulation reaches full
time. It opens a bounded settle phase and drains the outstanding authority first,
so `finish` is reported over a final boundary every peer has confirmed rather
than over the last `DELAY` ticks of prediction — see
[the driver's settle phase](match_driver.md#full-time-settles-before-it-completes)
for the bounds and for why it is not gated on hash agreement. A peer whose tail
never arrives ends `settle_timeout`, which is an input-channel failure and
deliberately not a desync. The host stays past its own confirmation until every
author it has heard from has reported confirming the final boundary too, because
in a star it is the only peer that can fan a missing row back out. On a clean
match that is two steps; on a lossy one it can be the whole settle window, and
[the driver's document](match_driver.md#what-the-settle-bounds-cost-measured)
measures both.

A peer that stopped being able to confirm *at all* — an unconfirmed tick that
fell below its retained rollback floor — ends `confirmation_stalled` at the step
that happens, rather than surfacing a whole match later as somebody else's
failure mode.

The screen follows that boundary rather than the tick the simulation stopped on:
`full_time()` reports `match_driver.settled`, so the visible "FULL TIME" banner
and the confirmed result cannot disagree. Result commitment is unchanged and
still gated strictly on the coordinator's acknowledged `result`.
