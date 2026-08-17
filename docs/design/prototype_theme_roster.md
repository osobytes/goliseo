# Design: GOLISEO prototype theme roster

> **Superseded 2026-08-14.** The three-theme (Medieval Fantasy / Galactic
> Sci-Fi / Toybox) prototype content budget below no longer reflects the
> accepted direction — see
> [`docs/design/gladiator_pivot.md`](gladiator_pivot.md). This document needs
> its own rescope to a single colosseum/gladiator roster before it drives
> production again; until then it is historical record, not a current
> contract.

- **Status:** accepted prototype content contract
- **Accepted:** 2026-07-23
- **Delivery:** post-showcase rigged and combat proofs
- **Related:** `docs/vision.md`,
  `docs/design/goliseo_theme_pivot.md`, and
  `docs/showcase_release.md`

## Purpose

This document turns the first three GOLISEO themes into the smallest useful
character-and-equipment roster for the rendering and fixed-loadout combat
proofs.

It answers five production questions:

1. Which character silhouettes should the prototype prepare?
2. Which equipment models are required before the prototype can be evaluated?
3. Which shared action family and animation verb does each item use?
4. How can several distinct players reuse one presentation asset?
5. What is deliberately deferred even when a suitable asset exists?

The roster is a presentation contract, not a class system. Character theme,
field position, authored player stats, presentation asset, and equipment family
remain separate axes.

## Documentation ownership

| Document or system | Owns | Does not own |
| --- | --- | --- |
| `docs/vision.md` | Product promise, identity, and proof gates | Individual assets or animation tracks |
| `docs/design/goliseo_theme_pivot.md` | Theme portfolio, combat rules, research, and technical boundaries | Asset-by-asset production status |
| This document | Prototype characters, equipment presentations, clip aliases, and acceptance criteria | Final balance values or implementation code |
| Future `data/` manifests | Validated machine-readable ids and mappings | Product rationale |
| GitHub issues | Execution ownership, dependencies, and completion status | Canonical design decisions |

If a prototype changes a product rule, update the pivot document first. If it
replaces a model, item, or clip without changing the rule, update this
document.

## Prototype content budget

Prepare:

- three themes: **Medieval Fantasy**, **Galactic Sci-Fi**, and **Toybox**;
- two `Rig_Medium`-compatible character presentations per theme;
- two initial equipment presentations per theme;
- four shared mechanical families across the set;
- three light-melee presentations with identical simulation behavior, one
  from each theme.

This produces six reusable character presentations and six equipment
presentations. It does not require ten unique character meshes for a 5v5
match. Prototype teams may reuse a character presentation with different team
colors, loadouts, names, numbers, cosmetic variants, and authored player data.

The three light-melee items are intentional duplication. They are the cheapest
way to prove the central content rule: a tournament sword, energy blade, and
foam sword can look and sound different without creating three combat systems.

## Reusable character presentations

Names are working presentation codenames. They are labels for production
tracking, not roster-player names, approved lore, or unique people. Several
persistent players may reference the same presentation.

| Presentation codename | Theme | Silhouette job | Presentation energy | Candidate asset leverage |
| --- | --- | --- | --- | --- |
| **Rook Emberguard** | Medieval Fantasy | Broad armored humanoid within the medium-rig envelope | Composed, protective, competitive | KayKit Adventurers knight or armored parts |
| **Bramble Quickstep** | Medieval Fantasy | Light, compact ranger or goblin-like humanoid | Clever, restless, delighted by a risky play | KayKit Adventurers ranger/fantasy parts |
| **Nova Quell** | Galactic Sci-Fi | Clean athletic space-ranger silhouette | Confident, precise, showy without being severe | KayKit Series 4 Space Ranger |
| **AX-7 “Axi”** | Galactic Sci-Fi | Compact humanoid robot with readable limb separation | Earnest, analytical, unexpectedly celebratory | KayKit Series 4 Robots; Series 5 mech only if conformable |
| **Moxie Modular** | Toybox | Heroic action-figure silhouette with visible joints | Bold, playful, treats each match as an adventure | KayKit Series 4 Action Figure |
| **Tock** | Toybox | Rounded wind-up robot or toy-soldier silhouette | Persistent, physical-comedy specialist | Series 4 Animatronic or Series 6 Toy Soldier |

These rows are `concept` status. Naming a candidate pack does not mean a source
mesh has passed licensing, rig, socket, material, performance, or visual
inspection.

### Character rules

- All six use one validated `Rig_Medium` skeleton contract. A source model that
  does not conform is reference or kitbash material, not a runtime exception.
- No codename implies a gameplay passive, required field position, stat range,
  or locked equipment family.
- Both teams apply a strong team-color treatment to every character. Theme
  color is secondary to team ownership.
- Each theme pair contrasts in mass, posture, face treatment, and personality
  while remaining compatible with one animation library.
- The initial set must not imply that armored characters are always defenders,
  small characters are always fast, robots are always ranged, or toys are
  mechanically weaker.
- Final character design broadens gender expression, skin tones, body shapes,
  age cues, and personality across the roster. Six candidate-pack derivatives
  are a pipeline test, not sufficient representation for release.

## Player uniqueness and controlled variation

A **presentation** is a reusable visual package. A **player** is a persistent
roster identity. That distinction lets the proof use six character
presentations without making two 5v5 teams into six cloned people.

Each prototype player is a separate authored record with:

- a stable player id and display name;
- a shirt number and field position;
- five authored stats: pace, strength, technique, stamina, and mental;
- a reusable presentation id;
- a team-color treatment and optional secondary cosmetic variant;
- a fixed prototype loadout;
- later, personality, signature celebration, history, and progression fields.

Two players may share a presentation while differing in every player-facing
field above. Conversely, changing a player's presentation never changes stats.
Theme and equipment appearance also never grant hidden stats.

### Uniqueness ladder

Build identity in layers so model production does not block gameplay variety:

1. **Prototype:** name, number, position, five-stat profile, loadout, and team
   color.
2. **Presentation proof:** secondary material palette, face or head variant,
   and one safe accessory slot where the source supports them.
3. **Roster depth:** preferred idle, celebration, voice treatment, personality
   tag, and short biography.
4. **Long-term attachment:** persistent form, match history, progression, and
   relationships.

Only layer 1 is required to test combat. Layer 2 helps judge whether asset reuse
is visually acceptable. Layers 3 and 4 wait until the core match works.

### Randomness policy

Randomness creates candidates; it does not make an established player
unstable.

- The fixed-loadout proof uses authored players and stats so family comparisons
  remain controlled.
- A later roster generator may use a deterministic roster seed to select a
  name, presentation, cosmetic variant, position tendency, and stat profile.
- Generated results become persistent player data at roster creation. They do
  not reroll when a match starts, a save loads, or equipment changes.
- Stat generation uses a narrow total budget and bounded,
  position-weighted profiles with at least one recognizable strength and one
  weakness. It does not roll five unrelated values independently.
- Theme, species, body size, gender expression, and presentation id never add
  hidden stat bonuses or restrict the possible stat range.
- Matches and replays consume resolved player records. They never regenerate a
  roster from ambient runtime randomness.

Exact generator ranges, budgets, archetypes, and duplicate rules remain
deferred until authored stat profiles and combat families are balanced.

## Minimum convincing equipment set

Build two equipment presentations per proof theme:

| Presentation id | Display name | Theme | Shared family | Attachment | Purpose |
| --- | --- | --- | --- | --- | --- |
| `medieval_heater_shield` | Emberguard Shield | Medieval Fantasy | `guard` | Left hand | Establish held defense silhouette and readability |
| `medieval_tournament_sword` | Tournament Sword | Medieval Fantasy | `light_melee` | Right hand | Familiar visual tutorial for melee |
| `scifi_energy_blade` | Vector Blade | Galactic Sci-Fi | `light_melee` | Right hand | Test emissive presentation without changing outcomes |
| `scifi_pulse_blaster` | Pulse Blaster | Galactic Sci-Fi | `ranged` | Right hand | Establish aim, telegraph, projectile, and recovery |
| `toy_spring_gloves` | Spring Gloves | Toybox | `unarmed` | Both hands | Make close range comic and explicitly nonlethal |
| `toy_foam_sword` | Foam Champion | Toybox | `light_melee` | Right hand | Prove one family can carry a radically different tone |

These are fixed prototype presentations, not ownership restrictions. Axi may
carry the medieval shield and Rook may use the foam sword. At least one
evaluation match uses mixed-theme loadouts so the proof does not read as three
isolated factions.

### Full flavor matrix, bounded by the proof

Only entries marked **build** belong to the first proof:

| Theme | Unarmed | Guard | Light melee | Ranged |
| --- | --- | --- | --- | --- |
| Medieval Fantasy | Tournament gauntlets — later | Emberguard Shield — **build** | Tournament Sword — **build** | Hand crossbow — later |
| Galactic Sci-Fi | Pulse gauntlets — later | Projected force shield — later | Vector Blade — **build** | Pulse Blaster — **build** |
| Toybox | Spring Gloves — **build** | Building-block shield — later | Foam Champion — **build** | Suction launcher — later |

Do not build the six “later” entries merely because a source pack contains a
suitable model. They enter only after the fixed-loadout match and theme-reskin
proof both pass.

## Action-family and animation mapping

Simulation refers only to semantic family, intent, and state ids. Presentation
maps those verbs to exact source tracks. Animation duration never defines
simulation timing.

The first candidate mapping for `Rig_Medium` is:

| Semantic presentation verb | Candidate KayKit track | Used by |
| --- | --- | --- |
| `combat_unarmed_ready` | `Melee_Unarmed_Idle` | Spring Gloves |
| `combat_unarmed_strike` | `Melee_Unarmed_Attack_Punch_A` | Spring Gloves |
| `combat_guard_raise` | `Melee_Block` | Emberguard Shield |
| `combat_guard_hold` | `Melee_Blocking` | Emberguard Shield |
| `combat_guard_impact` | `Melee_Block_Hit` | Emberguard Shield |
| `combat_light_melee_strike` | `Melee_1H_Attack_Slice_Horizontal` | Tournament Sword, Vector Blade, Foam Champion |
| `combat_ranged_aim` | `Ranged_1H_Aiming` | Pulse Blaster |
| `combat_ranged_release` | `Ranged_1H_Shoot` | Pulse Blaster |
| `combat_stagger` | `Hit_A` | Shared reaction |
| `presentation_celebrate` | `Cheering` | Shared match presentation |

Every track is a candidate until inspected through the real Menori path. Kick,
counterattack, continuous shooting, reload, magic, bow, two-handed, and
dual-wield tracks remain unused in the first proof.

### Animation integration rules

- Root motion is ignored. Pure simulation owns position, facing, contact, and
  outcomes.
- If a clip needs cropping, mirroring, retiming, or hand correction, create a
  derived production clip in Blender and keep the semantic verb stable.
- The three light-melee presentations call the same simulation action and
  semantic animation verb. Model, material, trail, particles, and sound may
  differ.
- A ranged projectile is a simulation entity with a presentation id. It is not
  advanced by animation frames.
- Check weapon sockets, scale, grip, and ball clearance during idle, run,
  wind-up, active, recovery, stagger, and celebration states.
- Football-specific animation is separate production work. No combat mapping
  substitutes for dribble, pass, shot, reception, or goalkeeper clips.

## Theme presentation contract

| Channel | Medieval Fantasy | Galactic Sci-Fi | Toybox |
| --- | --- | --- | --- |
| Shape | Plates, leather, heraldic blocks, forged edges | Clean panels, luminous seams, compact technology | Chunky pieces, visible joints, soft or molded edges |
| Impact | Dust, cloth snap, restrained sparks | Energy arc, pulse ring, brief emissive trail | Stars, springs, confetti, squeak or pop |
| Sound | Wood, leather, metal clack | Synth pulse, hum, charged snap | Plastic clack, rubber boing, toy percussion |
| Secondary palette | Warm metals and cloth | Cool luminous accents | Saturated collectible colors |
| Prop vignette | Tournament banner and rack | Equipment charging stand | Packaging blocks or toy chest |

Team color overrides secondary palette on the largest readable ownership
surfaces. Effects remain short and cannot hide the ball, facing, telegraph, or
contact result.

## Proof lineup

Each team uses:

- one protected, combat-disabled goalkeeper;
- one guard outfielder;
- one light-melee outfielder, with themed presentations rotated between
  controlled comparisons;
- one ranged outfielder;
- one unarmed outfielder.

Reuse and recolor the six character presentations to fill ten slots. Rotate
the three light-melee items between otherwise identical loadouts across
matches. This exposes presentation differences without changing balance
inputs.

## Acceptance criteria

The content proof passes only when:

- ten `Rig_Medium` player instances meet the native and browser performance
  gates owned by milestone 10;
- team ownership remains legible when one presentation appears on both teams;
- two players sharing one presentation remain distinguishable through
  player-facing identity without implying different stats through body shape;
- an observer distinguishes unarmed, guard, light-melee, and ranged intent
  before contact without relying only on color;
- Tournament Sword, Vector Blade, and Foam Champion produce identical
  simulation timing and outcomes;
- swapping those light-melee items changes only presentation data and asset
  references;
- every item stays attached through required animation states without
  unacceptable body or ball intersections;
- projectiles, trails, and impacts do not obscure the ball or active player;
- Toybox reads as playful without making the competition feel preschool;
- at least one mixed-theme lineup reads as a coherent sports team rather than
  unrelated asset packs;
- no deferred item, progression system, or additional theme is required to
  judge whether combat improves the soccer match.

## Production statuses

Use these terms consistently in issues and reviews:

| Status | Meaning |
| --- | --- |
| `concept` | Named here; no runtime asset selected |
| `sourced` | Candidate asset, version, source, and license recorded |
| `conformed` | Mesh, skeleton, materials, scale, and sockets meet the project contract |
| `integrated` | Loads through the real manifest and animation controller |
| `verified` | Passes visual, gameplay, license, and performance acceptance |

An item or character is not “done” merely because its GLB renders.

## Deferred until the proofs pass

- more than two character presentations per proof theme;
- the remaining three initial themes;
- the six later equipment presentations in the flavor matrix;
- character-specific abilities or theme passives;
- procedural roster generation and generated-player progression;
- heavy melee, dual wield, magic-specific rules, ammunition, and upgrades;
- final names, biographies, voice, narrative relationships, and rarity;
- unrestricted character proportions or a second production rig;
- a theme-specific arena for each family.

## Decisions this specification resolves

- The first presentation proof uses Medieval Fantasy, Galactic Sci-Fi, and
  Toybox.
- The reusable character budget is six presentations, not ten.
- Ten distinct player identities may reference those six presentations and
  keep different authored stats.
- The equipment budget is six presentations, not twelve.
- Light melee is the deliberate cross-theme reskin test.
- All characters and equipment remain cross-compatible within the validated
  `Rig_Medium` and socket contract.

Exact source meshes remain production decisions until candidate files are
inspected, licensed, conformed, and measured.
