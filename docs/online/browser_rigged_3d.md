# Rigged 3D under LÖVE in a browser (#360)

**Verdict: yes in Chrome, no in Firefox — and neither answer is the one the
project had recorded.**

#328's issue body states that "love.js is WebGL 1 and cannot supply a depth
attachment at all". #100's browser leg was abandoned on that sentence, PR #350
deliberately skipped back-to-front part sorting because of it, and §6.2 of
`docs/design/render_migration_decision.md` concludes from it that the browser leg
cannot be delivered — while recording that the constraint "has not been
re-measured". It has now.

The sentence is **false**. love.js supplies a depth attachment, LÖVE's rig3d
shader links, and ten rigged players render in a browser at 33 draw calls. What
blocks Firefox is a completely different defect that has nothing to do with
depth: **a LÖVE shader that declares a `varying` does not compile there**, and
asking for one takes the whole runtime down rather than falling back — which is
why rigged players stay opt-in for now (#391).

## How to reproduce

```sh
python3 scripts/web_build.py --output build/web
DISPLAY=:1 python3 -B scripts/lovejs_depth_probe.py --artifact build/web --output .bench/lovejs-depth
```

Headed, on a machine with a real GPU. The runner refuses to publish a result it
cannot prove ran on hardware, and `--prove-refusal` starts a real Chrome forced
onto SwiftShader to demonstrate that refusal firing:

```
$ python3 -B scripts/lovejs_depth_probe.py --prove-refusal --artifact build/web --output .bench/refusal
REFUSED as required: prove-refusal: refusing to publish a software-rasteriser
result (mapped drivers: libvulkan_lvp, swiftshader). #100 already published one
false negative from exactly this.
```

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
| GPU proven by | ANGLE renderer string + mapped `libGLX_nvidia` + open `/dev/nvidia0` | mapped `libGLX_nvidia` + open `/dev/nvidia0` only |

Firefox's GPU had to be proven from the process, not from what it said. It
answers `WEBGL_debug_renderer_info` with "NVIDIA GeForce GTX 980, or similar" on
a machine whose card is an RTX 2070 SUPER — specific enough that the #341
classifier reads it as a positive identification, and wrong. That string is now
demoted in `scripts/native_shell_bench.py` alongside WebKit's "Apple GPU", and
the process map decides instead.

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
same command against the post-fix build completes and passes its gate in Chrome,
and completes in Firefox for the procedural renderer. Since
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
budget was never close on this hardware. **In Chrome the shader links.** The
prediction holds.

**In Firefox a LÖVE shader that declares a `varying` does not compile.** The
ladder in `gl_probe.SHADER_LADDER` is built to straddle exactly that boundary,
and it does:

| rung | what it adds | Firefox |
| --- | --- | --- |
| `no_varying` | LÖVE entry points, no user `varying` — bloom's own shape | **compiles** |
| `baseline` | the same shader plus one `varying vec3` | **fails** |
| everything above, incl. `rig3d` | attributes, uniform arrays, dynamic indexing | fails |

```
Cannot compile vertex shader code:
0(21) : error C1503: undefined variable "webgl_d03fcbf3b75bcd9f"
```

The count of undefined identifiers tracks the number of user-declared
`varying`s — one for each failing ladder rung, four for rig3d, which declares
`v_normal`, `v_world`, `v_slot_color` and `v_material`. The mangled names are
the shader translator's; the `0(21) : error C1503` format is the NVIDIA GLSL
compiler's. The most likely reading is that the translated vertex shader reaches
the driver referencing varyings it never declared — **an inference, not a
measurement**: Firefox does not expose `WEBGL_debug_shaders`, so the translated
source was never read. What is measured is the boundary and the identifier
count. Both reproduce with stock Firefox preferences as well as with the ones
`scripts/babylon_bench.py` uses, so neither is a harness artefact.

**Bloom is not affected, and the browser build has not been running without it.**
An earlier draft of this document said otherwise, reasoning that bloom's shaders
are "more complex" than the failing rung. Complexity is not the axis — varyings
are, and `bloom.lua`'s `THRESHOLD_SRC` and `BLUR_SRC` declare none. Measured on
the post-fix build in Firefox: a full `--benchmark 300 120 procedural` run
completes, `GC_BENCH_GATE|renderer=procedural|pass=true`, and a complete console
capture contains **zero** "bloom disabled" lines.

What *is* affected is anything that reaches the rig3d shader, and it does not
degrade — it crashes. Reproduced first-hand in headed Firefox 153 on a build at
this HEAD, with `--benchmark 300 120 rigged`:

```
game/render/player_renderer_3d.lua:122: in function 'available'
game/render/pitch.lua:386: in function 'draw'
game/render/benchmark.lua:258: in function 'render_fn'
game/render/bloom.lua:237: in function 'draw'
```

followed by love.js's `alert("An error occurred before the game window could be
initialised…")`, and **no** "rigged 3D players disabled" line — the `pcall` in
`build()` never gets the chance to report, because the failure escapes it through
a secondary fault inside LÖVE's own `boot.lua` error path. This is the same shape
as the depth-canvas crash above, arriving a second time through the shader path,
and it is why `pitch.rigged_players` stays `false`: `pitch.draw` calls
`available()` unconditionally, so a default-on flip makes this every player,
every match. Tracked as #391; the flip is #361's.

## Ten rigged players, measured

Same fixture as #100 and the native runs: seed 20260803, 960×540, ten players,
600 measured frames after 300 warm-up, vsync off, on an RTX 2070 SUPER.

Every figure below was recomputed from the run that produced it — `report.json`
from `scripts/lovejs_depth_probe.py` for the browser rows, and a paired
`love . --benchmark 600 300 {rigged,procedural}` for the native rows. Source
revision `3bdd06b`, `source_dirty: false`, love.js runtime
`2dengine/love.js@495c5eb`. `/proc/loadavg` was `1.30 1.34 1.24` across the
browser run and `0.56 … 1.84` across the native pair.

| runtime | renderer | draw p50 | draw p95 | draw max | draw calls (mean / max) | update p95 | frame p95 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| native LÖVE 11.5 | rigged | 0.9273 | 2.0575 | 3.4285 | 33.2 / 35 | 0.3776 | 3.4733 |
| native LÖVE 11.5 | procedural | 0.6307 | 1.0050 | 2.1787 | 14.0 / 15 | 0.4262 | 2.5302 |
| love.js, Chrome | rigged | 4.6700 | **5.7450** | 7.9150 | **33.2 / 35** | 0.9100 | 11.4750 |
| love.js, Chrome | procedural | 2.4950 | 3.3800 | 5.0750 | 14.0 / 15 | 0.9100 | 9.2800 |
| love.js, Firefox | rigged | — did not run (no shader compiles) | | | | | |
| love.js, Firefox | procedural | 3.4000 | 4.4800 | 5.7000 | 14.0 / 15 | 1.1200 | 10.5600 |

All in milliseconds. Every completed run passed `benchmark.evaluate`'s absolute
omp0 gates, including Chrome's rigged run at 5.745 ms against the ≤8 ms draw p95
threshold.

Against #100's **feature-delta** budget, which `benchmark.evaluate` does not
check and which has to be subtracted by hand:

| | added draw p95 | of ≤2 ms budget | added update p95 | of ≤1 ms budget |
| --- | --- | --- | --- | --- |
| native | 2.0575 − 1.0050 = **1.0525 ms** | 53% — inside | 0.3776 − 0.4262 = **−0.0486 ms** | inside (below noise) |
| love.js, Chrome | 5.7450 − 3.3800 = **2.3650 ms** | **118% — over** | 0.9100 − 0.9100 = **0.0000 ms** | inside |

So the honest browser reading is: rigged 3D **renders** in Chrome, holds the
absolute frame gates with headroom, and costs the same 33.2 draw calls it costs
natively — but its added draw cost over the procedural renderer is 2.365 ms,
about 18% over the budget #100 sets for the feature. That is a number to decide
against, not a blocker to hide: the browser's per-draw overhead is roughly 2.4×
native (2.365 vs 1.0525 ms for the same 19.2 extra draw calls), which is what
crossing Lua → JS → WebGL per call costs.

`rigged_active` is `true` for every rigged row that ran, so no row is a
procedural run wearing a rigged label.

### The cross-runtime hash difference is #325, not a new mystery

The final simulation hash agrees perfectly *within* a runtime and differs
*between* runtimes. Both Chrome renderers and Firefox's procedural run all end
at `8c51961a801e136e`; both native renderers end at `15566ea777b5373e`. That the
renderer choice never changes the hash is the property the benchmark exists to
check, and it holds on both runtimes — presentation does not reach the
simulation.

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
  rigged 3D characters in Chrome. The rig3d shader's GLSL ES 1.00 budget
  prediction is correct.
- **Settled:** the browser leg of #100 is deliverable in Chrome and not in
  Firefox, for a reason unrelated to the one on record.
- **Not settled:** anything about a love.js rebuild. Correcting Firefox's
  shader generation would mean rebuilding LÖVE under emscripten, which was not
  attempted here — the depth question it was meant to answer turned out not to
  need it, since the fix is one list in `bloom.lua`.
- **Not settled:** minimum spec. The extension lists and limits captured in the
  report are the raw material for that, and it remains out of scope.
- **Not settled:** *why* a `varying` is what Firefox chokes on. The boundary is
  measured and the identifier count is measured; "the translator drops the
  declarations" is the inference that fits both, and it is not evidence. Firefox
  exposes no `WEBGL_debug_shaders`, so the translated source was never read.
  #391 owns closing that.
- **Not settled, and now attributed rather than open:** the cross-runtime hash
  difference is #325's known bot-driven divergence, not a finding of this issue.

## A side effect worth knowing about

`player_renderer_3d.available()` returns false whenever the current render
target has no depth attachment, which is correct and is what makes the browser
fall back safely. It also means the tier-4 pose gates
(`scripts/check_keeper_pose_snapshots.sh`,
`check_outfield_pose_snapshots.sh`) can never exercise the rigged renderer: they
call `pitch.draw` into a plain colour canvas created directly by
`spec/support/*_pose_snapshots.lua`, never through `bloom.draw`, so
`bloom.hasDepth()` is false and the procedural path is the only one reachable.
Turning the default on — tried on this branch and reverted — did not change a
single pixel of their baselines, and that is the demonstration, not the
assertion, that those gates are procedural-only. Closing that hole is #340, and
**#352 does not close it**: #352 gates on an explicit
`player_renderer_3d.required` flag, which covers "nothing tests rig3d" but not
"the gates cannot see the shipped default".

Related, and unmet: #361's "never a silent fallback" criterion. Where a real
fallback happens today the only signal is a `print()` to devtools — nothing
in-game, nothing a player or a gate can see. That belongs with the flip.

## Session deaths: two fingerprints, one of which is not ours

The Firefox WebDriver sessions in the published run died repeatedly, and an
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

No published number is contaminated — Firefox's rigged row is correctly "did not
run", the one published Firefox figure has an attributable fingerprint, and the
`report_incomplete.json` quarantine did its job. But the *causal* claim for the
high rungs rested on probes that may never have loaded, so it is now stated as
an inference from the low rungs, which are genuine and which the
`no_varying`/`baseline` pair reproduces cleanly on demand.

`scripts/lovejs_depth_probe.py` now records `saw_console_entry`,
`console_reads_ok` and the session duration on every death, and
`death_fingerprint()` turns them into `attributable` / `unattributable` so a
future run does not need the log archaeology. Runs should also be isolated on
their own display; that this one was not is a harness gap, not a browser
finding.
