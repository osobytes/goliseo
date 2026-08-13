# OMP-3 online match flow

> **Partly pre-port (LÖVE/Lua).** The contract this document describes is
> current, but it still names the Lua tree that commit `2c0d449` (#467) deleted.
> Read `sim/foo.lua` as `rust/crates/gc-sim/src/foo.rs`, `data/foo.lua` as
> `rust/crates/gc-data/src/foo.rs`, `sim.foo` as `gc_sim::foo`, and `game/**` /
> `spec/**` as `ts/packages/**`. Any `love .` command, `love.*` API, or
> `file.lua:LINE` citation is **pre-port evidence**, not something you can run
> or open. The live tree is described by `ARCHITECTURE.md`.

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

## A match has no goal limit, online or offline

**Decision on record** (repository owner, 2026-07-30, [#268](https://github.com/osobytes/goliseo/issues/268)):
there is **no goal limit**. A match is **decided on score at full time, in every
mode**.

It was not, until now, and the two paths did not even disagree consistently. The
content-derived online manifest ended a match on the **fifth** goal
(`match_manifest.DEFAULT_MAX_GOALS = 5`) and so did the protocol conformance
fixture (`game/online/protocol_fixture.lua:119`), while everything offline ended
it on the **third**: the simulation default (`sim/match.lua:965`), headless
batches (`sim/headless.lua:22`), the learning environment
(`sim/env_config.lua:77`), and both committed scope documents — "two minutes /
first to three" (`docs/showcase_release.md:147`) and "the existing 5v5,
two-minute / first-to-three match"
(`docs/design/goliseo_theme_pivot.md:321`). Nothing recorded that anyone had
decided either number, and nothing in the lobby explained the difference to a
player.

The reasoning for removing the limit rather than picking three or five. This
project already has one termination rule that every mode agrees on, settled
above: the clock. A goal cap is a **second** result condition layered on it, and
the only thing it can ever do is end a match *early*, with time still on it —
which at 120 seconds is a large fraction of the match. Keeping both means a
player has to hold two ways for a match to end, and every measurement has to say
which one produced it. Removing the cap leaves one rule that reads the same
online and offline: play the clock, the better score wins. That is also why this
issue closes by **convergence** rather than by documenting a deliberate
divergence — there is no difference left to make visible in the lobby, so no
lobby surface was added.

Three things a reader asking "why is the field still there, then?" should not
have to rediscover:

- **The field stays; only the value moved.** `max_goals` is part of the frozen
  session manifest and of the protocol's compared field set
  (`game/online/protocol.lua:534`), so *removing* it would be a protocol-version
  change. Nothing here is worth a version bump, so "no limit" is spelled as a
  value instead: **99**, which is `protocol.MAX_GOALS`
  (`game/online/protocol.lua:276`), enforced at manifest validation
  (`game/online/protocol.lua:962`), and mirrored for the simulation as
  `sim.match.NO_GOAL_LIMIT`. Ninety-nine goals is unreachable inside 7,200
  ticks, so the value means in practice exactly what the decision says.
- **The mechanism is still live, and still tested.** `sim/match.lua` still ends
  a match the moment either score reaches `s.max_goals`, and a caller that wants
  a goal-terminated match passes its own — evidence fixtures, rollback
  laboratories, and short-match specs all do. This is a change of default, not a
  removal of a rule, which is why a future "first to N" cup format needs no new
  machinery.
- **99 is the ceiling, and that is deliberate.** Unlike the duration, this value
  cannot later be raised without moving a wire bound. That is not a constraint
  anyone is pressing against: the point of the value is that it is unreachable,
  and a format that genuinely wants a cap sets a *lower* one.

Several fixtures deliberately keep an explicit `max_goals = 3` and are unchanged
by this decision, because they pass the value rather than inheriting it:

| Fixture | Why it overrides |
| --- | --- |
| `data/omp1_determinism.lua:45`, `sim/determinism_evidence.lua:147` | The OMP-1 authoritative recording. Its identity string is part of what the campaign pins; re-freezing it to restate a default would move determinism goldens for no evidentiary gain. |
| `data/outfield_ai_baseline.lua:20`, `sim/outfield_ai_baseline.lua:67` | The frozen combat-disabled control #148/#149 calibrate against, protected by `--refreeze-ack`. A control that moves is not a control. |
| `sim/rollback_validation.lua`, `spec/fixtures/short_match_tape.lua` | Laboratory and tape fixtures that want a short, goal-terminated match on purpose. |

The OMP-3 fault campaign is the one place that *does* inherit the new value:
unlike `duration_ticks`, `fault_harness` does not overwrite `max_goals`
(`game/online/fault_harness.lua:226-227`), nor does the session spec fixture
(`spec/fixtures/online_match_session.lua:42-43`). Both now carry 99. Neither
notices — their matches are 150 and 90 ticks — but it is worth knowing that the
campaign runs under the shipped rule rather than an overridden one.

One thing this decision defers rather than settles: the frozen Outfield AI
baseline was calibrated under a three-goal cap, so some of its matches ended
early. Whether #149's control should be re-frozen under the no-limit rule is a
calibration question for that issue, not a consequence of this one — but its
combat-active arm has to run under whichever rule the control does, or the two
arms stop being paired.

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

The host reaches a weaker version of the same conclusion from the other side.
Only a guest holds an expectation, so only a guest can *detect* the skew; the
host sees an abort and an empty seat. So the handshake now declares each peer's
`build_id`, and the host uses it for exactly one thing: naming the drop
`build_mismatch` instead of `protocol_violation`, and only when the guest
refused this session's identity. It is never grounds for refusing admission,
because that would take the detection away from the peer that does it — and
what the host reports is a correlation, not the cause the guest can establish.
See [the coordinator document](session_coordinator.md#departures).

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

The flow is built against versioned fixtures, and one dependency is open. This
criterion is **not** claimed satisfied:

- **#114, the accepted default combat disposition.** The manifest carries
  `combat_status = "provisional_114"` and a placeholder
  `combat_rules_id`/`gameplay_ai_policy_id`. When #114 accepts a disposition,
  `match_manifest` and the protocol fixture are the only places that change.

Three things that used to be listed here are closed, and what replaced them is
[the section below](#combat-through-the-online-path):

- ~~**#112, combat-aware gameplay AI.**~~ **Closed.** Non-live owned slots and
  declared bot fills now materialize `gameplay_ai/combat/v1` (`sim.combat_policy`)
  through `sim.slot_input`, using the same observation schema, option ordering,
  and reason vocabulary as the gameplay match AI. 1v1 leaned on this hardest —
  three of every human's four owned slots are AI-driven at any instant — and
  those slots can now use and counter all four families. *Can*, structurally;
  how often they actually do in open play is measured below, and the answer is
  "rarely". The manifest names the policy as `gameplay_ai.combat.v1`; the
  contract spells it `gameplay_ai/combat/v1`, and manifest ids may not contain
  slashes.
- ~~The **seven combat correction phases**.~~ **Pinned, twice.** #166 pinned them
  at the driver layer, as convergence on one snapshot hash through a correction
  taken in each phase. This flow pins the other half — what the *screen* is shown
  while that happens.
- ~~**"All four accepted families remain intentionally usable"**.~~ **Demonstrated
  through the online path**, from local keyboard and gamepad input rather than
  only at `sim` level. What remains unproven is the *balance* of that usability,
  which is #149's calibration and #114's disposition.

## Combat through the online path

### All four families, from the keyboard and the gamepad

The seating is content, not a fixture. This build's canonical home line carries
exactly one accepted family per slot:

| Slot | Player | Loadout | Family |
| --- | --- | --- | --- |
| `home_1` | brakka | Emberguard Shield | guard |
| `home_2` | veil_nyx | Tournament Sword | light melee |
| `home_3` | rok_tann | Pulse Blaster | ranged |
| `home_4` | zyro_vex | Spring Gloves | unarmed |

A 4v4 seats one human per slot, so four online match screens mounted off one
real lobby cover all four families with no loadout chosen for the test, and each
peer's single owned slot stays live for the whole match because a singleton owned
set makes switching inert. The away line is declared bot fill.
`spec/screens/online_match_flow_spec.lua` plays that session in three windows:

- **quiet** — nobody presses anything. `sim.match`'s 2.5 s kickoff hold expires
  inside it, and from the moment it does every controlled player reads `ready`:
  equipped, off cooldown, in no forced state, nothing already committed. Every
  human slot commits zero times across the window anyway.
- **keyboard** — `j` is toggled on a fixed period and `game.screens.match` polls
  it. Toggled rather than held because the families activate differently: `press`
  for unarmed and light melee, `held` for guard, `held_release` for ranged.
- **gamepad** — the gamepad's `b` is toggled instead, with `j` up. It has to be a
  real held button: the match screen re-polls the pad every update, so an
  abstract action event alone would be overwritten before the next tick.

The quiet window is what makes the other two mean something. A live slot is the
one slot its peer authors for itself — `match_driver.materialize_authored` hands
the human sample straight to the control slot and never asks the policy for that
row — so a confirmed `commit` carrying the live slot's player index can only have
come from local input. The quiet window shows that from the outside instead of by
reading the driver, and the readiness measurement is what makes the zero mean
something — for the gates it actually reads.

`sim.combat`'s `request_rejection` refuses a press for seven reasons, and the
quiet window covers five directly: `readiness == "ready"` is false unless the
loadout exists, the forced counter is zero, the phase is `ready` (nothing already
committed) and the cooldown is zero; and `kickoff_hold <= 0` is read separately
off the match state. The other two — `soccer_commitment` and
`aerial_state_or_recovery` — are **not read**. They are unreachable instead, and
by a property of the input stub rather than of the readiness reading: the spec
wires `j` and the pad's `b` and nothing else, so shoot, pass, dash, jockey, dodge
and the aerial actions can never be pressed, and no ground-commitment or aerial
timer can arm on a live slot. The zero-commit result is sound either way; it is
written down this way so that teaching the stub another key is a change somebody
has to re-check, rather than one that quietly invalidates the claim.

**The bot fills do not commit in open play**, and the control deliberately does
not lean on them. From a real kickoff, over 700 quiet frames, the declared away
fills produced no combat commit at all. `gameplay_ai/combat/v1` commits readily
from the rigged poses in `spec/support/online_combat_phases.lua`, where two lines
face each other 24 to 36 px apart, and rarely from open play, where a purpose
target inside reach *and* inside the front arc is a much scarcer event. Combined
with #166's finding that the policy almost never chooses guard, that is a
calibration observation for #149 and a disposition question for #114 — not
something the presentation layer should paper over.

Note what this says about **guard**. #166 found that `gameplay_ai/combat/v1`
almost never chooses it: `combat_phases.GUARD_PROBE` finds exactly one commit in a
240-step unarmed scrum and none at all once the delivery cadence shifts, so its
driver-level guard scenario raises a guard from held equipment on the canonical
input stream instead. Here that is not a workaround — a human on `home_1` raising
a shield with the keyboard *is* the criterion, and the AI's reluctance is #149's
and #114's business.

Readability is asserted from one frame per family — the first frame that family's
own telegraph is on the pitch — so the rest is provably simultaneous with it
rather than merely present at some other moment. On that frame:

- the presentation model gives the controlled player that family's telegraph:
  `guard_arc` for guard, `line` for ranged, `arc` for the two melee families, and
  the pitch renderer branches on exactly that field;
- ranged additionally has a projectile of its own in flight in the model;
- the HUD names the family (`equipment_label`) and its live phase
  (`equipment_state`) **beside** the scorebug, the clock, and possession, not
  instead of them;
- the online overlay still reports the network state (`net tick`, `rollbacks`)
  and the selection state (`control`, `owned`, `family`);
- and the frame really draws: `game.render.pitch` and `game.render.match_hud`
  both execute over it under a stubbed `love.graphics`.

These are presence checks against a no-op graphics stub, which is the ceiling
`AGENTS.md` §9 sets for UI work — "UI testing means testing UI *logic*, not
pixels". They prove each family's telegraph reaches the model and that the real
draw code runs over it. They cannot see overlap or occlusion, and nothing here
claims they can.

### What the screen is shown through a correction

`spec/game/online_match_presentation_spec.lua` takes #166's seven phases one
layer up. The driver-level claim is that peers converge on one snapshot hash; a
screen consumes the timeline, not the hash, so a driver that converges perfectly
can still leave a swing drawn twice or a hit drawn that never happened. Each case
rigs the phase's boundary zero, bursts delivery until real corrections happen,
checks that a correction really resimulated a tick *in* that phase — read inside
the step, before the driver evicts the boundary — and then asserts the
presentation contract over the whole run.

The contract has three operations, and they are deliberately not collapsed:

- **added** — the corrected timeline has a cue the speculative one did not. It
  publishes, once.
- **replaced** — the id survives with a different payload, because
  `sim.rollback_events` derives identity from causality. The confirmation owes
  the *corrected* payload; publishing the stale one is a distinct defect from
  publishing twice, and is counted separately.
- **revoked** — the id is gone. It must never be confirmed.

**A combat cue is almost never revoked, and that is a property of the netcode
rather than of the fixtures.** `sim.rollback_input_history` predicts a missing row
by repeating its held bits with `edges = 0`, and every combat encounter opens on a
press edge, so no peer ever speculates a commit it later has to take back. What
*can* be revoked is a cue derived from an already-open encounter — the contact it
lands, the forced state it inflicts, the ball it spills — when corrected geometry
resolves that encounter differently. One case exists solely to produce one, in the
unarmed scrum where eight bodies sit inside one 30 px reach; without it the
"a revoked cue is never confirmed" assertion would be vacuous.

Delivery in these cases is every 12th step rather than each scenario's own
period: a shorter burst corrects too little to rewrite a cue at all, and past
roughly 16 the confirmation ceiling falls behind `sim.rollback_events`'
unconfirmed window and the timeline gives up before the phase arrives.

### Where a bot fill's combat reason comes out

`sim.match` emits a `combat_commit_<reason>` MatchEvent for gameplay-AI
outfielders, but in slot mode every outfielder is slot-covered, so that path is a
structural no-op online — and declared fills are the dominant online population.

The slot path therefore reports its decisions as a **return value**:
`slot_input.materialize` returns a second value, an array of `SlotBotDecision`
records (slot, player index, action, stable reason, target, unavailable reason,
and the `combat_sim_observation/v1` digest the decision was made on), and
`slot_input.last_decision(producer, slot)` reads the most recent one back.

It is deliberately *not* a MatchEvent. In slot mode the decision happens inside
the producer, before the boundary; `state.events` is a hashed snapshot field, so
appending to it would make two peers diverge on whether their caller opted into
collecting the diagnostic. The materialized tape stays the only replay authority,
and `spec/sim/combat_ai_match_spec.lua` pins that collecting the report leaves
every boundary hash unchanged.

This is the #112 side of `ai_reason_reconciliation` (section 4.6 of the combat
evidence contract). #148 still recomputes the eligibility bitset and the
intervention envelope independently; it must not trust these labels as ground
truth.

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
