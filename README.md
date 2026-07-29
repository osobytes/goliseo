# GOLISEO

GOLISEO is a 5v5 arcade combat-soccer game built for impossible matchups. A
knight can guard midfield beside a space ranger. A wind-up striker can slip
past a pirate. An energy blade and a foam sword can share the same clear,
competitive rules.

Before kickoff, you assemble a five-character squad and choose its shape,
tactic, and—once the combat systems pass their prototype gates—equipment.
Then you take control and play the match yourself.

Soccer always comes first: goals are the only way to win. Combat is a tactical
tool for contesting possession and space through readable blocks, dodges,
staggers, knockbacks, and ball spills. There are no health bars, deaths, or
second victory condition. Every action must create a soccer consequence.

## The GOLISEO identity

- **Crossover spectacle:** characters from Medieval Fantasy, Galactic Sci-Fi,
  Street Sports, High Seas Adventure, Martial Legends, and Toybox can share a
  team.
- **Fast, controllable matches:** exaggerated arcade movement and immediate
  player control keep the action readable.
- **Tactical contact:** telegraphed equipment actions create passing lanes,
  interrupt possession, and reward counterplay rather than attack spam.
- **Meaningful setup:** squad, formation, tactic, player strengths, and
  loadout choices must make a visible difference in the next match.
- **One competitive language:** wildly different themes still obey the same
  soccer rules, team colors, action families, and broadcast presentation.

The accepted product direction is documented in
[docs/vision.md](docs/vision.md) and
[docs/design/goliseo_theme_pivot.md](docs/design/goliseo_theme_pivot.md).

## Playable today

The current build is the deterministic 5v5 arcade-soccer foundation GOLISEO is
growing from. Running `love .` opens the complete playable flow: title, squad
selection, formation, tactic, match, and post-match results.

It currently includes:

- Real-time movement momentum, sprinting, jockeying, tackles, shielding,
  passing, charged and curved shots, keeper control, crosses, and aerial
  finishes.
- A stat-driven simulation where pace, strength, technique, stamina, and
  mental attributes change real match behavior.
- Three formations and three tactics carried from the pre-match setup into
  the simulation.
- A code-driven 2.5D broadcast presentation with a perspective pitch, bloom,
  particles, synthesized audio, and slow-motion goal replays.
- Deterministic headless matches, balance metrics, parameter sweeps, and a
  checked-in gameplay regression tripwire.
- Strict LuaLS types and hundreds of headless logic, UI, flow, and rendering
  tests.

The multi-theme roster, equipment combat, and rigged 3D presentation are the
accepted direction, but they are not being advertised as finished features.
They enter production through bounded performance, readability, and gameplay
proofs.

## Development path

The immediate release stays intentionally narrow: finish one polished match
from title screen to result, with cohesive controls, onboarding, UI, settings,
packaging, and release media. Its cut line and definition of done live in
[docs/showcase_release.md](docs/showcase_release.md).

After that foundation is complete, three questions across two proof streams
test GOLISEO's new identity:

1. Ten rigged 3D players must render and animate within native and browser
   performance budgets.
2. Fixed equipment loadouts must make soccer decisions more interesting
   without creating stun-locks, damage races, or attack spam.
3. Medieval Fantasy, Galactic Sci-Fi, and Toybox samples must look like one
   game before the wider theme roster enters production.

Significant additions should be discussed in a GitHub issue so new breadth
does not outrun the playable core.

## Run locally

GOLISEO targets [LÖVE 11.5](https://love2d.org/) and LuaJIT / Lua 5.1
semantics.

```sh
./scripts/setup.sh
love .
```

`scripts/setup.sh` installs the supported local tools without `sudo` on
x86_64 Linux. If LÖVE 11.5 is already installed, running `love .` from the
repository root is enough to start the game. This is the native desktop path;
it does not use a browser.

A self-contained Linux download is planned in
[issue #31](https://github.com/osobytes/goliseo/issues/31), but is not
published yet. Windows and macOS native packages are deferred until after the
Linux packaging path is proven.

### Browser artifact

The OMP-0 browser proof can be built and served locally without committing
generated files:

```sh
./scripts/web_build.sh
./scripts/web_serve.sh build/web 8000
```

Open <http://127.0.0.1:8000/> in a desktop browser. See
[docs/online/browser_build.md](docs/online/browser_build.md) for runtime
provenance, headers, and the non-interactive smoke check.

## Quality checks

```sh
./scripts/check.sh
```

The project gate runs:

1. StyLua formatting checks.
2. Strict lua-language-server diagnostics.
3. The LÖVE-native headless test suite.
4. A seeded gameplay-signature comparison that catches accidental balance
   drift.

Useful simulation commands:

```sh
love . --test
love . --sim 30
love . --levers 30
love . --tripwire
```

## Architecture

The codebase keeps gameplay logic independent from LÖVE so that the same
simulation can power interactive matches, automated balance runs, and tests.

```text
core/  -> pure utilities
data/  -> typed content tables
sim/   -> deterministic game rules and metrics
game/  -> rendering, audio, input, screens, and LÖVE callbacks
spec/  -> headless logic, UI, flow, and rendering tests
```

Dependencies only point toward lower-level pure modules: `game` may consume
everything, while `sim` never imports LÖVE or `game`. See
[AGENTS.md](AGENTS.md) for the enforced engineering rules and
[docs/data_model.md](docs/data_model.md) for the main data shapes.

## Contributing

The public contribution workflow is described in
[CONTRIBUTING.md](CONTRIBUTING.md). The most useful contributions during the
showcase milestone are focused fixes, tests, accessibility improvements, and
small improvements that support the committed product scope. External code
and content contributions require acceptance of the
[Individual Contributor License Agreement](CONTRIBUTOR_LICENSE_AGREEMENT.md);
contributors keep ownership while granting the Project Owner commercial and
relicensing rights.

## License

Copyright © 2026 GOLISEO contributors.

GOLISEO is source-available under the
[PolyForm Noncommercial License 1.0.0](LICENSE). You may use, study, modify,
and redistribute it only for purposes permitted by that license. Commercial
use requires a separate license from the Project Owner.

Unless a file states otherwise, repository-owned code, documentation, and
assets use the same license. Third-party material retains its own license and
notices; see [THIRD_PARTY.md](THIRD_PARTY.md) for the current inventory and
release requirements.

Versions previously published under the GNU General Public License v3.0 or
later remain available to their recipients under those terms. This change does
not revoke permissions already granted for those versions.

## Project status

GOLISEO is in active development and is not yet a public release. Its complete
title-to-result flow and deterministic 5v5 match are playable; release
engineering, packaging, QA, and media remain. The application now carries the
GOLISEO name throughout its shell, window, browser artifact, and save identity.
Completed online evidence, fixture ids, and content hashes keep the prototype
name that produced them.

Combat, rigged 3D characters, and the multi-theme roster remain bounded proof
work rather than shipped features. Career mode, economies, progression, large
competitions, and other broader systems come only after the core match and new
identity prove themselves.
