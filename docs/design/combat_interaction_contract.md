# Design: GOLISEO combat interaction contract

- **Status:** accepted prototype contract
- **Accepted:** 2026-07-23
- **Delivery:** post-showcase combat-soccer proof
- **Related:** `docs/design/goliseo_theme_pivot.md`,
  `docs/design/prototype_theme_roster.md`, `docs/controls.md`,
  `docs/online/input_frame.md`,
  [`docs/design/combat_fun_evidence_contract.md`](combat_fun_evidence_contract.md),
  and milestone 11 issue #107

## Purpose

This document fixes the player-visible and deterministic rules for the first
fixed-loadout combat proof. It is the input and interaction authority for
issues #108–#114. It does not claim that the current showcase implements these
controls.

Goals remain the only victory condition. Combat has no health, damage, death,
injury, ammunition, rarity, proficiency, or progression system.

## Physical controls and abstract intent

The new control is deliberately separate from the existing contextual soccer
buttons:

| Action | Keyboard | Gamepad | Fixed-tick semantics |
| --- | --- | --- | --- |
| Move / face | WASD or arrows | Left stick or D-pad | Signed axes held each tick |
| ACTION: shoot / tackle / jockey / slide | Space | A | Existing press, held, and release rules are unchanged |
| PLAY: pass / switch | K | X | Existing press, held, and release rules are unchanged |
| Sprint | Shift | LB | Held |
| Lob / chip modifier | L | Y | Held |
| Juke / dodge | C | L3 | Press edge; existing juke becomes universal combat evasion |
| **EQUIPMENT** | **J** | **B** | Held state plus distinct press and release edges |
| Pause | P or Esc | Start | Presentation/app action; never encoded in match input |

`EQUIPMENT` produces one abstract vocabulary for humans and AI:

- `equipment_held` is true on every tick while the source is down;
- `equipment_pressed` is emitted once on the first fixed tick after a press;
- `equipment_released` is emitted once on the first fixed tick after a
  release.

Render updates that simulate zero ticks retain pending equipment edges. A
catch-up update emits an edge on its first fixed tick only. Missing-input
prediction repeats axes and held intent but clears both equipment edges. It
never invents a release or repeats an attack.

The producer collapses button chatter between fixed ticks to at most one
canonical transition sequence:

| Previous sampled state | Final physical state | Canonical sample |
| --- | --- | --- |
| Up, no transition | Up | neither edge; held false |
| Up, one or more transitions ending down | Down | press only; held true |
| Up, a completed tap ending up | Up | press **then** release; both edges; held false |
| Down, no transition | Down | neither edge; held true |
| Down, one or more transitions ending up | Up | release only; held false |
| Down, release/re-press chatter ending down | Down | press only; held true |

This deliberately permits only one action commit per zero-tick interval.
`pressed + released + held` is invalid; when both edges are set, their order is
always press then release and held must be false. Family handling is:

- unarmed/light melee commit once from the press; the release has no effect;
- guard completes its raise and immediately enters lower/recovery without an
  active held interval;
- ranged latches the release, completes its minimum aim, fires once, and
  enters recovery.

Issue #109 will assign versioned bit values. It must bump the input contract
rather than reinterpret version-1 bits. The frame contains intent only:
presentation ids, family ids, target ids, legal-target results, contacts, and
outcomes remain outside the payload.

## Facing and aim

The existing movement axes and authoritative facing are sufficient for this
proof; there is no second aim vector or target lock.

- A non-zero movement vector turns facing through the existing movement rules.
- With zero movement, the last non-zero authoritative facing remains the aim.
- Guard and ranged aim may reduce movement but do not create a hidden target.
- Every contact is resolved from fixed-tick positions, facing, front arc, and
  reach or projectile geometry.

An AI supplies the same axes and equipment held/edge intent as a human. It may
choose intent from observable state, but it cannot encode a target or set an
outcome directly.

## Family interpretation

| Family | Press edge | Held | Release edge |
| --- | --- | --- | --- |
| `unarmed` | Commit one strike | No repeated action | No effect |
| `guard` | Begin raise if legal | Raise, then maintain guard | Lower guard |
| `light_melee` | Commit one strike | No repeated action | No effect |
| `ranged` | Begin aim/telegraph | Maintain aim after minimum wind-up | Fire exactly one projectile |

Releasing ranged before its minimum wind-up latches the release and fires at
the first legal active tick. Holding longer does not add damage or power. A
release can never fire twice.

## Initial family bounds

All durations are integer 60 Hz ticks. They are conservative starting values,
not final balance. Issue #110 may tune within the product bounds, and issue
#114 owns evidence-backed calibration.

| Family | Wind-up / raise | Active | Recovery | Cooldown from commit | Reach / travel | Front arc | Movement multiplier |
| --- | ---: | ---: | ---: | ---: | --- | ---: | ---: |
| `unarmed` | 6 | 4 | 12 | 24 | 30 px | 100° | 0.80 |
| `guard` | 6 | held | 9 after release | none | self | 120° | 0.55 |
| `light_melee` | 12 | 5 | 21 | 42 | 42 px | 75° | 0.50 |
| `ranged` | 18 minimum aim | 1 release tick | 27 | 60 | 300 px/s, 60-tick lifetime | 20° aim cone | 0.40 |

Cooldown and recovery are both paid on a miss. Cooldown begins at commit and
may outlast recovery. Guard has no ammunition or cooldown, but raising and
lowering are committed transitions.

Initial unguarded outcomes are:

| Family | Interruption | Displacement | If the target owns the ball |
| --- | ---: | ---: | --- |
| `unarmed` | 10 ticks | 8 px | Spill |
| `light_melee` | 18 ticks | 18 px | Spill |
| `ranged` | 12 ticks | 10 px | Spill |

These values express horizontal tradeoffs: unarmed is fastest and shortest;
light melee buys reach with commitment; ranged buys lane influence with the
longest telegraph and cadence; guard gives up movement and initiative.

A combat hit inside a target's active guard arc causes no stagger and no ball
spill. It may apply at most 6 px of readable recoil. Guard does not prevent a
standing tackle, slide tackle, ordinary body collision, pass interception, or
other soccer rule.

## Legal targets and goalkeeper protection

- Any opposing **outfielder** inside the action geometry is legal.
- Friendly players never receive combat contacts.
- There is no carrier-only rule, active-challenger flag, nearest-target snap,
  or target lock.
- One action or projectile resolves at most one combat contact. If several
  opponents qualify on the same tick, choose the nearest along the facing or
  travel direction, breaking an exact tie by stable match-player index. A
  projectile expires on that first contact even when guard or immunity
  prevents its outcome.
- Off-ball contacts are legal. Wind-up, recovery, cooldown, lost formation,
  and AI utility scoring make purposeless harassment costly.
- Goalkeepers cannot equip or initiate combat and are ignored by combat
  contacts everywhere on the field.
- The penalty area grants no combat protection to outfielders.
- Existing keeper-possession protection and soccer distribution rules remain
  unchanged.

## Fixed-tick contact phase

Combat contacts use snapshot-then-resolve semantics. After authoritative
movement for a fixed tick, collect every active melee and projectile candidate
from the same pre-contact player/action snapshot. Select each source's single
target by the rule above before applying any combat outcome. A contact selected
for this phase is not retroactively canceled by an interruption produced in the
same phase, so two active opponents may trade.

Evaluate guard and recovery immunity from that same snapshot. When multiple
sources select one target, the target accepts at most one displacement,
interruption, ball spill, or guard recoil in the phase. Choose the dominant
contact by longest interruption, then greatest displacement, then lowest
attacker match-player index, then lowest authoritative source sequence.
Guard or immunity suppresses all selected outcomes for that target; guard may
apply the dominant contact's single bounded recoil. Every source still pays its
commitment and produces a stable contact result, and every contacting projectile
expires. Apply the chosen target outcomes in match-player-index order. This
ordering is authoritative simulation behavior, not presentation timing.

## State transitions

```text
                                  interrupted
                                     |
READY --attack press--> WIND_UP --> ACTIVE --> RECOVERY --> READY
  |                         |            \ miss/contact /
  |
  +--guard held--> RAISE --> GUARD --release--> LOWER/RECOVERY --> READY
  |
  +--ranged press--> AIM_WIND_UP --> AIM --release--> PROJECTILE + RECOVERY

STAGGER or KNOCKBACK --> READY + RECOVERY_IMMUNITY_TIMER
```

Rules shared by committed equipment states:

1. An action becomes authoritative only on a legal fixed tick.
2. After wind-up starts, the player cannot cancel into another equipment,
   soccer, aerial, sprint, or juke action.
3. A miss pays full recovery and cooldown.
4. An unguarded interruption cancels wind-up, active contact, guard, or ranged
   aim; its cooldown remains paid.
5. A ranged projectile is authoritative simulation state. Animation timing
   cannot create, advance, or contact it.
6. Expired recovery returns to normal movement unless another forced soccer
   state still applies.

## Soccer-action arbitration

When simultaneous inputs compete on the same tick, existing soccer commitments
win. An ignored edge is not queued for later.

| Current or same-tick state | May equipment start? | Resolution |
| --- | --- | --- |
| Normal outfielder, with or without ball | Yes | Interpret by fixed loadout |
| Shoot, pass, punt, or throw commit/release | No | Soccer action wins |
| Standing tackle, jockey, or slide | No | Existing challenge wins |
| Juke / dodge | No | Juke wins and supplies combat immunity |
| Equipment wind-up, active, aim, guard, or recovery | Already committed | Consume/latch the matching guard or ranged release; ignore new press and soccer edges |
| Aerial reception, strike, or recovery | No | Finish aerial state |
| Stagger or knockback | No | Finish forced state |
| Recovery immunity timer only | Yes | Player acts normally; incoming combat outcomes are suppressed |
| Kickoff hold | No | Enable equipment only after play becomes live |
| Goalkeeper | Never | Keeper has no combat loadout |

Holding guard or ranged input through an ineligible state does not synthesize a
press edge. The source must release and press again. This prevents local,
recorded, and predicted input from disagreeing about when an action began.

An eligible carrier's held shoot or pass control is an existing soccer
commitment: it starts or continues charge and blocks a same-tick equipment
press. A shoot/pass release also wins that tick. Once equipment commits, shot
and pass charge values reset to zero, remain zero, and do not accumulate until
equipment recovery ends. This prevents a hidden soccer action from charging
under guard or ranged aim.

Ordinary sprinting does not block an equipment press. Once equipment commits,
the family movement multiplier replaces the sprint boost and sprint stamina
does not drain; a still-held sprint resumes only after recovery. A simultaneous
slide/tackle edge still wins arbitration.

## Dodge and anti-chain protection

The existing C/L3 juke is the only universal dodge. During its existing
immunity window it avoids both tackle and combat contacts. It cannot cancel an
equipment commitment, and an equipment action cannot cancel it.

Combat-caused continuous loss of control is capped at 30 ticks (`0.5s`).
After stagger or knockback ends, the player receives at least 45 ticks
(`0.75s`) of interruption immunity.

Before immunity begins:

- the first interruption records a control-loss chain start and a hard expiry
  30 ticks later;
- a combat hit during that forced state may extend the current forced-state
  expiry to the later of its existing expiry or the new family's interruption,
  but never beyond the chain's hard expiry;
- that extending hit does not add displacement or spill the ball again, and it
  never shortens or replaces the current forced state;

Once immunity begins:

- new combat contacts cannot add stagger, knockback, or ball spill;
- the attacker still pays active, recovery, and cooldown costs;
- normal ball interception, tackling, body collision, and movement rules
  continue;
- immunity cannot be consumed early or shortened by another hit.

Issue #110 may choose a longer immunity or shorter family interruption, but
never a longer continuous combat disable without a new product decision.

## Fixed proof loadouts

Any family may be assigned to any outfield position. Theme, presentation,
species, body shape, and authored stats do not restrict it.

The default proof fixture has, per team:

- one combat-disabled goalkeeper;
- one unarmed outfielder;
- one guard outfielder;
- one light-melee outfielder; and
- one ranged outfielder.

Explicit calibration fixtures may repeat families to test degeneration, but
they must opt into that fixture policy. The default playable fixture validator
does not silently accept a repeated-family lineup.

## Required scenario behavior

| Scenario | Required result |
| --- | --- |
| Ball carrier attacks | Legal if otherwise ready; a simultaneous shoot/pass commit wins arbitration |
| Carrier is hit unguarded | Spill plus the family interruption/displacement |
| Carrier guards from the front | Combat spill/stagger prevented; soccer tackle remains legal |
| Active challenger is in arc | Same geometry rules as every other outfielder; no privileged target flag |
| Loose ball | Combat may move/interrupt outfielders but cannot spill an unowned ball |
| Keeper possession | Keeper remains immune; existing stand-off/distribution rules remain |
| Outfielder in penalty area | Normal combat target; the area is not a safe zone |
| Aerial contest | Equipment input ignored until aerial state and recovery finish |
| Kickoff hold | Equipment disabled until the ball is live |
| Off-ball opponent | Legal geometric target; full commitment/cooldown applies |
| Missing predicted input | Held aim/guard may persist; press/release edges are always cleared |
| Fast equipment tap between ticks | Both edges mean press then release: one strike, guard raise/lower, or one ranged shot after minimum aim |
| Repeated hit before immunity | Extend the forced-state expiry without another displacement/spill, never past 30 ticks from the first hit |
| Simultaneous trade | Both pre-contact-snapshot contacts resolve; neither same-phase interruption retroactively cancels the other |
| Multiple attackers, one target | Stable dominant contact supplies the sole target outcome; all sources pay commitment |
| Repeated hit attempt | Immunity prevents added disable/spill while every attacker pays commitment |

## Downstream implementation obligations

- #108 stores family/loadout identity and validates the default proof lineup.
- #109 versions local and eight-slot input with the three equipment signals.
- #110 owns pure action/contact/projectile resolution within these bounds.
- #111 snapshots, hashes, replays, predicts, and confirms every authoritative
  combat field/event.
- #112 emits only the same observable abstract intent available to humans.
- #113 presents these states without inventing outcomes in `game/`.
- #114 predeclares metrics and tunes without weakening the anti-chain cap.

Any downstream need to change targeting, control vocabulary, keeper immunity,
victory conditions, or the disable cap requires an explicit contract revision,
not an implementation-side exception.
