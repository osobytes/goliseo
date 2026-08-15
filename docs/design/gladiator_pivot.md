# Design: GOLISEO gladiator pivot

- **Status:** accepted product direction
- **Accepted:** 2026-08-14
- **Supersedes:** the six-theme "multi-theme combat-soccer showdown" and the
  **Crossover spectacle** identity pillar accepted in
  `docs/design/goliseo_theme_pivot.md` (2026-07-23) and reflected in
  `docs/vision.md` and `README.md`
- **Delivery:** post-showcase proofs, same as the document it supersedes
- **Related:** `docs/vision.md`, `docs/design/goliseo_theme_pivot.md`
  (historical), `docs/design/prototype_theme_roster.md` (historical, needs its
  own rescope), `docs/design/combat_interaction_contract.md` (unchanged —
  this document does not alter the mechanics contract), `docs/showcase_release.md`

## Decision summary

GOLISEO drops the six-theme crossover (Medieval Fantasy, Galactic Sci-Fi,
Street Sports, High Seas Adventure, Martial Legends, Toybox mixing freely on
one team). The genre-hopping "crossover spectacle" identity pillar is retired.

GOLISEO commits to **one setting: the gladiatorial colosseum.** Every
character is a gladiator; every match is staged as an arena contest, not an
open crossover of unrelated worlds.

The crossover does not disappear — it moves inside that single setting,
the same way it worked in the real institution. Historical gladiator classes
(Thraex, Murmillo, Secutor, Retiarius, Samnite, Gallus…) were themselves
built from the weapons and fighting styles of different peoples the arena put
on display. That gives GOLISEO two narrower, still-legible axes of variety
instead of one unbounded one:

1. **Weapon/equipment family** — ranged (thrown javelin, sling), short
   one-handed melee (gladius, curved sica), guard (scutum, small parma
   shield), and two-handed heavy weapons (open question — see below).
2. **Cultural/geographic silhouette on top of a weapon family** — a
   Nordic-styled axe gladiator and a Roman sword-and-shield gladiator can
   fight in the same colosseum under the same rules, the way a Thraex and a
   Murmillo did. This is flavor on a shared mechanical family, not a second
   genre.

**Only Rome/the colosseum is in production now.** Additional cultural
silhouettes built on the existing weapon families (a Nordic axe gladiator is
the concrete example already discussed) are documented direction and can
enter production later without another pivot, the same way the old six-theme
document gated Street Sports and High Seas behind Medieval/Sci-Fi/Toybox. They
are not committed content yet.

## What this changes

| Identity pillar (old, `goliseo_theme_pivot.md` / `vision.md`) | New |
| --- | --- |
| **Crossover spectacle:** any era or genre (knight, space ranger, pirate, toy) may share a team. | **One arena, many gladiators:** every character is a gladiator in the colosseum. Variety comes from weapon family and cultural silhouette within that one setting, not from genre-mixing. |
| Six-theme portfolio (Medieval Fantasy, Galactic Sci-Fi, Street Sports, High Seas Adventure, Martial Legends, Toybox), three in first production. | One flagship setting (the colosseum/gladiator thematic). No second setting is in production. |
| "GOLISEO is only the name of the game, not an in-world arena." | GOLISEO's world **is** the colosseum. The name and the setting are now the same thing. |

Everything else in `docs/vision.md`'s design constraints — soccer wins, no
health/death system, short readable combat states, horizontal loadouts,
`gc-sim`/renderer boundaries — is unchanged and still governs.

## Equipment vocabulary (initial: Rome)

Mapped onto the four locked action families from
`docs/design/combat_interaction_contract.md` (unarmed, guard, light melee,
ranged — that contract is mechanics, not theme, and is not being reopened by
this document):

| Family | Roman gladiator equipment |
| --- | --- |
| Unarmed | Bare-handed grapple, wraps |
| Guard | Scutum (large rectangular shield), parma (small round shield) |
| Light melee | Gladius (short sword), sica (curved Thracian blade) |
| Ranged | Pilum (thrown javelin), retiarius net-and-trident kit |

## Open design questions (flagged, not decided here)

These are noted so they aren't lost, and so nobody reads their absence from
the contract as an accidental omission:

- **Two-handed heavy weapons.** The user's stated interest (Nordic-style
  two-handed axes, and two-handed weapons generally) does not map onto any of
  the four existing action families cleanly. Whether that becomes visual
  variety inside an existing family (a two-handed skin on the light-melee or
  guard family) or a genuinely new mechanical family is undecided and needs
  its own pass against `docs/design/combat_interaction_contract.md` before
  any two-handed content is built.
- **Aztec ulama-inspired soldiers**, scoring with the hips/torso instead of
  the feet, is explicitly **not** part of the colosseum envelope this
  document defines — it's a different era, hemisphere, and sport-history
  entirely, not a gladiator archetype the way a Nordic axe-fighter is. It's
  flagged here as a future open question (a possible second setting, or a
  standalone mechanic) rather than committed to any roadmap.
- **Nordic/other cultural silhouettes beyond Rome** are documented direction
  only. Which one enters production next, and in what order, is undecided.

## Documentation migration ledger

| Document | Treatment |
| --- | --- |
| `README.md` | Updated to describe the colosseum/gladiator setting in place of the six-theme crossover pitch. |
| `docs/vision.md` | Updated identity and near-term product sections to match this decision. |
| `docs/design/goliseo_theme_pivot.md` | Marked superseded at the top; kept as historical record of the six-theme decision and its research (KayKit audit, ESA guardrails, license notes) rather than rewritten in place. |
| `docs/design/prototype_theme_roster.md` | Marked superseded at the top; its three-theme (Medieval/Sci-Fi/Toybox) prototype content budget no longer reflects the accepted direction and needs its own rescope before it drives production again. |
| `docs/design/combat_interaction_contract.md` | Untouched. It defines input and mechanics, not theme, and already reads as theme-neutral. |
| `docs/showcase_release.md` | Untouched, per the existing ledger in `goliseo_theme_pivot.md` — it remains the frozen-scope showcase boundary until explicitly rescheduled. |
