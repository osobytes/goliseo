# AGENTS.md — GOLISEO engineering constitution

This file is the source of truth for **how we write code** in this project. Humans and
AI agents both follow it. Keep it short, keep it enforced.

`ARCHITECTURE.md` is its structural companion: what lives where, why the Rust/TypeScript
line falls where it does, and the per-language house rules. This file is about *practices*.
Game vision and public product scope live in `docs/`.

> **The tree this file describes.** GOLISEO was written in Lua on LÖVE until commit
> `2c0d449` (#467) deleted that tree and promoted the Rust + TypeScript port to the root.
> Nothing below describes the Lua tree any more. Some documents under `docs/` still do,
> and say so at the top — treat an undated `sim/*.lua` path anywhere in `docs/` as
> pre-port history, not as a file you can open.

---

## 1. Stack

| Concern        | Choice                                   | Notes                                                        |
| -------------- | ---------------------------------------- | ------------------------------------------------------------ |
| Simulation     | Rust 1.93, edition 2024                  | `rust/rust-toolchain.toml`, `rust/Cargo.toml`                 |
| Presentation   | TypeScript, `strict`                     | `ts/`, a pnpm workspace                                       |
| Target         | WebAssembly, in a browser                | no native build; `gc-wasm` via `wasm-bindgen`                 |
| Renderer       | three.js                                 | `ts/packages/render`                                          |
| Formatting     | rustfmt + prettier                       | `rust/rustfmt.toml`, `ts/.prettierrc.json` — never hand-format |
| Linting        | clippy `-D warnings`; type-aware eslint  | `rust/clippy.toml`, `ts/eslint.config.mjs`                    |
| Testing        | `cargo test` + vitest, both headless     | tiers in §9                                                   |
| Package manager| pnpm — **never npm**                     | `ts/package.json`'s `packageManager`                          |

Run before every commit:

```bash
./scripts/check.sh              # every gate
./scripts/check.sh --self-test  # prove the gate can go red
```

That one script **is** the gate: `.github/workflows/ci.yml`'s `gate` job invokes it rather
than mirroring its steps, so the two cannot drift (§9). It takes minutes, not seconds —
that is the price of the things it catches, and there is no cheaper substitute that means
the same thing.

**Toolchain pins, and where each one is authored.** Never bump one in isolation:

- `rust/rust-toolchain.toml` — channel `1.93`, the `rustfmt` and `clippy` components, and
  the `wasm32-unknown-unknown` target. rustup activates it automatically for any cargo
  invocation under `rust/`.
- `rust/Cargo.toml` — `edition = "2024"`, `rust-version = "1.93"`, `resolver = "3"`, and
  the workspace lints (§5).
- `rust/crates/gc-wasm/Cargo.toml` — `wasm-bindgen = "=0.2.118"`. Exact, not semver: the
  `wasm-bindgen-cli` binary checks the schema version stamped into the module against its
  own **exactly**, so the crate and the CLI move together or not at all.
- `ts/package.json` — `"packageManager": "pnpm@11.1.2"` and `"engines": { "node": ">=22" }`.

`scripts/check.sh`'s `verify_toolchain_pins` stage enforces all of them up front, and
`.github/workflows/ci.yml` downloads each pinned, hash-verified asset before calling it.
`./scripts/setup.sh` installs the same set locally without `sudo`.

---

## 2. Architecture — layers, one direction

The graph below is the crates' own `Cargo.toml` dependency edges, not an aspiration.
An arrow points from a crate to what it is allowed to depend on:

```
 gc-core           gc-data         depend on nothing at all
    ▲                 ▲            (numeric primitives / content tables)
    └────────┬────────┘
          gc-sim                   the simulation: may use gc-core, gc-data
             ▲
      ┌──────┴──────┐
 gc-render      gc-netcode         siblings: may use core, data, sim —
      ▲              ▲             and never each other
      └──────┬───────┘
          gc-wasm                  the bridge: may use all five, and the
                                   only crate that knows a browser exists
```

**The only allowed dependency direction is upward.** Concretely:

- `gc-core` depends on **nothing** — no other crate in this workspace, no renderer, no
  game state. Numeric primitives only (`rng`, `deterministic_math`, `fnv1a64`, `vec2`).
- `gc-data` depends on **nothing** in this workspace either (serde, for serialization).
  It is content, and content does not import mechanism (§8).
- `gc-sim` may depend on `gc-core` and `gc-data`. It must **never** depend on a renderer —
  today that is `gc-render` and `@gc/render`, and it covers any future renderer just as
  much — nor on `gc-netcode`, `gc-wasm`, or anything that knows a browser exists.
- `gc-render` and `gc-netcode` may depend on `gc-core`, `gc-data` and `gc-sim`. They are
  **siblings**: neither depends on the other, and nothing may make them.
- `gc-wasm` may depend on all of them. It is the bridge, and the only crate that talks to
  the browser.
- **No TypeScript package imports from a Rust crate's source.** The built wasm module is
  the boundary; `@gc/wasm` is the only package that loads it.

The TypeScript side points one way too, and every edge is declared in a `package.json`:
`@gc/core` and `@gc/wasm` depend on nothing; `ui` → core, wasm; `input` → core, ui;
`presentation` → core; `transport` → core; `render` → core, presentation; `screens` →
core, ui, presentation, input, render; `online` → core, transport, wasm; `app` →
everything. There is no edge back down. A test-only edge is a `devDependency`
and must stay one — `@gc/screens` drives a real `@gc/wasm` session in a spec
without its production code importing it.

Why: `gc-core`, `gc-data`, `gc-sim` and `gc-render` stay pure, unit-testable without a
window or a browser, and portable to another renderer later. If you feel the urge to draw
or read input inside `gc-sim`, the boundary is wrong — return data and let the renderer
act on it.

Unlike the Lua tree this replaced, the Rust half of that rule is **compiler-enforced**: an
undeclared dependency is a build error, not a convention. On the TypeScript side pnpm's
isolated `node_modules` does the same job — a package can only import what its own
`package.json` declares. Neither mechanism can tell you the edge was a *good idea*, so a
new dependency between packages is still a review question.

`gc-render` is the sim-to-renderer boundary, **not** a renderer. `gc_render::frame::build`
turns one `MatchState` into one versioned `RenderFrame` payload; `@gc/render` draws that
payload with three.js and a future renderer draws the same payload without it. The payload
is crossed **once per rendered frame, in batch** — never per entity, never per tick
(rollback re-simulates several ticks inside one rendered frame; a per-tick crossing is
what would make a non-Rust renderer unaffordable). Per-entity data is flat, scalar,
structure-of-arrays, and `gc_render::frame_buffer` flattens the whole payload into one
`Float64Array` for the crossing itself. One disclosed exception remains: `RenderFrame.combat`
still carries the nested `FrameCombatModel` until that is flattened. Presentation-derived
state (gait, lean, correction smoothing, follow-through windows) is **not** simulation: it
stays in `@gc/render` and feeds the builder as an explicit input.

A function is "pure" here if it has no side effects and no I/O: same inputs → same outputs.
All gameplay math lives in pure functions; `gc-sim` does no I/O at all, and the app shell
is where mutation and effects live. Even there, UI screens isolate a pure core (layout /
hit-test / state transitions) from drawing, so the UI is testable without a GL context —
see §9.

---

## 3. Modules

**Rust.** One module per file, declared in the crate's `lib.rs`. Every module opens with a
`//!` doc comment saying what it is for — `missing_docs` is on (§5), so this is checked,
not requested. Order inside a file: module doc → `use` → types → constants → `impl` blocks
→ free functions → `#[cfg(test)] mod tests`.

```rust
//! Player progression: experience and levels. Pure — no I/O, no clock.

/// A player's progression state.
pub struct Progression {
    /// Experience earned so far.
    pub xp: i64,
}

/// Award experience.
pub fn add_xp(player: &mut Progression, xp: i64) {
    player.xp += xp;
}
```

- **No global mutable state, ever.** `static mut` is not merely discouraged, it is
  unreachable: `unsafe_code = "forbid"` (§5) makes it a compile error.
- **Never iterate a `HashMap` or `HashSet`.** Iteration order is not part of their
  contract and a desync is the failure mode; clippy denies both types in this workspace
  (`rust/clippy.toml`). Use `Vec`, `IndexMap`, or `BTreeMap`.
- Paths are `crate::`-rooted within a crate and `gc_sim::…` across crates.

**TypeScript.** One concern per file, named exports only, `snake_case` filenames. Relative
imports carry the `.ts` extension (`./formation.ts`); cross-package imports use the
package name (`@gc/core`). A screen exports its seam as one object — `export const
formation = { newState, layout, update };` — so a test can drive the whole screen through
one import.

---

## 4. Types and behaviour

Rust has no classes and we do not simulate them. Data is a `struct`; behaviour is an
`impl` block on it or a free function that takes it.

```rust
/// An immutable 2D vector.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    /// The horizontal component.
    pub x: f64,
    /// The vertical component.
    pub y: f64,
}

impl Vec2 {
    /// Construct a vector from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}
```

- Constructors are `new`, and take no `self`.
- `#[must_use]` on anything pure whose return value is the entire point.
- Derive `Clone, Copy, Debug, PartialEq` where the type is small and cheap; a value type
  that cannot be printed in a failure message costs more than it saves.
- **Everything a test touches is `pub`.** These crates are internal and never published
  (`publish = false`); do not fight visibility to make a test reach a helper.
- Numeric fields default to `f64`. Reach for an integer type only where the value is
  unambiguously an index, a count, or a bit pattern — getting this wrong changes results.

In TypeScript, prefer plain data plus functions over classes, and `readonly` plus
immutable updates in pure code. `update(state, event)` returns the next state; it does not
mutate the state it was handed, and a spec asserts that.

---

## 5. Typing rules

We treat both compilers as compilers. Untyped or undocumented public code is a bug.

**Rust.** The workspace lints in `rust/Cargo.toml` are the contract:

- `missing_docs = "warn"` — every public item needs a doc comment. It is authored as a
  warning and made **fatal** by gate 2's `cargo clippy --workspace --all-targets -- -D warnings`.
- `unsafe_code = "forbid"`. Two packages opt out, both deliberately and both documented in
  their own `Cargo.toml`: `gc-test-alloc` (a dev-only counting global allocator, never a
  normal dependency of anything) and `gc-wasm` (`#[unsafe(no_mangle)]` on its raw
  per-frame render exports, with every other workspace lint reproduced explicitly so
  nothing else is relaxed). A third opt-out needs the same standard of argument.
- `disallowed_types = "deny"` — see §3.

**TypeScript.** `strict`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`. No
`any` — enforced at error severity by eslint, not merely written down. No `!` non-null
assertions: narrow properly. `unknown` plus a type guard where input is genuinely untyped.

**Data shapes are typed too**, so content is checked by the compiler:

```rust
/// A single authored player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerData {
    /// Persistent identity, stable across presentation and loadout changes.
    pub id: &'static str,
    /// Field position.
    pub position: Position,
    /// The five canonical mechanical attributes.
    pub stats: StatBlock,
    /// Fixed prototype loadout; keepers have none.
    pub loadout_id: Option<&'static str>,
}
```

- Shared types (`Vec2`, `StatBlock`, `Position`, …) are declared once where they are
  defined and reused by name. Don't redefine a shape.
- Prefer an `enum` over magic strings for a closed set (positions, tactics, phases). A
  closed set that crosses the wasm boundary is defined twice by necessity and gated by a
  parity checker — see §9's cross-language tier before adding one.
- Absence is `Option<T>` in Rust and an explicit optional in TypeScript, and the caller
  must handle it.

---

## 6. Naming & style

| Thing                      | Convention            | Example                        |
| -------------------------- | --------------------- | ------------------------------ |
| Files / directories        | `snake_case`          | `match_rules.rs`, `match_hud.ts` |
| Crates                     | `kebab-case`          | `gc-netcode`                   |
| TypeScript packages        | `@gc/<name>`          | `@gc/render`                   |
| Rust functions / fields    | `snake_case`          | `add_xp`, `goal_count`         |
| TypeScript values / fns    | `camelCase`           | `newState`, `loadSimHost`      |
| Types                      | `PascalCase`          | `Vec2`, `StatBlock`            |
| Constants                  | `UPPER_SNAKE`         | `MAX_STAMINA`, `ROLLBACK_WINDOW_TICKS` |

rustfmt and prettier own whitespace — 100 columns on both sides (`rust/rustfmt.toml`,
`ts/.prettierrc.json`). Never argue with the formatter; run it. `cargo fmt --all` and
`pnpm format` are the writing halves of the two format gates.

Two naming rules that are not style:

- **Never reassociate or "simplify" floating-point arithmetic.** `a * b + a * c` is not
  `a * (b + c)` in IEEE 754, and the pinned determinism evidence is only valid if
  operation order matches what was captured.
- **Indices inside a serialized payload, a hash input, or a wire format keep their defined
  value**, even where that disagrees with ordinary zero-based indexing elsewhere. The
  protocol is not up for redesign to make indexing uniform.

---

## 7. Errors

Two distinct mechanisms, used deliberately:

- **`assert!(cond, "msg")` / `panic!` / `expect("why this is impossible")`** for
  *programmer errors and invariants* — things that should be impossible if the code is
  correct (missing required field, bad enum, broken state). Fail loud.
- **`Result<T, String>`** for *expected, recoverable failures* the caller is meant to
  handle (lookup miss, validation of external input). The caller **must** check it; never
  `unwrap()` one away and never collapse it into a panic.

Never use a panic for normal control flow, and never silently swallow an `Err`.

The bar for "impossible" is higher here than in a native program: the release profile is
`panic = "abort"`, so a panic on the simulation path does not unwind — it takes the match
down in a player's browser. If a failure has any legitimate cause outside the code's own
bugs, it is a `Result`.

TypeScript uses the shared discriminated union from `@gc/core`:

```ts
export type Result<T, E = string> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: E };
```

A plain `T | null` is only correct where a function has no error case to report at all.

---

## 8. Data is content, code is mechanism

New players, teams, formations, tactics, traits, and arenas are **data edits**, not code
edits. If adding content requires touching `gc-sim` or the app shell, the system isn't
data-driven enough — flag it. Keep `gc-data` free of logic (no behaviour in content
tables; its only dependency is a serializer).

The corollary, because `gc-data` is Rust and half the consumers are TypeScript:
**TypeScript never imports content tables — it receives them.** Take the table as an
explicit parameter rather than duplicating it. Where a duplicate is genuinely unavoidable
it is not trusted, it is **asserted** against the Rust source by a parity checker wired
into the gate; `ARCHITECTURE.md` §4 rule 6 records the two carve-outs taken on those terms
and the standard a third would have to meet.

---

## 9. UI: structure & testing

A browser has a DOM, but a WebGL canvas does not — so "UI testing" here means testing UI
*logic*, not pixels. We make that cheap by splitting every screen into a pure model and a
thin renderer: the same model/update/view seam that makes a reducer testable.

Each screen module exposes:

- `newState(viewport, content) -> State` — a plain object, built from injected content.
- `layout(state) -> Layout` — **pure**. Produces positioned widgets, e.g.
  `{ id: "formation_2-1-1", kind: "button", rect: { x, y, w, h } }`. No drawing.
- `update(state, event) -> [state, action?]` — **pure**. `event` is an abstracted input
  (`{ kind: "click", x, y }`, `{ kind: "key", key: "escape" }`), never a raw DOM event.
  Returns the next state and an optional action (e.g. `{ go: "tactic" }`). It does not
  mutate the state it was given.
- `draw(state, layout)` — **impure**. The ONLY place three.js appears.

Hit-testing is a pure helper: `hit.find(layout, id)` / `hit.at(layout, x, y)`, from
`@gc/ui`. Raw browser events live in the app shell and do nothing but translate input into
`event`s and dispatch to `update`. Because `layout` / `update` / `hit` touch no graphics
and no globals, they run under vitest with no display.

### Testing tiers

| Tier | What | Needs a display? | When |
| ---- | ---- | ---------------- | ---- |
| 1. Logic          | `gc-sim` math, rules, progression, tactic effects; `gc-core`, `gc-data`, `gc-netcode` | no | always |
| 2. UI logic       | layout positions, hit-testing, `update` transitions & emitted actions | no | always |
| 3. Flow           | drive the real screen stack or a real session with a scripted event sequence | no | for flows (title → team sheet → match → result) |
| 4. Cross-language | parity checkers, differential tests against pinned vectors, the wasm determinism digest | no | anything crossing the wasm boundary or the wire |
| 5. Browser evidence | real browsers, real GL, minutes of wall clock | yes | opt-in, by hand and on a schedule |

Tiers 1–4 are the contract — write them. Tier 5 is opt-in: it needs a GPU or software GL
and minutes rather than milliseconds, so it runs from `scripts/browser_*.py` by hand and
from `.github/workflows/scheduled.yml` nightly, and a green gate is **not** evidence about
the renderer.

Where tests live:

- Rust: integration tests in `rust/crates/<crate>/tests/<module>.rs`, mirroring the source
  module they cover; unit tests in a `#[cfg(test)] mod tests` beside the code.
- TypeScript: `*.spec.ts` **beside its source**, not in a mirrored tree —
  `ts/vitest.config.ts` includes `packages/*/src/**/*.spec.ts` and nothing else.

Example UI-logic test (no display), the real shape from
`ts/packages/screens/src/formation.spec.ts`:

```ts
const s = formation.newState(VP, CONTENT);
const [s2] = formation.update(s, clickOn(formation.layout(s), "formation_1-1-2"));
expect(s2.selected).toBe("1-1-2");
expect(s.selected, "update should not mutate its input state").toBe("2-1-1");
```

### A harness self-test is not a harness run

A `--self-test` proves a harness *controller's* own logic — its parsing, its
fixtures. Only starting the harness proves the system it measures. The two are
never interchangeable, and a step is named for the one it actually runs: a heading
that says "fault harness" over a command that started no harness is how a defect
breaking every online match passed nine green checks (#279). So:

- every gate in `scripts/check.sh` must also appear in `.github/workflows/ci.yml`,
  and vice versa — prefer a shared script both call over hand-mirrored steps;
- every gate must come with a demonstration that it can go red, e.g.
  `./scripts/check.sh --self-test`; and
- never trust one signal. A harness that prints failures and exits 0 must fail the
  gate anyway.

`docs/online/fault_harness.md` records the full case.

### A knob that cannot move its metric is not wired

The balance counterpart of the rule above, and it exists for the same reason: a
green signal that cannot go red tells you nothing. A sweep over dead knobs
produces confident nonsense.

> **Every feature ships a test asserting that moving its knob moves its
> metric.** Run the harness at the default and at a perturbed value; assert the
> registered metric moves in the documented direction by more than the
> *measured* noise floor. A knob that cannot shift its own metric is not wired —
> it is decoration, and it fails review.

Three things make that enforceable rather than aspirational:

- **Knobs are registered, not hand-listed.** A tier-1 tunable is authored in
  `gc_data::tunables::SIM_TUNABLES` and assembled by
  `gc_sim::tunable_registry`. The sweep, the F1 panel and the config hash all
  enumerate the registry, so a knob nobody wired still shows up — which is the
  only reason its emptiness is discoverable at all.
- **Metrics are registered too.** `gc_sim::metric_registry` owns each
  measurement's band and its extraction function, so a new metric folds into the
  fun score without a harness edit, and there is exactly one band table.
- **The noise floor is measured, not assumed.**
  `gc_sim::knob_contract::noise_floor` runs the metric at defaults on the
  caller's own seed set, and both entry points refuse a seed set below
  `knob_contract::MIN_SEEDS` — under that a lucky small standard error passes a
  shift nobody could reproduce.
- **The direction is declared and enforced, not checked by hand.**
  `KnobMoveOpts::expect` is where "the documented direction" above is
  documented, and `assert_moves` fails a shift that clears the noise floor with
  the opposite sign. A knob wired backwards passes any magnitude-only test, so
  this is not a nicety: it is the difference between a contract and a
  formality. Keep it distinct from `MetricDirection`, which is the metric's own
  desirability slope rather than a knob's causal claim.

`gc-sim`'s own `tests/knob_contract.rs` is the worked example: one passing case
(`AI_SHOOT_RANGE` shortens `longest_drought_s`) and **two** red demonstrations,
because this gate has two distinct ways to be broken — a knob that moves nothing
(`REPLAY_SLOWMO`, registered and swept and read by no simulation code) and a
knob whose metric moves the wrong way.

The tiers matter to this rule: only **tier 1** (sim-affecting scalars) and
**tier 3** (versioned AI membership band-sets, substituted whole) can move a
metric at all. **Tier 2** (presentation) lives in `gc-render`, which `gc-sim`
cannot depend on, so it is structurally incapable of moving one — see
`gc_data::tunables`'s module doc.

---

## 10. Workflow

- Format on save, or run `cargo fmt --all` and `pnpm format` before committing.
- `./scripts/check.sh` runs every gate — format, lint, type-check, tests, the wasm build
  and its determinism digest — and must pass before commit. **Run the workspace gate, not
  just your own targets:** `cargo clippy -p gc-sim --test my_test` passing is not the same
  as `cargo clippy --workspace --all-targets` passing, and two committed lints got through
  exactly that way. A scoped check is not the check.
- Tests go where §9 says: `rust/crates/<crate>/tests/` mirroring the source module, and
  `*.spec.ts` beside the TypeScript source.
- Branch names are `agent/<YYYY-MM-DD>-issue-<number>-<slug>`, e.g.
  `agent/2026-07-25-issue-180-rollback-shards`. Branches predating this rule use a `codex/`
  prefix — leave those alone; use `agent/` for anything new. No tooling keys off the prefix.
- Small, focused commits. Conventional-ish messages: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`.
- One change = one concern. Don't mix a refactor with a feature.
- **Never add a `Co-Authored-By` trailer (or any co-author / "Generated with" line)
  to commit messages.** Commits are authored as the repo owner, full stop.
- **Never pass `-c user.email` or `-c user.name` to git.** The repository's own config is
  correct; overriding it commits under the wrong address.

---

## 11. Agent etiquette

- Read this file, `ARCHITECTURE.md`, and `docs/` before writing code.
- Stay inside the committed scope in `docs/showcase_release.md`. Discuss
  substantial additions before building ahead.
- Respect the layer boundaries in §2 — they're the one rule that's expensive to fix later.
- When unsure about a shape, define the type first — the `struct` or the `interface` — then
  implement.
- Documents under `docs/` that describe the pre-port Lua tree carry a dated banner saying
  so. `ARCHITECTURE.md`, `README.md` and this file are current; a `docs/` page without a
  banner should be too, and one that contradicts them is a bug worth reporting.
- Use `scripts/gh-project` instead of the global `gh` CLI for any GitHub operation in this repo
  (issues, PRs, comments, checks). It scopes auth to a profile dedicated to this checkout instead
  of your default `gh` account. Authenticate once with
  `scripts/gh-project auth login --hostname github.com --git-protocol ssh`, then use
  `scripts/gh-project` anywhere you'd otherwise type `gh`. The wrapper is untracked, so it
  does not exist inside a worktree — call it by its absolute path from the main checkout.
