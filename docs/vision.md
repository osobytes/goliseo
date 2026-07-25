# Vision

GOLISEO is a 5v5 **arcade combat-soccer game with a manager's point of view**.
Its characters, equipment, and arenas can come from incompatible eras, genres,
and worlds. A medieval crossbow, an energy blade, a fantasy creature, and an
unarmed martial artist can share one team.

**GOLISEO is the name of the game, not the name of an in-world arena, league,
sport, or character class.** The crossover does not need to collapse into one
historical or science-fiction theme.

Its core promise is:

> **Pick the five. Set the shape. Play the plan.**

You make a few fast, legible decisions before kickoff, then personally execute
them in a short match. Formation, tactics, player strengths, and eventually
equipment loadouts are valuable only when they make the next match more
interesting.

## North star

> Manager choices visibly change what happens on the pitch.

A faster player moves faster. A stronger player hits harder. A different five
changes the available tradeoffs. A formation changes team shape. A tactic
changes off-ball behavior. A loadout changes how a player contests space and
possession. The player should be able to point at the consequence within one
match.

## Identity

- **Soccer first:** goals decide the match. Combat creates space, interrupts
  possession, and changes positioning; it is not a separate deathmatch.
- **Crossover spectacle:** original characters and equipment from any era or
  genre may meet on the same pitch.
- **Arcade readability:** short, exaggerated, controllable matches with
  telegraphed actions and fast recovery.
- **Fast management:** one meaningful setup beat, not recurring busywork.
- **Players as characters:** names, roles, stats, loadouts, and recognizable
  silhouettes. Species or origin may add flavor, but is not the mandatory
  organizing system.
- **Competitive broadcast:** the mixed themes are presented with one confident
  sports language: team colors, clear equipment families, and readable match
  states.
- **Engineering as part of the craft:** deterministic pure simulation,
  data-driven content, strict types, tests, and measurable balance.

## Near-term product

The existing source-available showcase and its deterministic 5v5 match remain
implementation baseline. `docs/showcase_release.md` still bounds current
delivery work until it receives a dedicated rescope; the pivot does not
silently add combat, progression, or a 3D renderer to that release.

The accepted post-showcase proof sequence asks:

1. Can ten rigged 3D players render and animate within the native and browser
   performance budgets?
2. Does a fixed-loadout combat prototype make soccer decisions more
   interesting without creating stun-locks or attack spam?
3. Can Medieval Fantasy, Galactic Sci-Fi, and Toybox presentation samples
   establish one coherent art language before the remaining initial themes
   enter production?

These three questions run through two proof streams. Milestone 10 evaluates
rigged-3D performance and three-theme presentation coherence; milestone 11
evaluates fixed-loadout combat within the deterministic soccer match.

The authoritative direction is in `docs/design/goliseo_theme_pivot.md`. The
bounded three-theme character, equipment, and animation content contract is in
`docs/design/prototype_theme_roster.md`.

## Long-term direction

If both proof streams succeed, GOLISEO can deepen in this order:

1. A small original roster drawing from the six initial themes and shared
   equipment families.
2. More teams and a short competition.
3. Player growth and equipment acquisition with visible one-match
   consequences.
4. A lightweight season and only then deeper manager systems.

The list above is direction, not a shipping commitment.

## Design constraints

- Readable over realistic.
- Goals are the only primary victory condition.
- Combat causes short states such as guard, stagger, knockback, and ball loss;
  no health bars, death loop, or long incapacitation.
- Weapon appearance is content; shared equipment families define mechanics.
  A crossbow and a blaster may use the same readable ranged rules.
- Loadouts are horizontal sidegrades. The first proof uses a bounded fixed
  lineup, not a success-to-money-to-power ladder.
- Boot-to-kickoff should be fast; setup should not outlast the match.
- At most two attribute-modifying layers may be active in a match.
- Content is data; mechanisms are code.
- Rigged 3D presentation stays in `game/`; gameplay outcomes stay in pure
  `sim/`.
- A scalable 2D presentation remains available as a fallback until the 3D path
  passes its performance and compatibility gates.
- `sim/` and `data/` stay independent from LÖVE.
- New breadth waits until the current loop is complete and publicly
  presentable.
