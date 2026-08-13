# Contributing to GOLISEO

Thanks for helping make GOLISEO better. The project currently prioritizes a
small, polished showcase release over new systems or content breadth.

Before starting work, read:

- [AGENTS.md](AGENTS.md) for typing, style, testing, and workflow rules.
- [ARCHITECTURE.md](ARCHITECTURE.md) for what lives where, and why the
  Rust/TypeScript line falls where it does.
- [docs/showcase_release.md](docs/showcase_release.md) for the committed
  product scope.
- [docs/vision.md](docs/vision.md) for the product principles behind that
  scope.

## Set up the project

GOLISEO is a Rust simulation compiled to WebAssembly plus a TypeScript
presentation layer on three.js. There is no native build: the browser is the
only target.

```sh
./scripts/setup.sh                  # pinned Rust toolchain, wasm-bindgen-cli, Node, pnpm
cd ts
pnpm install
pnpm --filter @gc/wasm build:web
pnpm dev                            # then open http://localhost:5173/
```

The setup script installs the supported development tools without `sudo`, at
the versions [rust/rust-toolchain.toml](rust/rust-toolchain.toml) and
[ts/package.json](ts/package.json) pin — the same versions CI uses. Use pnpm,
never npm. [README.md](README.md) has the longer version, including the
production build.

## Before opening a pull request

Run the full project gate:

```sh
./scripts/check.sh              # the gate; CI runs this exact script
./scripts/check.sh --self-test  # prove the gate can go red
```

It must pass formatting, lints, strict type checks on both sides, the Rust and
vitest suites, the wasm build, and its pinned determinism digest. It takes
minutes. New behavior needs tests at the cheapest useful tier
([AGENTS.md](AGENTS.md) §9):

- Pure simulation logic in `rust/crates/<crate>/tests/`.
- Pure screen layout, hit-testing and transitions in a `*.spec.ts` beside its
  TypeScript source.
- Whole-flow event sequences for navigation changes.
- Cross-language assertions for anything that crosses the wasm boundary or the
  wire.
- Browser or GPU evidence only when presentation code genuinely requires it —
  it needs a display and does not run in the gate.

## Scope and pull requests

Keep pull requests small and focused on one concern. During the showcase
milestone:

- Fixing a bug, improving accessibility, adding tests, or completing an
  agreed GitHub issue is welcome.
- Adding a season, economy, transfer system, new match verb, or other parked
  feature requires a scope discussion first.
- Content belongs in `gc-data`; gameplay rules belong in `gc-sim`; rendering,
  input and effects belong in `ts/packages/`.
- Do not mix a refactor with an unrelated feature.

Commit messages use short conventional prefixes such as `feat:`, `fix:`,
`test:`, `docs:`, or `refactor:`. Do not add co-author or generated-by
trailers.

## Contributor agreement

GOLISEO is source-available under the PolyForm Noncommercial License
1.0.0. External contributions are accepted only under the
[Individual Contributor License Agreement version
1.0](CONTRIBUTOR_LICENSE_AGREEMENT.md).
The agreement lets you keep ownership of your work while granting the Project
Owner the rights needed to use it commercially and offer it under other
licenses.

Before submitting a pull request:

- Read the contributor agreement.
- Check the external-contributor acceptance in the pull-request template and
  provide your legal name.
- Confirm that you own the contribution or have written permission to make
  every grant in the agreement.

If an employer or another legal entity owns the contribution, do not submit it
under the individual agreement. Contact the Project Owner for a separate
written corporate agreement. Maintainers must not merge an external
contribution without a complete acceptance record.

Do not add code, fonts, audio, images, or other material unless its license
permits the project's noncommercial use and redistribution and its provenance
can be recorded. Include the author/source, version or retrieval date, license
identifier, and required attribution with any third-party material.

## Reporting bugs

A useful report includes:

- The browser and version, and the operating system.
- The exact action or input sequence.
- What happened and what you expected.
- Whether it reproduces from a fresh launch.
- A screenshot, short recording, or terminal output when relevant.

If the bug affects match behavior, include the formation, tactic, score state,
and whether a saved tuning preset was active.
