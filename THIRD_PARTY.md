# Third-party software

GOLISEO's repository-owned code, documentation, and assets are licensed
under the PolyForm Noncommercial License 1.0.0 unless a file says otherwise.
Third-party software is not relicensed under PolyForm and retains its own
license terms and notices.

This file is a **live inventory**, not prose. Its present-tense sections
describe what the repository ships and builds with **today**; its historical
sections record obligations that attached to artifacts built from earlier
commits and do not expire because the code was deleted. Both halves are
load-bearing. Everything below was re-derived from the tree on **2026-08-12**
by the commands in [Reproducing this audit](#reproducing-this-audit).

---

## What GOLISEO distributes

There is **one** distributed artifact, and no native build. `pnpm build` in
`ts/` runs Vite and writes `ts/dist-app/` (gitignored, generated per release):

| File | Contents |
| ---- | -------- |
| `index.html` | repository-owned (`ts/index.html`), no third-party markup |
| `assets/index-<hash>.js` | one bundled ES module — see below |
| `assets/gc_wasm_bg-<hash>.wasm` | the Rust simulation — see below |
| `THIRD_PARTY_NOTICES.txt` | generated notices for everything below — see [Bundle notices: resolved 2026-08-15](#bundle-notices-resolved-2026-08-15) |

Nothing else is served, and no third-party file is copied in beside them.

### In the JavaScript bundle

| Component | Version | License | How it gets there |
| --------- | ------- | ------- | ----------------- |
| [three.js](https://github.com/mrdoob/three.js) | 0.180.0 | MIT, © 2010–2025 three.js authors | a `dependencies` entry of `@gc/render` and `@gc/app`; the only non-`@gc/*` runtime dependency in the workspace |
| [`wasm-bindgen`](https://github.com/wasm-bindgen/wasm-bindgen) generated glue | 0.2.118 | MIT OR Apache-2.0 | `wasm-bindgen --target web` emits `packages/wasm/dist/pkg-web/gc_wasm.js`, which the bundle inlines |
| [Vite](https://github.com/vitejs/vite) module-preload polyfill | 8.2.0 | MIT | injected by `vite build` into every bundle it emits |

three.js is genuinely compiled in, not merely declared: the bundle contains its
GLSL chunks and renderer class names. It is the only third-party library in the
JavaScript half of the artifact — no CDN fetch, no vendored copy, no runtime
download.

### In the WebAssembly module

`gc_wasm_bg.wasm` is `rust/crates/gc-wasm` built for `wasm32-unknown-unknown`.
Every crate below is a **normal** (linked) dependency on that target, resolved
by `rust/Cargo.lock`. Licenses were read from the crate sources in the local
cargo registry, not from memory.

| Crate | Version | License |
| ----- | ------- | ------- |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 |
| `itoa` | 1.0.18 | MIT OR Apache-2.0 |
| `memchr` | 2.8.3 | Unlicense OR MIT |
| `zmij` | 1.0.23 | MIT |
| `indexmap` | 2.14.0 | Apache-2.0 OR MIT |
| `equivalent` | 1.0.2 | Apache-2.0 OR MIT |
| `hashbrown` | 0.17.1 | MIT OR Apache-2.0 |
| `wasm-bindgen` | 0.2.118 | MIT OR Apache-2.0 |
| `wasm-bindgen-shared` | 0.2.118 | MIT OR Apache-2.0 |
| `js-sys` | 0.3.95 | MIT OR Apache-2.0 |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 |
| `cfg-if` | 1.0.4 | MIT OR Apache-2.0 |
| `unicode-ident` | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 |

`js-sys` enters only under `cfg(target_arch = "wasm32")`, which is the shipping
target, so it is listed here rather than as tooling. `unicode-ident` carries a
**Unicode-3.0** component alongside its MIT/Apache dual licence — that is a
separate notice with its own terms, not a formality, and it is the one entry in
this table whose obligations are not satisfied by an MIT/Apache notice alone.

The remaining crates in `Cargo.lock` are proc-macro or build-script
dependencies. They run inside the compiler and contribute no code of their own
to the module, so they are inventoried under [tooling](#build-and-test-tooling)
rather than here: `serde_derive`, `wasm-bindgen-macro`,
`wasm-bindgen-macro-support`, `proc-macro2`, `quote`, `syn` (2.0.119 and
3.0.3), `bumpalo`, `rustversion`.

### Bundle notices: resolved 2026-08-15

**This used to be an unmet requirement, recorded here rather than in an issue
tracker because it blocked publication, not development. It is resolved as of
#549.**

`ts/dist-app/assets/index-<hash>.js` contains three.js under the MIT licence,
and the MIT licence requires its copyright and permission notice to travel with
copies and substantial portions of the software. Vite's minifier strips comments
by default, and nothing re-adds them, so comment preservation was never a
viable fix on its own — and the same problem existed for the wasm module's
MIT/Apache-2.0 and Unicode-3.0 crates, which have no JavaScript comments to
preserve in the first place.

The fix is the first option this section used to describe: `pnpm build` now
emits `ts/dist-app/THIRD_PARTY_NOTICES.txt` alongside the bundle, generated by
[`ts/scripts/generate_third_party_notices.ts`](ts/scripts/generate_third_party_notices.ts)
from a `thirdPartyNotices` Vite plugin in `ts/vite.config.ts` (its `closeBundle`
hook). Nothing about the notice list is hand-maintained:

- **three.js** — the script reads the version and full `LICENSE` text straight
  out of `@gc/app`'s own resolved `node_modules/three`.
- **The wasm-linked Rust crates** — the script runs
  `cargo tree -p gc-wasm --target wasm32-unknown-unknown -e no-dev,no-build,no-proc-macro`
  (the same target-scoped, proc-macro-excluding query this file's own
  [Reproducing this audit](#reproducing-this-audit) section uses) and cross-references
  each crate against `cargo metadata` for its license expression and source
  directory, then reads that crate's own `LICENSE-MIT` text. Every dual
  MIT/Apache-2.0 crate is taken under its MIT option, exactly as this section
  used to say — the simpler path, and the one that does not require reproducing
  an upstream `NOTICE` file. `unicode-ident`'s separate Unicode-3.0 component is
  read from its own `LICENSE-UNICODE` and appended as its own notice, not folded
  into the MIT text.
- A build fails loudly (not silently) if `cargo` is unavailable, if a crate
  `cargo tree` reports is missing from `cargo metadata`, or if a crate has no
  `license` field — so this list cannot drift from what is actually linked
  without the build itself saying so.

`ts/index.html` references `./THIRD_PARTY_NOTICES.txt` in a `<head>` comment.
Re-derive the file locally with `node scripts/generate_third_party_notices.ts`
from `ts/`.

### Crate licence metadata: resolved 2026-08-15

**This used to be an open discrepancy. It is resolved as of #549.**

`rust/Cargo.toml` declared `license = "MIT"` in `[workspace.package]`, and all
seven crates inherited it via `license.workspace = true`, while the
repository's `LICENSE` is PolyForm Noncommercial 1.0.0 — the opening paragraph
of this file says repository-owned code is under PolyForm "unless a file says
otherwise", and seven `Cargo.toml` files said otherwise.

The repository owner decided PolyForm Noncommercial 1.0.0 is the intended
licence for these crates too. `[workspace.package]` now declares
`license = "PolyForm-Noncommercial-1.0.0"`, which every crate still inherits
via `license.workspace = true`. `publish = false` stays set workspace-wide, so
this was never going to be validated against crates.io's registry rules; it
was checked instead against what `cargo`, `cargo metadata` and `cargo clippy
--workspace --all-targets -- -D warnings` in `scripts/check.sh`'s gate accept,
and all three take the string without complaint. The TypeScript packages
declare no `license` field at all and so never had this problem.

---

## Build and test tooling

None of the following is part of the distributed artifact. Their licenses
govern the tools themselves, not GOLISEO source files or the ordinary output
produced by running them. They are inventoried because a tool that is *fetched
by a pinned hash* is a supply-chain fact worth keeping accurate, and because two
of them are not permissively licensed.

### Rust toolchain

Rust 1.93 with `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target,
pinned by `rust/rust-toolchain.toml` and installed by `rustup` (Rust is MIT OR
Apache-2.0). `wasm-bindgen-cli` 0.2.118 (MIT OR Apache-2.0) is installed by
`scripts/setup.sh` via `cargo install` and by both workflows from a pinned,
SHA-256-verified GitHub release asset.

Build-time-only crates from `rust/Cargo.lock`, all **MIT OR Apache-2.0**:
`serde_derive` 1.0.229, `wasm-bindgen-macro` 0.2.118,
`wasm-bindgen-macro-support` 0.2.118, `proc-macro2` 1.0.107, `quote` 1.0.47,
`syn` 2.0.119 and 3.0.3, `bumpalo` 3.20.3, `rustversion` 1.0.23. Also
build-time-only, and dev-only in every sense: `gc-test-alloc`, this
repository's own counting allocator.

### Node toolchain

Node.js 22.22.0 (MIT, with its own bundled-dependency notices) and pnpm 11.1.2
(MIT), both installed by `scripts/setup.sh` and both workflows from pinned,
SHA-256-verified release assets. `ts/pnpm-lock.yaml` resolves the workspace's
dev tree: **143 distinct packages**, installed into the gitignored
`ts/node_modules/`.

Direct devDependencies:

| Package | Version | License |
| ------- | ------- | ------- |
| `vite` | 8.2.0 | MIT |
| `vitest` | 4.1.10 | MIT |
| `typescript` | 7.0.2 | Apache-2.0 |
| `typescript` (in `ts/tools/lint`) | 6.0.3 | Apache-2.0 |
| `typescript-eslint` | 8.67.0 | MIT |
| `eslint` | 10.8.1 | MIT |
| `@eslint/js` | 10.0.1 | MIT |
| `prettier` | 3.9.6 | MIT |
| `@types/node` | 26.1.2 | MIT |
| `@types/three` | 0.180.0 | MIT (DefinitelyTyped) |

Across the whole installed tree the licence spread is 105 MIT, 18 Apache-2.0,
7 ISC, 6 BSD-2-Clause, 3 BSD-3-Clause, 1 BlueOak-1.0.0 — and **2 MPL-2.0**:

- `lightningcss` 1.33.0 and its prebuilt native binding
  `lightningcss-linux-x64-gnu` 1.33.0, pulled in transitively by Vite as its CSS
  transformer. **MPL-2.0 is file-level copyleft**, unlike everything else in
  this repository's dependency graph. It is a build tool that emits no code into
  the bundle, and this project does not modify it, so the obligation is dormant
  — but if lightningcss source is ever vendored or patched, the modified files
  must be published under MPL-2.0, and this entry is the reason to notice.

The only prebuilt binaries anywhere in the tree are platform-specific native
bindings that pnpm fetches into `node_modules/`:
`@rolldown/binding-linux-x64-gnu` 1.2.2 (MIT — Vite 8 bundles through Rolldown)
and `lightningcss-linux-x64-gnu` above. Neither is tracked.
`ts/package.json`'s `onlyBuiltDependencies` limits post-install build scripts to
`esbuild` — which no longer appears in the resolved tree at all, Vite 8 having
moved to Rolldown, so that allowance currently permits nothing.

### Browser-evidence Python closure

Tier-5 browser evidence (`scripts/browser_*.py`) runs from a pure-Python wheel
closure pinned **with hashes** in `scripts/browser_matrix-requirements.txt`,
resolved 2026-07-18 and installed with `pip install --require-hashes` into a
throwaway venv. Licenses below were read from each distribution's own `LICENSE`
file and core metadata in a local install, not from a summary service; a few of
those installs are a patch release off the pinned version, which is a difference
in wheel, not in licence:

| Package | Version | License |
| ------- | ------- | ------- |
| `selenium` | 4.43.0 | Apache-2.0 |
| `attrs` | 26.1.0 | MIT |
| `certifi` | 2026.6.17 | **MPL-2.0** |
| `h11` | 0.16.0 | MIT |
| `idna` | 3.18 | BSD-3-Clause |
| `outcome` | 1.3.0.post0 | MIT OR Apache-2.0 |
| `PySocks` | 1.7.1 | BSD |
| `sniffio` | 1.3.1 | MIT OR Apache-2.0 |
| `sortedcontainers` | 2.4.0 | Apache-2.0 |
| `trio` | 0.33.0 | MIT OR Apache-2.0 |
| `trio-websocket` | 0.12.2 | MIT |
| `typing-extensions` | 4.16.0 | PSF-2.0 |
| `urllib3` | 2.7.0 | MIT |
| `websocket-client` | 1.9.0 | Apache-2.0 |
| `wsproto` | 1.3.2 | MIT |

`certifi` is MPL-2.0 and additionally redistributes the Mozilla CA certificate
bundle. Like lightningcss it is a measurement-only tool here, never
distributed. The browsers and drivers these scripts launch — Firefox,
geckodriver, Chrome, chromedriver — are installed outside the repository, are
not vendored, and are not fetched by any tracked script.

### GitHub Actions

Both workflows pin every action to a commit SHA rather than a tag:
`actions/checkout@11bd719` and `actions/upload-artifact@ea165f8`, both MIT,
© GitHub, Inc.

---

## Repository content audit

As of **2026-08-12**, verified against `git ls-files`:

- **No Git submodule** (`.gitmodules` does not exist), **no vendored library**,
  and **no tracked third-party binary**.
- **No tracked third-party image, font, model, audio, or video asset** — in
  fact no tracked asset of those types at all. `git ls-files` matching the usual
  binary and media extensions returns nothing.
- **No `third_party/` directory.** The `third_party/lovejs.LICENSE.txt` this
  file used to require was a build product of the deleted LÖVE artifact
  pipeline; see [Historical records](#historical-records).
- **No Lua in tracked source.** The claim this section used to make about Lua
  `require` resolution is retired, not merely unverifiable: commit `2c0d449`
  (#467) deleted that tree.
- The repository tracks **four dependency manifest sets**, all of them
  first-party and all required to build or test the game:
  `rust/Cargo.toml` with `rust/Cargo.lock`; `ts/package.json` with
  `ts/pnpm-lock.yaml` (plus the ten `ts/packages/*/package.json` and
  `ts/tools/lint/package.json` members they resolve);
  `scripts/browser_matrix-requirements.txt`; and
  `rust/rust-toolchain.toml`. Unlike the pre-port tree, **the game itself now
  has dependency manifests** — that sentence's former negation is one of the
  things this revision exists to correct.
- Generated, gitignored, and therefore not part of this audit's "tracked"
  claims: `ts/dist-app/`, `ts/node_modules/`, `ts/packages/wasm/dist/`,
  `rust/target/`.

Any new dependency or asset must record its author, source, exact version or
retrieval date, license, required notices, and distribution obligations here
before it can be included in a release.

---

## Reproducing this audit

A licence inventory that cannot be re-derived rots silently, which is what
happened between the port on 2026-08-11 and this revision. These are the
commands that produced everything above:

```bash
cargo tree -p gc-wasm --target wasm32-unknown-unknown -e no-dev,no-build
```

Everything that command prints is linked into the shipped wasm; anything in
`rust/Cargo.lock` that it omits is build-time only. Crate licences come from
each crate's own `Cargo.toml` under `~/.cargo/registry/src/`, not from a
summary service. On the Node side, `ts/node_modules/.pnpm/*/node_modules/*/package.json`
carries the resolved version and `license` field for every installed package,
and the distributed set is exactly the `dependencies` (not `devDependencies`)
reachable from `@gc/app`. To confirm what actually reaches a player, grep the
built bundle for library markers — `WebGLRenderer`, `gl_FragColor` — rather
than trusting the manifest.

---

## Historical records

> **Pre-port records (LÖVE/Lua), kept as history.** Everything in this section
> was written against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted
> on **2026-08-11** when the Rust + TypeScript port reached parity. Its file
> paths, artifact names and commands describe that tree and **name nothing you
> can open or run today**. The live tree is `rust/crates/gc-*` and
> `ts/packages/*` — see `ARCHITECTURE.md`.
>
> These records are **not deleted, because licence obligations do not follow
> the source tree.** Any browser artifact built from a commit at or before
> `2c0d449` — including anything already given to a playtester or attached to
> an evidence tag — was assembled under the terms below, and its recipients
> still hold whatever rights and notices those terms conferred. Deleting the
> record would not withdraw the artifact; it would only make the obligation
> unauditable.

### Distributed browser runtime — live 2026-07-24 to 2026-08-11

The browser artifact combined a separately licensed runtime with
`goliseo.love`, the authored game package:

- Runtime: [`2dengine/love.js`](https://github.com/2dengine/love.js)
- Pinned commit: `495c5eb7eb55b54aaadfc21405c58f50a6d819c4`
- Source archive SHA-256:
  `89b56e7953935d6cb06c454d0ee0c0d8903e433b9a94d1d6d501fb8b516f5ff6`
- Notices: the complete upstream `license.txt` was copied to
  `third_party/lovejs.LICENSE.txt` in every generated browser artifact

The upstream notice identifies LÖVE under the zlib license, LuaJIT and several
other components under permissive licenses, and runtime libraries under the
LGPL. It also lists a GPL utility that was expressly not included in the
distribution. The project license applied to GOLISEO's game package, not to
those separately licensed runtime files.

**The LGPL relinking obligation is the one that outlives the deletion.** Anyone
who received an artifact built under this arrangement is entitled to the
notices, source availability and relinking ability that `license.txt`
describes. If such an artifact is still reachable — a hosted build, an
evidence tag, a copy handed to a playtester — the corresponding
`third_party/lovejs.LICENSE.txt` must remain with it. Do not treat the removal
of the build pipeline as satisfaction of the term.

Today's artifact contains **no** love.js, no LÖVE, no LuaJIT and no `.love`
package; see [What GOLISEO distributes](#what-goliseo-distributes).

### Native runtime prerequisites — retired 2026-08-11

LÖVE 11.5 and LuaJIT were development/runtime prerequisites; their source was
never stored in this repository. There is no native build of any kind now — the
browser is the only target, as `README.md` states — so the future-native
audit condition this section used to carry has no present subject. It would
have to be re-derived from scratch, against whatever a native package actually
contained, if native ever returns.

Likewise retired: StyLua, lua-language-server and Busted, which the pre-port
setup and CI scripts invoked. `scripts/setup.sh` and `scripts/check.sh` invoke
none of the four today; see [Build and test tooling](#build-and-test-tooling)
for what they invoke instead.

### Benchmark-only artifacts (#341) — fetched by a script deleted 2026-08-11

`scripts/babylon_bench.py` fetched three third-party artifacts on demand,
verified each against a pinned SHA-256, and staged them under the gitignored
`.bench/`. **None was ever tracked in this repository, and none was part of the
authored game package or any shipped artifact.** They existed so the Babylon
skinned-character benchmark in `docs/design/babylon_skinned_benchmark.md` could
be re-run from a clean checkout — that document survives, with its own pre-port
banner, and points here for licences.

- Babylon.js 9.19.1 — `babylonjs@9.19.1/babylon.js`
  (`d722288208ed611fa2ee6c19848908edfc7e01de1c0f644dc8f9022094405ae0`) and
  `babylonjs-loaders@9.19.1/babylonjs.loaders.min.js`
  (`99d1bd29cca1a97d639829191f544b0d957c2594f00a2cd574fbe56030a327ca`), both
  fetched from jsDelivr. Apache License 2.0, © Microsoft Corporation and
  contributors. Retrieved 2026-08-04.
- KayKit Adventurers Character Pack 1.0, `Knight.glb`
  (`60428e3abc09ba83e595d256e3af8c5c976b46cdae599f0802fc82b4a3445168`), fetched
  from the publisher's own GitHub mirror
  `KayKit-Game-Assets/KayKit-Character-Pack-Adventures-1.0` at commit
  `672074b73ba276876a19e8816ecdc5241817ab47`. **Creative Commons Zero 1.0
  (CC0)**, created and distributed by Kay Lousberg (www.kaylousberg.com), pack
  dated 13/03/2023. The pack's `LICENSE.txt` states it is free for personal,
  educational and commercial use, and that crediting is appreciated but not
  mandatory; the credit above is given voluntarily. Retrieved 2026-08-04.

The fetch script no longer exists, so nothing in the tree can pull these in
today. If either is ever included in a distributed artifact rather than fetched
for a local benchmark, its notices must ship with it and it must be re-entered
in the present-day inventory above. Babylon's Apache-2.0 terms in particular
require the licence text and attribution notices to travel with any
redistribution; CC0 imposes no such obligation on the character.

### Measurement-only native shells (#329) — tree deleted 2026-08-11

`bench/native_shell/` contained two minimal desktop shells — one Electron, one
Tauri — built solely to measure installer size, cold start and frame cost for
the native-route decision in `docs/design/native_route_decision.md`. **Neither
was part of the authored game package or any shipped artifact, and neither
vendored a third-party binary.** What was tracked was source and lockfiles,
all of it deleted with the rest of the pre-port tree:

- `bench/native_shell/electron/package.json` and `package-lock.json` pinned
  [Electron](https://github.com/electron/electron) 43.3.0 (MIT, © Electron
  contributors) and
  [electron-builder](https://github.com/electron-userland/electron-builder)
  26.15.3 (MIT), both as `devDependencies`. Electron redistributes Chromium
  (BSD-3-Clause and a long list of component licences) and Node.js (MIT); a
  *distributed* Electron build would have had to ship Electron's
  `LICENSES.chromium.html` and its own `LICENSE`. None was ever distributed.
- `bench/native_shell/tauri/Cargo.toml` and `Cargo.lock` pinned
  [Tauri](https://github.com/tauri-apps/tauri) 2.x (MIT or Apache-2.0 at the
  user's option, © Tauri Programme within The Commons Conservancy) and its
  dependency tree. On Linux a Tauri build links the system WebKitGTK
  (LGPL-2.1 / BSD); a distributed Tauri package would have had to satisfy
  WebKitGTK's relinking obligations. None was ever distributed.
- `@tauri-apps/cli` 2.11.4 was invoked through `npx` at build time and was
  never installed into the tree.
- The three bundler icons under `bench/native_shell/tauri/icons/` were
  **generated** by `scripts/native_shell_bench.py` using the Python standard
  library, and gitignored — which is why the "no tracked image asset" line in
  the audit above was true then and remains true now.

`bench/babylon_native/spike.js` was repository-owned source. It ran inside a
[Babylon Native](https://github.com/BabylonJS/BabylonNative) `Playground` build
(MIT, © Microsoft Corporation), which this repository never vendored, built or
distributed — the reader built it themselves per the instructions in
`docs/design/native_route_decision.md`. The character it loaded was the CC0
KayKit `Knight.glb` recorded above.
