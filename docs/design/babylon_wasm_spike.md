# Babylon driven by the wasm simulation (#328)

**Question.** #330 chooses between migrating presentation to Babylon.js and
optimising the LÖVE renderer. #341 measured Babylon from a *file* of captured
render frames and #332 built the wasm payload boundary; neither had ever been
connected to the other. This connects them, measures the result on real GPU
hardware in two browsers, and answers the one question #328 was opened to
answer: **does Babylon clear the frame budget with more headroom than optimised
LÖVE would?**

**Answer, stated up front and unfavourable to the migration-on-cost case: no.**

Ten rigged players, live from the simulation, render in a browser at a cost
that is *competitive with* optimised native LÖVE and not better than it. Once
both renderers are measured at their own floor the per-character draw-call cost
is a tie — 2.00 for Babylon against 1.94 for LÖVE — and Babylon's is the more
expensive tie because it buys a real shadow pass LÖVE does not draw. On CPU per
frame Babylon in Chrome is *behind*, because the browser-side simulation costs
about five times what the native one does. In Firefox it is behind on every
measure.

What did *not* change is the case #328's own issue text put first, and it is not
a frame-cost case: built-in IK, bone masking, glTF retargeting and GPU skinning
are bought rather than built, and WebGL2 mandates `DEPTH24_STENCIL8`, which
love.js cannot supply at any price. Those remain true. **The migration has to be
argued on the animation pipeline and on the depth buffer. It can no longer be
argued on frame cost, in either direction.**

Everything below is the measurement that says so, and the caveats that bound it.

---

## What was built

`#341` renders from `love . --capture-frames`. `#332` exposes one render frame
per crossing out of a wasm module. #328 is the join, and deliberately nothing
more:

```
render/frame_buffer.lua  →  Rust host  →  linear memory  →  frame_payload.js
       (#332)                                                      │
                                                                   ▼
                                                       bench/babylon/scene.js
                                                        (the #341 scene, shared)
```

| file | what it is |
| --- | --- |
| `bench/babylon/scene.js` | the scene, character build, clip table and measurement loop — **extracted unchanged from #341's `bench.js`** so both drivers render the same thing |
| `bench/babylon/bench.js` | the captured-file driver (#341), now the adapter only |
| `bench/babylon/wasm_bench.js` | the **live** driver: one `_goliseo_payload_frame` call per rendered frame, zero copy |
| `bench/babylon/wasm_index.html` | the page, which loads `simhost.js` with `--payload` |
| `scripts/babylon_wasm_bench.py` | the runner: stages, serves, drives headed Chrome and Firefox, refuses software rasterisers, and gates every verdict rule |

The scene extraction is what makes the two comparable at all. A captured-payload
row and a live-payload row differ in exactly one thing — where a pose came from
— because every other line of code is the same file.

### One crossing per rendered frame, measured rather than asserted

Per frame the page calls `_goliseo_payload_frame(1, 0)` once. It advances the
match one fixed tick, builds the `RenderFrame`, serialises it into linear memory
and returns a pointer; `frame_payload.js` hands back `Float64Array` subarrays
over the module's own memory. Nothing is copied and nothing is marshalled per
entity.

That is not taken on trust. The page wraps **every** `_goliseo_payload_*` export
in a counter — discovered dynamically, exactly as `verify_payload.js` does it, so
a second call made anywhere inside the render loop is counted — and reports the
measured rate. Every row in the tables below reported **1.00 payload crossings
and 0.00 other crossings per rendered frame**, over 900 frames each, and the
runner fails the run on anything else.

The block is **266 words, 2128 bytes** for ten players, matching #332's figure
exactly.

`Module.HEAPF64` is re-read every frame because `ALLOW_MEMORY_GROWTH` is on and
growing the heap detaches every existing view.

### The simulation runs on the render thread, and that is a choice

A shipped build would put the match in a worker so an eight-tick rollback burst
cannot stall a frame; `verify_payload_browser.py` measures that arrangement.
This page deliberately does not, for two reasons. It is the arrangement in which
the payload is genuinely zero-copy — a worker has to clone the block through
`postMessage` — so it measures the boundary #332 designed rather than a
transport layered on it. And it puts the simulation's cost inside `update`,
beside the draw cost, where a reader can see both. **Frame time here is
therefore a pessimistic single-thread figure. Draw time is unaffected, because
`draw` times only `scene.render()`.**

---

## Method

```
wasm/sim-host/build.sh                                   # Docker + emcc, once
DISPLAY=:1 python3 -B scripts/babylon_wasm_bench.py      # the measurement
python3 -B scripts/babylon_wasm_bench.py --self-test     # the gate; no browser
```

Machine: Linux 7.0.0-28-generic, NVIDIA GeForce RTX 2070 SUPER, `DISPLAY=:1`,
render target 960x540. Chrome 150.0.7871.181 / chromedriver 150.0.7871.124;
Firefox 153.0.1 / geckodriver 0.37.1. Babylon.js 9.19.1, LÖVE 11.5.0. Measured
2026-08-05.

Module under test, digested from what was actually *served* rather than what was
asked for:

```
simhost.wasm  sha256 a38c5ee0564b46538b1678e1b0183af3e690d397c686b91d768811f18d328468
simhost.js    sha256 6c91c1d21cbde427905575db78978d378c809094ead80c432442557720911d73
```

GPU renderer strings, verbatim, as the runner recorded them:

- Chrome: `ANGLE (NVIDIA Corporation, NVIDIA GeForce RTX 2070 SUPER/PCIe/SSE2, OpenGL 4.5.0)`
- Firefox: `NVIDIA GeForce GTX 980, or similar` (vendor `NVIDIA Corporation`)

Firefox sanitises the model name for fingerprinting resistance. That string
proves **hardware**; it does not prove **which** hardware. Both browsers ran on
the same machine and the same GPU; only Chrome's string names it.

600 measured frames after 300 warm-up frames, three interleaved passes per
configuration in a single browser session, median across passes. Draw calls are
deterministic and identical across passes; the runner fails if they are not.

**Machine load, recorded because other agents build on this box.** The browser
matrix ran at load average 2.39 → 1.47 on 16 cores (uncontended). The native
LÖVE baseline ran minutes earlier at load 3.35 → 3.03 (uncontended). Both are
therefore quiet-machine numbers, and both are labelled as such. The earlier
contended run of the same matrix — load 6.6 → 56 — produced draw p95 figures
40–70% higher and is not reported here except as the reason the numbers carry
their conditions.

### Variants, and why there are now three

| variant | meshes per character | draw calls per character |
| --- | ---: | ---: |
| `authored` | 8 (six skinned + helmet + cape, as the pack ships) | 16.0 |
| `merged` | 3 (six skinned collapsed into one + helmet + cape) | 6.0 |
| **`merged_all`** | **1 (gear folded into the skin as well)** | **2.0** |

`merged_all` is new in #328 and it exists because of a comparison error the
earlier framing would have made. #341's `merged` collapses only the *skinned*
meshes: the KayKit knight's helmet and cape carry no bone weights, Babylon's
`MergeMeshes` will not mix the two, and they stay standing as separate meshes.
That is three meshes per character, doubled by the shadow pass, which is the 6.0
the #341 doc reports.

#337 slice 2's optimised LÖVE renderer went further: it folded **all 28 rigid
parts including gear** into one static mesh. So quoting LÖVE's 33.4 against
Babylon's 87 compares LÖVE at its floor with Babylon above its own. `merged_all`
closes that: a bone-parented unskinned mesh is the degenerate case of skinning —
every vertex weighted 1.0 to one bone — so `scene.js` expresses the helmet's and
cape's vertices in the skin's local space at rest, gives them weight 1 on their
parent bone, and appends them to the skin's own vertex arrays. Babylon uploads
`restInverse * absolute` per bone, which is identity at rest, so the baked
vertices sit exactly where the bone-parented meshes sat and then follow the bone.

That it is a *transform* and not a *change of picture* was checked rather than
assumed: rendering the identical frame in all three variants and diffing the
screenshots, `merged` and `merged_all` differ in 1392 of 1 864 886 pixels
(0.075%), which is **less** than `merged` differs from `authored` (3900 pixels,
0.21%). The residue in both is silhouette and shadow-map sampling.

`merged_all` is as shippable as LÖVE's equivalent and no more: both weld gear
into the character mesh, so both would need a rebuild to swap a cosmetic piece.
Neither is a per-frame cost.

---

## Results — Babylon, from the live wasm payload

Ten players, one per roster slot, each driven by its own stream out of the
simulation.

| browser | variant | chars | draw calls | calls/char | draw p50 (ms) | draw p95 (ms) | update p95 (ms) | frame p95 (ms) | crossings/frame |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| chrome | authored | 10 | 187 | 18.70 | 1.830 | 2.775 | 1.715 | 7.950 | 1.00 |
| chrome | merged | 10 | 87 | 8.70 | 1.550 | 2.720 | 1.760 | 8.380 | 1.00 |
| chrome | **merged_all** | 10 | **47** | **4.70** | **1.185** | **2.070** | 1.745 | 6.730 | 1.00 |
| firefox | authored | 10 | 187 | 18.70 | 4.440 | 6.860 | 2.360 | 19.860 | 1.00 |
| firefox | merged | 10 | 87 | 8.70 | 3.580 | 5.780 | 2.380 | 17.660 | 1.00 |
| firefox | **merged_all** | 10 | **47** | **4.70** | **3.080** | **4.500** | 2.220 | 14.760 | 1.00 |

Per-pass draw p95 spread, so nobody has to trust a median:

| browser | variant | pass 1 | pass 2 | pass 3 |
| --- | --- | ---: | ---: | ---: |
| chrome | authored | 2.115 | 2.775 | 2.890 |
| chrome | merged | 2.195 | 2.720 | 3.020 |
| chrome | merged_all | 1.900 | 2.755 | 2.070 |
| firefox | authored | 6.860 | 6.600 | 7.180 |
| firefox | merged | 5.780 | 5.120 | 5.900 |
| firefox | merged_all | 4.400 | 4.500 | 4.620 |

Draw calls are `27 + characters * meshes * 2`: 27 fixed for pitch, goals and
ball, and every character drawn twice because of the shadow pass. The
`calls/char` column above therefore includes the fixed scene amortised over ten
characters; the **marginal** cost of one more character is 16.0 / 6.0 / 2.0.

`update` is the whole per-frame simulation cost: one fixed tick with its
rollback snapshot capture, the `RenderFrame` build, the encode into linear
memory, the JavaScript-side read, and placing ten characters. It is not
comparable to `love.update`, which does the tick and nothing else — see the
verdict.

`draw` is CPU time inside `scene.render()`. `frame` is one whole loop iteration
including a `gl.finish()`, so the GPU has retired the frame; the LÖVE baseline's
`frame` is not the same measurement and the two must not be subtracted.

---

## Beside the native LÖVE baseline

Re-measured on this same machine minutes before the browser run rather than
quoted, because #337 slice 2 landed two hours before this work started and the
comparator is the whole point. `love . --benchmark 1800 300 {procedural,rigged}`,
vsync off, seed 20260803, 960x540, ten players, load average 3.35 → 3.03. Two
paired runs each:

| renderer | draw calls | calls/char (marginal) | draw p50 (ms) | draw p95 (ms) | update p95 (ms) | frame p95 (ms) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| LÖVE procedural (native) | 14.0 | — | 0.682 / 0.672 | 1.224 / 1.185 | 0.355 / 0.356 | 2.643 / 2.597 |
| **LÖVE rigged, optimised (native)** | **33.4** | **1.94** | 1.019 / 1.017 | **2.482 / 2.501** | 0.365 / 0.355 | 3.842 / 3.951 |
| Babylon `merged_all` (Chrome) | 47 | **2.00** | 1.185 | **2.070** | 1.745 | 6.730 |
| Babylon `merged_all` (Firefox) | 47 | **2.00** | 3.080 | **4.500** | 2.220 | 14.760 |
| Babylon `merged` (Chrome) | 87 | 6.00 | 1.550 | 2.720 | 1.760 | 8.380 |
| Babylon `merged` (Firefox) | 87 | 6.00 | 3.580 | 5.780 | 2.380 | 17.660 |

Both LÖVE runs report `state_hash=51875e4b2a3adac1`, identical across renderers:
presentation does not reach the simulation on that side either.

My 33.4 and 1.94 reproduce PR #350's exactly. My added draw p95 —
2.482 − 1.224 = **1.258 ms**, and 2.501 − 1.185 = **1.316 ms**, i.e. 63–66% of
the 2 ms feature-delta budget — is a little above the 1.132 ms (56.6%) that PR
recorded, which is run-to-run spread on a machine that was not idle in either
case. The draw-call columns are exact and identical.

---

## The verdict

**Does Babylon clear the frame budget with more headroom than optimised LÖVE
would? No.** Three readings, and none of them favours Babylon on cost.

**1. Draw calls, floor to floor, are a tie — and Babylon's tie is the more
expensive one.** 1.94 per character for optimised LÖVE against 2.00 for Babylon
`merged_all`. Both put the character in exactly **one** mesh and one call; the
remainder is a second pass in each case, and the two second passes are not the
same feature. Babylon's is a real shadow-map render of the character. LÖVE's
~0.94 is a 2D ground-contact ellipse plus the controlled player's selection
rings — a blob, not a shadow. So at equal draw-call cost Babylon is drawing
strictly more, which is a point *for* Babylon on quality and *not* a point for
it on headroom. Nothing here is a step change in either direction. Draw-call
counts are exact and deterministic; every other number in this document is not.

**2. On draw time alone Chrome is genuinely ahead, and that is the encouraging
part.** 2.070 ms against native LÖVE's 2.482–2.501 ms — a browser beating a
native renderer at the same character count, at 47 draw calls against 33.4. It
is not a large margin, it sits inside the pass-to-pass spread of both, and it
comes with a feature asymmetry in both directions (Babylon draws a shadow map
LÖVE does not; LÖVE draws bloom Babylon does not). Read it as parity, not as a
win.

**3. On whole-frame CPU Babylon is behind, and the reason is the simulation, not
the renderer.** Per frame:

| | update p95 | draw p95 | sum | of a 16.67 ms frame |
| --- | ---: | ---: | ---: | ---: |
| LÖVE rigged, optimised (native) | 0.365 | 2.482 | **2.85 ms** | 17% |
| Babylon `merged_all` (Chrome) | 1.745 | 2.070 | **3.82 ms** | 23% |
| Babylon `merged_all` (Firefox) | 2.220 | 4.500 | **6.72 ms** | 40% |

The `update` gap is roughly five-fold and it is **not** like-for-like: LÖVE's
0.365 ms is one simulation tick, while Babylon's 1.745 ms is a tick *plus* the
rollback snapshot capture, the `RenderFrame` build, the encode into linear
memory, the JavaScript read and the placement of ten characters. `#332`'s own
node breakdown apportions it: 0.838 ms mean for the tick and its snapshot,
0.203 ms for the whole payload extraction. So the payload boundary this issue
was built on is **not** the expensive part — the simulation running as wasm
rather than as LuaJIT is. That is a finding about hosting the simulation in a
browser at all, which is #327/#332's territory rather than Babylon's, and it
would be paid by any browser renderer including a love.js one.

**Firefox does not clear the budget with headroom by any reading.** `frame` p95
of 14.76 ms is 89% of a 60 Hz frame with ten characters, a pitch and nothing
else on screen, and that is measured with a `gl.finish()` in the loop on an
uncontended RTX 2070 SUPER. Chrome's 6.73 ms is comfortable; Firefox's is not,
and Firefox is 2.2x Chrome's draw time at every variant. A migration decision
that only looks at Chrome is not looking at the web.

**So the honest summary.** #341 found that Babylon's win over the *then* LÖVE
renderer was a lower constant rather than a different curve. With LÖVE optimised
the constant is gone too: the two renderers now cost about the same to draw the
same ten characters, and Babylon additionally pays for its simulation being in
wasm. **#330 cannot buy headroom by migrating.** What it can buy is a real
animation pipeline it does not have to write — `BoneIKController` for #318's
foot planting and keeper hands, bone masking that `rig3d/masks.lua` currently
hand-rolls, documented glTF retargeting — and a depth buffer that WebGL 1 cannot
give it at all. Those were the first reasons listed in #328's own issue text and
they survive this measurement untouched. Frame cost was the fourth reason, and
it is now a wash.

---

## Simulation hashes are unchanged from the frozen contract while rendering

This is the one genuinely new correctness property in #328 — rendering must not
perturb the simulation — and it is checked three independent ways. All three
held, in both browsers.

### 1. The same match, rendered and unrendered, in one page

`_goliseo_payload_boot` restarts the match, so the page runs 600 ticks drawing
every frame, hashes the state, reboots, runs the same 600 ticks drawing nothing
at all, and hashes again. A renderer reaching back into simulation state —
through an aliased typed-array view written instead of read, say — shows up here
as two different hashes.

| runtime | 600 ticks, rendering | 600 ticks, no rendering | verdict |
| --- | --- | --- | --- |
| Chrome | `ab55d5912c3b6009` | `ab55d5912c3b6009` | AGREE |
| Firefox | `ab55d5912c3b6009` | `ab55d5912c3b6009` | AGREE |
| node (no renderer in the process) | — | `ab55d5912c3b6009` | AGREE |

`ab55d5912c3b6009` is not a value this work chose. It is the same 600-tick hash
`docs/online/wasm_webview_determinism.md` recorded as "a fourth, unpinned
agreement… nobody chose it as a contract", printed there by the phase-0 probe
and here by the payload host, which are different entry points of the same
module driven by different callers.

### 2. The frozen 7201-tick contract, in a worker, while the page renders

A second module instance runs the frozen determinism fixture in a Web Worker
while the main thread renders ten characters from the live payload. The page
does not stop rendering to let it finish — it rendered a further **128 378
frames** (Chrome) and **105 679 frames** (Firefox) during the wait, on top of
the 900 measured ones.

| browser | final hash | sequence digest | verdict |
| --- | --- | --- | --- |
| Chrome | `bfbb106aea5480f8` | `a190b60058a64e63` | MATCH |
| Firefox | `bfbb106aea5480f8` | `a190b60058a64e63` | MATCH |

The page, the worker and the verdict rule are `verify_common.PAGE`-family code
reused verbatim — `verify_common.WORKER` and `verify_common.check` — so this run
is judged by exactly the rule #327 and #342 were judged by, including its
`(?<!MIS)MATCH` guard. The fixture was still running when measurement began in
both browsers, which is recorded per run as `fixture_live_at_start=yes` rather
than inferred from both having been started.

### 3. Cross-runtime agreement

Chrome, Firefox and a node control that has no renderer in the process at all
must reach the same hash after the same tick count. They did, above. The node
control exists so that a hash agreeing between "rendering" and "not rendering"
*inside one browser*, but wrong in both, still gets caught.

### The worker starvation finding

The frozen fixture takes about **3.3 s** of CPU on an idle machine. Behind this
page's render loop it took **175.6 s in Chrome and 176.6 s in Firefox** — a
~53x slowdown, near-identical in both engines, on an uncontended 16-core
machine.

That is a property of the *benchmark* loop and not of the payload: `scene.js`
drives itself from a `MessageChannel` port with a `gl.finish()` in it, which is
deliberately unthrottled so frame time is free to fall below the refresh
interval. An unthrottled main thread starves the worker.

It is recorded because a shipped build would put the *simulation* in that worker.
Under `requestAnimationFrame` the main thread yields between frames and the
problem does not arise in the same form, but nothing here has measured that, and
"the renderer can starve the simulation thread by ~50x if it does not yield" is
the kind of thing that is much cheaper to know now than to discover in a match.
An earlier contended run of this same matrix exceeded a 300 s deadline in
Firefox on exactly this and correctly refused to publish; the deadline is now
900 s and a run that hits it is a reported failure, never a silent pass.

---

## Caveats a reader must have

- **Feature sets differ, in both directions.** The Babylon frame pays a shadow
  pass, which is why draw calls double; the LÖVE baseline pays bloom and draws a
  2D contact ellipse instead of a shadow. Neither draws the other's effects.
  The comparison is indicative, not like-for-like, and the asymmetry does not
  point consistently one way.
- **Pose-family coverage is 19 of 32, and the gap favours Babylon.** The live
  fixture is #100's, which runs combat disabled, so all seven `combat_*`
  families — priority 80–90 in `render/player_pose.lua`, i.e. selected *often*
  in a match with combat on — never appear, and neither does `aerial_bicycle`,
  which maps to the most elaborate clip in the mapping.
  `docs/design/babylon_skinned_benchmark.md` documents this in full. A capture
  exercising the combat band would make Babylon's per-character cost higher, not
  lower. Since the finding here is already unfavourable to Babylon, the gap
  strengthens it — but 59% coverage is not a neutral sample and must not be read
  as one.
- **The sample windows differ.** The Babylon rows are 600 measured frames (~10 s
  of match) and the LÖVE rows are 1800 (~30 s). Both disable vsync and both emit
  the same summary fields, so the comparison is not invalid — but a shorter
  window has fewer chances to catch a rare stall, so read the Babylon `max` and
  `over33` columns as less complete rather than as better.
- **Whole-pitch framing, so this is submission-bound rather than fill-bound.**
  The camera is identical in every configuration and frames the whole pitch. At
  105 m across a 960-wide target a 1.8 m character is about 16 px. Nothing here
  says anything about what happens at a broadcast zoom, where per-character
  shading would start to matter and the two renderers could diverge again.
  The native LÖVE baseline frames the whole pitch too.
- **`update` is not `love.update`.** Stated in the verdict, repeated here
  because it is the single easiest number in this document to misread: the
  Babylon `update` column carries the payload build and the character placement
  as well as the tick.
- **The simulation is on the render thread.** A shipped build would not put it
  there. This makes the Babylon `frame` column pessimistic and leaves the
  worker-transport cost — a `postMessage` clone per frame — unmeasured.
- **Counts above ten repeat poses.** Not exercised in this document, which
  measures ten, but the page supports more: past the roster's own ten, copies
  read a *different slot of the same live frame* rotated elsewhere on the pitch,
  because a live simulation has exactly one current frame. #341's copies could
  read a different point in *time* because its whole capture was on disk. Every
  skeleton is still independently posed; the set of poses on screen repeats every
  ten characters. At ten — the count #328 is accountable for — every character is
  its own player and the caveat does not apply.
- **Firefox's GPU string is sanitised**, as noted in the method.
- **Ordering within a pass is fixed.** Three interleaved passes with a 300-frame
  warm-up remove most session drift, but the variants are visited in the same
  order inside each pass, so a monotonic drift would land at the same relative
  position every time. Shuffling per pass would close it properly.
- **`merged_all` is a benchmark transform, not an authoring pipeline.** It gives
  the gear rigid attachment, which is what it already had. Real authored weights
  would let a cape deform, cost the same per frame, and belong to #318. The bake
  also assumes no non-uniform scale in the bone chain — the pack has none, and
  the code throws rather than guess if the vertex layout is not the one it knows
  how to append to.

---

## The gates

Per AGENTS.md §9, and named for what they actually run.

| gate | in `check.sh` | in `ci.yml` | what it runs |
| --- | --- | --- | --- |
| `babylon_wasm_bench.py --self-test` | yes | yes | the runner's verdict logic. **No browser, no wasm, nothing rendered.** |
| `babylon_bench.py --self-test` | yes (pre-existing) | yes | the software-rasteriser refusal, which this runner imports rather than copies |
| `babylon_bench.py --prove-refusal` | no — by hand, on the measuring machine | no | a real Chrome forced onto SwiftShader, refused |
| `babylon_wasm_bench.py` (no flags) | no — by hand, needs a GPU, a display, two browsers and a built module | no | this document's measurement |

The self-test is **not** coverage of the benchmark. It drives every rule that
decides the verdict, and drives each one red as well as green: a crossing rate
that is not exactly 1.00, a non-payload crossing, two disagreeing state hashes, a
page that reported no hash at all, disagreeing runtimes, a frozen-contract run
with the wrong hash or a `DIVERGED` verdict or no output at all, a
software-rasteriser or masked GPU string, a partial matrix reaching the
evidence-of-record filename, and a run with failures exiting 0. Sabotage any of
those and the step goes red.

The last item is deliberate. A previous harness in this campaign printed
`OVER BUDGET` and then exited 0 with `PAYLOAD OK`; `exit_code()` here is a named
function with its own self-test assertion so the same shape cannot recur
unnoticed.

`report.json` means the whole matrix ran and every rule held. A run with any
failure writes `report_incomplete.json` instead, deletes any stale
`report.json`, and exits non-zero — so a reader who forgets to check the exit
code cannot pick up a partial matrix and believe it.

## Assets

The Babylon bundles and the character are fetched on demand and verified against
pinned SHA-256 hashes, never committed, exactly as #341 does it. The wasm module
is built by `wasm/sim-host/build.sh` and is not committed either. See
THIRD_PARTY.md for the licences.

## Out of scope here

IK (#318), the worker-transport arrangement a shipped build would use, native
packaging (#329), feature parity with the LÖVE renderer, and the decision itself
— which is #330's, and which this document is evidence for rather than a
substitute for.
