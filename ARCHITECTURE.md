# GOLISEO — Architecture

GOLISEO is a browser multiplayer sports game. The simulation is Rust, compiled
to WebAssembly; the renderer is TypeScript, built on three.js. There is no
native build — the browser is the only target, and nothing in this tree reads
or depends on anything outside it at runtime.

---

## 1. The layer split, and why

**The Rust/TypeScript line is the determinism line, not the logic/display
line.**

Anything that can change simulation state, or that must produce
byte-identical results on every client, is Rust. Anything that only *reads*
simulation state is TypeScript.

This is not a stylistic preference. ECMAScript specifies `sin`, `cos`, `tan`,
`exp`, `log` and `acos` as *implementation-approximated* — two browsers may
return different bits for the same input. A rollback netcode desyncs on one
bit. wasm float semantics are spec-pinned and libm compiles *into* the
module, so every runtime executes identical instructions. That is the whole
reason the simulation is Rust and can never be TypeScript.

The test for any file: **can this code change what the simulation computes?**
If yes it is Rust, even when it feels like presentation.

**Rust owns:**

| Crate | Contents |
| --- | --- |
| `gc-core` | `rng`, `deterministic_math`, `fnv1a64`, `vec2` — the numeric primitives the simulation depends on |
| `gc-data` | Content tables: players, teams, formations, tactics, traits, arenas |
| `gc-sim` | The simulation itself |
| `gc-render` | The `RenderFrame` producer |
| `gc-netcode` | Wire encoding, resim scheduling, the replicated session state machine |
| `gc-wasm` | The `wasm-bindgen` bridge that exposes `gc-sim`/`gc-netcode` to the browser |

**TypeScript owns:**

| Package | Contents |
| --- | --- |
| `@gc/core` | Presentation math: `mat4`, `quat`, and a duplicate `vec2` (see §1.2) |
| `@gc/ui` | Layout and hit-testing |
| `@gc/input` | Input capture and bindings |
| `@gc/presentation` | Cosmetics, themes, branding |
| `@gc/render` | The three.js renderer |
| `@gc/screens` | Screens |
| `@gc/transport` | WebRTC / WebSocket |
| `@gc/online` | Lobby, signalling, diagnostics |
| `@gc/wasm` | Loads the built wasm module and types its exports |
| `@gc/app` | App shell |

`gc-core` holds exactly four modules because the simulation depends on
exactly four numeric primitives: `rng`, `deterministic_math`, `fnv1a64`, and
— the one that is easy to miss — `vec2`, used across more than a dozen
simulation modules including `match`, `match_snapshot`, `keeper`, `combat`
and `slot_input`.

`vec2` consequently exists on **both** sides: Rust for the simulation,
TypeScript for the renderer. That duplication is deliberate. A 54-line
immutable value type is cheaper to maintain twice than to marshal across the
wasm boundary, and its `length()` uses `sqrt`, which IEEE 754 specifies as
correctly rounded and is therefore safe on the determinism path where the
transcendentals are not.

`mat4` and `quat` are presentation-only — neither the simulation crate nor
the `RenderFrame` producer requires them — so they stay TypeScript alone.

### 1.1 Where the line falls inside the online stack

This was the one genuinely contested boundary, so it was decided by
evidence: which modules touch input-frame encoding, match-snapshot state, or
rollback bookkeeping, and which of them produce state that two peers must
agree on bit for bit.

**Rust (`gc-netcode`).** Wire encoding, resim scheduling, and the replicated
session state machine: `coordinator`, `match_driver`, `protocol`,
`fault_harness`, `input_protocol`, `fault_scenarios`, `fault_transport`,
`coordinator_driver`, `desync_package`, `match_manifest`, `match_session`,
and their conformance/fixture modules. `fake_relay` and `fake_star` are a
separate case — not shipped session logic, but real in-process transports
that exist purely so `gc-netcode`'s own test suite has something concrete to
drive the fault harness against.

`coordinator` is the surprising one — it reads like lobby code. It is Rust
because it records per-tick hashes, decides `hash_mismatch` / `late_input` /
`desync` outcomes, and freezes the session at an agreed `first_input_tick`.
Both peers run it over the same event stream and must land in the same
state; a divergence there is a protocol violation, not a cosmetic
difference. It is a pure `update(state, event) -> state, actions` reducer.

**TypeScript (`@gc/online`).** Observability and coordination that no peer
has to agree with: `net_diagnostics`, `diagnostic_transport`, `lobby_link`,
`match_presentation`, `fault_campaign`, and `net_diagnostics_fixture`.

Two boundaries worth restating because they are easy to get backwards:

- **Input capture is TypeScript; input *encoding* is Rust.** Reading a
  gamepad is a browser concern. Turning it into an `InputFrame` is what goes
  on the wire and into the resim, so it must be bit-identical.
- **Correction smoothing is TypeScript, and strictly one-directional.**
  Visually easing a rollback correction is presentation. The moment it feeds
  back into simulation state, non-determinism has entered the loop.

### 1.2 Modules that must exist in both languages

Four so far, for two different reasons.

**`vec2`** — a 54-line immutable value type. Cheaper to keep twice than to
marshal across the wasm boundary. Divergence is implausible and harmless.

**`fnv1a64` / `diagnostics_schema`** — these are *not* in that category, and
the distinction matters. `diagnostics_schema` is a canonical serializer with
a versioned content digest (`DIGEST = "fnv1a64/v1"`), whose own header states
that "delimiter-joined or table-order hashing is ambiguous and is
forbidden". It is consumed by `net_diagnostics` (TypeScript) *and* by
`desync_package` (Rust), and a desync package is evidence peers exchange. So
a digest computed in Rust on one client and in TypeScript on another **must
agree**.

Duplicating a hash function across two languages is exactly how that stops
being true. Therefore: both implementations are pinned by a shared vector
file — a list of inputs and their expected hex digests, checked into
`tools/lua_reference/` and asserted by a test in each language (see that
directory's README). A duplicate without shared vectors is not acceptable
here.

**`rng` / `network_conditions`' impairment half** — the same category as
`fnv1a64`, for the same reason. `gc-sim`'s `network_conditions` impairs the
native rollback matrix; `packages/transport/src/impairment.ts` impairs
browser evidence, and `packages/transport/src/impairment_rng.ts` ports
`gc-core`'s minstd generator so the two consume identical rolls from
identical seeds. Evidence gathered under one is compared against evidence
gathered under the other, so they **must agree** — and a disagreement throws
nothing anywhere, it just makes two green suites mean different things.
Therefore: both implementations are pinned by a shared transcript literal
asserted by a test in each language, and by gate 0c
(`scripts/check_network_profile_parity.mjs`), which requires the two literals
to be byte-identical. Only the impairment half is duplicated; the redundant
input history, the authoritative ledger and `drain` stay Rust-only.

---

## 2. Directory layout

```
rust/
  Cargo.toml               workspace
  crates/gc-core            rng, deterministic math, fnv1a64, vec2
  crates/gc-data             content tables
  crates/gc-sim               the simulation
  crates/gc-render             RenderFrame producer
  crates/gc-netcode              deterministic online
  crates/gc-wasm                  wasm-bindgen bridge to the browser
  crates/gc-test-alloc              dev-only allocation-budget test harness
ts/
  pnpm-workspace.yaml
  packages/core             presentation math
  packages/ui                 layout + hit-testing
  packages/input                input capture
  packages/presentation           cosmetics, themes, branding
  packages/render                   three.js renderer
  packages/screens                    screens
  packages/transport                    WebRTC / WebSocket
  packages/online                         lobby, signalling, diagnostics
  packages/wasm                             loads/types the built wasm module
  packages/app                                app shell
```

---

## 3. House rules — Rust

1. **Numeric fields default to `f64`.** Reach for an integer type only where
   the value is unambiguously an index, a count, or a bit pattern — getting
   this wrong changes results.

2. **Never reassociate or "simplify" floating-point arithmetic.** `a * b +
   a * c` is not `a * (b + c)` in IEEE 754, and the determinism evidence in
   `tools/lua_reference/` is only valid if operation order matches exactly
   what was captured. Any change to accumulation order in a loop on the
   determinism path invalidates the pinned vectors and requires
   re-justifying against them.

3. **Indices inside a serialized payload, a hash input, or a wire format
   keep their defined value**, even where it disagrees with ordinary
   zero-based indexing elsewhere in the codebase — player/roster references
   on the wire are one-based, for instance. The protocol is not up for
   redesign to make indexing uniform.

4. **Never `HashMap` or `HashSet`.** Iteration order is not part of their
   contract and a desync is the failure mode. Use `Vec`, `IndexMap`, or
   `BTreeMap`. Clippy denies the hash types in this workspace.

5. **Errors, two mechanisms, kept distinct** (AGENTS.md §7):
   - `assert!(cond, "msg")` — programmer error, invariant violation.
   - `Result<T, String>` — expected, recoverable failure. Never collapse
     this into a panic.

6. **Everything a test touches is `pub`.** Crates are internal; do not fight
   visibility.

7. **Differential-test anything on the determinism path.** A passing unit
   test proves the code satisfies the assertions someone wrote down; it does
   not prove two clients produce identical bits. Compare against the pinned
   reference vectors in `tools/lua_reference/README.md`.

8. **`#![deny(missing_docs)]` is on.** Every public item needs a doc
   comment.

---

## 4. House rules — TypeScript

1. **TypeScript, `strict`, `noUncheckedIndexedAccess`,
   `exactOptionalPropertyTypes`.** No `any`. No `!` non-null assertions —
   narrow properly. `unknown` plus a type guard where input is genuinely
   untyped.

   The `any` half of this rule is now **enforced** rather than merely
   written down: `@typescript-eslint/no-explicit-any` runs at error severity
   in gate 7b (#471). The non-null-assertion half is still prose only —
   `no-non-null-assertion` reports 94 existing uses, far too many to fold
   into the change that introduced the gate, so it is deliberately deferred
   rather than silently dropped.

2. **Relative imports carry the `.ts` extension** (`./foo.ts`).
   `rewriteRelativeImportExtensions` is on. Cross-package imports use the
   package name (`@gc/core`).

3. **Wire-format indices**: the same rule as Rust house rule 3, and the same
   exception.

4. **Recoverable failures use the shared discriminated union from
   `@gc/core`** (`result.ts`):
   ```ts
   export type Result<T, E = string> =
     | { readonly ok: true; readonly value: T }
     | { readonly ok: false; readonly error: E };
   ```
   A plain `T | null` is only correct where a function has no error case to
   report at all.

5. **Screens keep the model/view seam** (AGENTS.md §9). `layout(state,
   viewport)` and `update(state, event)` stay **pure** — no three.js, no
   DOM, no globals — so they test headless. `draw` is the only impure
   function, and it is the only place three.js appears.

6. **TypeScript never imports content tables — it receives them.** `data/`
   is Rust-owned, because most of it feeds the simulation. But presentation
   code legitimately needs the cosmetic half (`loadouts`,
   `equipment_presentations`, `character_presentations`,
   `cosmetic_variants`, `arenas`, themes).

   Do **not** solve that by duplicating the tables into TypeScript — that
   creates two sources of truth for content, and AGENTS.md §8 exists to stop
   exactly that. Instead, take the tables as an explicit parameter:

   ```ts
   export function model(state: CombatState, data: CombatPresentationData) { ... }
   ```

   This keeps the package free of any dependency on Rust sources, makes the
   wasm data boundary explicit rather than hidden in imports, and is more
   testable — a spec can pass two different content tables instead of
   mutating a shared global and restoring it.

   The single shared content schema, generating types for both languages, is
   a future project. Parameter injection is the interim answer, and it is a
   deliberate one, not a placeholder.

   **One carve-out, and it is gated rather than trusted (#447).**
   `packages/render/src/rig3d/presentation_content.ts` *does* restate two
   `gc-data` mappings — `character_presentations`' theme column and
   `loadouts`' equipment column — because the renderer resolves them per
   player inside `characterMesh`, which is reached from `pitch.draw` and has
   no parameter to inject a content table through without threading one down
   every render call. The prohibition above still stands everywhere else,
   and it stands here in substance: the duplicate is not trusted, it is
   **asserted** against the Rust source by
   `scripts/check_presentation_parity.mjs`, run as gate 0b of
   `./scripts/check.sh` — see §6.1. That is the same answer #433 reached for
   the wire enums, which are duplicated for the same reason and guarded the
   same way. A second carve-out must be argued on the same terms — a
   cross-language assertion, wired into the gate, landing with the
   duplicate — or not taken.

   **A second carve-out, taken on exactly those terms (#472).**
   `packages/transport/src/network_profiles.ts` restates `gc-data`'s four
   authored network profiles, because the impairment decorator must run in a
   plain vitest process with no wasm module loaded, and nothing in `gc-wasm`
   exports the profile table today. The duplicate is not trusted either: it
   is asserted against the Rust source by
   `scripts/check_network_profile_parity.mjs`, run as gate 0c — and that gate
   goes further than the other two, because a drifted profile value throws
   nowhere at all. It also pins the impairment generator's constants and the
   shared five-scenario impairment transcript both languages assert. Reading
   the table through the wasm bridge instead, which would make the duplicate
   impossible rather than merely detectable, remains the better long-term
   answer.

7. **Prefer `readonly` and immutable updates in pure code.** Use in-place
   mutation only where a module already does so deliberately (a
   performance-sensitive hot path, an established call-site convention) —
   don't introduce new mutation.

---

## 5. The renderer: what three.js absorbs, and what stays hand-written

`@gc/render`'s `rig3d/` describes and animates this game's characters. Most
of the underlying machinery is three.js:

- **Geometry construction** — `BufferGeometry`/`BufferAttribute`, not a
  hand-rolled triangle builder.
- **Rendering and materials** — `WebGLRenderer`, `MeshStandardMaterial`.
- **Bloom** — `EffectComposer` + `UnrealBloomPass`, GPU-profiled and built
  in, rather than a hand-rolled threshold-extract-and-blur pass pair.
- **Capability detection** — `WebGLRenderer.capabilities` and
  `getContext()`, read synchronously at construction, rather than a
  hand-rolled survey-and-probe module.

**Separate the mechanism from the content.** A module that *implements*
skinning is engine machinery three.js already ships — `skeleton.ts`'s
skinning math is `THREE.Skeleton`/`THREE.Bone`. A module that *describes
this game's characters* happens to be expressed in terms of that machinery,
but it is content, not mechanism: `body.ts`, `equipment.ts`, `headgear.ts`,
`face.ts`, `clips.ts`, `themes.ts`, `action_pose.ts`, `masks.ts`,
`proportions.ts`, `palette.ts`, and the specific bone hierarchy inside
`skeleton.ts` itself. None of that is something three.js could provide —
it's the game.

Two capabilities three.js does not provide at all, so the renderer must
supply them itself:

- **Two-bone IK.** `CCDIKSolver` is example-tier, CCD-based, and
  MMD-oriented — not the constraint this game's ground-contact and reach
  poses need. This one is not built yet: `rig3d/action_pose.ts` states
  plainly that there is no IK anywhere in the rig's source, poses are
  transforms of the rig root bone rather than limb solves, and planting one
  foot while the other lifts is tracked as an open two-bone leg-solve gap
  (#318).
- **Bone masking.** `AnimationMixer`'s `PropertyMixer.accumulate` normalizes
  by *cumulative* weight, so a full-weight overlay stacked on a full-weight
  base lands at a 50/50 blend, not an override — there is no per-action
  weight that overrides a bone while the base still drives it, other than
  zero. Additive blending (`AnimationUtils.makeClipAdditive`) is the wrong
  *look* for these clips, which are authored as absolute poses rather than
  differences from a base. So masking stays hand-written (`rig3d/masks.ts` +
  `clips.layer`, one `AnimationMixer` per layer), and this was tried and
  reconsidered once already (#425) — the reasoning is recorded there so it
  isn't re-litigated.

---

## 6. Commands

**Run the workspace gate, not just your own targets.** `cargo clippy -p
gc-sim --test my_test` passing is not the same as `cargo clippy --workspace
--all-targets` passing, and two committed lints got through exactly that
way. This is the same trap AGENTS.md §9 names for harnesses: a scoped check
is not the check. Before declaring done, run the full commands below.

```bash
cd rust && cargo test --workspace
cd rust && cargo clippy --workspace --all-targets -- -D warnings
cd rust && cargo fmt --all --check

cd ts && pnpm install
cd ts && pnpm test             # vitest
cd ts && pnpm typecheck        # tsc --build
cd ts && pnpm lint             # eslint, type-aware
cd ts && pnpm format:check     # prettier --check
```

`pnpm lint:fix` and `pnpm format` are the writing halves of the last two.

Use **pnpm**, never npm.

### 6.1 The CI gate

The commands above are useful for running one thing by hand, but the actual
enforced gate — the one wired into `scripts/check.sh` and
`.github/workflows/ci.yml`'s `gate` job, so the two cannot drift — is:

```bash
./scripts/check.sh              # run every gate
./scripts/check.sh --self-test  # prove the gate can go red
```

It is stricter than the commands above in ways that matter:

- it checks **cross-language wire-enum parity** before anything is built
  (`node scripts/check_wire_enum_parity.mjs`, gate 0). Every closed set on
  the `RenderFrame` boundary is defined twice — a Rust enum with a `*_code`
  numbering in `crates/gc-render/src/frame_buffer.rs`, a TypeScript union
  with a `*FromCode` numbering in `packages/render/src/frame_buffer.ts` —
  and each side is compiler-checked only against itself. A variant added on
  one side only first surfaces as `frame_buffer: unknown pose id code 33`
  thrown in a player's browser, mid-match; a reordering that preserves
  membership while shifting codes surfaces as nothing at all, since `team`,
  `species shape` and `event kind` decode through `requireDecode`, where a
  shifted code is a different *valid* value rather than an error. The gate
  compares membership **and** numeric codes for all eleven enums, and
  refuses to pass if it finds a `*_code`/`*FromCode` pair its registry does
  not cover (#433).
- it checks **cross-language presentation-content parity**, also before
  anything is built (`node scripts/check_presentation_parity.mjs`, gate 0b).
  §4 rule 6 above forbids a TypeScript package importing a Rust crate's
  content tables, so `packages/render/src/rig3d/presentation_content.ts`
  restates by hand which theme each `character_presentations` entry belongs
  to and which equipment each `loadouts` entry carries. That is the same
  each-side-checked-only-against-itself shape the wire enums had. A renamed
  presentation throws `no theme for presentation id` in a player's browser
  the first time that player is drawn; a loadout repointed at different
  equipment throws **nothing at all** and simply renders the wrong item
  forever. The gate compares all three tables key-for-key and
  value-for-value, checks that every equipment id it resolves has a
  `rig3d/equipment.ts` builder and a `rig3d/body.ts` socket, and compares
  the duplicated `ROSTER_STRING_FIELD_COUNT` — which, unlike
  `LAYOUT_VERSION`, is not stamped into the wire, so nothing else can catch
  it drifting (#447).
- it checks **cross-language network-impairment parity**, beside the other
  two (`node scripts/check_network_profile_parity.mjs`, gate 0c). `gc-data`
  authors four network profiles — `clean`, `omp0_parity`, `playable`,
  `stress` — and the native rollback matrix drives every scenario through
  `gc-sim`'s `network_conditions` under them. Browser evidence now drives the
  same profiles through `packages/transport/src/impairment.ts`. If the two
  impair traffic differently, **nothing throws in either language**: the
  browser suite and the native suite go on measuring different networks while
  both stay green, so a soak that ran a "stress" link at a tenth of the
  authored loss rate reports a clean hour and proves nothing. The gate
  compares every profile's seven tuning fields, the impairment generator's
  `MOD`/`MULT` constants, and the five-scenario impairment transcript that
  `rust/crates/gc-sim/tests/browser_impairment_parity.rs` and
  `ts/packages/transport/src/impairment_parity.spec.ts` each assert — byte
  for byte, so a drift is caught even when only one language's tests run. It
  additionally requires that transcript to still record a loss, a burst, a
  duplicate and a reordering: two identical literals prove nothing if both
  sides quietly became a pass-through (#472).
- `cargo clippy -p gc-wasm --target wasm32-unknown-unknown -- -D warnings`
  runs as an **explicit, separate** step from the workspace clippy run. The
  native workspace run never compiles `gc-wasm`'s wasm-only code paths at
  all, so a lint that only exists under `#[cfg(target_arch = "wasm32")]`, or
  inside wasm-bindgen's own codegen for that target, is invisible to it.
- it **lints and format-checks TypeScript** (`pnpm exec prettier --check .`,
  gate 5b; `pnpm exec eslint . --max-warnings 0`, gate 7b). Between #467 —
  which deleted Lua, and with it `stylua --check` and
  `lua-language-server --check` — and #471 there was no lint or formatting
  gate for TypeScript at all, of any kind, while TypeScript became roughly
  half the codebase. `tsc` is strict here but sets neither `noUnusedLocals`
  nor `noUnusedParameters`, and no type-checker catches a **floating
  promise**; in `packages/render/src/rig3d/**` an unawaited promise is the
  shape of defect that reaches a frame. So the lint is **type-aware**, which
  has two consequences worth knowing: it runs after the wasm build and the
  typecheck (without `@gc/wasm`'s generated `.d.ts` on disk, everything
  downstream of it resolves to an error type and the rules quietly find
  less), and neither gate may be built out of an exit code alone — eslint
  exits 0 over an empty file set, and prettier exits 0 *and prints its
  success line* when every file it was handed was ignored. Both gates
  therefore also require a floor on the number of files they really covered,
  and gate 7b additionally asserts through `eslint --print-config` that
  `no-floating-promises`, `no-explicit-any` and `no-unused-vars` are still at
  error severity for a real `rig3d/` source file.
- `pnpm exec tsc --build --force`, never plain `--build`. An incremental
  build reuses `.tsbuildinfo` and can report clean over source that changed
  but whose mtime was not newer than the recorded build (the normal outcome
  of a `git checkout` or a container layer copy) — that shape passed for
  several successive changes before a forced build caught what an
  incremental one had been silently missing.
- it **builds the wasm artifact itself**
  (`node ts/packages/wasm/scripts/build.mjs`) before testing, rather than
  trusting whatever happens to already be on disk. `dist/pkg/` is
  gitignored, so a Rust fix that was never folded into a rebuilt artifact is
  a fix nothing downstream can see.
- it asserts, twice and independently, that the freshly built module's
  `runDeterminismEvidence()` returns exactly `final_hash=bfbb106aea5480f8` /
  `sequence_digest=0bfd0ed355f87322` — once through
  `packages/wasm/src/determinism.spec.ts`'s own vitest assertions, and again
  by loading the same module directly (bypassing vitest) and comparing in
  plain bash. This is one of the most important assertions in the
  repository: it is what proves the wasm build did not perturb float
  behaviour.
- it separately builds the **browser** wasm target
  (`node ts/packages/wasm/scripts/build_web.mjs`), runs `pnpm exec vite
  build`, and byte-compares the wasm asset that lands in `dist-app/assets`
  against the freshly built `dist/pkg-web/gc_wasm_bg.wasm`. The Node-target
  artifact the steps above exercise and the browser-target artifact the
  browser actually loads are two separate `wasm-bindgen` outputs from the
  same cargo build. On 2026-08-07 this gate passed while the browser
  artifact was thirteen hours stale, so every match in the browser ran the
  simulation from before a real fix had landed — a defect in the shipped
  path invisible to every other step, because each of them looked at the
  other artifact. This is what makes "the gate is green" mean "the thing
  that ships is the thing that was tested."

Toolchain pins this gate enforces: the Rust channel and components in
`rust/rust-toolchain.toml`; `wasm-bindgen-cli` exactly `0.2.118` (matching
`crates/gc-wasm/Cargo.toml`'s `wasm-bindgen = "=0.2.118"`, because the CLI
checks its generated glue against the crate's schema version exactly, not
semver); Node >= 22; pnpm exactly `11.1.2` (`ts/package.json`'s
`"packageManager"`).

One toolchain oddity, because it will otherwise look like a mistake: the
workspace builds with `typescript@7`, whose npm package deliberately ships
**no JavaScript compiler API** (its whole entry point exports `version` and
`versionMajorMinor`). typescript-eslint's type-aware rules need that API, and
its peer range says so (`>=4.8.4 <6.1.0`). So `ts/tools/lint/` is a tiny
workspace package that exists only to carry its own `typescript@6.0.3` — the
last JS-API release, and the same language as 7.0 — which pnpm's isolated
`node_modules` hands to typescript-eslint without the root ever seeing it.
`ts/eslint.config.mjs` reaches it through `ts/tools/lint/tseslint.mjs`, which
documents the whole arrangement and says when to delete it.

---

## 7. Rules inherited from AGENTS.md that still bind

- Layer dependencies point one way only. `gc-sim` never depends on
  `gc-render`; no TypeScript package imports from a Rust crate's source.
- Data is content, code is mechanism (§8). New players, teams, formations
  and tactics stay data edits.
- Small, focused commits; conventional prefixes. One change, one concern.
- **Never** add a `Co-Authored-By` or "Generated with" trailer to a commit.
- Never pass `-c user.email` / `-c user.name` to git.
- GitHub operations go through
  `/home/oscar/Coding/galactic-cup/scripts/gh-project` by absolute path,
  never bare `gh`.
