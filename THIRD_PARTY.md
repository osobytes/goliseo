# Third-party software

GOLISEO's repository-owned code, documentation, and assets are licensed
under the PolyForm Noncommercial License 1.0.0 unless a file says otherwise.
Third-party software is not relicensed under PolyForm and retains its own
license terms and notices.

## Distributed browser runtime

The browser artifact combines a separately licensed runtime with
`goliseo.love`, the authored game package:

- Runtime: [`2dengine/love.js`](https://github.com/2dengine/love.js)
- Pinned commit: `495c5eb7eb55b54aaadfc21405c58f50a6d819c4`
- Source archive SHA-256:
  `89b56e7953935d6cb06c454d0ee0c0d8903e433b9a94d1d6d501fb8b516f5ff6`
- Notices: the complete upstream `license.txt` is copied to
  `third_party/lovejs.LICENSE.txt` in every generated browser artifact

The upstream notice identifies LÖVE under the zlib license, LuaJIT and several
other components under permissive licenses, and runtime libraries under the
LGPL. It also lists a GPL utility that is expressly not included in the
distribution. The project license applies to GOLISEO's game package, not to
those separately licensed runtime files.

Do not remove or replace `third_party/lovejs.LICENSE.txt`. Before publishing a
browser artifact, review the exact pinned runtime and satisfy every applicable
notice, source-availability, and relinking requirement in that file.

## Benchmark-only artifacts (#341)

`scripts/babylon_bench.py` fetches three third-party artifacts on demand,
verifies each against a pinned SHA-256, and stages them under the gitignored
`.bench/`. **None of them is tracked in this repository, and none of them is
part of the authored game package or any shipped artifact.** They exist so the
Babylon skinned-character benchmark in `docs/design/babylon_skinned_benchmark.md`
can be re-run from a clean checkout.

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

If any of these is ever included in a distributed artifact rather than fetched
for a local benchmark, its notices must ship with it and this section must be
updated to say so. Babylon's Apache-2.0 terms in particular require the licence
text and attribution notices to travel with any redistribution; CC0 imposes no
such obligation on the character.

## Measurement-only native shells (#329)

`bench/native_shell/` contains two minimal desktop shells — one Electron, one
Tauri — built solely to measure installer size, cold start and frame cost for
the native-route decision in `docs/design/native_route_decision.md`. **Neither
is part of the authored game package or any shipped artifact, and neither
vendors a third-party binary.** What is tracked is source and lockfiles:

- `bench/native_shell/electron/package.json` and `package-lock.json` pin
  [Electron](https://github.com/electron/electron) 43.3.0 (MIT, © Electron
  contributors) and
  [electron-builder](https://github.com/electron-userland/electron-builder)
  26.15.3 (MIT). Both are `devDependencies`, fetched by `npm install` into the
  gitignored `node_modules/`. Electron redistributes Chromium (BSD-3-Clause and
  a long list of component licences) and Node.js (MIT); an Electron build that
  is ever *distributed* must ship Electron's `LICENSES.chromium.html` and its
  own `LICENSE`, and this section must be updated to say so.
- `bench/native_shell/tauri/Cargo.toml` and `Cargo.lock` pin
  [Tauri](https://github.com/tauri-apps/tauri) 2.x (MIT or Apache-2.0 at the
  user's option, © Tauri Programme within The Commons Conservancy) and its
  dependency tree, fetched by `cargo` into the gitignored `target/`. On Linux a
  Tauri build links the system WebKitGTK (LGPL-2.1 / BSD); a distributed Tauri
  package must satisfy WebKitGTK's relinking obligations, and this section must
  be updated before that happens.
- `@tauri-apps/cli` 2.11.4 is invoked through `npx` at build time and is never
  installed into the tree.
- The three bundler icons under `bench/native_shell/tauri/icons/` are
  **generated**, not vendored: `scripts/native_shell_bench.py` writes them with
  the Python standard library, and they are gitignored. That keeps the audit
  below true.

`bench/babylon_native/spike.js` is repository-owned source. It runs inside a
[Babylon Native](https://github.com/BabylonJS/BabylonNative) `Playground` build
(MIT, © Microsoft Corporation), which this repository does **not** vendor,
build or distribute — the reader builds it themselves per the instructions in
`docs/design/native_route_decision.md`. The character it loads is the CC0
KayKit `Knight.glb` already recorded above.

## Native runtime

LÖVE 11.5 and LuaJIT are development/runtime prerequisites; their source is
not stored in this repository. A future native package that includes LÖVE or
its libraries must ship the corresponding upstream license file and be
audited against the exact binaries in that package before release.

## Development and CI tools

The setup and CI scripts invoke external tools including LÖVE, StyLua,
lua-language-server, Busted, Selenium, Python, Node.js, and pinned GitHub
Actions. These tools are not included in the authored game package. Their
licenses govern the tools themselves, not GOLISEO source files or the ordinary
output produced by running them.

## Repository content audit

As of 2026-08-04:

- Every literal Lua `require` in tracked source resolves to a module in this
  repository.
- The repository has **no Git submodule, no vendored library, and no tracked
  third-party binary**. The #341 benchmark artifacts above are fetched and
  hash-verified at run time into the gitignored `.bench/`, not tracked.
- The repository now tracks **two dependency manifests**, both added by #329 and
  both measurement-only: `bench/native_shell/electron/package.json` (with its
  `package-lock.json`) and `bench/native_shell/tauri/Cargo.toml` (with its
  `Cargo.lock`), described above. Neither is required to build, test or run the
  game; `scripts/check.sh` does not install from either. The game itself still
  has no dependency manifest.
- No third-party image, font, model, audio, or video asset is tracked. The
  bundler icons #329 needs are generated at build time and gitignored.

Any new dependency or asset must record its author, source, exact version or
retrieval date, license, required notices, and distribution obligations here
before it can be included in a release.
