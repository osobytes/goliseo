# Third-party software

GOLISEO's repository-owned code, documentation, and assets are licensed
under the PolyForm Noncommercial License 1.0.0 unless a file says otherwise.
Third-party software is not relicensed under PolyForm and retains its own
license terms and notices.

## Distributed browser runtime

The browser artifact combines a separately licensed runtime with
`galactic-cup.love`, the authored game package:

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

As of 2026-07-24:

- Every literal Lua `require` in tracked source resolves to a module in this
  repository.
- The repository has no dependency manifest, Git submodule, vendored library,
  or tracked third-party binary.
- No third-party image, font, model, audio, or video asset is tracked.

Any new dependency or asset must record its author, source, exact version or
retrieval date, license, required notices, and distribution obligations here
before it can be included in a release.
