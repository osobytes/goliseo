# Browser artifact

Galactic Cup's OMP-0 browser proof uses the `2dengine/love.js` LÖVE 11.5
runtime. The runtime is fetched during the build rather than vendored into the
repository or committed as generated output. The generated `player.js` is a
small project-owned loader that fetches the package and runtime assets directly;
the upstream IndexedDB-backed loader is retained as
`third_party/lovejs-player.js` for provenance but is not the boot path.
The project loader keeps the runtime's IDBFS mount at LÖVE's save root. It
waits for the runtime's populate synchronization before the game starts and
serializes a flush after each writable save-file close, so the existing
`love.filesystem` settings path persists without a browser-specific Lua path.

IndexedDB failure is recoverable. The loader records
`window.__GALACTIC_CUP__.storage.state = "unavailable"` and emits a
`GC_BROWSER|storage_error` warning with `recoverable=true`, then continues on
the mounted in-memory filesystem. The issue #16 browser runner also loads
`?storage=unavailable` as a deterministic failure probe and requires that page
to reach Title and accept an in-memory settings change.

## Build and serve

From a clean checkout with Python 3 and network access:

```sh
./scripts/web_build.sh
./scripts/web_serve.sh build/web 8000
```

Open <http://127.0.0.1:8000/> in a desktop browser. The server adds the
cross-origin isolation and WebAssembly headers required by the runtime:

- `Cross-Origin-Opener-Policy: same-origin`
- `Cross-Origin-Embedder-Policy: require-corp`
- `Content-Security-Policy: script-src 'self' 'unsafe-eval';`

The CSP is intentionally narrow to same-origin scripts but includes
`unsafe-eval` because the selected upstream WASM player requires it. Do not
copy these headers into a public deployment without reviewing the security
policy for that deployment.

The generated host keeps the 960×540 logical canvas at 16:9. Non-16:9
windowed viewports center the largest fitting canvas rectangle over black
bars; the runtime maps pointer input through the resulting scale and offset.

## Packaging smoke check

The non-interactive smoke check exercises save, populate/reload, and
storage-unavailable host semantics; builds the artifact twice; compares the
deterministic `.love` packages; checks the required runtime files; validates
the ZIP entries; verifies the pinned runtime manifest; and self-tests the
expected 800×540 tall, 1280×540 wide, and required 16:9 canvas geometry:

```sh
./scripts/web_smoke.sh
```

CI can run this command without opening a browser. A normal browser should be
used for the title-screen and complete-flow checks; those compatibility and
performance checks remain part of issue #3.

## Evidence campaign

The evidence runner needs Python 3.11 or newer. Its complete 15-wheel Selenium
4.43.0 closure is version- and hash-locked in
`scripts/browser_matrix-requirements.txt`; install it with
`pip --require-hashes`. The Windows entrypoint does this in a disposable
environment and gives Selenium Manager a campaign-local cache. Each packet
records versions, paths, sizes, and SHA-256 hashes for Python, Selenium
Manager, the stable-channel browser, and its driver.

On an attended Windows 11 host, install current stable Chrome and Firefox, then
run from PowerShell:

```powershell
.\scripts\windows_browser_campaign.ps1
```

The entrypoint refuses a reused output root, creates an evidence-only virtual
environment, builds and serves the artifact, and runs one fresh-profile
session for every browser/viewport pair. The 960×540 session is the
600-second row; the other two sessions exercise the complete flow and controls.
After the Firefox heap companion, it stops the server with a bounded cleanup
and archives each packet. Raw packets, ZIPs, server logs, operator
observations, and `campaign-summary.json` are under
`.cache\omp0-windows-campaign\<timestamp>\` by default and remain ignored.
Each packet's `environment.json` includes a bounded
`Win32_VideoController` inventory, so Intel and AMD adapters are retained
alongside any `nvidia-smi` result. Missing Windows GPU metadata makes the
campaign incomplete.

Keep the desktop unlocked and foregrounded with hardware acceleration enabled,
Windows scaling at 100%, and enough physical resolution for an exact
1920×1080 browser viewport (2560×1440 or larger is recommended). Enable audible
playback and connect one physical controller exposed by the browser with
`mapping="standard"`; listen during Match and press physical A then B in every
browser/viewport session. After each session, the operator must answer
`yes`, `no`, or `unsure` for audible output and physical A/B. Only `yes` passes;
the answers are written inside the raw packet before its ZIP is hashed. Do not
substitute a virtual HID or infer a pass from a connected device without both
input events. No Actions workflow is provided: the fixed attended desktop,
GPU/audio, resolution, and physical-input requirements are not available on
an eligible repository runner.

### Firefox heap companion

Firefox process-tree RSS remains useful supplemental process memory, but it is
not JavaScript heap. `performance.memory` is a non-standard Chromium-only API.
The Firefox 960×540 runner therefore keeps the exact artifact server alive,
launches a second clean WebDriver-controlled Firefox profile with the same
Selenium Manager-resolved browser and driver, verifies the inner viewport
before and after every checkpoint, and guides an attended companion:

1. Let the runner reach stable Result, then open that tab's Firefox DevTools
   **Memory** panel in an undocked window.
2. At t0, t5, and t10, use `about:memory` **Minimize memory usage** to force
   GC/CC and save `about-memory-<label>.json.gz`. Immediately take and save the
   tab snapshot as `heap-<label>.fxsnapshot`.
3. After t10, paste the packet's generated `firefox-heap-extractor.js` into
   that isolated profile's Browser Console. Firefox itself reopens each saved
   snapshot and runs a root `{ by: "count" }` heap census. The runner accepts
   `report.bytes` only when each generated census JSON matches the wrapper's
   independent snapshot SHA-256 and size. If Firefox shows its first-paste
   protection prompt, follow that prompt in this disposable profile, then
   paste the unchanged generated file.

The embedded snapshot creation times must put t5/t10 within 15 seconds of
300/600 seconds after t0. Six non-empty raw files (three `about:memory` reports
and three snapshots), the three generated census outputs, the exact extractor
source and hash, session metadata, and `firefox-heap-summary.json` are
required. The summary computes `(t10 - t0) / t0 * 100`, reports whether
t0 ≤ t5 ≤ t10, applies the unchanged 25% threshold, and records the actual
companion session/profile, Firefox `appBuildID`/`platformBuildID` and
capabilities, browser/driver hashes, source and package revisions, exact
viewport/Result checks, and authoritative snapshot timing. The Firefox long
row and campaign cannot pass unless this companion passes.

Mozilla documents
[`about:memory` GC/CC and reports](https://firefox-source-docs.mozilla.org/performance/memory/about_colon_memory.html),
the [Firefox Memory tool](https://firefox-source-docs.mozilla.org/devtools-user/memory/index.html),
and [snapshot save/diff operations](https://firefox-source-docs.mozilla.org/performance/memory/basic_operations.html).
Mozilla's
[`HeapSnapshot` interface](https://searchfox.org/firefox-main/source/dom/chrome-webidl/HeapSnapshot.webidl)
defines offline snapshot loading, embedded creation time, and census
collection; the
[`Debugger.Memory` census reference](https://searchfox.org/firefox-main/source/js/src/doc/Debugger/Debugger.Memory.md)
defines the root byte count used here.
MDN documents
[`performance.memory` as non-standard and Chromium-only](https://developer.mozilla.org/en-US/docs/Web/API/Performance/memory).
The Browser Console extractor is enabled only in the disposable campaign
profile and reads only the named local snapshots. Geckodriver's
[`--allow-system-access`](https://firefox-source-docs.mozilla.org/testing/geckodriver/Flags.html)
grants browser-UI-process privileges and full system access, so it remains
disabled.

## Reproducibility and provenance

The build packages only the authored runtime inputs needed by LÖVE:
`conf.lua`, `main.lua`, and the `core/`, `data/`, `game/`, and `sim/` trees.
Specs, documentation, local tooling, and generated files are not placed in the
game package.

The generated `build/` directory is ignored by Git. Every artifact contains a
`manifest.json` with the game-package hash, source revision, runtime revision,
and hashes for the generated files. `goliseo.love` uses normalized ZIP
timestamps and sorted entries so identical source inputs produce identical
authored package bytes.

Runtime source and license:

- Repository: <https://github.com/2dengine/love.js>
- Pinned commit: `495c5eb7eb55b54aaadfc21405c58f50a6d819c4`
- Download archive SHA-256:
  `89b56e7953935d6cb06c454d0ee0c0d8903e433b9a94d1d6d501fb8b516f5ff6`
- Runtime licenses: zlib, permissive, and LGPL terms recorded in the complete
  upstream `license.txt`, copied into `third_party/lovejs.LICENSE.txt` in the
  generated artifact

See [`THIRD_PARTY.md`](../../THIRD_PARTY.md) for the repository audit and the
release obligations that apply to separately licensed runtime files.

The upstream player documents its LÖVE 11.5 support, direct `.love` loading,
browser limitations, and required server headers in its own README. The
browser artifact is a spike output, not a public release package yet.
