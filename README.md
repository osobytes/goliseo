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

GOLISEO runs in a browser. The simulation is Rust compiled to WebAssembly and
the presentation is TypeScript on three.js; there is no native build, and the
browser is the only target. Starting the dev server (see
[Run locally](#run-locally)) opens the complete playable flow: title, squad
selection, formation, tactic, match, and post-match results.

It currently includes:

- Real-time movement momentum, sprinting, jockeying, tackles, shielding,
  passing, charged and curved shots, keeper control, crosses, and aerial
  finishes.
- A stat-driven simulation where pace, strength, technique, stamina, and
  mental attributes change real match behavior.
- Three formations and three tactics carried from the pre-match setup into
  the simulation.
- A rigged 3D broadcast presentation: skinned characters driven by an explicit
  pose table, a coliseum stadium, a following broadcast camera, bloom,
  particles, synthesized audio, and slow-motion goal replays.
- A simulation whose compiled wasm module is pinned by a checked-in
  determinism hash, so every client computes bit-identical results — the
  precondition for rollback netcode.
- Strict types on both sides — `deny(missing_docs)` Rust and `strict`
  TypeScript — and hundreds of headless logic, UI, flow, and rendering tests.

The multi-theme roster and equipment combat are the accepted direction, but
they are not being advertised as finished features. They enter production
through bounded performance, readability, and gameplay proofs.

## Development path

The immediate release stays intentionally narrow: finish one polished match
from title screen to result, with cohesive controls, onboarding, UI, settings,
packaging, and release media. Its cut line and definition of done live in
[docs/showcase_release.md](docs/showcase_release.md).

After that foundation is complete, three questions across two proof streams
test GOLISEO's new identity:

1. Ten rigged 3D players must render and animate within the browser
   performance budget.
2. Fixed equipment loadouts must make soccer decisions more interesting
   without creating stun-locks, damage races, or attack spam.
3. Medieval Fantasy, Galactic Sci-Fi, and Toybox samples must look like one
   game before the wider theme roster enters production.

Significant additions should be discussed in a GitHub issue so new breadth
does not outrun the playable core.

## Run locally

You compile the Rust simulation to WebAssembly once, then run the dev server
and open the game in a desktop browser.

### Toolchain

```sh
./scripts/setup.sh
```

That installs everything the game and its gate need, without `sudo`: `rustup`
plus the channel, components, and `wasm32-unknown-unknown` target pinned by
[rust/rust-toolchain.toml](rust/rust-toolchain.toml); `wasm-bindgen-cli` at
exactly 0.2.118; Node.js >= 22; and pnpm at exactly 11.1.2. The pins mirror
CI's `gate` job, so a contributor's machine and the runner end up with the
same toolchain. Make sure `~/.local/bin` is on your `PATH` afterwards.

`wasm-bindgen-cli`'s version is exact rather than semver on purpose: the CLI
checks its generated glue against the crate's schema version exactly, and a
mismatch fails opaquely inside wasm-bindgen's own codegen. Use pnpm, never
npm.

Node and pnpm are downloaded from pinned, hash-verified releases, and only
for linux-x86_64; on any other platform the script tells you to install those
two yourself and re-run.

### Start the game

```sh
cd ts
pnpm install
pnpm --filter @gc/wasm build:web
pnpm dev
```

Then open <http://localhost:5173/>. Controls are in
[docs/controls.md](docs/controls.md).

`pnpm dev` serves [ts/index.html](ts/index.html), which loads the app shell
entry point `packages/app/src/browser_main.ts`.

The `build:web` step compiles `crates/gc-wasm` for `wasm32-unknown-unknown`
and runs `wasm-bindgen --target web` into `ts/packages/wasm/dist/pkg-web/`.
That directory is gitignored, so a fresh clone must build it before the dev
server will boot, and **any change under `rust/` needs it rebuilt** — nothing
invalidates a stale artifact for you, and the dev server will keep serving
the old simulation without complaint.

### Production build

```sh
cd ts
pnpm build
pnpm preview
```

`pnpm build` emits a static bundle to `ts/dist-app/`, with the `.wasm` binary
hashed in as an ordinary asset; any static file server can host it.
`pnpm preview` serves that bundle on <http://localhost:4173/>.

A self-contained downloadable build is not published yet.

## Quality checks

```sh
./scripts/check.sh              # the gate
./scripts/check.sh --self-test  # prove the gate can go red
```

This one script *is* the gate: CI's `gate` job invokes it rather than
mirroring its steps, so the two cannot drift. It runs the cross-language
wire-enum and presentation-content parity checks first (they need no
toolchain and no build, so drift is reported in seconds), then `cargo fmt`,
`cargo clippy`
across the workspace, `cargo test`, a *separate* clippy run against the wasm
target, `pnpm install --frozen-lockfile`, prettier, a forced `tsc --build`,
eslint, and vitest.

Two of its steps are worth knowing about, because they exist in response to
real defects that everything else missed:

- It asserts twice, independently, that the freshly built wasm module
  reproduces a pinned determinism hash. That is what proves a wasm build did
  not perturb float behavior.
- It builds the browser wasm target and byte-compares the `.wasm` asset that
  lands in `dist-app/assets` against it. The artifact the tests exercise and
  the artifact the browser loads are two separate `wasm-bindgen` outputs; on
  2026-08-07 the gate passed while the browser one was thirteen hours stale.

Individual commands, useful while iterating:

```sh
cd rust && cargo test --workspace
cd rust && cargo clippy --workspace --all-targets -- -D warnings
cd ts && pnpm test
cd ts && pnpm typecheck
cd ts && pnpm lint
```

`pnpm lint:fix` and `pnpm format` are the writing halves of the lint and
format checks.

## Architecture

The Rust/TypeScript split is the determinism line, not the logic/display
line. Anything that can change simulation state, or that must produce
byte-identical results on every client, is Rust; anything that only *reads*
simulation state is TypeScript. This is not a style preference. ECMAScript
specifies `sin`, `cos`, `exp` and friends as implementation-approximated, so
two browsers may return different bits for the same input, and rollback
netcode desyncs on one bit. wasm float semantics are spec-pinned and libm
compiles into the module.

```text
rust/crates/
  gc-core     -> rng, deterministic math, fnv1a64, vec2
  gc-data     -> typed content tables
  gc-sim      -> the simulation
  gc-render   -> the RenderFrame producer
  gc-netcode  -> wire encoding, resim scheduling, session state machine
  gc-wasm     -> the wasm-bindgen bridge to the browser

ts/packages/
  core         -> presentation math
  ui           -> layout and hit-testing
  input        -> input capture and bindings
  presentation -> cosmetics, themes, branding
  render       -> the three.js renderer
  screens      -> screens
  transport    -> WebRTC / WebSocket
  online       -> lobby, signalling, diagnostics
  wasm         -> loads the built wasm module and types its exports
  app          -> app shell
```

Dependencies point one way only: `gc-sim` never depends on `gc-render`, and
no TypeScript package imports a Rust crate's source. Screens keep a pure
`layout` / `update` seam so they test headless, without a GL context.

[ARCHITECTURE.md](ARCHITECTURE.md) is the full account — where the line falls
inside the online stack, which modules deliberately exist in both languages,
the house rules for each side, and what three.js absorbs. See
[AGENTS.md](AGENTS.md) for the engineering constitution and
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
title-to-result flow and deterministic 5v5 match are playable in a browser;
release engineering, packaging, QA, and media remain. The application carries
the GOLISEO name throughout its shell, window, browser artifact, and save
identity. Completed online evidence, fixture ids, and content hashes keep the
prototype name that produced them.

The game was originally written in Lua on LÖVE. That tree was deleted once the
Rust + TypeScript port reached parity, so some documents under `docs/` still
describe the old implementation and its file paths; `ARCHITECTURE.md` is
current and authoritative. Combat and the multi-theme roster remain bounded
proof work rather than shipped features. Career mode, economies, progression,
large competitions, and other broader systems come only after the core match
and new identity prove themselves.
