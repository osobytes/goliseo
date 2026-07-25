# Contributing to Galactic Cup

Thanks for helping make Galactic Cup better. The project currently prioritizes a
small, polished showcase release over new systems or content breadth.

Before starting work, read:

- [AGENTS.md](AGENTS.md) for architecture, typing, style, and testing rules.
- [docs/showcase_release.md](docs/showcase_release.md) for the committed
  product scope.
- [docs/vision.md](docs/vision.md) for the product principles behind that
  scope.

## Set up the project

Galactic Cup targets LÖVE 11.5 and LuaJIT / Lua 5.1 semantics.

```sh
./scripts/setup.sh
love .
```

The setup script installs the supported development tools without `sudo` on
x86_64 Linux. You may also install LÖVE 11.5, StyLua, and
lua-language-server independently.

## Before opening a pull request

Run the full project gate:

```sh
./scripts/check.sh
```

It must pass formatting, strict type checks, headless tests, and the seeded
gameplay tripwire. New behavior needs tests at the cheapest useful tier:

- Pure simulation logic under `spec/sim/`.
- Pure screen layout and transitions under `spec/screens/` or `spec/ui/`.
- Whole-flow event sequences for navigation changes.
- Rendering smoke or visual tests only when presentation code requires them.

## Scope and pull requests

Keep pull requests small and focused on one concern. During the showcase
milestone:

- Fixing a bug, improving accessibility, adding tests, or completing an
  agreed GitHub issue is welcome.
- Adding a season, economy, transfer system, new match verb, or other parked
  feature requires a scope discussion first.
- Content belongs in `data/`; gameplay rules belong in `sim/`; LÖVE effects
  and mutation belong in `game/`.
- Do not mix a refactor with an unrelated feature.

Commit messages use short conventional prefixes such as `feat:`, `fix:`,
`test:`, `docs:`, or `refactor:`. Do not add co-author or generated-by
trailers.

## Contribution license

Galactic Cup is source-available under the PolyForm Noncommercial License
1.0.0. By submitting a contribution, you agree to license that contribution
under the same terms and confirm that you have the right to do so.

Do not add code, fonts, audio, images, or other material unless its license
permits the project's noncommercial use and redistribution and its provenance
can be recorded. Include the author/source, version or retrieval date, license
identifier, and required attribution with any third-party material.

## Reporting bugs

A useful report includes:

- The LÖVE version and operating system.
- The exact action or input sequence.
- What happened and what you expected.
- Whether it reproduces from a fresh launch.
- A screenshot, short recording, or terminal output when relevant.

If the bug affects match behavior, include the formation, tactic, score state,
and whether a saved tuning preset was active.
