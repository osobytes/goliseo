# v2 — Rust + TypeScript/three.js

This directory is the port of Galactic Cup off LÖVE/Lua and onto **Rust compiled
to wasm** (the simulation) plus **TypeScript with three.js** (the presentation).
Native support is deliberately dropped; the browser is the only target.

`v2/` does not replace the Lua tree yet. Both live side by side until the port is
complete. Nothing in `v2/` may `require` or read the Lua sources at runtime.

---

## 1. Scope of the current milestone

**In scope:** translate every module and every unit test, and make the tests pass.

**Out of scope:** the glue that makes a playable browser build — wasm bindings,
the JS↔wasm marshalling layer, asset pipeline, bundling, a running app. That is a
separate milestone. Do not build it, and do not block on it.

Concretely: `cargo test` is green in `v2/rust`, and `pnpm test` is green in
`v2/ts`. That is the finish line.

---

## 2. The layer split, and why

**The Rust/TypeScript line is the determinism line, not the logic/display line.**

Anything that can change simulation state, or that must produce byte-identical
results on every client, is Rust. Anything that only *reads* simulation state is
TypeScript.

This is not a stylistic preference. ECMAScript specifies `sin`, `cos`, `tan`,
`exp`, `log` and `acos` as *implementation-approximated* — two browsers may return
different bits for the same input. The Lua sim has 31 such call sites. A rollback
netcode desyncs on one bit. wasm float semantics are spec-pinned and libm compiles
*into* the module, so every runtime executes identical instructions. That is the
whole reason the sim is Rust and can never be TypeScript.

The test for any file: **can this code change what the simulation computes?**
If yes it is Rust, even when it feels like presentation.

| Lua source | → | v2 home |
| --- | --- | --- |
| `core/rng.lua`, `core/deterministic_math.lua`, `core/fnv1a64.lua` | Rust | `crates/gc-core` |
| `core/vec2.lua`, `core/mat4.lua`, `core/quat.lua` | TS | `packages/core` |
| `data/**` | Rust | `crates/gc-data` |
| `sim/**` | Rust | `crates/gc-sim` |
| `render/**` (the RenderFrame *producer*) | Rust | `crates/gc-render` |
| `game/online/**` — rollback scheduling, input encode/decode, state hashing, protocol | Rust | `crates/gc-netcode` |
| `game/online/**` — lobby, signalling, session coordination, diagnostics | TS | `packages/online` |
| `game/render/**` | TS | `packages/render` (three.js) |
| `game/screens/**` | TS | `packages/screens` |
| `game/ui/**` | TS | `packages/ui` |
| `game/input/**` — capture and bindings | TS | `packages/input` |
| `game/presentation/**` | TS | `packages/presentation` |
| `game/transport/**` | TS | `packages/transport` |
| `game/` root files | TS | `packages/app` |

`sim/` requires exactly two `core/` modules — `core.rng` (8 call sites) and
`core.deterministic_math` (1). `core/mat4` is presentation-only: neither `sim/`
nor `render/` ever requires it. That is why `core/` splits rather than moving.

### 2.1 Where the line falls inside `game/online/`

This was the one genuinely contested boundary, so it was decided by evidence: which
modules import `sim.input_frame` / `sim.match_snapshot` / `sim.rollback_*`, and
which of them produce state that two peers must agree on bit for bit.

**Rust (`gc-netcode`), ~13,900 lines.** Wire encoding, resim scheduling, and the
replicated session state machine:

```
coordinator.lua 3306          match_driver.lua 2297       protocol.lua 1904
fault_harness.lua 1446        input_protocol.lua 1121     fault_scenarios.lua 602
fault_transport.lua 553       coordinator_driver.lua 441  desync_package.lua 426
match_manifest.lua 288        match_session.lua 279       protocol_fixture.lua 254
coordinator_conformance.lua 224  match_driver_fixture.lua 183  live_slot.lua 171
input_protocol_fixture.lua 162   input_protocol_conformance.lua 157
protocol_conformance.lua 141     coordinator_fixture.lua 102
```

`coordinator.lua` is the surprising one — it reads like lobby code. It is Rust
because it records per-tick hashes, decides `hash_mismatch` / `late_input` /
`desync` outcomes, and freezes the session at an agreed `first_input_tick`. Both
peers run it over the same event stream and must land in the same state; a
divergence there is a protocol violation, not a cosmetic difference. It is already
a pure `update(state, event) -> state, actions` reducer, so the port is mechanical.

**TypeScript (`@gc/online`), ~4,000 lines.** Observability and coordination that
no peer has to agree with:

```
net_diagnostics.lua 1770      diagnostics_schema.lua 764   diagnostic_transport.lua 441
lobby_link.lua 314            net_diagnostics_fixture.lua 305
match_presentation.lua 244    fault_campaign.lua 163
```

Two boundaries worth restating because they are easy to get backwards:

- **Input capture is TS; input *encoding* is Rust.** Reading a gamepad is a
  browser concern. Turning it into an `InputFrame` is what goes on the wire and
  into the resim, so it must be bit-identical.
- **Correction smoothing is TS, and strictly one-directional.** Visually easing a
  rollback correction is presentation. The moment it feeds back into sim state you
  have put non-determinism in the loop.

---

## 3. Directory layout

```
v2/
  rust/
    Cargo.toml            workspace
    crates/gc-core        rng, deterministic math, fnv1a64
    crates/gc-data        content tables
    crates/gc-sim         the simulation
    crates/gc-render      RenderFrame producer
    crates/gc-netcode     deterministic online
  ts/
    pnpm-workspace.yaml
    packages/core         presentation math
    packages/ui           layout + hit-testing
    packages/input        input capture
    packages/presentation cosmetics, themes, branding
    packages/render       three.js renderer
    packages/screens      screens
    packages/transport    WebRTC / WebSocket
    packages/online       lobby, signalling, diagnostics
    packages/app          app shell
```

---

## 4. File and test mapping

**One Lua file becomes one Rust module or one TS file, same `snake_case` name.**
Do not merge modules, do not split them, do not rename them. A reviewer must be
able to diff `sim/aerial.lua` against `crates/gc-sim/src/aerial.rs` line by line.

**Tests mirror the same way.**

| Lua spec | Rust | TypeScript |
| --- | --- | --- |
| `spec/sim/aerial_spec.lua` | `crates/gc-sim/tests/aerial.rs` | — |
| `spec/screens/formation_spec.lua` | — | `packages/screens/src/formation.spec.ts` |

Lua specs use `spec/support/runner.lua` (`t.describe`, `t.it`, `t.eq`,
`t.is_true`). Translate:

- Rust — one `#[test] fn` per `t.it`, named `<describe>_<it>` in `snake_case`.
  Group with `mod`. Use `assert_eq!` / `assert!`.
- TypeScript — vitest `describe` / `it` / `expect`, one to one.

**Every assertion must survive.** Do not drop a case because it is awkward. If a
test genuinely cannot be expressed, port it as `#[ignore]` / `it.skip` with a
comment saying why, and report it — never delete it silently.

---

## 5. Porting rules — Lua → Rust

1. **Every Lua number is an f64.** Default to `f64`. Reach for an integer type
   only where the source is unambiguously an index, a count, or a bit pattern.
   Getting this wrong changes results.

2. **Preserve arithmetic exactly.** Do not reassociate, do not factor, do not
   "simplify". `a * b + a * c` is not `a * (b + c)` in floating point, and the
   determinism evidence depends on bit-exact output. Keep the operation order the
   Lua code has, including the order of accumulation in loops.

3. **1-based → 0-based.** Convert internal indexing. But any index that appears in
   a **serialized payload, a hash input, or a wire format keeps its original
   value** — the network protocol is not being redesigned.

4. **Never `HashMap` or `HashSet`.** Iteration order is not part of their contract
   and a desync is the failure mode. Use `Vec`, `IndexMap`, or `BTreeMap`. Clippy
   denies the hash types in this workspace.

5. **Errors, two mechanisms, kept distinct** (AGENTS.md §7):
   - `assert(cond, msg)` — programmer error, invariant → `assert!(cond, "msg")`.
   - `return nil, err` — expected recoverable failure → `Result<T, String>`.
     Never collapse the second into a panic.

6. **Shapes.** A Lua table used as a record becomes a `struct` deriving
   `Clone, Debug, PartialEq`. Used as an array, `Vec<T>`. Used as a string-keyed
   map, `IndexMap<String, T>` — or a struct when the key set is fixed. LuaCATS
   `---@class` is your struct; `---@alias X "a"|"b"` is an `enum`.

7. **`function m.f(t, ...)` that mutates and returns `t`** becomes
   `fn f(&mut self, ...)`. Do not invent a cloning API.

8. **Everything a test touches is `pub`.** Crates are internal; do not fight
   visibility.

9. **`#![deny(missing_docs)]` is on.** Every public item needs a doc comment.
   Port the Lua comment where one exists rather than inventing prose.

---

## 6. Porting rules — Lua → TypeScript

1. **TypeScript 7, `strict`, `noUncheckedIndexedAccess`,
   `exactOptionalPropertyTypes`.** No `any`. No `!` non-null assertions — narrow
   properly. `unknown` plus a type guard where input is genuinely untyped.

2. **Relative imports carry the `.ts` extension** (`./foo.ts`).
   `rewriteRelativeImportExtensions` is on. Cross-package imports use the package
   name (`@gc/core`).

3. **1-based → 0-based**, same rule and same exception as Rust.

4. **`return nil, err`** becomes the shared discriminated union from `@gc/core`:
   ```ts
   export type Result<T, E = string> =
     | { readonly ok: true; readonly value: T }
     | { readonly ok: false; readonly error: E };
   ```
   A plain `T | null` is only correct where the Lua returned a bare `nil` with no
   second value.

5. **Screens keep the model/view seam** (AGENTS.md §9). `layout(state, viewport)`
   and `update(state, event)` stay **pure** — no three.js, no DOM, no globals — so
   they test headless. `draw` is the only impure function, and it is the only
   place three.js appears.

6. **`---@class` → `interface`** for data shapes; a `class` only where the Lua used
   metatable OOP with `.new` plus `:` methods. `---@alias` → a union type.

7. **Prefer `readonly` and immutable updates** in pure code. Mirror Lua's
   in-place mutation only where the Lua deliberately mutates.

---

## 7. What three.js absorbs, and what it does not

`game/render/` is 9,339 lines. Roughly 2,900 of those are hand-written engine
features three.js ships, and they should be **deleted, not translated**:

| Lua | three.js replacement |
| --- | --- |
| `rig3d/skeleton.lua`, `rig3d/body.lua`, `rig3d/meshbuilder.lua`, `rig3d/shapes.lua` | `Skeleton`, `Bone`, `SkinnedMesh`, `BufferGeometry` |
| `rig3d/renderer.lua` and its hand-written GLSL | `WebGLRenderer`, `MeshStandardMaterial` |
| `bloom.lua` | `UnrealBloomPass` |
| `gl_probe.lua` | three.js capability detection |

**Do not delete anything without saying so in your report.** Name the file and
the replacement.

Two things three.js does **not** provide, which stay hand-written:

- **IK.** `CCDIKSolver` is example-tier, CCD-based and MMD-oriented; it is not the
  two-bone constraint this game needs.
- **Bone masking.** `AnimationMixer` blends whole clips by action weight.

With native dropped, these get written **once** rather than twice. That is the
specific thing that makes three.js viable in this configuration.

---

## 8. Commands

```bash
cd v2/rust && cargo test          # Rust unit + integration tests
cd v2/rust && cargo clippy --all-targets -- -D warnings
cd v2/rust && cargo fmt --check

cd v2/ts && pnpm install
cd v2/ts && pnpm test             # vitest
cd v2/ts && pnpm typecheck        # tsc --build (TypeScript 7)
```

Use **pnpm**, never npm.

---

## 9. Rules inherited from AGENTS.md that still bind

- Layer dependencies point one way only. `gc-sim` never depends on `gc-render`;
  no TS package imports from a Rust crate's source.
- Data is content, code is mechanism (§8). New players, teams, formations and
  tactics stay data edits.
- Small, focused commits; conventional prefixes. One change, one concern.
- **Never** add a `Co-Authored-By` or "Generated with" trailer to a commit.
- Never pass `-c user.email` / `-c user.name` to git.
- GitHub operations go through
  `/home/oscar/Coding/galactic-cup/scripts/gh-project` by absolute path, never
  bare `gh`.
