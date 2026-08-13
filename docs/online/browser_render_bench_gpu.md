# Making the #100 browser render benchmark trustworthy (W16-B)

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

A prior pass at `scripts/browser_render_bench.py` produced numbers nobody would
stand behind. Three specific reasons, and what changed for each, followed by
the evidence this pass actually collected — with the caveats that evidence
does and does not support.

**This is a harness fix, not a verdict on v2's draw-call cost.** A separate,
in-flight fix to the real defect (`v2/ts/packages/render`'s character
draw-call splitting) was landing elsewhere while this work was done — see
"Pre-fix or post-fix?" below for what this run actually measured.

---

## 1. It ran on SwiftShader

`launch()`'s default is now **hardware**, not software. Chrome gets
`--use-gl=angle --use-angle=gl-egl --ignore-gpu-blocklist`, verified in this
environment by reading `UNMASKED_RENDERER_WEBGL`:

```
ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL ES 3.2)
```

reached with **no `DISPLAY` needed at all** — headless=new's ANGLE/EGL path
does not touch a display server. `--use-angle=vulkan` also reaches the GPU
(confirmed) but EGL was kept as the default per the task brief, so the
`ANGLE (..., OpenGL ES 3.2)` string matches PR #400's own recorded "hardware
ANGLE (RTX 2070 SUPER)" run for comparability. `--gpu-mode software` still
exists (`--use-angle=swiftshader --enable-unsafe-swiftshader`) for a machine
with no GPU.

**Firefox could not be gotten onto the GPU, and is reported as software.**
`-headless` forces WebRender onto `RenderCompositorSWGL` regardless of
`webgl.force-enabled` / `layers.acceleration.force-enabled` — confirmed (not
guessed) by launching with `MOZ_LOG=Compositor:5,WebRender:5,RenderThread:5`
and reading which `RenderCompositor*` class actually got constructed. Dropping
`-headless` does reach `RenderCompositorEGL`, but only by opening a real
window on `DISPLAY=:1` — this machine's only display, and it is the operator's
live desktop. `docs/online/browser_rigged_3d.md` already records why that is
off the table ("Not on `:1`": an earlier probe fought the machine owner for
the screen and they closed windows mid-run) and documents the alternative that
sidesteps it — a private `Xvfb` display plus NVIDIA PRIME render offload.
That path was checked here too: **`Xvfb` is not installed on this machine, and
this task has no sudo to add it.** So Firefox runs software, every Firefox run
records `firefox_compositor_backend` read from that MOZ_LOG evidence, and the
report below treats that field — not the WebGL-reported renderer string — as
ground truth. The renderer string is independently unreliable for Firefox: it
reports `"NVIDIA GeForce GTX 980, or similar"` on this exact RTX 2070 SUPER,
Firefox's own fingerprinting-resistance fuzzing, also already documented in
`browser_rigged_3d.md`.

## 2. `rigged_active` and the GPU string were not recorded per run

Two separate gaps, both closed:

- **The v2 side was worse than unrecorded — it was fake.** `render_bench.ts`'s
  `rendererPort.draw()` returned `rigged_active: true` unconditionally, a
  hardcoded literal, not a sample. `Benchmark.result().rigged_active` — the
  exact field `evaluate()`'s own gate checks — was therefore always `true`
  regardless of whether any character actually rendered rigged. Fixed: it now
  reads `sceneRoot.pitchGroup.children.some(c =>
  c.userData.riggedCharacter === true)` right after that frame's `populate()`
  ran — a real, sampled check against `pitch.ts`'s own tag on every rigged
  character wrapper, mirroring the "sampled rather than assumed" contract the
  Lua original already had (`pitch.rigged_players and
  player_renderer_3d.available()`).
- **Neither build's per-run record carried `rigged_active` or a GPU string
  as first-class, structured fields.** The Python read `rigged_active` for a
  console print and then dropped it; the aggregate JSON's `raw_runs` held it
  only buried inside build-specific pipe-marker text. Fixed:
  `lua_repeat_meta`/`v2_repeat_meta` now surface `rigged_active`,
  `gpu_mode_requested`, `gpu_renderer`/`gpu_vendor` (from a Python-side
  `probe_gpu()` that queries the SAME canvas element the build under test
  rendered through — `#canvas` for love.js, `#gl-canvas` for v2 — independent
  of what either build self-reports), and `firefox_compositor_backend` as a
  `per_run_meta` list on every aggregated result.
- **Every run now asserts its own `rigged_active`.** `run_lua_benchmark` /
  `run_v2_benchmark` raise immediately if `rigged_active` came back false —
  comparing a rigged run against a procedural fallback is worse than no
  measurement, so that repeat fails loudly rather than silently entering the
  averages.
- **Cross-build agreement is checked after the full matrix runs.** For every
  browser present in both builds' results, the script now compares the sets of
  `rigged_active` values lua/v2 reported; any disagreement (or anything other
  than "both always true") is a hard failure (`errors`, exit 1), printed with
  which browser and which values disagreed.

## 3. Draw calls had no per-source breakdown

`v2/tools/browser_render_bench/web/render_bench.ts` now computes a
`draw_call_breakdown` **once, after the timed loop, on one more freshly-built
frame** — never inside `bench.update()`/`bench.draw()`, which would leak extra
GPU work into the exact samples this file exists to keep honest.

`SceneRoot.render` is one call: `populate` (assemble `pitchGroup`/`hudGroup`,
no rasterizing) then one `Bloom.draw`, which renders the whole scene through
`EffectComposer` in one pass plus several fullscreen-quad post passes. There
is no per-category hook inside that pipeline, and this task does not own (and
did not touch) `scene.ts`/`pitch.ts`/`bloom.ts` to add one. So the breakdown
is built from **outside**, using only what `SceneRoot` already exposes
publicly (`scene`, `camera`, `pitchGroup`, `hudGroup` are `readonly` fields)
plus one tag `pitch.ts` already sets on every rigged character wrapper
(`userData.riggedCharacter = true`):

- `characters`, `pitch` (arena/markings/goals/effects/combat minus
  characters), `hud` — three RAW (non-bloom, non-composited)
  `renderer.render(scene, camera)` calls, isolated by toggling
  `Object3D.visible` on `pitchGroup`/`hudGroup`'s children between calls.
- `total` — the REAL production path (`SceneRoot.render`, bloom included) on
  the same frame, visibility restored first so the diagnostic toggling cannot
  leak into it.
- `bloom_overhead` — not independently measured (there is no "just the
  post-process passes" mode to call into); it is `total - (characters + pitch
  + hud)`, i.e. everything the composited render costs beyond one plain scene
  render, which is exactly what `Bloom.draw`'s `EffectComposer` branch adds
  over `renderer.render(scene, camera)`.

`hud` is always `0` in this harness, honestly, not because HUD is free: this
standalone benchmark page never passes `SceneRenderOptions.hud`, so
`SceneRoot.populate`'s own `else` branch clears `hudGroup` every frame. The
report field `hud_exercised: false` says so explicitly rather than leaving a
zero to be misread as "the HUD is cheap."

## 4. The prior run's timeout

`webdriver.Chrome(...)` / `webdriver.Firefox(...)` is one opaque call with **no
default HTTP timeout** on selenium's `RemoteConnection` — if chromedriver's
"new session" handshake stalls, the whole script blocks forever with nothing
in it to notice, until an external wrapper kills the process. `launch()` had a
constant, `CONNECT_TIMEOUT_SECONDS = 60`, clearly meant to bound exactly this
— and it was dead code, never referenced anywhere. That is very plausibly what
"a script timeout killed the second repeat" actually was: not a code defect in
the page under test, but an unbounded connect phase that got unlucky once,
against a machine now driving a real, finite, shared GPU (confirmed
independently: this machine runs ~200 pre-existing Chrome processes belonging
to the operator's desktop, plus, during this task's own evidence run, a load
average of 7–9 from several concurrent agent sessions) instead of
always-available SwiftShader.

Fixed by wiring the dead constant up rather than raising it: `bounded_launch()`
builds the `Service` object itself (instead of letting the all-in-one
`webdriver.Chrome`/`Firefox` constructor own it invisibly), runs construction
in a thread, and if it has not returned within `CONNECT_TIMEOUT_SECONDS`, kills
the driver's process group directly — `Service.process` is set early, well
before the handshake that can stall, so it is available even though the
constructor call itself never returned — and raises immediately instead of
leaking the child or hanging silently. Verified directly: forcing
`connect_timeout` down to 0.02–0.2 s against a real launch reliably raised a
clear error in well under a second, with **zero leaked chromedriver/chrome
processes afterward** (checked via `ps`/`pgrep`), for every value tried.

This is a bounded-fast-failure fix, not a guarantee the original hang cannot
recur — it converts "the whole script hangs with no diagnostic" into "this one
repeat fails immediately with a clear message," which the existing
error-collection path already knows how to report and move past.

---

## Evidence

Command:

```sh
python3 scripts/browser_render_bench.py \
  --builds lua,v2 --browsers chrome,firefox --repeats 3 \
  --frames 900 --warmup 120 \
  --output docs/online/evidence/browser_render_bench_hardware_gpu_2026-08-07.json
```

Full machine-readable output:
[`evidence/browser_render_bench_hardware_gpu_2026-08-07.json`](evidence/browser_render_bench_hardware_gpu_2026-08-07.json).
`--frames 900 --warmup 120` (half of this script's own defaults, 1800/180) —
a disclosed time-budget tradeoff for this session, not a default change; each
repeat still measures 900 samples (15 simulated seconds) per build/browser.
Exit code was `0`: no repeat failed its own `rigged_active` assertion, and the
lua/v2 cross-build agreement check found no disagreement, for either browser.

### GPU / backend actually used, per browser

| Browser | `rigged_active` (lua, v2) | GPU renderer (Python-probed, both builds agree with self-report) | Ground truth |
| --- | --- | --- | --- |
| Chrome | true, true (3/3 repeats each) | `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL ES 3.2)` | Hardware. `gpu_mode_requested=hardware` and the string is not spoofed. |
| Firefox | true, true (3/3 repeats each) | `NVIDIA GeForce GTX 980, or similar` (Firefox's own fuzzed string — do not trust) | **Software.** `firefox_compositor_backend=SWGL` every run, both builds. |

Both builds were requested and confirmed `rigged=true`/rigged for all 3
repeats on both browsers — the comparison below is rigged-vs-rigged, not
rigged-vs-procedural, on both sides.

### Update / draw percentiles (3 repeats, 900 measured frames each)

All times ms. `n` = repeats aggregated (not frames — each repeat already
computes its own percentile over 900 samples; `stdev`/`cv` below are across
those 3 already-computed p95 values, the honest way to show repeat-to-repeat
noise rather than hiding it inside a single pooled number).

| Build/Browser | update p95 (mean±stdev, cv) | draw p95 (mean±stdev, cv) | draw p95 range (min–max) | draw calls |
| --- | --- | --- | --- | --- |
| lua/chrome | 0.815 ± 0.005 (cv 0.6%) | 5.602 ± 0.136 (cv 2.4%) | 5.475–5.745 | 33.3 |
| lua/firefox | 0.953 ± 0.012 (cv 1.2%) | 7.160 ± 0.106 (cv 1.5%) | 7.040–7.240 | 33.3 |
| v2/chrome | 0.100 ± 0.010 (cv 10.0%) | 62.618 ± 36.541 (**cv 58.4%**) | 33.430–103.600 | 454.0 |
| v2/firefox | 0.113 ± 0.023 (cv 20.4%) | 50.673 ± 41.234 (**cv 81.4%**) | 26.180–98.280 | 454.0 |

`omp0_gate_pass` (the shared `update_p95≤8ms / draw_p95≤8ms / ...` gate from
`docs/online/omp0_acceptance.md`): lua passes on both browsers, all 3 repeats
each; v2 fails on both, all 3 repeats each, on the draw side only —
`draw_p95`/`draw_max` are well over the 8/33 ms gate every repeat, `update`
is not remotely close to its own limit. Frame jank counted independently
(frames over 33 ms, out of 900, **per repeat, not averaged** — the spread is
the point): lua `0, 0, 0` both browsers, all 6 runs. v2/chrome `121, 49, 485`;
v2/firefox `669, 7, 41`. A same-config repeat swinging from 7 to 669 janky
frames out of 900 is the same noise `draw p95`'s cv already shows, from a
second, independent angle.

**`update` is the trustworthy comparison here** (the actual wasm-vs-LuaJIT
question this migration is about): v2's update p95 is sub-0.15 ms against
lua's ~0.8–1.0 ms, consistently, with low-to-moderate noise. **`draw` is not
trustworthy at this sample size, on this machine, right now** — see "What
this does NOT show."

### Draw-call breakdown (v2 only; consistent across all 6 v2 runs)

| characters | pitch (arena/markings/effects/combat) | hud | bloom overhead | total |
| --- | --- | --- | --- | --- |
| 10 | 430 | 0 (not exercised — see §3) | 14 | 454 |

**Characters are 10 draw calls for ten players — one each**, not the
"several hundred, one per part-material transition" defect this task
described as in-flight elsewhere. `player_renderer_3d.ts`'s own header now
reads "ONE material, not three (draw-call fix...) ... collapsing the whole
character back to ONE draw call" — that fix has already landed in this
worktree by the time this evidence was collected. **This run is therefore
post-fix, not pre-fix** — flagged per this task's brief rather than left
implicit. The dominant bucket is now `pitch` (430 of 454, 95%): arena
backdrop/frame, markings, goals, effects and combat telegraphs, not
characters. That number is not further broken down here (out of this task's
scope), but it is now the correct target for anyone chasing the next
draw-call reduction, not characters.

### `pose_lod`: v2 does not wire it, and — a correction to this task's own brief

v2 deliberately does not wire `packages/render/src/rig3d/pose_lod.ts`
(present, ported, tested, explicitly documented as "PORTED BUT DELIBERATELY
NOT WIRED IN" pending a re-profile on this stack — its own justification was
the cost of clip sampling and 28 bone transforms through a wasm-hosted Lua 5.1
interpreter, an argument that does not obviously transfer to compiled
three.js). That part of this task's brief is confirmed by reading the file.

The other half is not confirmed. This task states "the Lua build has" pose
LOD. Searching this worktree's `game/` tree for it found nothing: no
`game/render/rig3d/pose_lod.lua` (the file `pose_lod.ts`'s own header cites
as its Lua original), no held-pose/refresh-schedule logic anywhere under
`game/render/`. `docs/online/browser_rigged_3d.md` — the most recent, most
detailed doc on this exact code path — lists the matching optimization
(**#393**/**#394**, "the per-character path: pose evaluation, bone-row
assembly, and the table churn around them") in a table of **expected, not yet
landed** levers ("With both landed, browser draw plausibly reaches ~2.5–3
ms" — future conditional), which is consistent with not finding it. So: **the
"Lua has it, v2 doesn't" framing in this task could not be verified from
source in this checkout**, and this report does not repeat it as fact. Lua's
`update`/`draw` figures above stand regardless — that determination doesn't
change what was measured, only whether the measured comparison should be
read as "pose LOD vs no pose LOD" (unconfirmed) or "two independently-tuned
renderers" (confirmed).

---

## What this does NOT show

- **`draw` timing on this machine, right now, is not resolved from noise.**
  v2/chrome's draw p95 ranged 33–104 ms across three back-to-back repeats of
  the identical configuration (cv 58%); v2/firefox ranged 26–98 ms (cv 81%).
  Lua's draw p95 was stable to 1–2% across the same repeats. That asymmetry —
  not just the raw noise — is itself informative: Lua issues 33 draw calls,
  v2 issues 454, and a busier submission path is the more plausible one to be
  sensitive to host contention, not a difference in v2's steady-state cost.
  This machine ran a load average of 7–9 during evidence collection (`uptime`,
  checked directly), from several concurrent agent sessions plus the
  operator's ~200-process desktop Chrome — this is not a quiet benchmark box.
  **Do not read v2's draw p95 as a settled number.** Three repeats say "this
  is noisy," not "this is the number." A trustworthy draw comparison needs
  either a quiet machine or many more repeats than fit this session's time
  budget, and ideally both.
- **This is not a full-scale run.** 900+120 frames (15 s measured), not this
  script's own defaults of 1800+180 (30 s) — a disclosed reduction for this
  session's time budget. Percentile computation itself is not starved (900
  samples per repeat), but total simulated coverage is half of what a full
  run would give.
- **The draw-call breakdown is a diagnostic re-render, not a timed sample.**
  It runs once per repeat, after the loop, outside every timed `update`/`draw`
  measurement — it explains what `draw_calls_mean` is made of, it is not
  additional evidence about frame *timing*.
- **`hud` is 0 because this harness never draws one**, not because HUD
  rendering is free — see §3.
- **The pose-LOD asymmetry this task's brief asserted could not be verified**
  as actually present on the Lua side in this checkout — see above. Treat the
  lua vs v2 comparison as two independently-tuned renderers unless someone
  confirms otherwise from a checkout that has the Lua file.
- **Neither build's final simulation hash matches the other's**, which is
  expected (independent ports, not required to agree bit-for-bit) but is
  called out so nobody mistakes the shared "live match, not inert" liveness
  check for a cross-build correctness check — it is not one. Each build's own
  hash *does* agree across both browsers (lua: `4ec116199dea1110` on both;
  v2: `65e251aa788caf91` on both), which is the determinism property this
  harness can actually speak to.
- **Firefox's hardware path was investigated, not solved.** `Xvfb` +
  NVIDIA PRIME offload (the technique `docs/online/browser_rigged_3d.md`
  already uses for a different tool) would very likely work, but `Xvfb` is
  not installed here and this task has no sudo — someone with package-install
  access should try it before concluding Firefox categorically cannot reach
  the GPU headlessly on this class of machine.
