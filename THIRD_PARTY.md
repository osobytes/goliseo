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
- The repository has no dependency manifest, Git submodule, vendored library,
  or tracked third-party binary. The #341 benchmark artifacts above are fetched
  and hash-verified at run time into the gitignored `.bench/`, not tracked.
- No third-party image, font, model, audio, or video asset is tracked.

Any new dependency or asset must record its author, source, exact version or
retrieval date, license, required notices, and distribution obligations here
before it can be included in a release.
