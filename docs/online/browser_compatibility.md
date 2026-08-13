# OMP-0 browser compatibility report

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

Status: **accepted for the current Linux development scope**. Stable Linux
Chrome and Firefox pass the automated flow, pacing, keyboard/input,
persistence, and letterboxing gates. Physical gamepad A/B and Firefox
JavaScript heap remain unverified, so this is not broader browser release
certification. Windows 11 is deferred to issue
[#30](https://github.com/osobytes/goliseo/issues/30). Missing evidence is
not treated as a pass.

## Artifact and durable evidence

- Original full-matrix source: `5f8e76cf46ce85f488be7a3ee8e88105cd43ab19`;
  package SHA-256:
  `c939d74873cb49fe8d587c66af9d7363c15580a3523846ee2ea210921c5aaef5`;
  [raw Linux baseline](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-evidence-5f8e76c).
- Reviewed Chrome audio/geometry probe:
  [source `806f7a3`](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-review-evidence-806f7a3).
- Corrected exact-source Chrome/Firefox audio probes:
  [source `c451727`](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-pr29-final-c451727)
  (supersedes the
  [intermediate `ee56d8a` packet](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-16-pr29-ee56d8a)).
- Persistence remediation:
  [#20 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-20-evidence-d2b175b).
- Letterboxing and pointer remediation:
  [#24 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-24-evidence-5813c53).
- Authoritative Chrome heap remediation:
  [#22 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-22-evidence-dab866b).
- Final Linux pacing/input campaign:
  [#21 evidence](https://github.com/osobytes/goliseo/releases/tag/omp0-issue-21-evidence-d7fc8cf),
  clean source `d7fc8cfcd3ebf6bfc8a4ad6e54ed86c2afb1df75`, package
  `3542846f22b64249bdef454ddbfce07d84c9ccbe620435dc68c2bf557f2f8daa`.

Raw evidence remains release assets rather than committed generated output.
Packets contain browser/driver and OS/GPU metadata, served-file hashes,
capabilities, screenshots, console/service logs, memory samples, and summaries.

## Required environment matrix

| Row | 960×540 | 1280×720 | 1920×1080 | Remaining |
| --- | --- | --- | --- | --- |
| Linux Chrome 150 | Flow/pacing/input pass; 600 s stability and Chrome heap pass | Flow/pacing/input pass | Flow/pacing/input pass | Physical gamepad |
| Linux Firefox 152 | Flow/pacing/input pass; 600 s stability pass | Flow/pacing/input pass | Flow/pacing/input pass | Physical gamepad, JS heap |
| Windows 11 Chrome | Unavailable | Unavailable | Unavailable | Attended hardware campaign |
| Windows 11 Firefox | Unavailable | Unavailable | Unavailable | Attended hardware campaign plus manual JS heap |

Every final Linux row used hardware WebGL, completed Title → Result, passed
`web_report.py --require-flow`, retained a clean page-runtime console and
terminal health, and passed the unchanged update/draw/frame/input thresholds.
Both 600-second rows retained late input/settings and Match focus recovery.
Persistence now flushes and reloads `muted=true` in both browsers, while
storage-unavailable boot remains recoverable. Tall/wide canvas geometry and
real pointer hit-testing now pass in both browsers.

The final exact-head pacing campaign's worst complete samples had at most one
frame over 33 ms and none over 250 ms. Whole-row input p95/max remained below
100 ms in all six rows. Chrome's corrected one-document, forced-GC heap
measurement changed from the original apparent leak to
`2,639,224 → 2,632,180` bytes (-0.27%), passing the fixed 25% gate.

## Controls and memory interpretation

The runner observes exactly 11 ordered samples over roughly five seconds after
entering Match. A pass requires an unmuted setting before Match, user
activation, no autoplay warning, positive master volume, and at least one
active source. Malformed, missing, non-finite, duplicate, or out-of-order
samples fail the check without aborting evidence capture.

The full matrix, 600-second stability, and Chrome heap claims above are from
Chrome 150 remediation packets, including clean pacing source `d7fc8cf` and
the authoritative #22 heap packet. They are not attributed to the later
focused audio run.

The positive Chrome packet at source `806f7a3` is historical evidence.
Pre-fix source `4b446ceb` left the persistence probe's `muted=true` setting in
place, so its otherwise complete Chrome and Firefox probes correctly observed
zero sources. Corrected source `c451727` persists `muted=false` before the
product flow; its focused zero-stability Chrome 151 and Firefox 152 packets
each pass with all 11 samples positive, volume 1, user activation, and no
autoplay warning. Those focused packets prove audio only; they do not replace
the Chrome 150 full-matrix, stability, or heap provenance.

No physical standard-mapped controller was available for the Linux packets.
The attended operator must expose `mapping="standard"` and produce both
`gamepad_a` and `gamepad_b`; the ASRock LED controller at `/dev/input/js0` is
not evidence.

Firefox's recorded process-tree RSS is not JavaScript heap.
`performance.memory` is non-standard and Chromium-only, so the required
Firefox t0/t5/t10 heap companion remains manual. The concise procedure and
Mozilla sources are in [`browser_build.md`](browser_build.md).

## Firefox WebGL shader translator workaround (#391)

Firefox's WebGL shader translator emits invalid GLSL for any shader whose
`varying` declarations sit below `main()` in the source. LÖVE stitches every
shader as its own boilerplate, its own `main()`, a `#line 1`, and then the
user's code, so **every** LÖVE shader that declares a `varying` lands in that
shape. The translator injects the WebGL-mandated output-variable
initialisation at the top of `main()` but leaves declarations in source order,
so it writes to `webgl_<hash>` ten lines above its own
`out vec3 webgl_<hash>`. The driver then rejects the output -- NVIDIA with
`error C1503: undefined variable`, Mesa with `undeclared`. The defect is in
what Firefox emits, not in what any driver does with it, and it reproduces in
four lines of plain WebGL with no LÖVE and no wasm.

Upstream is [Bugzilla 2039887](https://bugzilla.mozilla.org/show_bug.cgi?id=2039887),
filed 2026-05-15 by the Castle Game Engine developer with the same diagnosis,
still broken in Nightly 153.0b13 after Mozilla's ANGLE update (bug 1908744).
Castle Game Engine shipped an engine-level hoisting workaround rather than
wait; `scripts/browser_shader_hoist.js` is the same workaround shape, applied
at the WebGL boundary because LÖVE's stitcher owns the placement. It is
embedded in the bootstrap by `scripts/web_build.py` and needs no love.js
rebuild.

A second, unrelated defect was masked by the first, and that one is ours: a
uniform declared outside `#ifdef VERTEX` / `#ifdef PIXEL` compiles into both
stages, where LÖVE's generated headers give it different default float
precision. GLSL ES 1.00 requires cross-stage uniform precision to match;
Firefox enforces it at link time (`Uniform 'u_palette' is not linkable between
attached shaders`) and Chrome does not. `game/render/rig3d/renderer.lua`
declares its vertex-only uniforms inside the vertex stage for that reason, and
`spec/render/rig3d_spec.lua` pins it.

`scripts/check_shader_hoist.sh` gates the transform and the build wiring in
both `scripts/check.sh` and CI. It starts no browser: a green run there is not
browser evidence. The browser evidence is the headed run below.

### Measured

**Reproducing this needed two branches when it was written. It no longer does.**
`--gl-probe` and `game/render/gl_probe.lua` were not on `main` and were not in
the change that carries this section: they arrive with
[#390](https://github.com/osobytes/goliseo/pull/390)
(`agent/2026-08-05-issue-360-lovejs-depth`), whose merge base with the #391 fix
branch was plain `main`, so a checkout of either branch alone could not run what
is below. That constraint is now historical: the fix is on `main`, #390 is
merged up to it, and one ordinary checkout of #390 runs the whole matrix — which
is how #360 re-measured it, at `b11b0e4`, `source_dirty: false`, finding every
ladder rung and the rig3d shader `ok=true` in **both** browsers. See
[`browser_rigged_3d.md`](browser_rigged_3d.md).

The tree that produced the numbers *in this section* is tagged
[`issue-391-verification-81c0904`](https://github.com/osobytes/goliseo/tree/issue-391-verification-81c0904)
-- an evidence-only merge, not a branch to build on:

| ingredient | revision |
| --- | --- |
| the `--gl-probe` harness and `game/render/gl_probe.lua` | `ddfd86c73fb146191dea4cfe586e4e3da05c42f4` (#390 tip) |
| the #391 fix (loader hoist + uniform stage placement) | `f42a588c34d021ab3c1edadaaf0cfa1f1213abaa` |
| three-line relocation of the ladder rungs' own uniforms, which belongs to #390 and is posted there | `81c0904682dc30649a2576fe36b40dd624c5afc5` |
| merged evidence tree | `81c0904682dc30649a2576fe36b40dd624c5afc5`, `source_dirty: false` |

Only the middle row is in this change.

The fix branch did not stop at `f42a588`, so the difference is stated rather
than glossed. Everything after it is this document, plus two review fixes that
cannot move any of the numbers above: a refusal to hoist a conditionally-guarded
declaration when no `#line` directive exists below `main()` to absorb the line
shift -- a path LÖVE cannot reach, because its stitcher always emits `#line 1`,
so every shader measured here takes the same branch either way -- and a
tightening of `check_shader_hoist.sh`, which is a gate and ships in no
artifact.

Artifacts, all built by `scripts/web_build.py` from that tree against the pinned
`2dengine/love.js@495c5eb7eb55b54aaadfc21405c58f50a6d819c4` runtime:

| artifact | `goliseo.love` SHA-256 | difference |
| --- | --- | --- |
| both fixes | `d9ce67ac9109640558e6e634a3f1162c3e68aa360a8d82c27528811fef46c2ce` | -- |
| hoist disabled | `d9ce67ac…` (same package) | one line of the built `player.js`: `installed: install()` replaced by `installed: []` |
| uniforms outside the stage blocks | `e1c574a27a6a50a962e830f953e97e05ed57446d18f6713b4d58108fee22ad3f` | `renderer.lua` and `gl_probe.lua` reverted to `ddfd86c` |

The hoist-disabled control shares its package with the fixed run on purpose: the
Lua is byte-identical and only the loader differs, so nothing but the hoist can
account for the result.

The console marker lines each row is read from are quoted in
[PR #395](https://github.com/osobytes/goliseo/pull/395). The raw per-page JSON
records are not committed, per the same policy as every other row in this file:
generated evidence stays out of the tree.

Headed Chrome 151.0.7922.76 and Firefox 153.0.1 on an RTX 2070 SUPER.

| run | baseline rung | rig3d rung | `bloom.hasDepth` | `player_renderer_3d.available` |
| --- | --- | --- | --- | --- |
| Firefox, hoist disabled | fails | fails | not reached | not reached |
| Firefox, hoist + uniform placement | **ok** | **ok** | **true** (depth16) | **true** |
| Chrome, hoist disabled | ok | ok | true (depth16) | true |
| Chrome, hoist + uniform placement | ok | ok | true (depth16) | true |
| native LÖVE 11.5, either uniform placement | ok | ok | true (depth24stencil8) | true |

Chrome and native LÖVE are unchanged by both fixes, which is the property that
makes them safe to apply unconditionally rather than behind a browser sniff.

Every rung of the `--gl-probe shader` ladder passes in Firefox with both fixes
applied -- `baseline`, `one_custom_attribute`, `four_custom_attributes`,
`uniform_array_constant_index`, `uniform_array_dynamic_index`,
`bone_array_dynamic_index` and `rig3d`, all `ok=true`. Two runs isolate the two
causes: with the hoist disabled, `baseline` already fails to **compile**
(`error C1503: undefined variable "webgl_d03fcbf3b75bcd9f"`); with the hoist
enabled but the uniforms back outside the stage blocks,
`uniform_array_constant_index` compiles and then fails to **link**
(``Uniform `u_palette` is not linkable between attached shaders``).

The display was a private `Xvfb :77`, never the desktop's `:1`, with NVIDIA
PRIME render offload (`__NV_PRIME_RENDER_OFFLOAD=1`,
`__GLX_VENDOR_LIBRARY_NAME=nvidia`,
`__EGL_VENDOR_LIBRARY_FILENAMES=.../10_nvidia.json`, `MOZ_X11_EGL=1`). Without
that env Firefox on Xvfb silently renders on Mesa llvmpipe, and the run proves
nothing about a real driver -- so the GPU was confirmed two ways that do not
depend on `native_shell_bench.gpu_verdict`, whose false-positive path is
[#392](https://github.com/osobytes/goliseo/issues/392): the WebGL vendor string
(`NVIDIA Corporation` on the offloaded run, `Mesa` / `llvmpipe, or similar`
without it) and `nvidia-smi`'s own process list, which reports the Firefox
process holding 128 MiB of GPU memory as a graphics client.

## Deferred validation

- Issue [#30](https://github.com/osobytes/goliseo/issues/30) retains the
  serialized Windows 11 Chrome/Firefox campaign, physical controller, audible
  playback, and Firefox heap requirements for a later support expansion.
- Linux physical standard-gamepad coverage and Firefox t0/t5/t10 heap evidence
  are still required before making a broader public browser-support claim.
- Issue [#31](https://github.com/osobytes/goliseo/issues/31) tracks a
  self-contained Linux download separately from browser certification.

Issue [#16](https://github.com/osobytes/goliseo/issues/16) completed the
repository-owned evidence tooling and is closed. The owner-accepted delivery
policy and its narrower current support scope are recorded in
[`platform_decision.md`](platform_decision.md).
