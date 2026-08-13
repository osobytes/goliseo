# Rigged 3D under LÖVE in a browser (#360)

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

**Verdict: yes in Chrome and yes in Firefox — and neither answer is the one the
project had recorded.**

#328's issue body states that "love.js is WebGL 1 and cannot supply a depth
attachment at all". #100's browser leg was abandoned on that sentence, PR #350
deliberately skipped back-to-front part sorting because of it, and §6.2 of
`docs/design/render_migration_decision.md` concludes from it that the browser leg
cannot be delivered — while recording that the constraint "has not been
re-measured". It has now.

The sentence is **false**. love.js supplies a depth attachment, LÖVE's rig3d
shader links, and ten rigged players render in a browser at 33 draw calls, in
both supported browsers.

## Superseded: this document said "no in Firefox" first, and meant it

The first measurement, at `ddfd86c`, found that **no LÖVE shader declaring a
`varying` compiled in Firefox at all**, that this took the whole runtime down
rather than falling back, and therefore that entering a match in a browser build
crashed there. That was real, it was reproduced on hardware, and it was filed as
#391. It is left standing below wherever it explains why something is the way it
is, marked as superseded rather than deleted.

**#391 is fixed (PR #395) and the Firefox column has been re-measured on a tree
that has both halves.** Two root causes, only one of them a browser bug:

1. Firefox's WebGL shader translator injects the WebGL-mandated
   "initialise every output variable" assignment at the top of `main()` while
   leaving declarations in source order. LÖVE always stitches user code *below*
   its own `main()`, so a user `varying` is declared after its injected use and
   the emitted GLSL is invalid. Upstream is
   [Bugzilla 2039887](https://bugzilla.mozilla.org/show_bug.cgi?id=2039887),
   open. `scripts/browser_shader_hoist.js` hoists those declarations above
   `main()` at the `shaderSource` boundary, in every browser, with no love.js
   rebuild.
2. **Ours:** vertex-only uniforms were declared outside the `#ifdef VERTEX` /
   `#ifdef PIXEL` blocks, so they compiled into both stages, where LÖVE's
   generated GLSL ES headers give them different default float precision. GLSL ES
   1.00 requires cross-stage uniforms to match; Firefox enforces it at *link*,
   Chrome and desktop GL do not. Fixed in `rig3d/renderer.lua` by #395, and in
   this document's own instrument — `gl_probe.SHADER_LADDER` — on this branch,
   where three rungs had the same defect and would otherwise have failed at link
   for a reason that has nothing to do with the construct they exist to isolate.

**One tree, both halves.** #395's verification needed `--gl-probe` from this
branch *and* the fix from that one, and no single checkout had both — a reviewer
flagged that, and it was answered there with an evidence-only merge tag. It no
longer applies: `main` now carries the fix, this branch is merged up to it, and
everything below was run from one ordinary checkout of this branch.

## How to reproduce

```sh
DISPLAY=:77 python3 -B scripts/lovejs_depth_probe.py --output .bench/lovejs-depth
```

The runner builds its own artifact under `--output`. Building one by hand into
`build/` also works but currently makes `./scripts/check.sh` fail: the #343
wasm-embed-manifest gate rejects any unclassified top-level directory, including
git-ignored build output.

Headed, on a machine with a real GPU. The runner refuses to publish a result it
cannot prove ran on hardware, and `--prove-refusal` starts a real Chrome forced
onto SwiftShader to demonstrate that refusal firing:

```
$ DISPLAY=:77 python3 -B scripts/lovejs_depth_probe.py --prove-refusal \
    --artifact .bench/lovejs-depth/web --output .bench/refusal
REFUSED as required: prove-refusal: refusing to publish a software-rasteriser
result (mapped drivers: libglx_nvidia, libnvidia-glcore, libvulkan_lvp,
swiftshader). #100 already published one false negative from exactly this.
```

Note what that driver list shows: the refusal fires on a Chrome that has the
NVIDIA drivers mapped *and* SwiftShader, which is the case a "did it touch the
GPU?" check would wave through.

**Not on `:1`.** `:1` is the machine owner's desktop; an earlier run of this
matrix fought them for the screen and they closed browser windows mid-probe,
which is where this document's "unattributable session deaths" section comes
from. Runs go on a private display instead — a `Xvfb :77` with NVIDIA PRIME
render offload (`__NV_PRIME_RENDER_OFFLOAD=1`,
`__GLX_VENDOR_LIBRARY_NAME=nvidia`,
`__EGL_VENDOR_LIBRARY_FILENAMES=…/10_nvidia.json`, `MOZ_X11_EGL=1`). Xvfb is a
software X server with no DRI of its own; the offload is what puts the *client*
on the NVIDIA GPU, which is why the combination works at all.

The controller's own logic
— marker decoding, the terminator rule, every clause of the verdict, the GPU
refusal, the partial-matrix refusal — is gated in `scripts/check.sh` and CI under
a step that says it starts no browser, because it does not.

The instrument is `love . --gl-probe` (see `game/render/gl_probe.lua`), which
runs natively too:

```sh
love . --gl-probe                             # capability survey
love . --gl-probe canvas depth24stencil8 false  # exactly one canvas
love . --gl-probe shader rig3d                  # exactly one shader
```

## What the runtime is

| | Chrome | Firefox |
| --- | --- | --- |
| WebGL version obtained | WebGL 1.0 (OpenGL ES 2.0 Chromium) | WebGL 1.0 |
| WebGL 2 context | no | no |
| shading language | WebGL GLSL ES 1.0 | WebGL GLSL ES 1.0 |
| context attributes | `depth: true`, `stencil: true` | `depth: true`, `stencil: true` |
| `MAX_VERTEX_UNIFORM_VECTORS` | 1024 | 1024 |
| `MAX_VERTEX_ATTRIBS` | 16 | 16 |
| unmasked renderer | `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL 4.5.0)` | `NVIDIA GeForce GTX 980, or similar` |

### How the GPU was proven, and why not from the verdict

`native_shell_bench.gpu_verdict` says `hardware` for both browsers, and that is
**not** what the hardware claim here rests on. #392 records its false-positive
path, and #395 showed a sharper version of the same trap: without the PRIME
offload environment, Firefox on this display holds `/dev/nvidia0`,
`/dev/nvidiactl`, `/dev/nvidia-modeset` and `/dev/dri/renderD128` open **and
still renders on llvmpipe**. An open device node proves nothing.

Firefox also cannot be taken at its word. It answers `WEBGL_debug_renderer_info`
with "NVIDIA GeForce GTX 980, or similar" on a machine whose card is an RTX 2070
SUPER — specific enough that the #341 classifier read it as a positive
identification, and wrong. That string is demoted in
`scripts/native_shell_bench.py` alongside WebKit's "Apple GPU", and the report
records it as `engine_string_verdict: spoofed`.

So two independent signals decide instead, the same pair #395 used:

1. **The WebGL vendor string flips with the offload environment, on the same
   display, same browser, same page.**

   | | unmasked vendor | unmasked renderer | in `nvidia-smi` |
   | --- | --- | --- | --- |
   | Firefox, no offload env | **`Mesa`** | `llvmpipe, or similar` | **absent** |
   | Firefox, offload env | **`NVIDIA Corporation`** | `NVIDIA GeForce GTX 980, or similar` | present, 138 MiB |
   | Chrome, offload env | `Google Inc. (NVIDIA Corporation)` | `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL 4.5.0)` | present, 4 processes |

   The same flip is visible outside the browser: `glxinfo -B` on `:77` reports
   `Mesa` / `llvmpipe (LLVM 20.1.2, 256 bits)` / `Accelerated: no` without the
   offload env, and `NVIDIA Corporation` with it. Xvfb is a software X server
   with no DRI of its own, so this is the offload moving the *client* onto the
   GPU, not the display.

2. **`nvidia-smi`'s own graphics-client list names the browser**, sampled while
   the page was live and resolved through `/proc/<pid>/cmdline` before the
   process exited — driver-side evidence no renderer string can spoof:

   ```
   |  0  N/A  N/A  1725105  G  ...ar/.local/opt/firefox/firefox  138MiB |
   cmd[1725105]=/home/oscar/.local/opt/firefox/firefox --marionette … -profile /tmp/rust_mozprofile8ca3vR
   ```

   The `--marionette` / `rust_mozprofile` command line is what distinguishes the
   browser this harness started from the desktop's own, which is also on the
   card. Without the offload env the same page produces an **empty** client list
   while Firefox still holds `/dev/nvidia0`, `/dev/nvidiactl`,
   `/dev/nvidia-modeset` and `/dev/dri/renderD128` open — which is #392's
   false-positive path, reproduced.

`love.graphics.getSupported()` agrees between the two browsers: `instancing`,
`lighten`, `pixelshaderhighp` and `shaderderivatives` true; `clampzero`,
`fullnpot`, `glsl3` and `multicanvasformats` false. So do the system limits:
`canvasmsaa` 0, `multicanvas` 1, `texturelayers` 1, `texturesize` 32768.

## The depth question, answered

WebGL 1 has packed depth-stencil in **core** — `renderbufferStorage(RENDERBUFFER,
DEPTH_STENCIL, …)` — rather than as an extension, and both browsers also expose
`WEBGL_depth_texture`. Building that renderbuffer by hand from JavaScript on the
love.js canvas returns `gl.getError() == 0` and a framebuffer that reports
`FRAMEBUFFER_COMPLETE`. The platform was never the constraint.

What refuses is LÖVE. `love.graphics.getCanvasFormats()` under love.js reports,
identically in both browsers:

| format | love.js | native (for comparison) |
| --- | --- | --- |
| `depth24stencil8` | **false** | true |
| `depth32fstencil8` | false | true |
| `depth32f` | false | true |
| `depth24` | **false** | true |
| `depth16` | **true** | true |
| `stencil8` | true | true |

Creating them one per process agrees with the table exactly, in both browsers:
`depth16` non-readable and `stencil8` succeed; every other format comes back
from `love.graphics.newCanvas` as nil, having printed "The … canvas format is
not supported by your graphics drivers." So love.js can supply a depth
attachment; it just cannot supply the *packed 24-bit* one `bloom` was hardcoded
to ask for.
That is a LÖVE-side gate — its GLES2 backend keys packed and 24-bit depth off
extensions WebGL 1 does not advertise, precisely because WebGL 1 has them in
core — not a WebGL 1 limit.

`game/render/bloom.lua` now asks `getCanvasFormats()` first and walks
`depth24stencil8 → depth24 → depth16`. Native still takes the first rung, so
nothing about the desktop build changed; love.js takes the third and gets its
depth buffer.

### Asking for an unsupported format was not survivable

The old code wrapped the creation in a `pcall` on the theory that a runtime
which cannot supply the attachment degrades gracefully. In **both** browsers it
did not. Rebuilding base `main` (`d61e441`) and running
`love . --benchmark 300 120 rigged` against it printed, in Chrome *and* in
Firefox,

```
The depth24stencil8 canvas format is not supported by your graphics drivers.
```

as its last console line and then raised love.js's own
`alert('An error occurred before the game window could be initialised. Please
check the console!')`. Neither browser reached `GC_BENCH_GATE`; both alerted. The
same command against the post-fix build completes and reaches `GC_BENCH_GATE` in
both browsers, on both renderers. Since
`game/screens/match.lua` composes every match frame through `bloom.draw`, that
was a live crash on entering a match in the browser build **on every browser**,
not a theoretical one and not a Chrome-only one — this fix is worth more than an
earlier draft of this document credited it with.

The narrow probe mode exists because of this: `--gl-probe canvas FORMAT
READABLE` creates exactly one canvas per process, so a format that takes the
runtime down cannot swallow the answers for the formats after it, and a missing
terminator marker is recorded as "asking killed the process" rather than as an
ordinary refusal.

## The rig3d shader question, answered

`game/render/rig3d/renderer.lua` predicts that its shader links against the
128-vector floor GLSL ES 1.00 guarantees: 26 bones × 3 rows = 78, plus 12
palette, plus 12 matrix = 102. The probe recomputes that arithmetic rather than
quoting it and gets 102, and both browsers report 1024 vectors available, so the
budget was never close on this hardware. **The shader links in both browsers**,
`budget_used=102` against `budget_floor=128`. The prediction holds.

**In Firefox it now links too, at every rung.** Post-#395, on this branch, with
the ladder's own uniforms moved into the vertex stage:

| rung | what it adds | Chrome | Firefox |
| --- | --- | --- | --- |
| `no_varying` | LÖVE entry points, no user `varying` — bloom's own shape | ok | ok |
| `baseline` | the same shader plus one `varying vec3` | ok | ok |
| `one_custom_attribute` | one custom vertex attribute | ok | ok |
| `four_custom_attributes` | all four of rig3d's, same names and types | ok | ok |
| `uniform_array_constant_index` | a 12-element uniform array, constant index | ok | ok |
| `uniform_array_dynamic_index` | the same array, attribute-derived index | ok | ok |
| `bone_array_dynamic_index` | a 78-element array at three dynamic indices | ok | ok |
| `rig3d` | the real shader | ok | ok |

### Superseded: what the first measurement found here

At `ddfd86c` this section read **"in Firefox a LÖVE shader that declares a
`varying` does not compile"**, with `no_varying` passing and every rung from
`baseline` up failing on

```
Cannot compile vertex shader code:
0(21) : error C1503: undefined variable "webgl_d03fcbf3b75bcd9f"
```

That was correct, and the boundary the ladder located — varying-declaring
shaders fail, others work — was the right boundary. The reading offered for it
was hedged as "an inference, not a measurement", because Firefox exposes no
`WEBGL_debug_shaders` and the translated source had not been read.

**#395 read it**, out of a browser that does expose it, and the inference was
half right: the declarations are not *dropped*, they are left *below* the
initialisation Firefox injects at the top of `main()`. Use at line 19,
declaration at line 29, in Firefox's own emitted GLSL. Hoisting them fixes it.

The second failure this uncovered was ours and was invisible until the first was
gone: with the hoist active but uniforms declared outside the stage blocks,
Firefox compiles and then refuses to **link** —
``Uniform `u_palette` is not linkable between attached shaders`` — one rung
earlier than rig3d. Three ladder rungs had that defect and are fixed on this
branch; `spec/render/gl_probe_spec.lua` now gates the placement, and
`spec/render/rig3d_spec.lua` gates it on the real shader.

**Bloom is not affected, and the browser build has not been running without it.**
An earlier draft of this document said otherwise, reasoning that bloom's shaders
are "more complex" than the failing rung. Complexity is not the axis — varyings
are, and `bloom.lua`'s `THRESHOLD_SRC` and `BLUR_SRC` declare none. Measured on
the post-fix build in Firefox: a full `--benchmark 300 120 procedural` run
completes, `GC_BENCH_GATE|renderer=procedural|pass=true`, and a complete console
capture contains **zero** "bloom disabled" lines.

### The crash this used to cause, and the part of it that is still true

At `ddfd86c`, anything that reached the rig3d shader in Firefox did not degrade —
it crashed. Reproduced first-hand in headed Firefox 153 with
`--benchmark 300 120 rigged`:

```
game/render/player_renderer_3d.lua:122: in function 'available'
game/render/pitch.lua:386: in function 'draw'
game/render/benchmark.lua:258: in function 'render_fn'
game/render/bloom.lua:237: in function 'draw'
```

followed by love.js's `alert("An error occurred before the game window could be
initialised…")`, and **no** "rigged 3D players disabled" line — the `pcall` in
`build()` never got the chance to report, because the failure escaped it through
a secondary fault inside LÖVE's own `boot.lua` error path.

**The trigger is gone.** The same command on the post-#395 build runs to
completion in Firefox with the rigged renderer active and reaches its gate rather
than an alert. It does not *pass* that gate — see the table below, where its draw
p95 lands over the absolute threshold — but "over budget" and "takes the page
down" are different findings, and this is now the first one.

**The mechanism is not.** Under love.js a shader the runtime will not take is
still not a catchable Lua error, so `player_renderer_3d.available()`'s documented
fallback still does not work there, and `pitch.draw` still calls it
unconditionally. #391 removed the one shader that tripped it, not the class. Any
future rig3d GLSL change that some browser rejects is a crash on entering a match
again, and there is still no in-band guard available, because learning whether a
shader compiles requires compiling it. `love . --gl-probe shader rig3d` is the
out-of-band form — a separate process whose death is the answer — and it is why
that mode exists. The hazard notes in `pitch.lua`, `main.lua` and
`player_renderer_3d.lua` are narrowed to exactly this, and no further.

## Ten rigged players, measured

Same fixture as #100 and the native runs: seed 20260803, 960×540, ten players,
600 measured frames after 300 warm-up, vsync off, on an RTX 2070 SUPER.

Every figure below was recomputed from the run that produced it — `report.json`
from `scripts/lovejs_depth_probe.py` for the browser rows, and a paired
`love . --benchmark 600 300 {rigged,procedural}` for the native rows. Source
revision `b11b0e4`, `source_dirty: false`, love.js runtime
`2dengine/love.js@495c5eb`. `/proc/loadavg` was `1.64 4.29 4.07` → `1.66 3.53
3.92` across the browser matrix and `1.99 1.73 2.73` → `2.07 1.77 2.72` across
the native pair. The box is shared with other agent sessions; passes were
interleaved rather than waiting for quiet, per #329 and #350.

| runtime | renderer | draw p50 | draw p95 | draw max | draw calls (mean / max) | update p95 | frame p95 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| native LÖVE 11.5 | rigged | 1.1610 | 2.7159 | 5.0587 | 33.2 / 35 | 0.4863 | 18.7094 |
| native LÖVE 11.5 | procedural | 0.8155 | 1.2944 | 2.3295 | 14.0 / 15 | 0.5297 | 14.4023 |
| love.js, Chrome | rigged | 5.0300 | **6.5500** | 9.1150 | **33.2 / 35** | 1.1300 | 12.5000 |
| love.js, Chrome | procedural | 2.5800 | 3.5800 | 5.9300 | 14.0 / 15 | 1.0200 | 9.5950 |
| love.js, Firefox | rigged | 6.6000 | **8.6400** | 12.6200 | **33.2 / 35** | 1.3800 | 15.0400 |
| love.js, Firefox | procedural | 3.4000 | 4.6400 | 6.1000 | 14.0 / 15 | 1.1200 | 11.1200 |

All in milliseconds. **Firefox's rigged row is the one row that does not pass
`benchmark.evaluate`'s absolute omp0 gates**: `draw p95 8.64 ms > 8 ms`. Every
other row passes. `rigged_active` is `true` on both rigged rows and `false` on
both procedural ones, so no row is a procedural run wearing a rigged label.

### Is Firefox's 8.64 ms real, or is it the shared box?

Both, and the honest answer is that the gate result is not stable at this load.
Re-running the same phases on a quiet box, same artifact, same revision:

| browser | renderer | runs | draw p95 | `/proc/loadavg` (1 min) | gate |
| --- | --- | --- | --- | --- | --- |
| Firefox | rigged | 3 | 7.14, 7.00, 6.96 | 0.84 → 0.46 | **passes** all three |
| Firefox | procedural | 2 | 4.00, 4.30 | 0.46 → 1.10 | passes |
| Chrome | rigged | 2 | 6.00, 6.15 | 1.10 → 2.17 | passes |

So Firefox's rigged draw p95 sits at **~7.0 ms against an 8 ms gate** when the
machine is idle and crosses it at moderate contention. The published matrix row
above is left as measured rather than replaced by the quieter number — but a
feature with ~1 ms of headroom on the slower of two browsers is a thin margin,
and that is the finding, not "it fails" and not "it is fine".

Against #100's **feature-delta** budget, which `benchmark.evaluate` does not
check and which has to be subtracted by hand, using the published matrix rows:

| | added draw p95 | of ≤2 ms budget | added update p95 | of ≤1 ms budget |
| --- | --- | --- | --- | --- |
| native | 2.7159 − 1.2944 = **1.4215 ms** | 71% — inside | 0.4863 − 0.5297 = **−0.0434 ms** | inside (below noise) |
| love.js, Chrome | 6.5500 − 3.5800 = **2.9700 ms** | **149% — over** | 1.1300 − 1.0200 = **0.1100 ms** | inside |
| love.js, Firefox | 8.6400 − 4.6400 = **4.0000 ms** | **200% — over** | 1.3800 − 1.1200 = **0.2600 ms** | inside |

So the honest browser reading is: rigged 3D **renders in both browsers** and
costs the same 33.2 draw calls it costs natively — but its added draw cost over
the procedural renderer is 1.5× (Chrome) to 2× (Firefox) #100's feature budget,
against 0.7× natively. That is a number to decide against, not a blocker to
hide, and the thing it is measured against is a procedural renderer the project
is retiring — so whether the delta gate is still the right gate is #100's
question, not this document's.

### Where that browser cost actually is, and where it is not

**Corrected.** An earlier revision of this section attributed the browser's
per-draw overhead to "crossing Lua → JS → WebGL per call". **That is measured
false.** It matters more than a stray sentence normally would, because this
document feeds #330's migrate-or-optimise decision, and a reader who believes
the boundary is the problem reaches for VAOs, instancing, uniform batching or
WebGL 2 — every one of which is a dead end.

The entire WebGL-internal cost of a rigged browser frame is **~0.28 ms of
5.25 ms, about 5%**, established three independent ways:

1. **No-op subtraction** — a shim swallowing WebGL calls *after* the emscripten
   glue, active only inside the measured window. Vertex-stream churn is ~90% of
   that 0.28 ms, uniforms ~0.08 ms, and the draw calls themselves were
   unmeasurably small.
2. **A raw-JS per-call microbench** in the same browser on hardware Vulkan:
   `drawElements` 0.19 µs, VAO bind 0.18 µs, instanced draw 0.10 µs, and the
   *entire* 78-vec4 bone upload 1.87 µs — so all ten characters' bone traffic is
   **19 µs/frame**. Census × unit costs lands at 0.2–0.4 ms/frame, agreeing with
   the subtraction.
3. **A batched rig3d prototype** cutting GL calls **756 → 429 per frame (−43%)**
   and LÖVE draw calls 33 → 24 moved browser draw by **−0.10 to −0.13 ms, about
   2%** — a 43% cut in submission buying 2% of the time.

The cost is the **wasm-hosted Lua interpreter executing draw-side code**. love.js
ships a plain Lua 5.1 interpreter, because LuaJIT cannot JIT under wasm. The tell
is in the phase timers: `pitch.draw` is 4.5–5.3× slower in the browser while
**bloom is *cheaper* in the browser** (0.065 vs 0.100 ms), precisely because
bloom is enqueue-only and barely touches the interpreter.

This is the same finding `docs/design/render_migration_decision.md` §3 already
records for the `update` path — PUC Lua under wasm rather than LuaJIT, **not**
the boundary — arriving a second time through the draw path.

**Bounded by that 0.28 ms, and therefore not worth doing:** VAOs, instancing,
bone textures, uniform batching, WebGL 2 for performance, emscripten GL flags.
**The levers that are real are Lua-side:**

| | lever | expected |
| --- | --- | --- |
| **#393** | bake the static scene — pitch markings, goals, arena — into a canvas or persistent mesh instead of re-deriving it in Lua every frame | 1–2 ms |
| **#394** | the per-character path: pose evaluation, bone-row assembly, and the table churn around them | 0.5–1 ms |

**The floor is honest too.** With both landed, browser draw plausibly reaches
**~2.5–3 ms — still about 2.5× native**. That residual is the
interpreter-in-wasm multiplier, and no graphics-side change addresses it.

### The cross-runtime hash difference is #325, not a new mystery

The final simulation hash agrees perfectly *within* a runtime and differs
*between* runtimes. All four browser runs — both renderers in both browsers —
end at `8c51961a801e136e`; both native renderers end at `15566ea777b5373e`. That
the renderer choice never changes the hash is the property the benchmark exists
to check, and it now holds across two browsers as well as within one, which is a
stronger form of the same result than the first measurement could show.

The runtime-to-runtime difference is the known bot-driven divergence tracked by
**#325**. This fixture is bot-driven (`game/render/benchmark.lua` drives it with
`sim.bot`), native LÖVE is LuaJIT and love.js is PUC Lua 5.1, and
`scripts/phase0_sim_host.lua` already records that the bot's state hash differs
between the two VMs — almost certainly `pairs()` iteration order, which the Lua
spec leaves unspecified. The bot is deliberately not part of the determinism
contract, which is why the OMP-1 browser determinism gate is unaffected and
stays green. Nothing new is being reported here; the numbers are just consistent
with #325.

## What this does and does not settle

- **Settled:** love.js supplies a depth attachment. LÖVE renders depth-tested
  rigged 3D characters in **both** supported browsers. The rig3d shader's GLSL ES
  1.00 budget prediction is correct.
- **Settled:** the browser leg of #100 is deliverable, on both browsers, and
  neither the reason it was thought undeliverable nor the reason Firefox was
  briefly excluded was the reason on record.
- **Settled:** *why* a `varying` is what Firefox chokes on. This document listed
  it as "not settled" and offered "the translator drops the declarations" as an
  inference. #395 read Firefox's emitted GLSL out of a browser exposing
  `WEBGL_debug_shaders`: the declaration is not dropped, it is left below the
  initialisation Firefox injects at the top of `main()`. Upstream is Bugzilla
  2039887, still open; we hoist rather than wait.

### The default is on, and it now runs everywhere

`pitch.rigged_players` is `true`. That is a decision, not an oversight: rigged 3D
everywhere is the project's direction, retiring the procedural 2.5D look is the
point of the current work, and the numbers above say it fits.

**Superseded:** this section used to add that a browser build produced from this
tree crashed on entering a match in Firefox, and that release gating — not
code — held the two apart. That was true at `ddfd86c` and is no longer. The
browser-build hold is lifted. `--procedural-players` is back to being the
comparison switch that #100's before/after evidence needs.

What has *not* changed is the guard problem, and it is worth keeping written
down so nobody spends an afternoon on it: you cannot ask "does the rig3d shader
compile?" from inside the process without compiling it, and under love.js a
compile the runtime refuses takes the process with it. The
`getCanvasFormats()`-first trick that fixed the depth canvas has no equivalent,
because LÖVE exposes no "would this shader compile" query. Any guard has to be
out-of-band — which is what `love . --gl-probe shader rig3d` is, and why it runs
one rung per process.
- **Not settled:** anything about a love.js rebuild. It was the fallback plan for
  a depth failure that did not happen and for a Firefox shader defect that turned
  out to be fixable at the WebGL boundary, so it has still never been attempted.
- **Not settled:** minimum spec. The extension lists and limits captured in the
  report are the raw material for that, and it remains out of scope.
- **Not settled:** whether `player_renderer_3d.available()`'s fallback can be
  made real under love.js. It cannot today; #361 owns deciding what to do about
  that, with the out-of-band-guard constraint above as an input.
- **Not settled, and now attributed rather than open:** the cross-runtime hash
  difference is #325's known bot-driven divergence, not a finding of this issue.

## A side effect worth knowing about

`player_renderer_3d.available()` returns false whenever the current render
target has no depth attachment. That half of it does work — it is the *shader*
half that does not survive love.js, per "the crash this used to cause" above.
It also means the tier-4 pose gates
(`scripts/check_keeper_pose_snapshots.sh`,
`check_outfield_pose_snapshots.sh`) can never exercise the rigged renderer: they
call `pitch.draw` into a plain colour canvas created directly by
`spec/support/*_pose_snapshots.lua`, never through `bloom.draw`, so
`bloom.hasDepth()` is false and the procedural path is the only one reachable.
Turning the default on — which this branch does — did not change a single pixel
of their baselines, and that is the demonstration, not the assertion, that those
gates are procedural-only. Closing that hole is #340, and
**#352 does not close it**: #352 gates on an explicit
`player_renderer_3d.required` flag, which covers "nothing tests rig3d" but not
"the gates cannot see the shipped default".

Related, and unmet: #361's "never a silent fallback" criterion. Where a real
fallback happens today the only signal is a `print()` to devtools — nothing
in-game, nothing a player or a gate can see. That belongs with the flip.

## Session deaths: two fingerprints, one of which is not ours

**Historical, and resolved.** The re-measurement above lost no session in either
browser: every phase of the matrix completed and the run landed on
`report.json` rather than the `report_incomplete.json` quarantine. Both causes
below are accounted for — the LÖVE-side one was #391, and the harness gap the
last paragraph names is now closed by running on a private display. It is kept
because the reasoning about how to tell the two apart is the reusable part.

The Firefox WebDriver sessions in the `ddfd86c` run died repeatedly, and an
earlier draft of this document attributed all of them to LÖVE aborting the
runtime. The raw geckodriver logs show two distinct signatures, and only one
supports that reading:

- **Attributable** — the page loaded, console entries were read back, a real Lua
  crash appears, then `AsyncShutdown`/`ABORT` on quit. Sessions 91–150 s. These
  are the low ladder rungs, and the conclusion drawn from them stands.
- **Unattributable** — no console entry was ever observed, sessions 3.5–82 s,
  `Exiting due to channel error`, no ABORT sequence. These are the high rungs.
  They are indistinguishable from the browser being killed from outside, and
  during that run one was: windows were being closed by hand on the same
  desktop.

No number published from that run is contaminated — its Firefox rigged row was
correctly "did not run", its one published Firefox figure has an attributable
fingerprint, and the `report_incomplete.json` quarantine did its job. (Those rows
are superseded by the table above, which was measured on a private display with
no session deaths at all.) But the *causal* claim for the
high rungs rested on probes that may never have loaded, so it is now stated as
an inference from the low rungs, which are genuine and which the
`no_varying`/`baseline` pair reproduces cleanly on demand.

`scripts/lovejs_depth_probe.py` now records `saw_console_entry`,
`console_reads_ok` and the session duration on every death, and
`death_fingerprint()` turns them into `attributable` / `unattributable` so a
future run does not need the log archaeology. Runs are now isolated on their own
display, per "How to reproduce" above; that the `ddfd86c` run was not is a
harness gap, not a browser finding.
