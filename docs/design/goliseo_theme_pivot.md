# Design: GOLISEO theme and combat-soccer pivot

> **Superseded 2026-08-14.** The "Crossover spectacle" identity pillar and the
> six-theme portfolio below are no longer the accepted direction. See
> [`docs/design/gladiator_pivot.md`](gladiator_pivot.md), which commits
> GOLISEO to a single colosseum/gladiator setting instead. This document is
> kept as the historical record of that decision and its research (the
> KayKit audit, ESA representation guardrails, and asset/license notes below
> remain useful reference); do not treat the theme slate as current.

- **Status:** accepted product and prototype contract
- **Accepted:** 2026-07-23
- **Delivery:** post-showcase proofs
- **Related:** `docs/vision.md`, `docs/showcase_release.md`,
  `docs/design/prototype_theme_roster.md`,
  `docs/design/combat_interaction_contract.md`,
  [rigged-player contract issue #93][issue-93],
  [combat interaction contract issue #107][issue-107],
  [rigged 3D milestone 10][milestone-10], and
  [combat-soccer milestone 11][milestone-11]

## Decision summary

GOLISEO pivots from a single intergalactic/species theme to a
**multi-theme combat-soccer showdown**.

- GOLISEO is only the name of the game. It is not an in-world colosseum,
  league, sport, or character label.
- Characters, equipment, and arenas may draw from different original fantasy,
  historical, modern, science-fiction, supernatural, or deliberately absurd
  themes.
- The match remains soccer. Teams win by scoring goals.
- Combat is a tactical way to contest possession and space. It does not add a
  health, death, or second victory system.
- Rigged 3D characters are the intended presentation direction, subject to the
  milestone-10 native and browser ten-player gate.
- The first combat proof uses authored players and fixed loadouts. Economy,
  unlocks, proficiencies, generated rosters, and progression wait until the
  match interaction is fun without them.

This is the authoritative product direction and bounded prototype contract. It
does not rewrite the current release contract in
`docs/showcase_release.md`: both milestone 10 and milestone 11 are
post-showcase proofs unless the showcase is explicitly rescheduled.

## Product fantasy

Before kickoff, the player picks five characters, a formation, a tactic, and
eventually a constrained equipment loadout. On the pitch they still pass,
dribble, shoot, defend, and score, but they may also block a challenge, stagger
a carrier, dodge a committed action, or use a telegraphed ranged action to
break a passing lane.

The crossover is the spectacle:

- a medieval crossbow and a science-fiction blaster can share ranged rules;
- a sword and an energy blade can share light-melee rules;
- a wooden shield, riot shield, and projected force shield can share guard
  rules;
- boxing, wrestling, capoeira-inspired movement, and powered gauntlets can
  share an unarmed family.

Era and genre are presentation layers. A small set of shared mechanical
families keeps the match learnable.

## Hard design rules

1. **Soccer wins.** Goals are the only primary victory condition. Combat must
   produce a soccer consequence: space, possession, passing-lane control, shot
   prevention, or a transition.
2. **No damage race.** The first slice has no health, death, elimination, or
   injury system. A successful action may guard, stagger, knock back, or spill
   the ball.
3. **Short loss of control.** Every disabling state is brief and protected
   against chaining. A player spends the large majority of the match moving
   and making soccer decisions.
4. **Readable counterplay.** Strong actions have visible wind-up, range, and
   recovery. Guarding, spacing, dodging, or interrupting are possible answers.
5. **One ruleset, many skins.** Equipment data maps an item presentation to a
   shared action family. Adding an era must not require a combat subsystem.
6. **Horizontal loadouts.** Equipment changes strengths and weaknesses. It
   does not make an equipped player strictly better than an unequipped one.
7. **No economy before proof.** Purchases, rarity, upgrades, proficiencies, and
   loot cannot rescue an unfun fixed-loadout match.
8. **Manager choices stay visible.** A loadout earns its setup cost only when
   the player can identify its effect during the next match.
9. **Presentation does not own outcomes.** `gc-sim` resolves actions and emits
   states. `ts/packages/render` selects animations, particles, sound, camera
   response, and attached models.

## Multi-theme art rules

The game does not need one canonical era, but it does need one visual grammar.

- Use original characters and equipment, not recognizable copies of existing
  crossover properties.
- Team color remains more important than a character's origin theme.
- Every equipment family gets a consistent silhouette and effect language:
  guard reads broad and defensive; light melee reads directional and
  committed; ranged exposes aim and release.
- Production characters share a small number of compatible humanoid skeletons
  and proportion envelopes. Theme variety comes from meshes, materials,
  attachments, effects, and animation selection.
- A reusable presentation asset is not a roster identity. Different players
  may reference the same mesh while retaining distinct names, numbers, stats,
  loadouts, and persistent cosmetic variants.
- Non-humanoid characters are future exceptions, not a requirement for the
  first roster.
- Arenas may mix themes, but the pitch, goals, ball, boundaries, teams, and
  actionable states must remain readable immediately.

Species may remain character flavor where useful, but it is no longer the
mandatory roster taxonomy or an automatic match-modifier layer.

## Initial theme slate

**Decision date:** 2026-07-23

Three design lenses evaluated the slate:

- market and brand reach;
- stylized 3D art and small-team production;
- arcade-sports systems and equipment readability.

Medieval Fantasy, Galactic Sci-Fi, High Seas Adventure, and Toybox rated
strongly in all three reviews. Street Sports and Martial Legends each rated
strongly in two. Those six form the initial theme portfolio:

| Theme | Portfolio role | Character fantasies | Equipment vocabulary |
| --- | --- | --- | --- |
| **Medieval Fantasy** | Classic fantasy and the clearest equipment tutorial | Knight, ranger, goblin, barbarian, wizard | Sword, shield, hammer, bow, crossbow, staff, rune |
| **Galactic Sci-Fi** | Futuristic spectacle and continuity with the old setting | Alien, android, astronaut, space ranger, compact mech | Energy blade, projected shield, gravity maul, blaster, drone |
| **Street Sports** | Contemporary football and athletic expression | Freestyler, skater, parkour runner, boxer, masked ring fighter | Gloves, pads, reinforced board, ball launcher, speakers |
| **High Seas Adventure** | Family adventure, physical comedy, and displacement props | Privateer, deckhand, navigator, eccentric sailor, sea creature | Cutlass, buckler, anchor, harpoon, rope, net, barrel |
| **Martial Legends** | Skill fantasy and a positive identity for unarmed play | Boxer, wrestler, capoeira-inspired fighter, kickboxer, staff fighter | Wraps, bracers, staff, blade, bow, training equipment |
| **Toybox** | Whimsy, nonlethal tone, and permission for future absurdity | Action figure, toy soldier, wind-up robot, doll, plush competitor | Spring gloves, block shield, foam sword, squeaky hammer |

Together they cover **classic, future, present, adventure, skill, and comedy**.
They are content families, not factions or classes:

- mixed teams appear in key art and normal play;
- a character's theme grants no passive or statistical bonus;
- equipment may be cross-equipped when the shared skeleton and sockets allow
  it;
- mechanical tooltips name the shared family, while item names and visuals
  carry theme flavor;
- content is balanced by equipment family, not by giving every theme one item
  in every family.

The theme labels are production taxonomy. Individual characters receive
specific identities rather than labels such as “the medieval character.”

### Audience and representation guardrails

The [ESA's 2025 U.S. study][esa-2025] reports an average player age of 36, a
47% women / 52% men player split, and substantial parent/child and social play.
That U.S. industry study supports testing a broad audience hypothesis; it is
not global market evidence and does not prove this theme slate will appeal
broadly.

- Every theme should support varied genders, body types, ages, personalities,
  and human/non-human silhouettes. Do not isolate women, non-human characters,
  or comic relief into one theme.
- Street Sports is the contemporary football anchor. Use that name rather than
  “urban,” which can carry unintended racial coding.
- Martial Legends is not a generic “Asian faction.” It draws from clearly
  differentiated fictional schools and global combat-sport traditions.
  Specific cultural clothing, names, or practices require informed reference
  and, where appropriate, consultation.
- High Seas Adventure avoids colonial flags, alcohol jokes, and realistic
  firearm treatment.
- Toybox reads as collectible action fantasy, not preschool decoration.
- Bloodless, exaggerated impacts, sparks, stars, cloth, smoke, confetti, and
  elastic knockback serve the desired tone better than injury or realistic
  violence. The [ESRB guide][esrb-guide] distinguishes fantasy violence from
  graphic, realistic-looking intense violence, but only ESRB can assign a
  rating to a submitted product.
- Marketing leads with the ball, teamwork, and mixed teams. Combat is the
  disruptive hook, not the first or only image.

Before production lock, test unlabeled character and equipment thumbnails with
football-first, fighting-game, action, and family/casual players across more
than one region. Test recognition and added reason-to-care, not merely whether
someone already likes pirates or knights.

### Production order inside the six

The first rendering and combat presentation proof uses only:

1. **Medieval Fantasy** — strongest free character/equipment prototype fit;
2. **Galactic Sci-Fi** — strongest contrast and project continuity;
3. **Toybox** — establishes the nonlethal, anything-goes tone.

The bounded six-character and six-item contract is in
`docs/design/prototype_theme_roster.md`. It implements all four prototype
action families while reskinning light melee across the three themes.

[KayKit Adventurers][kaykit-adventurers] supplies free CC0 medieval characters
and equipment. KayKit's current [Series 4][kaykit-series-4],
[Series 5][kaykit-series-5], and [Series 6][kaykit-series-6] packs list robots,
a space ranger, a combat mech, an action figure, an animatronic, and a toy
soldier. [Space Base Bits][kaykit-space-base] supplies matching CC0 environment
props. Some of these character packs are paid even though the assets are CC0;
they are prototype candidates, not committed production art.

Street Sports requires the most original character work and therefore provides
the best early test that the pipeline can produce representation beyond asset
pack recolors. High Seas Adventure has useful CC0 prototype material in the
[Quaternius Pirate Kit][quaternius-pirate], but an external character must be
conformed to GOLISEO's rig, proportions, materials, and sockets before use.

### Deferred theme families

- **Gothic Supernatural / Undead** is a strong expansion or seasonal family,
  but overlaps Medieval Fantasy at launch.
- **Wild Frontier** offers a strong control fantasy through a lasso, but is
  firearm-heavy and culturally narrower. Avoid real nations, tobacco/alcohol
  iconography, and Indigenous stereotypes.
- **Modern Warfare** is not an initial family. Contemporary conflict, national
  uniforms, and realistic weapons narrow the tone and duplicate safer
  Galactic Sci-Fi ranged mechanics.
- **Mythic Antiquity** needs culturally specific treatment and overlaps
  Medieval shields, spears, and bows.
- **Clockwork / Steampunk** is a useful later bridge between Medieval and
  Sci-Fi, but a weak launch contrast for that reason.
- **Wasteland**, **Primal / Dino Wilds**, **Carnival**, **Culinary**, and
  **Elemental** remain expansion research rather than launch pillars.

## KayKit Character Animations audit

- Source: [KayKit - Character Animations][kaykit-animations]
- Audited download: `Free 1.1`, 14 MB
- Audited runtime files: FBX and binary glTF (`.glb`)
- Published license: [CC0 1.0][cc0]
- Rigs: `Rig_Medium` and `Rig_Large`

The current product page advertises 161 humanoid animations, both rig types,
FBX and glTF delivery, a free animation tier, and a paid source tier with
organized Blender files and a basic control rig. The audited `Free 1.1`
archive exposes 159 rig-specific action clips plus one semantic T-pose for
each rig; category files also repeat the relevant rig's T-pose track. The
archive count is direct inspection evidence, not a correction to the
publisher's differently scoped headline count.

### Audited action inventory

`Rig_Medium` is the prototype baseline because it has the complete melee,
ranged, movement, reaction, and presentation coverage:

| Rig | Set | Action clips, excluding `T-Pose` |
| --- | --- | ---: |
| Medium | Movement basic | 10 |
| Medium | Movement advanced | 12 |
| Medium | General | 14 |
| Medium | Melee | 21 |
| Medium | Ranged | 19 |
| Medium | Simulation / emotes | 13 |
| Medium | Special | 14 |
| Medium | Tools | 28 |
| **Medium** | **Total** | **131** |
| Large | Movement | 6 |
| Large | General | 5 |
| Large | Melee | 15 |
| Large | Simulation | 1 |
| Large | Special | 1 |
| **Large** | **Total** | **28** |

The medium archive includes the exact verbs needed to evaluate the first
families: directional dodges; unarmed punch/kick; one-handed, two-handed, and
dual-wield attacks; block raise/hold/hit/counter; one- and two-handed ranged
aim/shoot/reload; bow and magic variants; `Hit_A` / `Hit_B`; and `Cheering`.
The large archive has useful melee and reaction coverage but no ranged set and
little presentation coverage. The source page likewise warns that the
Rig_Large library is smaller.

### Rig observations

- The animation-library GLBs include `handslot.l` and `handslot.r` joints.
  Those are the preferred weapon attachment sockets.
- The free `Mannequin_Large.glb` includes the hand slots in its skin. The free
  `Mannequin_Medium.glb` exposes `hand.l` and `hand.r` but does not include the
  hand slots in its skin. Attachment behavior must be verified against every
  actual character model; matching a rig label is not proof.
- KayKit says non-KayKit humanoids rely on engine retargeting and may not look
  as intended. GOLISEO does not assume runtime retargeting in Menori. Custom
  characters are conformed or retargeted in Blender and exported as validated
  GLBs.
- The source tier may help with mirrored clips, root-motion cleanup,
  football-specific derivatives, or attachment corrections. It is not
  required for the first import/render feasibility proof.

### What the pack makes cheap

| Match need | Available evidence | Prototype use |
| --- | --- | --- |
| Locomotion | Run, walk, strafe, backward walk, crouch, jump, land | Readable body motion |
| Evasion | Four directional dodges on both rigs | Existing juke/dodge presentation |
| Unarmed | Punch and kick on both rigs | Fast short-reach contest |
| Light melee | One-handed chops, slices, stabs, and slashes | Directional challenge family |
| Guard | Block start, hold, hit, and counter | Defensive answer |
| Ranged | Aim, fire, reload, bow, and magic sets | One telegraphed projectile family |
| Reactions | Two medium hit reactions and one large reaction | Stagger feedback |
| Presentation | Cheer, wave, sit, flex, spawn, taunt | Match personality |

The availability of heavy melee, dual wield, magic, reload, death, and many
other clips does not authorize those systems.

### What the pack does not solve

There are no dedicated soccer clips. GOLISEO still needs original or edited
animations for:

- dribble locomotion and directional ball touches;
- short pass, driven pass, lob, and shot contacts;
- first touch, trap, chest control, volley, and header;
- standing tackle and any retained slide-tackle variant;
- goalkeeper set, shuffle, dive, catch, parry, distribution, and recovery;
- weapon-aware soccer contacts and shield locomotion;
- knockback, stumble, and get-up tuned for brief arcade recovery;
- mirrored attacks where directional readability requires both sides.

Football animation remains the critical custom-animation workload. Combat
clips make the pivot more feasible; they do not replace the sports set.

## First combat proof

### Scope

- Preserve the existing 5v5 two-minute match, decided on score at full time.
- Use authored players/stats and fixed loadouts on both teams.
- Prototype four families: **unarmed**, **guard**, **light melee**, and
  **ranged**.
- Give each family one equipment action while preserving the existing soccer
  verbs.
- Protect goalkeepers from combat.
- Keep the 2D renderer sufficient for gameplay validation. The optional
  `Rig_Medium` presentation proof remains milestone 10.
- Do not add heavy melee, dual wield, magic-specific rules, ammunition
  inventory, economy, rarity, upgrades, proficiency XP, injuries, or loot.

### Shared outcome vocabulary

The simulation initially needs only four combat outcomes:

- `guard` — protects against a contest in the front arc while held;
- `stagger` — a brief action interruption;
- `knockback` — short displacement with immediate recovery;
- `ball_spill` — controlled possession becomes a contestable loose ball.

There is no health value. Death clips are not used during ordinary play.

Each action family uses the same bounded fields: wind-up, active and recovery
duration; reach or projectile speed; front arc; movement during commitment;
cooldown; unguarded outcome; and guarded outcome. Exact numbers are shared
prototype tuning, not independently authored sword, blaster, or character
content.

| Family | Advantage | Cost / counterplay |
| --- | --- | --- |
| Unarmed | Fastest recovery; always viable; useful while moving | Shortest reach and weakest displacement |
| Guard | Protects possession and denies one approach lane | Gives up speed and proactive pressure while held |
| Light melee | Reliable reach and directional threat | Visible commitment; punishable miss |
| Ranged | Influences a passing lane without joining a collision | Longest telegraph, recovery, and cooldown |

These are horizontal roles. A team budget may constrain repetitive lineups
after the fixed proof establishes meaningful costs; the first proof lineup
uses one of each family rather than inventing an economy.

### Owner-approved interaction defaults

These decisions are specified normatively in
`docs/design/combat_interaction_contract.md`, which resolves
[issue #107][issue-107] with the input table, state transitions, coexistence
cases, and initial tuning bounds:

- Equipment uses a dedicated action: keyboard `J`, gamepad `B`. Existing
  Space/A shoot-tackle and K/X pass-switch mappings stay intact.
- Unarmed and light-melee actions activate on press. Guard is held. Ranged aims
  while held and fires on release.
- Any opposing outfielder inside the action's front arc and reach is a legal
  geometric target; there is no target lock. Off-ball attacks remain legal but
  should be positionally costly.
- An unguarded carrier spills the ball. A non-carrier briefly staggers or is
  knocked back. Family differences come from reach, timing, displacement,
  cooldown, and recovery, never damage.
- Guard is held front-arc protection with slower movement. It prevents stagger
  and ball spill but may allow a small push. The first proof has no parry.
- Ranged has unlimited ammunition, a long telegraph, recovery, and cooldown.
  There is no reload, inventory, or shared team resource.
- The existing C/L3 juke supplies combat immunity; there is no second dodge.
  Carriers may use equipment, but equipment cannot overlap shooting, passing,
  tackling, aerial actions, or another committed recovery state.
- Keepers have no combat loadout and ignore combat contacts everywhere. The
  penalty area does not protect outfielders. Existing keeper-possession
  protection remains.
- Any family may be assigned to any outfield role. The proof lineup uses one
  of each family; keepers use none.
- Continuous combat-caused loss of control is capped at `0.5s`, followed by at
  least `0.75s` interruption immunity. Detailed tuning stays within those
  bounds.

### Anti-frustration gates

Before progression or content breadth, the fixed slice must demonstrate:

- repeated hits cannot keep a player continuously disabled;
- an unequipped or unarmed player remains viable;
- attacking away from the ball is usually a positional mistake;
- combat creates turnovers and openings without replacing passing or
  dribbling;
- soccer metrics remain healthy under `docs/design/fun_metrics.md`;
- AI uses and responds to every family without privileged information;
- ten characters, equipment, telegraphs, projectiles, the ball, and HUD remain
  readable.

## Technical boundary

The architecture stays one-directional:

```text
gc-data
    player identity, presentation ids, fixed loadouts, family ids
        |
        v
gc-sim
    validates intents and resolves guard/stagger/knockback/ball_spill
        |
        v
ts/packages/render
    maps states to clips, attachments, VFX, SFX, camera, and HUD
```

- `gc-sim` never imports three.js, character assets, GLB files, bone names, or
  animation durations.
- `ts/packages/render` owns the animation controller, loops, one-shots,
  priorities, crossfades, and deterministic resets.
- Runtime assets are glTF 2.0 binary (`.glb`). Editable production sources stay
  as `.blend`.
- A semantic clip manifest maps game verbs to source tracks; match code does
  not contain source names such as `Melee_1H_Attack_Slice_Horizontal`.
- Players may share immutable presentation data, but each player instance owns
  its pose and animation state.
- The procedural 2D renderer remains a selectable fallback until the native
  and browser gates pass.

## Asset and license direction

KayKit publishes the animation pack, Adventurers, Series 4–6, and Space Base
Bits under [CC0 1.0][cc0]; Quaternius publishes Pirate Kit under CC0. CC0
permits copying, modification, and distribution, including commercially,
without permission. Preserve each downloaded license and record provenance,
version, source URL, hashes, and modifications even when attribution is not
required.

Paid access and copyright license are separate facts: purchasing a CC0 pack
does not make an unaudited download a verified production asset. Conversely,
marketplace assets with redistribution restrictions must not be committed or
bundled merely because the source repository is private. The preferred final
character art is original or commissioned with explicit rights to distribute
runtime GLBs and, ideally, editable Blender sources under a
project-compatible asset license.

## Documentation migration ledger

The pivot does not authorize a blind rename. This ledger classifies every
committed Markdown document found by the 2026-07-23 repository-wide audit to
contain Galactic Cup, species-first, planetary, or intergalactic assumptions:

| Document | Classification | Treatment |
| --- | --- | --- |
| `README.md` | **retain** | Remains the public entry point for the active Galactic Cup showcase and its current run, license, and release-status claims. Replace its title and pitch only through an explicit product/release migration. |
| `CONTRIBUTING.md` | **retain** | Remains the contributor contract for the active showcase. Its repository name and showcase priorities change with the public product migration, not ahead of it. |
| `AGENTS.md` | **retain** | Remains the engineering constitution. Its title and `PlayerData` example describe the current repository; update them alongside the implemented schema migration without weakening its architecture or workflow rules. |
| `docs/vision.md` | **replace** | Replaced in this decision with the GOLISEO product identity and proof gates. |
| `docs/data_model.md` | **generalize** | Keeps implemented showcase fields and records the post-showcase player/presentation separation. Runtime migration belongs to milestone 11. |
| `docs/showcase_release.md` | **retain** | Remains the active Galactic Cup showcase boundary. Its teams, species, arena, and public title are intentionally unchanged until explicit rescheduling. |
| `docs/visual_style.md` | **retain** | Remains the showcase UI contract. GOLISEO production branding will replace it only after a dedicated visual-system task. |
| `docs/design/broadcast_presentation.md` | **retain** | Remains the implemented Helios Crown showcase contract and evidence; it is not the future mixed-theme art direction. |
| `docs/design/aerial_reception.md` | **generalize** | Retain implemented aerial mechanics. Future `species` and arena modifier hooks are not GOLISEO authority and must become neutral, explicit modifier seams before reuse. |
| `docs/online/browser_build.md` | **historical record** | Keep the name used by the completed OMP-0 artifact and evidence. |
| `docs/online/omp0_acceptance.md` | **historical record** | Keep the accepted proof wording and evidence identity unchanged. |
| `docs/online/platform_decision.md` | **retain** | Browser-first online direction remains valid; the old product name is incidental migration copy. |
| `docs/online/input_frame.md` | **generalize** | Keep the shipped v1 contract. Milestone 11 will version content identity for presentation/loadout data rather than edit a historical protocol in place. |
| `docs/online/omp1_determinism.md` | **historical record** | Preserve `nebula-orion` fixture and content ids because they identify reproducible evidence. |

All other committed Markdown documents were audited and contained no
product-name or species-first assumption requiring classification. Completed
online evidence, fixture ids, and hashes must never be silently renamed.

The obsolete four-species production issue [#103][issue-103] is closed as not
planned. Its replacement work is explicitly scoped to the six accepted
character presentations in [#115][issue-115] and the six equipment
presentations in [#116][issue-116]; no active implementation issue depends on
the old four-species production plan.

## Deferred decisions

The following do not block the two proofs:

1. Whether the six reusable presentations provide enough final-release
   personality and representation.
2. Whether or when `Rig_Large` enters production.
3. Which persistence/progression model can add attachment without a
   success-to-money-to-power snowball.
4. Exact production meshes, final character names, lore, economy, and
   procedural roster-generation rules.

[cc0]: https://creativecommons.org/publicdomain/zero/1.0/
[esa-2025]: https://www.theesa.com/annual-esa-study-reveals-video-games-universal-appeal-across-generations/
[esrb-guide]: https://www.esrb.org/ratings-guide/
[issue-93]: https://github.com/osobytes/goliseo/issues/93
[issue-103]: https://github.com/osobytes/goliseo/issues/103
[issue-107]: https://github.com/osobytes/goliseo/issues/107
[issue-115]: https://github.com/osobytes/goliseo/issues/115
[issue-116]: https://github.com/osobytes/goliseo/issues/116
[kaykit-adventurers]: https://kaylousberg.itch.io/kaykit-adventurers
[kaykit-animations]: https://kaylousberg.itch.io/kaykit-character-animations
[kaykit-series-4]: https://kaylousberg.itch.io/kaykit-series-4
[kaykit-series-5]: https://kaylousberg.itch.io/kaykit-series-5
[kaykit-series-6]: https://kaylousberg.itch.io/kaykit-series-6
[kaykit-space-base]: https://kaylousberg.itch.io/space-base-bits
[milestone-10]: https://github.com/osobytes/goliseo/milestone/10
[milestone-11]: https://github.com/osobytes/goliseo/milestone/11
[quaternius-pirate]: https://quaternius.com/packs/piratekit.html
