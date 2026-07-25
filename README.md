# Galactic Cup

> **Pick the five. Set the shape. Play the plan.**

Galactic Cup is a 5v5 intergalactic arcade soccer game where a short management
setup flows directly into a fast, controllable match. Choose the squad,
formation, and tactic, then take control and find out whether the plan works
on the pitch.

The project is being shaped into a small, complete source-available showcase rather
than a broad career-mode prototype. The current build is playable and its
simulation, rendering, and test infrastructure are already in place; the
active milestone is product polish around that foundation.

Running `love .` now opens the complete product shell: title, squad selection,
formation, tactic, the real match, and post-match results. The deterministic
fake adapter remains available to the headless flow tests.

## What is already here

- A real-time 5v5 match with movement momentum, sprinting, jockeying, tackles,
  shielding, passing, charged and curved shots, keeper control, crosses, and
  aerial finishes.
- A stat-driven simulation: player pace, strength, technique, stamina, and
  mental attributes change real match behavior.
- Three formations and three tactics carried from pure pre-match screens into
  the match simulation.
- A code-driven 2.5D broadcast presentation with a perspective pitch, bloom,
  particles, synthesized audio, and slow-motion goal replays.
- Deterministic headless matches, balance metrics, parameter sweeps, and a
  checked-in gameplay regression tripwire.
- Strict LuaLS types and hundreds of headless logic, UI, flow, and rendering
  tests.

## Showcase release

The first public release is intentionally narrow: one polished match from
title screen to post-match result, with no dead ends and no placeholder
screens. It will add the high-leverage product pieces that prototypes usually
skip—controller support, onboarding, a real squad picker, cohesive UI,
results, settings, packaging, and release media.

The committed scope, cut line, and definition of done live in
[docs/showcase_release.md](docs/showcase_release.md). Significant additions
should be discussed in a GitHub issue before implementation so the public
project stays focused on that playable loop.

## Run locally

Galactic Cup targets [LÖVE 11.5](https://love2d.org/) and LuaJIT / Lua 5.1
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
[issue #31](https://github.com/osobytes/galactic-cup/issues/31), but is not
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

Copyright © 2026 Galactic Cup contributors.

Galactic Cup is source-available under the
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

Galactic Cup is in active development and is not yet the public showcase
release. The new shell, real match, and Helios Crown presentation package are
integrated; release engineering, packaging, QA, and media remain. Career mode,
leagues, transfers, signature skills, and other larger systems are design
material for later and are not part of the current shipping scope.
