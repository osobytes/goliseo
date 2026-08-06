# Migrate the presentation layer to Babylon, or optimise LÖVE (#330)

Status: **evidence assembled — the decision line below is deliberately unfilled and
awaits the repo owner.** Assembled 2026-08-04 against `c181c55`.

Synthesises [`babylon_wasm_spike.md`](babylon_wasm_spike.md) (#328),
[`babylon_skinned_benchmark.md`](babylon_skinned_benchmark.md) (#341) and
[`native_route_decision.md`](native_route_decision.md) (#329), plus
[PR #350](https://github.com/osobytes/goliseo/pull/350) and
[PR #339](https://github.com/osobytes/goliseo/pull/339) (#337, the optimised-LÖVE
renderer), [PR #346](https://github.com/osobytes/goliseo/pull/346) (#332) and
[`wasm_webview_determinism.md`](../online/wasm_webview_determinism.md) (#342).

---

## The decision

> **Decision:** _(unfilled — migrate / stay / defer)_
> **Decided by:** _(repo owner)_
> **Date:** _____
> **Reasoning:** _____
> **What would reverse it:** _____

**This is blank on purpose, and it is the one thing in this document that no
agent should fill in.**

A decision *was* recorded on this issue on 2026-08-04, on behalf of the repo
owner: drop LÖVE as the 3D renderer entirely, native as well as web, and move
the pitch to Babylon. That comment named two assumptions it rested on and said
both were unproven. **Both have since been measured, and neither survived**
(§2). It also stated that the optimised-LÖVE column "will not be produced" —
and #337 produced it (§1, row 3).

So the ground the recorded decision stood on has moved in three places at once.
Reaffirming it or reversing it are both calls for the owner, not for the agent
that assembled the evidence. What follows is the three-way comparison the issue
asked for, the current state of every reversal condition the recorded decision
named, and **both** contingency branches written out in full, so that whichever
way the line above is filled in, the next step is already sequenced.

---

## What a player gets out of this

Nothing directly, and nothing is blocked from a player's point of view today:
the game renders ten rigged players inside budget on native LÖVE right now
(§1, row 3). What this decides is which renderer the *next* two years of visible
work is built on — the animation quality of a keeper's dive and a striker's
first touch (#101, #102, #318), the character variety on the pitch (#115, #116),
and whether the online client keeps its current browser host or gets a new one
(§7). Getting it wrong is expensive in months of work that a player never sees.

---

## 1. The one table

This is the side-by-side the issue's first acceptance criterion asks for:
Babylon, current LÖVE and optimised LÖVE in one place. Ten players in every row.

| # | configuration | host | draw calls | marginal calls/char | draw p50 (ms) | draw p95 (ms) | added draw p95 vs procedural | update p95 (ms) | frame p95 (ms) | source |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 1 | LÖVE procedural (the baseline the budget is measured against) | native | 14.0 | — | 0.599 / 0.634 | 1.054 / 1.133 | — | 0.305 / 0.305 | 2.441 / 2.459 | #328 |
| 2 | **LÖVE rigged, current** | native | **331.6** | 31.76 | — | 2.74–2.80 | **89% of 2 ms** | — | — | #328 issue text, measured 2026-08-03 |
| 2b | LÖVE rigged, immediately before slice 2 (`edd0d61`) | native | 303.4 | 28.94 | — | 3.0732 | 1.8243 ms — **91.2%** | — | — | PR #350 |
| 3 | **LÖVE rigged, optimised (#337)** | native | **33.4** | **1.94** | 0.952 / 0.913 | **2.300 / 2.276** | 1.246 / 1.143 ms — **62% / 57%** | 0.345 / 0.338 | 3.666 / 3.684 | #328 re-measure |
| 3b | LÖVE rigged, optimised — PR #350's own paired run | native | 33.4 | 1.94 | — | 2.4189 | 1.1324 ms — **56.6%** | — | — | PR #350 |
| 4 | **Babylon `merged_all`** | Chrome 150 | **47** | **2.00** | 1.130 | **1.675** | n/a | 1.665 | 5.785 | #328 |
| 5 | **Babylon `merged_all`** | Firefox 153 | **47** | **2.00** | 2.120 | **3.440** | n/a | 1.880 | 12.860 | #328 |
| 6 | Babylon `merged` | Chrome 150 | 87 | 6.00 | 1.350 | 2.010 | n/a | 1.685 | 6.455 | #328 |
| 7 | Babylon `merged` | Firefox 153 | 87 | 6.00 | 2.600 | 3.740 | n/a | 1.840 | 12.640 | #328 |
| 8 | Babylon `authored` | Chrome 150 | 187 | 16.00 | 1.695 | 2.915 | n/a | 1.710 | 7.870 | #328 |
| 9 | Babylon `authored` | Firefox 153 | 187 | 16.00 | 3.040 | 4.860 | n/a | 1.900 | 14.480 | #328 |
| 10 | Babylon `merged`, native shell | Electron 43.3.0 | 87 | 6.00 | 1.23 | 1.96 | n/a | — | 4.09 | #329 |
| 11 | Babylon `merged`, native shell | Tauri / WebKitGTK | 87 | 6.00 | 5.42 | 6.22 | n/a | — | 20.44 | #329 |

**How to read the columns, because three of them are easy to misread.**

- **`draw calls`** is the only exact, deterministic column. Every other number in
  this table is a timing, with a pass-to-pass spread of 2–21% of the median
  (#341). Where a row shows two values separated by `/`, those are two paired
  runs, not a range of a distribution.
- **`marginal calls/char`** is the cost of one *more* character, not the
  amortised total. Babylon's scene has a fixed 27 calls for pitch, goals and
  ball, and every character is drawn twice because of the shadow pass:
  `27 + characters × meshes × 2`. LÖVE's marginal 1.94 is `(33.4 − 14.0) / 10` —
  one character mesh plus roughly 0.94 for the 2D ground-contact ellipse and the
  controlled player's selection rings.
- **`added draw p95`** is the #100 feature-delta budget: rigged draw p95 minus
  procedural draw p95 in the *same run*, against a ≤2 ms target. It exists only
  for the LÖVE rows, because there is no procedural Babylon scene to subtract;
  the Babylon cells say `n/a` rather than being left ambiguous.
- **`update p95` is not comparable between hosts.** LÖVE's 0.345 ms is one
  simulation tick. Babylon's 1.665 ms is a tick *plus* the rollback snapshot
  capture, the `RenderFrame` build, the encode into linear memory, the
  JavaScript-side read and the placement of ten characters. See §2.3.
- **`frame p95` is not comparable between hosts either.** Babylon's brackets a
  `gl.finish()`; LÖVE's does not.

**Rows 2 and 2b are two different "before" measurements and both are real.**
331.6 calls / 89% of budget is the 2026-08-03 measurement quoted in #328's issue
text and carried into #341's and #329's documents. 303.4 calls / 91.2% is
PR #350's own paired before-run at `edd0d61`, i.e. after slice 1 landed, at load
average 3.70–4.54. They are not in conflict; they are different commits under
different load. The honest one-line summary of the optimisation, which both
support, is **331.6 → 33.4 draw calls, and added draw p95 from ~90% to ~57% of
the 2 ms budget.**

### The floor-to-floor reading

Rows 3, 4 and 5 are the comparison that matters, because they put both renderers
at *their own* minimum mesh count. Quoting LÖVE's 33.4 against Babylon's 87
(row 6) is not floor to floor: `merged` collapses only Babylon's *skinned*
meshes, leaving the helmet and cape standing, while #337 slice 2 folded all 28
rigid parts including gear into one mesh. `merged_all` (#328) closes that gap by
baking the bone-parented gear into the skin at weight 1.

- **Draw calls, floor to floor, are a tie:** **2.00 per character for Babylon
  against 1.94 for optimised LÖVE.** Both put the character in exactly one mesh
  and one call; the remainder is a second pass in each case, and the two second
  passes are not the same feature. Babylon's is a real shadow-map render of the
  character. LÖVE's ~0.94 is a 2D ground-contact ellipse plus selection rings.
  **At equal draw-call cost Babylon draws strictly more** — a point for Babylon
  on quality, and not a point for it on headroom.
- **On draw time alone Chrome is ahead:** 1.675 ms against optimised LÖVE's
  2.276–2.300 ms, i.e. Babylon in a browser draws in about **73%** of a native
  LÖVE renderer's CPU draw time. #328 reads this as parity rather than a win,
  because the pass-to-pass spread on both sides is a substantial fraction of the
  margin and the two frames are not drawing the same things.
- **Firefox is the weakest Babylon result.** Draw p95 **3.440 ms** is about
  **1.5x** optimised LÖVE's, and frame p95 **12.860 ms is 77% of a 16.67 ms
  frame** with ten characters, a pitch and nothing else on screen, on an
  uncontended RTX 2070 SUPER. Chrome's 5.785 ms is 35% of the same frame.

### Whole-frame CPU, and why the gap is not the renderer

| | update p95 | draw p95 | sum | of a 16.67 ms frame |
| --- | ---: | ---: | ---: | ---: |
| LÖVE rigged, optimised (native) | 0.345 | 2.300 | **2.64 ms** | 16% |
| Babylon `merged_all` (Chrome) | 1.665 | 1.675 | **3.34 ms** | 20% |
| Babylon `merged_all` (Firefox) | 1.880 | 3.440 | **5.32 ms** | 32% |

Reproduced from `babylon_wasm_spike.md`'s table, including its rounding: the
first row's exact sum is 2.645 ms and that document prints 2.64.

The ~5x `update` gap is the simulation running as PUC Lua 5.1 under wasm rather
than as LuaJIT, **not** the payload boundary. #328 decomposed it with five runs
of `node wasm/sim-host/verify_payload.js`: one tick with its snapshot capture
cost **0.6210–0.6694 ms mean** and the whole payload extraction **0.1353–0.1442 ms
mean**, so the simulation is about **4.5x** the boundary. PR #346's measurement
table puts the boundary at **0.2000 ms p95 in Chrome (2.5% of the 8 ms update
budget) and 0.3000 ms p95 in Firefox (3.8%)** — both marked `*` in that table,
meaning they are p95 over batch means rather than per-call samples and are
therefore **lower bounds** on the true per-call p95. (#346's own summary prose
says "0.25–0.31 ms p95 … about 3–4%", which does not match its table; the table
is quoted here. See §9.)

That cost is paid by *any* browser renderer that hosts the simulation in wasm,
including a love.js one. It is #327/#332 territory, not Babylon's, and it does
not belong in either side of this decision.

---

## 2. The two assumptions the recorded decision rested on

The 2026-08-04 comment stated both as assumptions and named the issue that would
prove each. Both are now measured.

### 2.1 "Babylon handles skeletons materially better than LÖVE" — falsified (#341)

#330's own stated test was: *a materially flatter marginal cost per character
supports the assumption; curves rising in parallel with Babylon merely lower
means the win is constant overhead, not skeleton handling.* The measurement is
unambiguously the second one.

Marginal cost of one more character, 10 → 20 against 20 → 40:

| browser | variant | 10 → 20 | 20 → 40 | change |
| --- | --- | ---: | ---: | --- |
| chrome | authored | 328.5 µs | 348.8 µs | +6% |
| chrome | merged | 235.5 µs | 257.8 µs | +9% |
| firefox | authored | 408.0 µs | 462.0 µs | +13% |
| firefox | merged | 338.0 µs | 337.0 µs | −0.3% |

Nothing flattens. **Draw calls scale exactly linearly: 16.0 per character in
every `authored` step and 6.0 in every `merged` step, at 10, 20 and 40, in both
browsers.** No amortisation with count.

The honest bound #341 puts on its own timing column, which must travel with the
finding: the percentage changes above sit *inside* the 2–21% three-pass spread,
so the defensible claim is **"flat, not flattening"**, not "steepening". The
draw-call column is exact and carries the result on its own.

The `merged_static` control says the same from the other side: strip blending
entirely and per-character cost falls by a fifth to a quarter, but the curve
keeps its shape. Linearity is the base cost of an independently posed skinned
character, and Babylon does not amortise it away.

### 2.2 "BabylonNative works well enough to carry the native build" — falsified for now (#329)

Babylon Native builds, and its capability spike passes every check this game
needs, including the one Babylon was chosen for: `BoneIKController` attached to
`upperarm.l` and its end effector travelled **0.29209–0.29283 units** to follow a
moved target, in **5 of 5 runs**, across two independently configured builds.
That is real evidence for the bought-not-built side and it is not damaged by
what follows.

What fails is the shipping case:

- Its own pixel-comparison validation suite **segmentation-faults after 9 of 720
  tests**, and again after 196 more once the first crash is skipped — about
  **205 of 720 (28%)** runs before the process dies. Reproducible across two
  builds, both GL paths, and the upstream CI's own compile flags. Upstream CI is
  green on the same commit; #329 does not claim an upstream regression, and it
  names the one untried experiment (`xvfb-run`) that would discriminate
  "upstream is immature" from "this environment differs".
- **0 GitHub releases; 4 version tags, all dated 2020-06-08.** README: "public
  preview in source form only", with no backward-compatibility contract.
- **No audio and no particles at all.** `docs/design/sound.md` is a committed
  design and the combat telegraphs need particles.
- Single-pointer input only; no packaging story, so there is no installer to
  measure.

### 2.3 What moved underneath both of them

**Optimised LÖVE now exists and is better than the number the recorded decision
was taken against.** #337 slice 2 (PR #350) draws a whole rigged character in
**one draw call**: 331.6 → 33.4 for ten players, added draw p95 from ~90% to
**56.6%** of the 2 ms budget, both team palettes on screen simultaneously at one
draw call per character. #328 independently reproduced 33.4 and 1.94 exactly and
bracketed the added-draw figure at 1.143–1.246 ms (57–62%).

A colour variant costs **zero extra meshes and zero extra draw calls** — one
uniform upload. A *geometry* variant still costs its own merged mesh, and
PR #350 records that **the same constraint applies under Babylon**.

---

## 3. Where the recorded decision's own reversal conditions stand

The 2026-08-04 comment named three. Their current state, stated without
interpretation:

| Reversal condition, as written | Status |
| --- | --- |
| "#328 showing Babylon's skinned-character cost is not materially better than LÖVE's at the same character count." | **Met.** Floor to floor, 2.00 draw calls per character against 1.94 (§1). #341 separately showed the curve does not flatten (§2.1). |
| "#329 finding BabylonNative unviable **and** no acceptable native alternative (Electron/Tauri), leaving no native story." | **Not met.** The first half holds (§2.2). The second does not: #329 recommends **Electron**, measured at **1.23 ms draw p50 / 1.96 ms draw p95** for the same 87 draw calls — *faster* than the Chrome browser row — reaching a rendered scene in **876.6 ms** (median of five interleaved launches, span 839.0–1132.0), packaged as a **94.2 MiB** AppImage on the first attempt. Ruling out Babylon *Native* is not ruling out Babylon on a Chromium shell. |
| "The port cost proving out of proportion once #326's boundary meets a real renderer." | **Not met on the boundary.** #328 drove Babylon live from the wasm payload at **exactly one crossing per rendered frame, min 1 / max 1 over 900 frames**, zero non-payload crossings, 266 words / 2128 bytes for ten players. The boundary met a real renderer and held. The port cost of `game/` itself is a separate question — §4.2. |

---

## 4. What the migration buys, and what it costs

### 4.1 The trade is the animation pipeline, not determinism

Phase 0 settled determinism: the Lua simulation is kept either way, and #342
showed the same `simhost.wasm` produces byte-identical hashes under V8,
SpiderMonkey, JavaScriptCore and node. Nothing in this decision touches the
simulation.

What a migration buys is an **animation pipeline that is bought rather than
built**, against four issues that currently plan to hand-write it:

| issue | hand-written under LÖVE | bought under Babylon |
| --- | --- | --- |
| **#318** — presentation-only IK | A pure two-bone analytic solver in `core/`, plus a post-FK pass that rewrites affected bones and recomputes their subtrees | `BoneIKController` — measured working **natively** (#329 §1.4, 5/5 runs). **Not exercised in either browser benchmark**: #328 and #341 both list IK as out of scope. |
| **#101** — outfield soccer clip library | Clip authoring plus the controller that plays it | Clip authoring against a real animation system: GPU skinning by default, documented glTF retargeting |
| **#102** — goalkeeper clip library | as above | as above |
| **#115** — six character presentations | Conform assets to a rigid-part-per-bone rig | A skinning pipeline: skinned glTF characters loaded directly |
| — | `rig3d/masks.lua` hand-rolls bone masking | Built-in bone masking since Babylon 7.0 |

Two qualifications that belong beside that table, both from the issues
themselves:

- **#318's known limit is a genuine Babylon advantage.** The LÖVE rig has *no
  per-vertex skin weights* — it is a rigid part per bone by design — so IK that
  flexes a knee or shoulder hard makes rigid parts visibly separate, which caps
  how far a constraint can pull. A skinned glTF character does not have that
  cap.
- **#318's solver is not open-ended work.** Its own body records that
  `thigh → shin → foot → toe` and `upper_arm → forearm → hand` are clean
  two-bone chains with no in-chain twist bones — "exactly the topology
  closed-form two-bone IK wants — law of cosines, no CCD/FABRIK iteration" — and
  that `core/mat4.lua` and `core/quat.lua` are already pure Lua. The *solver*
  is the small half of #318 either way; the constraint set (foot-plant latch,
  dribble contact, keeper hands, head look-at, release-on-rollback-correction)
  is project work under both renderers.

And one more, from #338: the rigged renderer's action poses are **parameterised
root transforms driven by simulation timers**, not keyframe clips, precisely
because every one of them is continuous and reads a timer directly. #338 states
that "that rationale and those numbers are the spec any renderer implements,
Babylon included" — so that tuned content is not lost by a migration, and not
bought by one either.

### 4.2 The cost: `game/` measured, not estimated

The issue and the recorded decision both describe `game/` as "41,825 lines
across 116 files, all presentation". Recounted at `c181c55`:

**42,914 lines across 116 files** — and it is *not* all presentation.

| directory | lines | files | lines in files that mention `love.` | `love.graphics` refs |
| --- | ---: | ---: | ---: | ---: |
| `game/online` | 18,058 | 26 | 2,460 (2 files) | 0 |
| `game/screens` | 5,444 | 19 | 1,912 (4 files) | 24 |
| `game/render` | 4,831 | 15 | 3,435 (9 files) | 344 |
| `game/` (top level) | 4,284 | 21 | 825 (3 files) | 0 |
| `game/transport` | 3,990 | 6 | 1,080 (2 files) | 0 |
| `game/render/rig3d` | 3,902 | 15 | 1,074 (3 files) | 13 |
| `game/presentation` | 1,012 | 4 | 0 | 0 |
| `game/ui` | 794 | 7 | 525 (2 files) | 65 |
| `game/input` | 599 | 3 | 487 (1 file) | 0 |
| **total** | **42,914** | **116** | **11,798 (26 files)** | **446** |

Three things fall out of that, all checkable by `grep` — the file total is
`grep -rl 'love\.' game --include='*.lua' | wc -l` → **26**:

1. **90 of 116 files never mention `love.` at all.** 11,798 lines (27.5%) live in
   a file that does. That is a coarse proxy for engine coupling — a file can be
   engine-bound without the literal token, and a file with one
   `love.timer.getTime()` call is not wholesale engine-bound — but it is a
   measured proxy rather than an estimate.
2. **`game/online` is the largest directory in `game/` at 42% of it, and is
   effectively engine-free.** Of its 26 files exactly two mention `love.`: one is
   a comment, and the other is
   `if love ~= nil and love.timer ~= nil and love.timer.getTime ~= nil then` with
   a non-LÖVE fallback. The netcode does not have to be rewritten for a renderer
   change; it has to be *re-hosted* if the runtime host changes (§7).
3. **344 of 446 `love.graphics` references are in `game/render`'s 9 files.** The
   renderer surface is concentrated, not diffuse. Adding `game/render/rig3d`,
   `game/ui` and `game/screens` accounts for all 446.

Also measured, correcting a detail of the recorded decision: `game/screens`
holds 19 *files* (not 19 screens — four of them are models and fixtures), and
**two** of them require the pitch renderer: `match.lua` and
`combat_feedback_fixture.lua`.

What is **not** quantified, and should not be guessed at: the person-time to
port 344 `love.graphics` call sites and 19 screens to a DOM/Babylon UI, and the
cost of re-hosting the 1981-spec `love . --test` harness. Nobody has costed
either, and this document does not.

---

## 5. Contingency: if the owner decides **migrate**

### 5.1 What closes, or changes shape, as bought-not-built

Nothing on this list should be closed on the strength of this document alone;
each is what the acceptance criterion asks to be *identified*.

| issue | disposition under migrate |
| --- | --- |
| **#94** Pin and isolate Menori behind a typed rendering adapter | **Closes as moot.** Menori is not the renderer. |
| **#95** Create a benchmark-ready Rig_Medium GLB | **Largely bought.** Babylon loads glTF directly; what remains is asset authoring, not a loader. |
| **#97** Shared rigged-player asset and instance management | **Bought.** Babylon owns instancing and resource sharing. |
| **#98** Presentation-only rigged animation controller | **Mostly bought.** Blending, masking and clip evaluation are engine features; the *semantic* mapping (pose family → clip) stays project work. |
| **#96** Depth-aware bloom for hybrid 2D/3D | **Re-scoped** onto Babylon's post-process pipeline. |
| **#318** Presentation-only IK | **Re-scoped to Babylon `BoneIKController` integration.** Does not close: the constraint set and the rollback-release behaviour remain project work (§4.1). |
| **#101 / #102** Outfield and goalkeeper clip libraries | **Re-scoped** to clip authoring against a real animation system. The volume of authoring does not shrink; the runtime that plays it is bought. |
| **#115** Six character presentations | **Re-scoped** onto a skinning pipeline instead of conforming to a rigid-part rig. |
| **#116** Six equipment presentations | **Re-scoped**; attachment becomes bone-parenting in glTF. |
| **#100** Ten-player native and browser benchmark | Its browser leg becomes a Babylon benchmark rather than a love.js one; #328 and #341 already supply most of it. Record `revise` or supersede. |
| **#340** Rigged renderer untested | **Does not go away.** It becomes the same question about the Babylon renderer, whose only coverage today is the two bench runners. |
| **#338** Rescue rigged pose and species content | **Still worth landing** — its own body says the rescued content is engine-independent and survives the migration. |
| **#337** Optimise the LÖVE renderer | Already closed. Its data-model half (palette slots, per-vertex bone indices) is what a Babylon port consumes too, per #337's own body — not throwaway. |

### 5.2 Sequencing for `game/`

Ordered so that each step is independently landable and the game keeps running:

1. **`game/render` + `game/render/rig3d`** — 8,733 lines, 12 files that touch
   `love.`, 357 of the 446 `love.graphics` references. This is the renderer swap
   proper. `render/` (the pure payload layer) is untouched: #326 already made
   `pitch.draw` consume only the versioned `RenderFrame`, and #328 proved a second
   renderer can drive from that same payload at one crossing per frame.
2. **The desktop shell** — Electron (#329), so the native and browser builds share
   one V8/Chromium surface and #341's browser numbers transfer instead of needing
   to be retaken.
3. **`game/screens` + `game/ui`** — 6,238 lines across 26 files (19 + 7), 89
   `love.graphics` references concentrated in 5 of them; only 2 of `game/screens`'
   19 files draw a pitch. AGENTS.md §9's model/layout/update split means
   `layout`, `update` and `hit` are already pure and portable; only `draw` is
   engine-bound.
4. **`game/input`, `game/audio`, timing, and the test harness** — 599 lines of
   input plus the `love.*` timing and filesystem calls scattered through the top
   level, and the `love . --test` runner that executes **1981** specs at
   `c181c55` (the recorded decision's "1896" is stale). This is the step nobody
   has costed.
5. **`game/online` + `game/transport`** — 22,048 lines, 51% of `game/`, and
   effectively engine-free (§4.2). A host change, not a rewrite — but see §7.

### 5.3 What must survive the migration

- AGENTS.md §2's layering. #336 already generalised it to forbid rendering
  engines by category rather than by naming `love`, so the rules survive without
  further edits.
- The frozen determinism contract `bfbb106aea5480f8` / `a190b60058a64e63`, which
  #328 verified holds in a worker while the page renders, in both browsers.
- #325 (bot-driven hash divergence between Lua VMs) stays open either way; the
  wasm host is the PUC Lua 5.1 arm of it.

---

## 6. Contingency: if the owner decides **stay**

### 6.1 The optimisation work that is scheduled

#337 is closed and its two slices have landed. What remains named and unscheduled:

- **Back-to-front part sorting was deliberately not implemented**, and PR #350
  records why: parts merged into one static mesh drawn in one call cannot be
  reordered by reordering draws. Sorting only ever bought self-occlusion
  *without* a depth buffer — i.e. the WebGL 1 / love.js case. The mechanism that
  restores it without giving back the single draw call is documented in
  `renderer.beginPass`: keep the one static vertex buffer and call
  `Mesh:setVertexMap` per frame with each part's index range concatenated
  back-to-front, one index-buffer upload per character per frame, still one draw
  call. **This becomes scheduled work only if the browser LÖVE path is revived.**
- **The browser leg has never been run.** PR #350's residual risk says it
  plainly: the uniform budget and float-attribute choices are written for
  GLSL ES 1.00 and asserted against its guaranteed 128-vector floor, but nothing
  has been *run* on love.js. "The WebGL 1 reasoning is a design constraint
  honoured, not a result measured."
- **#340's remainder.** PR #350 added tier-1 specs and a tier-4 visual gate that
  provably execute and provably go red (§8), but nothing exercises
  `player_renderer_3d.draw`'s pose → `boneRows` → `renderer.draw` path headlessly.
- **#338** lands the rescued action poses and species presentation.
- **#93–#99, #101, #102, #104, #115, #116, #318** proceed as written, with #318's
  rigid-part flex cap (§4.1) becoming load-bearing rather than theoretical.

### 6.2 What #100 records

#100's three allowed outcomes are `proceed`, `revise` and `stop`. The evidence
now available, stated without choosing for it:

- **The native leg passes.** Added draw p95 is 1.1324 ms (56.6%) or 1.143–1.246 ms
  (57–62%) against a ≤2 ms target, and added update p95 is **0.033–0.040 ms**
  (0.345 − 0.305 and 0.338 − 0.305) against a ≤1 ms target — both inside, on an
  RTX 2070 SUPER.
- **The browser leg is deliverable, and the recorded reason it was not is
  wrong.** #100's required runtime matrix is LÖVE 11.5 native Linux *plus the
  supported Chrome and Firefox love.js builds*. #328 and #338 both record that
  love.js is WebGL 1 and cannot supply a `DEPTH24_STENCIL8` depth attachment.
  #360 measured it (`docs/online/browser_rigged_3d.md`) and the blanket claim is
  **false**: love.js supplies a depth attachment, LÖVE's rig3d shader links, and
  ten rigged players render. What is true is much narrower — love.js refuses the
  *packed 24-bit* format `bloom` used to hardcode and offers `depth16`, which is
  all the 3D pass ever needed, so the fix was one list in `bloom.lua` rather than
  a renderer.

  **Firefox was excluded in an earlier revision of this bullet, and no longer
  is.** That text said Firefox was out for an unrelated defect — a LÖVE shader
  declaring a `varying` did not compile there, and asking for one aborted the
  runtime instead of falling back — which made #391 a release blocker for the
  browser build. It was measured and it is now fixed (#395: hoist those
  declarations above `main()` at the WebGL boundary, and stop declaring
  vertex-only uniforms in both stages at mismatched precision). #360 re-measured
  afterwards on a single tree carrying both halves: the browser leg passes in
  **both** browsers. The rigged default is on, and there is no release blocker
  under it.
- #100 forbids silently loosening a gate: "any revised threshold requires the
  original failure evidence, measured bottleneck, bounded optimization,
  tradeoff, and before/after rerun". PR #350 supplies exactly that shape of
  evidence for the *draw-call* bottleneck.

The reading those three points support is still **`revise`**, but no longer with
a narrowed runtime *matrix* — that narrowing was Firefox, and Firefox is back.
#100's full matrix runs: native Linux inside the measured envelope, and both
browsers rendering rigged players over #100's added-draw budget — Chrome at 149%
of it, Firefox at 200%. Firefox is also the one row that crosses an *absolute*
omp0 gate, at `draw p95 8.64 ms > 8 ms` under contention, though it comes in
around 7.0 ms on an idle box. What is left to revise is therefore the added-draw
budget, measured on a browser runtime against a procedural renderer being
retired, with Firefox's absolute headroom as the tighter of the two constraints.
`stop` is contradicted by the native numbers and now by both browsers.
**Writing that record is #100's, and the owner's.**

---

## 7. Multiplayer re-sequencing, both ways

#330 "blocks resuming online multiplayer work". Open at `c181c55`: **OMP-3 4
issues, OMP-4 5, OMP-5 18, OMP-6 0.**

The finding that applies **whichever way the decision goes**, from PR #346
(#332): in an eight-tick rollback burst the **resim is 96–98%** and the payload
is 3–5%. The burst fits on a quiet machine — 5.25 ms p95 with 34% headroom at
load ~1.3–2.9, 6.49 ms with 19% at load ~4.0 — and goes **over the 8 ms budget
under contention: 10.4 ms p95 in Firefox, 13.1 ms in node.** *Rollback depth is
the thing near the line, not rendering.* That is a milestone-level constraint on
OMP-5's #295 (input delay / rollback window sweep) and #296 (long-duration soak)
regardless of renderer.

**If migrate.** The browser online client's host changes from the love.js
artifact to the wasm sim host plus a JS renderer. What that touches:

- Re-sequence **before** OMP-3's #169 (eight-client fault harness), #170 (real
  WebRTC performance) and #171 (OMP-3 evidence gate). Every browser online gate —
  `scripts/browser_determinism.py`, `scripts/browser_matrix.py`,
  `scripts/fault_harness.py`, `game/transport/browser.lua`,
  `game/transport/browser_star.lua` — is written against the love.js artifact and
  must be re-hosted before it can produce evidence again.
- The **determinism half is already de-risked**: #342 measured byte-identical
  hashes for the same `simhost.wasm` under V8, SpiderMonkey, JavaScriptCore and
  node, and #328 held the frozen 7201-tick contract in a worker while the page
  rendered 120,820 frames (Chrome) / 106,001 frames (Firefox). The **harness
  half is not**.
- OMP-4 (#246 relay server, #247 control plane) and OMP-5 are largely
  transport-and-protocol work in the 22,048 engine-free lines, so they are
  affected by *when* the host swap happens, not by *whether* Babylon draws.
- `docs/online/platform_decision.md` records browser-first for online
  development and would need amending to name the new browser artifact.

**If stay.** The online path is unchanged: the love.js artifact, the existing
browser evidence harnesses and `game/transport/browser.lua` all keep working, and
OMP-3 → OMP-4 → OMP-5 resume in their current order with no re-hosting step.
The renderer decision does not reach `game/online` at all. What does *not*
resolve on this branch is the browser rigged-player question (§6.2) — online
matches keep rendering with the procedural renderer in the browser.

**If defer.** #337 has already removed the urgency that made this a blocking
decision: the native renderer is inside budget today. The cost of deferring is
that #101, #102, #115, #116 and #318 stay blocked, because each is expensive
work whose target runtime is what this decides.

---

## 8. Caveats that bound everything above

These are not boilerplate. Several of them point the *opposite* way from the
numbers in §1 and every one of them is load-bearing.

- **Pose-family coverage is 19 of 32, and it favours Babylon.** Verified against
  `render/player_pose.lua`, which defines exactly 32 families in
  `player_pose.PRIORITY`. The 13 the capture never exercised are not a random
  sample: all seven `combat_*` families at priority 80–90 (`combat_knockback` 90,
  `combat_stagger` 89, `combat_guard` 84, `combat_active` 83, `combat_windup` 82,
  `combat_aim` 81, `combat_recovery` 80), plus `keeper_punt` (121), `keeper_tip`
  (110), `aerial_bicycle` (95, mapped to the most elaborate clip in the set),
  `kick_follow` (45), `fatigue` (20) and `keeper_ready_low` (15). The fixture is
  #100's, which runs combat disabled. **A capture exercising the combat band
  makes Babylon's per-character cost higher, not lower — and this is
  uncorrected.** Since §1's finding is already unfavourable to Babylon the gap
  strengthens it, but 59% coverage is not a neutral sample and must not be read
  as one.
- **Nothing here is like-for-like.** Babylon pays a shadow pass; LÖVE pays bloom
  and draws a 2D contact ellipse. Sample windows differ (600 measured frames for
  Babylon against 1800 for LÖVE), so read Babylon's `max` and `over33` figures as
  less complete rather than better. The camera frames the whole pitch, so at
  105 m across a 960-wide target a character is about **16 px** and the whole
  comparison is **submission-bound, not fill-bound** — nothing here says anything
  about a broadcast zoom, where per-character shading would start to matter and
  the two renderers could diverge again.
- **The optimised-LÖVE column's confidence is bounded by #340, which is open.**
  Until PR #350 no test in this repository had ever executed
  `game/render/rig3d/` or `player_renderer_3d.lua`: `player_renderer_3d.build()`
  calls `love.graphics.newShader`, which is nil headless, so the rigged renderer
  disabled itself and the suite reported a full pass. PR #350 partly closes that
  — tier-1 specs that `require` `meshbuilder`, `body` and `skeleton` directly and
  provably execute, demonstrated red against three faults injected into
  production code, plus a tier-4 visual gate demonstrated red against a corrupted
  baseline and against the one-mesh property broken in `body.lua`. **What remains
  uncovered is the integration**: `player_renderer_3d.draw`'s pose → `boneRows` →
  `renderer.draw` sequence is exercised only by the benchmark and by playing the
  game. The 33.4 and 1.94 figures come from that benchmark.
- **Draw calls come from LÖVE's own `getStats().drawcalls` counter**, not a GL
  trace. Same counter #100 used, so before/after are comparable; not independent
  instrumentation.
- **Firefox is the weakest Babylon result** (§1) and its GPU renderer string is
  sanitised to `NVIDIA GeForce GTX 980, or similar` for fingerprinting
  resistance — it proves *hardware*, not *which* hardware.
- **The Babylon simulation ran on the render thread.** A shipped build would put
  it in a worker, which makes the `frame` column pessimistic and leaves the
  worker-transport cost (a `postMessage` clone per frame) unmeasured. #328 also
  recorded that an unthrottled render loop **starved a worker by ~50x** (3.3 s of
  work took 170.0 s in Chrome and 183.9 s in Firefox) — a property of the
  benchmark loop, not the payload, but the kind of thing that is cheaper to know
  now.
- **One machine, one afternoon, one platform.** Every number in this document is
  a Linux 7.0.0-28-generic box with an RTX 2070 SUPER. No Windows and no macOS
  evidence exists at all, and #329's Tauri result is specifically a **WebKitGTK**
  result that would very likely not reproduce on Windows, where Tauri's webview
  is WebView2 — Chromium and V8, the same engine as Electron.
- **`native_route_decision.md` disagrees with itself on cold start.** Its §4
  point 3 says "Electron reaches a rendered scene in 988 ms; Tauri takes
  1829 ms", while its §3 table records `scene_ready` medians of **876.6 ms**
  (span 839.0–1132.0) and **1657.3 ms** (span 1624.3–1673.5) across five
  interleaved launches. 1829 ms is outside the table's own recorded Tauri span,
  and the table's pair is what its "1.89x cold start" ratio is computed from
  (1657.3 / 876.6 = 1.891). **§3's table is the figure to quote.** Reported here
  rather than silently picking one. Filed as
  [#358](https://github.com/osobytes/goliseo/issues/358).
- **PR #346 disagrees with itself the same way.** Its summary prose gives the
  payload boundary as "0.25–0.31 ms p95 … about 3–4% of the 8 ms update budget";
  its measurement table gives Chrome **0.2000 ms p95 (2.5%)** and Firefox
  **0.3000 ms p95 (3.8%)**. Neither reading changes the conclusion the boundary
  is nowhere near the line, and the table is what §1 quotes. Inherited from that
  PR, not introduced here.
- **Babylon Native's frame cost is unknown**, so the possibility that it is
  faster than Electron is untested. It does not change #329's recommendation — a
  route with no audio, no particles and no releases is not blocked on being fast.
- **`merged_all` is a benchmark transform, not an authoring pipeline**, and it is
  as shippable as LÖVE's equivalent and no more: both weld gear into the
  character mesh, so both need a rebuild to swap a cosmetic *geometry* piece.
  Neither is a per-frame cost.

---

## 9. What was verified for this document, and what was taken on trust

Numbers in this measurement area have been mis-stated repeatedly during this
campaign — this document found two more (§8: `native_route_decision.md`'s cold
start, and PR #346's payload boundary), and review found one **in this
document**, corrected below — so this section is part of the deliverable rather
than a footnote.

**The failure has one shape every time: prose restating a table from memory.**
Not a disputed measurement, not a methodology difference — a figure retyped a
paragraph away from the table it came from, and then travelling. Two instances
are recorded in §8; a third is this document's own, recorded below.
[#358](https://github.com/osobytes/goliseo/issues/358) raises the systemic point
across the seven known so far.

**Recommendation, made with the standing of having just done it by hand.** A
document like this one should not carry hand-transcribed figures at all. The
three benchmark runners already emit machine-readable evidence
(`.bench/babylon/report.json`, `.bench/native_shell/report.json`) whose filenames
are chosen so a partial matrix cannot be mistaken for a complete one, and the
`game/` composition numbers in §4.2 are one `grep` each. Both classes of figure
should be **generated into this document rather than typed into it**, with the
generator checked in and its output diffed in CI — which would turn the whole
class of error into a red build instead of a review catch. That is a real change
with a real gate and it is deliberately **not** in this PR: it is code, it needs
its own demonstration that it can go red (AGENTS.md §9), and this PR is
documentation-only. Filing it is the right next step regardless of how the
decision above lands.

**Recomputed from source at `c181c55`:**

- `game/` size and composition: **42,914 lines, 116 files**, the per-directory
  table in §4.2, the **26** `love.`-mentioning files and their line totals, and
  the 446 `love.graphics` references and their distribution. **The issue's
  "41,825 lines" is stale**; the file count is unchanged at 116.
  *Corrected in review:* an earlier revision printed a total of 28
  `love.`-mentioning files and therefore "88 of 116" — an addition slip over
  correct per-directory cells (2+4+9+3+2+3+0+2+1 = 26), not a
  matching-methodology difference. The figures are **26** and **90 of 116**.
- **32 pose families** in `player_pose.PRIORITY`, and every priority quoted in
  §8 read off `render/player_pose.lua` directly. 19 covered + 13 missing = 32.
- Two of `game/screens`' 19 files require the pitch renderer, not one.
- `love . --test` reports **1981 passed, 0 failed** at `c181c55`; the recorded
  decision's "1896 specs" is stale.
- Every derived figure in §1: the `27 + characters × meshes × 2` draw-call
  formula against all six Babylon rows; `(33.4 − 14.0)/10 = 1.94`;
  `(331.6 − 14.0)/10 = 31.76`; `(303.4 − 14.0)/10 = 28.94`; the added-draw
  percentages (1.246/2 = 62.3%, 1.143/2 = 57.2%, 1.1324/2 = 56.6%,
  1.8243/2 = 91.2%); the whole-frame sums and their percentages of 16.67 ms
  (2.64/16%, 3.34/20%, 5.32/32% — the first is 2.645 exactly, printed as 2.64 to
  match its source); Firefox 12.860/16.67 = 77%; the §2.1 marginal percentages;
  205/720 = 28%; and `1657.3/876.6 = 1.891`.

**Read directly from the artifact each number is attributed to** — every table
cell in §1 against `babylon_wasm_spike.md`, `babylon_skinned_benchmark.md`,
`native_route_decision.md`, PR #350's before/after table and PR #346's budget
table. Nothing is carried from another document's prose.

**Taken on trust, and flagged as such:**

- **The measurements themselves.** Nothing was re-run for this document — no
  browser, no benchmark, no Babylon Native build. This is a synthesis of runs
  recorded elsewhere, and it inherits every caveat those documents place on
  their own numbers.
- **The 89% figure in row 2** comes from #328's issue text describing a
  2026-08-03 run; no committed report for that run exists in `docs/`. Row 2b is
  the reproducible one.
- **love.js being WebGL 1 and unable to supply `DEPTH24_STENCIL8`.** Was
  recorded in #328's and #338's issue text and in
  `game/render/rig3d/renderer.lua`'s comments, and was not re-measured for this
  document. **It has since been measured and corrected** — #360,
  `docs/online/browser_rigged_3d.md`. love.js is WebGL 1 and does supply a depth
  attachment; it refuses `depth24stencil8` and accepts `depth16`. §6.2 above is
  updated; treat the original sentence as withdrawn wherever else it appears.
- **Babylon Native's segfault is not root-caused**, and upstream CI is green on
  the same commit. #329 names `xvfb-run` as the untried experiment that would
  discriminate.

---

## 10. Gates

**This change adds no gate, and none is warranted.** It is a document that
synthesises measurements taken elsewhere; it adds no code, no harness and no
runnable claim of its own. Per AGENTS.md §9, a gate would have to appear in both
`scripts/check.sh` and `.github/workflows/ci.yml` with a demonstration it can go
red — there is nothing here to make red.

The gates that already cover the evidence this document rests on, all of them
self-tests that start no browser and no shell, and none of which is evidence for
the measurements themselves:

| gate | in `check.sh` | in `ci.yml` |
| --- | --- | --- |
| `babylon_bench.py --self-test` | yes | yes |
| `babylon_wasm_bench.py --self-test` | yes | yes |
| `native_shell_bench.py --self-test` | yes | yes |
| `check_verify_webview.sh` | yes | yes |
| `check_rig3d_palette_snapshots.sh` | opt-in, needs a display | — |

`./scripts/check.sh` was run against this branch and passes.
