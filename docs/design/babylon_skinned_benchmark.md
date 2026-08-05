# Babylon skinned-character benchmark (#341)

**Question.** #330 chooses between migrating presentation to Babylon and optimising
the LÖVE renderer, and it rests on an assumption: that a real animation system
handles skeletons cheaply enough that character count stops being the binding
constraint. This measures that assumption.

**Answer, stated up front and unfavourable.** It does not hold in the form #330
assumed. Babylon's cost per added character is **flat to slightly rising** between
10, 20 and 40 characters in every configuration and in both browsers. Draw calls
scale exactly linearly. Babylon buys a **lower constant** — a quarter of the draw
calls of the current LÖVE renderer, and about a fifth less draw time — not a
different shape of curve. In #330's own words, this is the second reading: the win
is overhead, not skeleton handling.

The full numbers, method, and the caveats that bound them are below.

---

## What was measured

Three things the issue required, all three present in every row:

1. **Draw-call count alongside frame time**, for every configuration.
2. **The scaling curve at 10, 20 and 40 characters**, reported as a shape (the
   marginal cost of the next character) rather than as endpoints.
3. **Chrome and Firefox, headed, on real GPU hardware**, with the GPU renderer
   string captured verbatim as proof.

Machine: Linux, NVIDIA GeForce RTX 2070 SUPER, driver 595.71.05, `DISPLAY=:1`,
render target 960x540. Chrome 150.0.7871.181 with chromedriver 150.0.7871.124;
Firefox 153.0.1 with geckodriver 0.37.1. Babylon.js 9.19.1. Measured 2026-08-04.

GPU renderer strings, verbatim, as the runner recorded them:

- Chrome: `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL 4.5.0)`
- Firefox: `NVIDIA GeForce GTX 980, or similar` (vendor `NVIDIA Corporation`)

Firefox sanitises the model name for fingerprinting resistance. That string proves
**hardware**, and it does not prove **which** hardware. Both browsers ran on the
same machine and the same GPU; only Chrome's string names it.

## How it works

```
love . --capture-frames 1800 300 .bench/babylon/render_frames.json
DISPLAY=:1 python3 -B scripts/babylon_bench.py
```

- `scripts/capture_render_frames.lua` runs #100's fixture — same seed (20260803),
  same teams, same bot, same fixed timestep, same 960x540 pitch — and writes 1800
  `RenderFrame` payloads as flat frame-major streams (994 KB of JSON). No drawing,
  no `love` at all: it is pure `sim/` + `render/`.
- `bench/babylon/` loads a CC0 rigged humanoid, instantiates N independent
  skeletons, and drives every one of them from those streams: position, facing,
  locomotion blended by speed across `Idle`/`Walking_A`/`Running_A`, plus a pose
  clip per pose family. `render/player_pose.lua` defines 32 families and the
  capture exercised **19 of them, 59%** (`locomotion`, the keeper vocabulary
  including `keeper_dive`, `keeper_spread`, `keeper_stretch`, `keeper_central`,
  `keeper_grab`, `keeper_throw`, `keeper_get_up`, `keeper_set`,
  `keeper_shuffle`, `keeper_ready_tall`, plus `aerial_action`, `contain`,
  `settle`, `slide`, `tackle`, `stumble`, `run_telegraph`, `soccer_windup`).
  **The 13 that are missing are not a random sample, and their absence favours
  Babylon** — see "The coverage gap has a direction" below.
- The scene is a football frame, not a character viewer: pitch with real marking
  geometry, two goals, ball, directional light with a shadow pass, hemispheric
  fill.
- `scripts/babylon_bench.py` serves it, drives headed Chrome and Firefox, and
  folds the `GC_BENCH_*` markers into `.bench/babylon/report.json`. That filename
  means "the whole matrix ran": a run with any failed configuration writes
  `report_incomplete.json` and deletes any stale `report.json`, so a reader that
  forgets to check the exit code cannot pick up a partial matrix and believe it.

Babylon's cost to animate ten skinned characters does not depend on where the
poses came from, which is why this could be answered before the wasm boundary
(#332) landed. When it lands, the data source swaps and these numbers stand.

### Why the run loop is not `requestAnimationFrame`

rAF is vsync-locked, so under it every configuration reports 16.67 ms whether it
has 90% headroom or none — the exact trap `game/render/benchmark.lua` disables
vsync to avoid. The page drives itself from a `MessageChannel` port instead.

### What `draw` and `frame` mean

- `draw` — CPU time inside `scene.render()`: culling, material binds, bone-matrix
  uploads, draw-call issue. The direct counterpart of the native baseline's
  `draw` sample, which also times only the CPU side of one LÖVE draw.
- `frame` — wall time for one loop iteration **including a `gl.finish()`**, so the
  GPU has retired the frame. This is what a player feels. It has no counterpart in
  the native baseline and must not be compared against it.

The server sends COOP/COEP so the page is cross-origin isolated: without it
browsers clamp `performance.now()` to 100 µs, a fifth of the difference being
resolved. Isolated, the clamp is 5 µs.

### Variants

| variant | meshes per character | animation |
| --- | --- | --- |
| `authored` | 8 (six skinned + helmet + cape, as the pack ships) | 3 blended locomotion clips + 1 pose clip |
| `merged` | 3 (six skinned collapsed into one + helmet + cape) | same |
| `merged_static` | 3 | **one** clip at full weight — a control, not a candidate |

`merged` is legal only because the whole pack shares a single material. It is the
Babylon-side equivalent of the rigid-GPU-skinning optimisation #330 costs for
LÖVE, so measuring only `authored` would have answered a different question.
`merged_static` isolates the cost of skeletal animation: the gap between it and
`merged` is blending and nothing else.

## Results

600 measured frames after 300 warm-up frames, three interleaved passes per
configuration in a single browser session, median across passes. Draw calls are
deterministic and identical across passes; the runner fails if they are not.

| browser | variant | chars | draw calls | calls/char | draw p50 (ms) | draw p95 (ms) | frame p95 (ms) | p50 spread |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| chrome | authored | 10 | 187 | 18.7 | 1.86 | 2.95 | 5.79 | 1.82–1.95 |
| chrome | authored | 20 | 347 | 17.4 | 5.14 | 6.67 | 13.38 | 4.49–5.18 |
| chrome | authored | 40 | 667 | 16.7 | 12.12 | 16.07 | 31.60 | 11.77–12.62 |
| chrome | merged | 10 | 87 | 8.7 | 1.35 | 2.29 | 4.61 | 1.31–1.44 |
| chrome | merged | 20 | 147 | 7.3 | 3.71 | 5.76 | 12.89 | 3.65–4.34 |
| chrome | merged | 40 | 267 | 6.7 | 8.86 | 10.96 | 23.25 | 8.74–10.33 |
| chrome | merged_static | 10 | 87 | 8.7 | 1.08 | 1.84 | 3.60 | 1.04–1.11 |
| chrome | merged_static | 20 | 147 | 7.3 | 2.21 | 3.53 | 6.80 | 2.08–2.32 |
| chrome | merged_static | 40 | 267 | 6.7 | 6.72 | 8.93 | 17.85 | 6.12–7.56 |
| firefox | authored | 10 | 187 | 18.7 | 3.32 | 4.54 | 12.14 | 3.28–3.58 |
| firefox | authored | 20 | 347 | 17.4 | 7.40 | 11.52 | 25.22 | 6.62–7.96 |
| firefox | authored | 40 | 667 | 16.7 | 16.64 | 20.30 | 44.50 | 15.76–17.38 |
| firefox | merged | 10 | 87 | 8.7 | 2.80 | 4.08 | 11.02 | 2.50–2.82 |
| firefox | merged | 20 | 147 | 7.3 | 6.18 | 8.98 | 22.30 | 6.06–6.20 |
| firefox | merged | 40 | 267 | 6.7 | 12.92 | 17.92 | 40.84 | 12.44–13.66 |
| firefox | merged_static | 10 | 87 | 8.7 | 1.62 | 2.44 | 6.66 | 1.38–1.70 |
| firefox | merged_static | 20 | 147 | 7.3 | 3.84 | 5.62 | 16.02 | 3.46–3.98 |
| firefox | merged_static | 40 | 267 | 6.7 | 9.24 | 14.44 | 31.36 | 9.06–10.54 |

Draw calls are `characters * meshes * 2` plus a fixed 27 for pitch, goals and
ball. The `* 2` is the shadow pass; every character is drawn twice.

## The shape of the curve

Marginal cost of one more character, which is the number the assumption lives or
dies on:

| browser | variant | 10 → 20 | 20 → 40 | change |
| --- | --- | ---: | ---: | --- |
| chrome | authored | 328.5 µs | 348.8 µs | +6% |
| chrome | merged | 235.5 µs | 257.8 µs | +9% |
| chrome | merged_static | 113.5 µs | 225.2 µs | +98% |
| firefox | authored | 408.0 µs | 462.0 µs | +13% |
| firefox | merged | 338.0 µs | 337.0 µs | −0.3% |
| firefox | merged_static | 222.0 µs | 270.0 µs | +22% |

Draw calls per character: 16.0 in every `authored` step, 6.0 in every `merged`
step, at both ends of the range, in both browsers. Exactly linear.

**How much of this table is signal.** The per-configuration percentage changes
in the right-hand column are *within* the ~10% run-to-run spread documented in
the caveats below, so the precise slope of any one row is not load-bearing and
should not be quoted as one. What survives the spread is the absence of the
thing the assumption needed: no configuration shows a materially *flatter*
marginal cost at 40 characters than at 10, in either browser, and the draw-call
column is exact.

**Reading.** Nothing here flattens. Five of six configurations measure *more*
expensive per character as the count rises and the sixth is flat — but for four
of those five the rise is inside the spread, so the defensible statement is
"flat, not flattening" rather than "steepening". Only `chrome merged_static`
(+98%) moves far enough to be a genuine steepening on its own.

#330's stated test was "a materially flatter marginal cost per character supports
the assumption; curves rising in parallel with Babylon merely lower means the win
is constant overhead, not skeleton handling." This is unambiguously the second
one: no configuration comes close to flatter, in either browser, at any spread
you care to allow. Babylon's curves sit lower and run parallel.

The `merged_static` control says the same thing from the other side: strip the
blending entirely and the per-character cost falls by about a fifth to a quarter,
but the curve keeps its shape — and in Chrome it steepens. So the linearity is
not an artefact of blending four clips per character — it is the base cost of an
independently posed skinned character, and Babylon does not amortise it away.

## The coverage gap has a direction

19 of 32 pose families is not a random 59% sample, and the 13 that are missing
are systematically the *expensive* ones:

| missing family | priority | maps to |
| --- | ---: | --- |
| `keeper_punt` | 121 | `Unarmed_Melee_Attack_Kick` |
| `keeper_tip` | 110 | `Dodge_Right` |
| `aerial_bicycle` | 95 | `2H_Melee_Attack_Spin` |
| `combat_knockback` | 90 | `Hit_B` |
| `combat_stagger` | 89 | `Hit_A` |
| `combat_guard` | 84 | `Blocking` |
| `combat_active` | 83 | `Unarmed_Melee_Attack_Punch_A` |
| `combat_windup` | 82 | `Unarmed_Melee_Attack_Punch_B` |
| `combat_aim` | 81 | `1H_Ranged_Aiming` |
| `combat_recovery` | 80 | `Interact` |
| `kick_follow` | 45 | `Unarmed_Melee_Attack_Kick` |
| `fatigue` | 20 | `Unarmed_Idle` |
| `keeper_ready_low` | 15 | `Blocking` |

All seven `combat_*` families are absent, and they sit at priority 80–90 in
`render/player_pose.lua` — above everything except keeper saves — so in a match
with combat enabled they would be *selected* often, not occasionally.
`aerial_bicycle` is also absent, and it maps to `2H_Melee_Attack_Spin`, the most
elaborate clip in the whole mapping. The fixture is #100's, which runs combat
disabled; that is why they never appear.

**So the finding above is a best case for Babylon.** A capture that exercised the
combat band would put more full-body clips into the action slot more of the time,
which makes the per-character cost higher and the curve worse, not better. Since
the finding is already the unfavourable reading, the gap strengthens it — but a
#330 reader must not mistake 59% coverage for a neutral sample.

## Against the native LÖVE baseline

From #328, ten players on this same RTX 2070 SUPER:

| | draw calls | draw p95 |
| --- | ---: | ---: |
| LÖVE procedural (native) | 14 | 0.95–1.01 ms |
| LÖVE rigged (native) | 331.6 | 2.74–2.80 ms |
| Babylon `authored` (Chrome) | 187 | 2.95 ms |
| Babylon `merged` (Chrome) | 87 | 2.29 ms |
| Babylon `merged` (Firefox) | 87 | 4.08 ms |

Babylon in Chrome, merged, cuts draw calls by 3.8x against the current rigged
LÖVE renderer and draw p95 by about 18% — **in a browser, against a native
baseline**, which is the genuinely encouraging part of this. But it is not the
step change the migration case wanted, and Firefox is 1.5x slower than native
LÖVE at the same character count.

This is **not** the verdict. The verdict needs optimised LÖVE (#337 slice 2) and
belongs to #330, which requires all three numbers in one table. #330 estimates
optimised LÖVE at ~10 draw calls for ten characters via rigid GPU skinning; if
that estimate holds, LÖVE-optimised would have *fewer* draw calls than Babylon
merged, and the migration case has to rest on the animation pipeline being
bought-not-built rather than on frame cost.

## Caveats a reader must have

- **Feature sets differ.** The Babylon frame pays a shadow pass (which is why
  draw calls double); the LÖVE baseline pays bloom. Neither draws the other's
  effects. The comparison is indicative, not like-for-like.
- **The sample windows differ.** These runs measure 600 frames, about 10 seconds
  of match; `game/render/benchmark.lua` defaults to 3600 frames, about 60
  seconds, and that is what the #328 native baseline was taken over. Both
  disable vsync and both emit the same summary fields, so the comparison is not
  invalid — but a 10-second window has fewer chances to catch a rare stall, so
  read the Babylon `max` and `over33` columns as less complete than the native
  ones rather than as better. The medians are what the finding rests on.
- **Pose-family coverage is 19 of 32, and the gap is not neutral** — see "The
  coverage gap has a direction" above.
- **Characters past ten are not new simulation.** Copies read the same ten
  captured streams from a different point in time and rotated into a different
  part of the pitch. Every skeleton is independently posed and independently
  evaluated — which is what the scaling curve is about — but no roster of 40
  players was simulated.
- **Whole-pitch framing.** The camera is identical at 10, 20 and 40 so nothing is
  culled and per-character pixel coverage does not change with count. At 105 m
  across a 960-wide target a 1.8 m character is about 16 px, so this measurement
  is submission- and skeleton-bound rather than fill-bound. The native baseline
  frames the whole pitch too.
- **Firefox's GPU string is sanitised**, as noted above.
- **Session and ordering matter more than expected.** The first version of this
  runner launched a fresh browser per configuration and ran each to completion
  before the next. Under that scheme the variant with a *third* of the draw calls
  measured *slower*, because browser start-up, GPU-process init and the NVIDIA
  clock ramp all landed on whichever configuration went first. One session with
  interleaved passes and a 300-frame warm-up fixed it. Anyone re-running this
  should keep both.
- **Run-to-run spread is about 10%, and it bounds what can be claimed.** Across
  the three passes the min-to-max span of draw p50 ranges from 2% to 21% of the
  median depending on configuration, ~10% typically. That is far smaller than the
  differences under test — the steps from 10 to 20 to 40 characters are 2–3x
  each — but it is larger than the percentage changes in the marginal-cost table,
  which is why the finding is stated as "does not flatten" rather than as a
  precise slope. Every median in the tables is accompanied by its span so a
  reader can apply this themselves.
- **Ordering within a pass is still fixed.** `run_browser` repeats the matrix
  three times, but visits the variants in the same order inside each pass, so a
  monotonic session drift would land at the same relative position every time
  rather than being cancelled by the median across passes. The 300-frame warm-up
  removes most of what would drift, but shuffling the order per pass would close
  it properly.
- **The machine was not idle.** These runs were taken with several agents working
  on the same box; `uptime` reported a load average of roughly 2–5 during the
  measurement window. The three-pass spread is the evidence that this did not
  dominate, not an assumption that the machine was quiet.

## The software-rasteriser refusal

Headless Chrome falls back to SwiftShader, and #100 published one false negative
from exactly that. So `scripts/babylon_bench.py` refuses:

- it runs **headed** and exits if `DISPLAY` is unset;
- it reads `WEBGL_debug_renderer_info` and carries the string into the report;
- a renderer matching a software rasteriser is a **hard failure**, and so is a
  renderer the browser declines to name — unproven is not publishable either;
- there is no override flag, because an override flag is how the false negative
  gets published the second time.

Two levels of evidence, and they are not interchangeable:

```
python3 -B scripts/babylon_bench.py --self-test      # parsing + refusal logic; starts no browser
DISPLAY=:1 python3 -B scripts/babylon_bench.py --prove-refusal   # a REAL Chrome forced onto SwiftShader
```

The self-test runs in `scripts/check.sh` and in CI. `--prove-refusal` runs on the
measuring machine, and on 2026-08-04 it produced:

```
refusing to publish a software-rasteriser result. GPU renderer reported as
'ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)'.
```

## Assets

The Babylon bundles and the character are **fetched on demand and verified against
pinned SHA-256 hashes**, never committed — the same pattern `scripts/web_build.py`
uses for the love.js runtime, and consistent with THIRD_PARTY.md's record that this
repository tracks no third-party binary. See THIRD_PARTY.md for the licences.

The character is KayKit Adventurers 1.0 `Knight.glb`: CC0 1.0, 41 joints, six
skinned meshes, 3716 vertices, 76 clips, one shared material. Weapons and shields
are disabled — a footballer carries none of them, and left visible they would
inflate the draw-call count with geometry the game will never show.

The pose-family-to-clip mapping in `bench/babylon/bench.js` is deliberately
literal. A knight's `PickUp` is not a keeper's smother, and nobody should read it
as authored animation. What has to be right for a *benchmark* is that each family
costs a real skeletal clip evaluated on a real skeleton and blended over
locomotion, and that is what it buys.

## Out of scope here

Live wasm integration (#332 and the rest of #328), the verdict against optimised
LÖVE (#337 slice 2, decided in #330), IK (#318), native packaging (#329), and
feature parity with the LÖVE renderer.
