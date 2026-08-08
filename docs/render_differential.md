# Render differential: closing the gap the simulation port never had

## Context

The v2 migration ran five differential suites against the real Lua on the
determinism path (RNG, hashing, the wire format, diagnostics, desync
identity/input samples) — see `v2/tools/lua_reference/`. It never ran the
same discipline against **rendering**. `packages/render/src/pitch.spec.ts`
said so in its own header before this work: "No Lua spec targets
`game/render/pitch.lua` with a claimable, self-contained fixture." That gap
is why a defect could survive to a live NVIDIA/SwiftShader build (washed-out
team colours, wrong-feeling movement) with every other gate green.

This adds that gate — `v2/tools/render_reference/` — and uses it. It found
one confirmed, high-impact bug, two smaller confirmed divergences, and ruled
out the leading hypothesis (camera/scale) outright.

## What the harness covers, and what it deliberately does not

Full detail and the "why" for each boundary is in
`v2/tools/render_reference/README.md`. Summary:

**Covered, full command-level diff:** the entire static pitch scene (arena
backdrop, floor trapezoid + glow, hex tiling, markings, goal nets/frames,
outline, arena frame chevrons), the loose ball (position, shadow, height
lift), and the full per-frame overlay (landing reticle, pass-target preview,
charge meter). Comparison is a **content** (multiset) match for the static
scene — pitch.ts's own code comment documents a deliberate, harmless
reordering of the arena chevrons relative to the goals (a caching split, not
a bug: they don't overlap on screen, so painter's-algorithm order between
them is invisible) — and an **ordered**, exact match for the reticle and for
`camera.project` itself (see below).

**Covered, full per-player payload diff:** the `(sx, sy, r, color)` screen
anchor AND the complete `PlayerRenderOptions` struct (pose id/priority/
source, windup, aerial\*, dive\*, grab, throw, holding, dashing, controlled,
team, species\*, facing) pitch.lua/pitch.ts hand to the player renderer, in
depth-sorted order. This is the exact boundary `player_renderer_3d.ts`'s
rigged pass consumes (`pitch.ts`'s shared `playerOptions()` builds this same
struct for both branches) — so it directly answers "does pose/windup/aerial/
dive reach the rig" without touching the off-limits rig files.

**Also new: a `camera.project` numeric differential** (`camera.spec.ts`),
run against the product's real 960×540/1280×720 dimensions (not a shrunk
fixture) across the fixed projection, a 2× zoomed follow view, and
perspective mode.

**Deliberately narrowed away**, stated honestly: the polygon/line soup
`player_renderer.lua`/`.ts` draw *inside* one player's silhouette (limbs,
gait, equipment) — reproducing LÖVE's `push/translate/rotate` transform
stack faithfully enough to diff that one-for-one is a second, much larger
project, `player_renderer.ts` already has its own spec, and `pitch.lua`
delegates to it as one opaque call this harness intercepts rather than lets
run. Also narrowed: the relative depth order **between** a player and the
ball — both languages' comparator (`table.sort` vs `Array.prototype.sort`
over an identical `{index, depth}` structure) were read side by side and
found identical; not worth a second capture.

`view_state.ts`/`correction_smoothing.ts` did not get a new runtime capture.
Both are short, non-transcendental arithmetic with no branches that plausibly
diverge between languages, and direct line-by-line reading (not a spec claim
— actually read both files side by side) found them faithful transliterations
of `game/render/view_state.lua`/`correction_smoothing.lua`. The budget went
to the pitch/camera harness instead, which is where the actual bugs were.
That said, `correction_smoothing`'s *wiring* — not its own math — is a real
finding; see below.

## Findings, in priority order

### 1. `pitch.ts` drops the one-based→zero-based conversion for `frame.control.controlled`/`pass_target` (confirmed, high impact)

**This is the standout finding and the most likely explanation for the
"charging kick" complaint specifically.**

`frame.control.controlled` and `frame.control.pass_target` are **one-based
roster slots**, not zero-based array indices — confirmed three independent
ways:

- `crates/gc-render/src/frame.rs`: `RenderFrameControl.controlled:
  state.controlled` — copied verbatim from `gc-sim`'s own one-based
  `MatchState.controlled` (`&state.players[(state.controlled - 1) as
  usize]`, i.e. gc-sim itself subtracts 1 to use it).
- `packages/render/src/frame_buffer.ts`'s decoder passes both fields through
  **raw**, off the wire, with its own doc comment on the neighbouring field:
  *"Roster slot, one-based. `hud.controlled_id` is dropped from the wire —
  recover it as `roster.ids[hud.controlled - 1]`."*
- `packages/screens/src/match.ts` handles the **identical field** correctly,
  two call sites away: `// hud.controlled is one-based ... const
  fallbackControlled = (frame.hud.controlled ?? 1) - 1;`

`pitch.ts`'s `drawPitchAfterItems` does not do this. It reads both fields
straight into a **zero-based** array:

```ts
const controlled = frame.control.controlled;
const px = players.x[controlled] ?? 0;   // charge meter
...
const target = frame.control.pass_target;
const tx = players.x[target] ?? 0;        // pass-target preview
```

The Lua original has no equivalent bug — `game.render.pitch` indexes its own
(also one-based) Lua array with the same one-based value, so the two stay
aligned there. This is not a stylistic nit: **every time a player charges a
shot or a pass, or a pass target is selected, the charge meter and the
pass-target reticle render under/at the wrong player** — off by one slot for
the charge meter, and out-of-bounds (defaulting to world origin `(0, 0)` via
`?? 0`) whenever the target's slot equals the roster count, which is common
for a late-roster player.

Confirmed empirically by the harness, not just by reading: a fixture with
`control.controlled = 1` (roster slot 1, "home-1") and `control.pass_target =
3` (roster slot 3, "home-2") produces, in the real Lua, a charge meter at
`x=665.72` (home-1's own projected position) and a pass-target ring at
`(834.67, 384)` (home-2's own position). The current `pitch.ts` produces a
charge meter at a different player's position and a pass-target ring at
world `(0, 0)`. Pinned as two `it.fails` tests in `pitch.spec.ts`
("KNOWN BUG: pitch.ts drops the one-based-to-zero-based conversion...") —
they currently fail as expected (proving the gate catches it) and will start
passing, loudly, the moment `drawPitchAfterItems` subtracts 1 before
indexing. `pitch.ts` is outside this task's file ownership, so it is pinned,
not fixed, here.

### 2. Hex floor tiles render thinner than the Lua original actually renders them (confirmed, minor)

`game/render/arena.lua`'s backdrop draw ends by leaving LÖVE's graphics
context at `setLineWidth(math.max(2, viewport.h / 180))` (4px at 720p) for
its ribbon markers, and never resets it. `pitch.lua`'s `draw_hex_floor` never
calls `setLineWidth` itself, so — because LÖVE's graphics API is a mutable,
stateful context — the hex floor actually renders with **4px line width**,
not the 1px you'd guess from reading `draw_hex_floor` in isolation. This is
confirmed by the capture, and per this migration's own rule ("a port
reproduces behaviour, not intent" — v2/README.md §9, restated in this task's
brief), it is real behaviour to reproduce, however accidental its origin.

`pitch.ts`'s `drawHexFloor` has no equivalent ambient state to inherit from —
`arena.ts`'s backdrop is a separate, pure command-producing function, and
`draw2d.ts`'s commands carry their own explicit `lineWidth` rather than
reading a shared context — so it draws hex lines at the `DrawList` default
(1px): visibly fainter/thinner grid than the original ever actually
rendered. Minor on its own, but it is one more small piece of "the pitch
doesn't quite look right." Pinned as an `it.fails` in `pitch.spec.ts`. Fix
(outside this task's ownership) is to thread `Math.max(2, vp.h / 180)` into
that one `dl.polygon` call.

### 3. `dive_dir` representational mismatch (confirmed, unclear impact — flagged, not asserted as a bug)

`pitch.lua` unconditionally constructs `player_opts.dive_dir = { x =
players.dive_dir_x[index], y = players.dive_dir_y[index] }` for **every**
player, even when neither component is set (a table with two `nil` fields,
which is indistinguishable from an empty table/array). `pitch.ts`'s
`playerOptions()` only includes the `dive_dir` key at all when **both**
components are defined, omitting it entirely otherwise. Both convey "no dive
direction data," but one is "key present, values absent" and the other is
"key absent." Whether this matters depends on whether
`player_renderer_3d.ts`/`rig3d` (out of this task's scope) branch on the
key's presence or the values — worth that agent checking. Documented in
`pitch.spec.ts`'s differential test, not asserted as a failure, since I
can't confirm which side is "correct" without reading the rig code.

### 4. `render_pose` (correction smoothing) never reaches the wasm-produced `RenderFrame` — real gap, currently dormant

`game/screens/match.lua` feeds `correction_smoothing`'s displayed pose into
`render_frame.build(s, { render_pose = ..., ... })` every frame, live or not.
`crates/gc-wasm/src/session.rs`'s `frame_options()` builds
`RenderFrameOptions` with `render_pose: None` unconditionally (`..
Default::default()`) — there is no call path in the TypeScript app
(`grep -r "render_pose\|renderPose" packages/` returns nothing outside
`dist/`) that ever passes a smoothed pose across the wasm boundary, despite
`correctionSmoothing`/`viewState` being correctly computed and even
diagnosed in the rollback debug HUD.

**This is dormant for the current browser build**, not a currently-visible
bug: `packages/app/src/real_match_factory.ts` never sets up online/rollback
play (`grep -c online` is 0), and in **base** (local) mode `corrected` is
always `false` in the Lua original too (`#self._rollback_corrections > 0` is
always 0 with no netcode), so `_render_pose` equals the raw authoritative
position in Lua as well — no smoothing actually happens in local play on
either side today. The gap becomes live and visible the moment online/
rollback play ships in the browser build; at that point a correction (a
resimulation jump) will render as a hard snap in v2 where the Lua original
eases it over ~0.1s. Worth a tracked follow-up before online play ships, not
before.

### 5. What I confirmed correct — ruled out, not merely unexamined

- **Camera/scale is NOT the cause of "speed feels wrong."** `camera.ts`'s
  `project`/`view` — fixed projection, 2× zoomed follow view, and
  perspective mode — matches the real Lua to 1e-6 (1e-3 for perspective
  mode's trig-routed terms) across seven sample points per mode, at the
  product's actual 960×540/1280×720 dimensions. This was my leading
  hypothesis going in, per the task brief; it's ruled out by direct
  measurement, not just code reading.
- **Ball height (`z`/`vz`) is rendered, and faithfully.** Both the Lua
  original and `pitchDrawCommands` draw the ball as a flat, shrinking/rising
  2D circle + shadow (`hk = 1 / (1 + z/80)`), not a real 3D sphere — that's
  what the Lua actually did too, confirmed identical in the differential.
  `pitch.ts`'s rigged branch (`pitch.draw`) uses the exact same
  `drawLooseBallCommands` for the ball even when players are rigged, so this
  holds for the shipped (rigged-by-default) path as well, not just the
  procedural reference path this harness directly exercises.
- **The pose/windup/aerial/dive/grab/throw/holding/dashing/team/species data
  handoff into the rig is correct.** The full `PlayerRenderOptions` payload
  `pitch.ts` builds (shared by both the procedural and rigged branches)
  matches the Lua reference field-for-field, aside from the narrow
  `dive_dir` representation nuance in finding 3. If a pose or aerial
  animation isn't visibly driving the rig, the data reaching it is not the
  cause — the question is entirely inside `player_renderer_3d.ts`/`rig3d`'s
  own clip-selection logic, which this task did not (and was told not to)
  inspect.
- **Team colour assignment logic itself is correct** in the code this
  harness covers: `roster.teams[index] === "home" ? opts.home_color :
  opts.away_color` matches the Lua original exactly, verified for the goal
  nets, the pass-target preview, and the per-player anchor color in the
  differential. If team colours are washed out in the live build, the defect
  is downstream of this hand-off — inside `player_renderer_3d.ts`/`rig3d`'s
  own material/shader path (out of scope here, and the other agent's
  current work).

## What I could not do

- Could not verify anything inside `player_renderer_3d.ts`/`rig3d/**` —
  explicitly off limits (another agent's concurrent work). Findings 3 and
  the "team colour assignment is correct up to the hand-off" note in #5 are
  as far upstream as this task could push that investigation.
- Did not build a runtime differential for `view_state.ts`/
  `correction_smoothing.ts`'s own arithmetic — judged low-value given direct
  line-by-line reading found them faithful and non-transcendental; budget
  went to pitch/camera instead, where the confirmed bugs were.
- Did not attempt a pixel-level (live GL) comparison — out of this task's
  tooling (a third agent owns `scripts/browser_render_bench.py`/
  `v2/tools/browser_render_bench/`) and, per `v2/README.md` §1, out of this
  milestone's scope (no running app / wasm bindings).

## Files

- `v2/tools/render_reference/capture_pitch_reference.lua` — the checked-in
  Lua capture script (recording `love.graphics` stand-in + player-renderer
  stand-in), and `v2/tools/render_reference/README.md` for how to re-run it
  and why it's shaped the way it is.
- `v2/ts/packages/render/src/pitch.spec.ts` — the differential itself (static
  scene, reticle, per-player payload, and the three pinned `it.fails` bugs),
  appended after the existing tests.
- `v2/ts/packages/render/src/camera.spec.ts` — the `camera.project` numeric
  differential, appended after the existing tests.

## Fixed: the washed-out characters

Characters render pale and near-white; team colours are present but heavily
desaturated. Two independent scratch experiments on a real RTX 2070 SUPER
narrowed it, and reading the shader against the camera finishes the job.

**It is not content, and not the cel constants.** The constants are exact
ports of `game/render/rig3d/renderer.lua`'s (bands 0.55/0.12 → 1.0/0.72/0.42,
bounce 0.18, rim `pow(1-NdotV,3)` smoothstep 0.35–0.95, metal spec `pow 24`
smoothstep 0.20–0.42). Zeroing `RIM_TINT` snaps colours back to fully
saturated (`nearWhiteFraction` 0.0024 → 0), so the rim term is what floods.

**The mechanism: `vViewPosition` is a perspective view vector, and the camera
that now draws characters is orthographic.**

`cel_shader.ts` computes `gcViewDir = normalize(vViewPosition)`. three.js sets
`vViewPosition = -mvPosition.xyz` — the vector from the fragment to the *view
space origin*. That is the correct eye direction for a PERSPECTIVE camera,
where the eye sits at that origin. It is wrong for an ORTHOGRAPHIC one, where
all rays are parallel and the view direction is the constant `(0,0,1)`
regardless of where the fragment sits.

`SceneRoot` draws everything through one shared
`OrthographicCamera(0, w, 0, h)`. So `gcViewDir` varies steeply with a
fragment's screen position instead of staying constant, `NdotV` is computed
against a direction that is not the view direction, and `1 - NdotV` comes out
large across the whole silhouette rather than only at its edge. The rim stops
being a rim and becomes a wash.

The Lua never hit this: `renderer.characterCamera` builds a real per-character
camera with `u_cam_pos` sent as a uniform, and the shader uses
`normalize(u_cam_pos - v_world)` — an actual eye-to-fragment vector.

**How it arrived.** Characters used to render through
`characterCameraParams`'s asymmetric per-character frustum, offscreen, one
pass each. The single-pass draw change moved them into the shared scene and
its orthographic camera. That change was right — it took character draw calls
from 6.0 to 1.0 each — and it silently invalidated an assumption the shader
had been written against. Both changes were verified in isolation; the
composite was not.

**Supporting data point.** Replacing the wrapper's `CHARACTER_DEPTH_SCALE`
with a uniform `ppm` Z scale clips characters into fragments (that file's own
comment already predicted this), but the surviving fragments render fully
saturated. Consistent with the same story: that edit changes the normals, and
therefore the erroneous dot product, rather than being a second cause.

**Likely fix.** Use a constant view direction under an orthographic camera
rather than `normalize(vViewPosition)`. Do NOT retune the cel constants — they
are correct, and changing them would hide this rather than fix it.

### What the fix turned out to be

The rim diagnosis above was right and incomplete. Fixing the view direction
alone would not have helped much, because two more things were wrong in the
same place, and each on its own is enough to flatten a character. All three are
the same root cause seen from three angles: **the shading was being evaluated
in a space that is not a space** — pixels on X/Y, a 0.05 bookkeeping axis on Z.

1. **The normals were destroyed, not merely mis-dotted.** A normal matrix is
   the inverse transpose, so pitch.ts's `wrapper.scale.set(ppm, -ppm, 0.05)`
   arrives in the shader as `diag(1/ppm, -1/ppm, 20)` — the Z component
   amplified ~500x at a typical `ppm` of 25. Measured against that exact
   transform chain: **32 of 32** sample normals spread around a character
   collapse onto ±Z. `ndl` is then two values, both between
   `BAND_MID_THRESHOLD` and `BAND_HIGH_THRESHOLD`, so the whole figure renders
   in two flat tones and the bright band is never reached at all. The "toon"
   quantisation had no form left to quantise. This is what the "supporting
   data point" above was actually showing.
2. **`vViewPosition` was wrong**, as diagnosed — and worse than "orthographic
   vs perspective": the shared camera sits at `z = 1` in a scene where a
   character is tens of pixels wide, so the vector points mostly *sideways*.
   `dot(n, viewDir) ≈ 0`, `rim` saturates to 1 across the silhouette, and every
   fragment gains a flat `RIM_TINT * 0.55` wash.
3. **`if (!gl_FrontFacing) n = -n;` does not port.** It was carried over
   verbatim from the Lua. `gl_FrontFacing` is raster state, not geometry, and
   the two hosts disagree: three.js flips the front-face convention per object
   when `matrixWorld.determinant() < 0` (which pitch.ts's Y mirror makes true),
   LÖVE — whose Y flip lives in the projection matrix — does not. Writing the
   sign of N·V straight to the framebuffer on an RTX 2070 SUPER showed it
   **negative across the entire silhouette**: every visible fragment was
   holding an inward normal, which pins `rim` at 1 by itself.

The fix rebuilds rig3d/renderer.lua's own shading frame in the vertex stage
instead of consuming three.js's camera-dependent one. `modelMatrix`'s upper 3×3
is `S * R` (the wrapper carries only scale and translation, the mesh only
rotation) and `R` is orthonormal, so each row's length recovers that row's
scale factor exactly; the Y mirror's sign comes back from the determinant
rather than being hardcoded. Removing pitch.ts's elevation tilt — which the Lua
keeps in its camera, not its model — lands in the Lua's frame, where
`light_dir` and the `SHADING_EYE_DISTANCE = 24` eye are used verbatim with no
camera state at all. The two-sided rule becomes `N·V < 0 → flip`, which is what
the Lua's line *means* independent of raster convention.

**The cel constants were not touched**, as this section instructed.

The six tests in `cel_shader.spec.ts`'s "shading frame" block were confirmed to
fail against the pre-fix shader and pass after — including one that
reimplements the vertex-stage recovery in TypeScript and checks it against a
directly-composed ground truth, so it fails on wrong *math* rather than on
changed spelling. The 23 tests that existed before passed in both states, which
is why this reached a browser.

## Not a defect: `dashing` / `grab` / `aerial_outcome` on the rigged path

These three fields are carried all the way from the simulation into the render
frame and are then read by `player_renderer.ts` — the 2D billboard — and never
by `player_renderer_3d.ts`. That was reported as a port gap. It is not one:
`game/render/player_renderer_3d.lua` does not read them either, and neither
does anything under `game/render/rig3d/`. `pitch.lua` populates them at the one
call site that feeds BOTH renderers, exactly as `pitch.ts` does. The billboard
is still a live fallback (`pitch.rigged_players && available()`), so they are
not dead code on either side.

**Update (#415): in v2 they now are dead, and the count is larger than three.**
`player_renderer.ts` is deleted, so on the TypeScript side nothing reads
`is_keeper`, `dashing`, `grab`, `aerial_outcome`, `species_shape`,
`species_color` or `combat` off `PlayerRenderOptions` — grep for `opts.<field>`
across `packages/render/src` returns zero hits outside `pitch.ts`'s own
`playerOptions()`, which produces them. They are deliberately still produced:
the Rust `crates/gc-render` frame builder writes them, `frame_buffer.ts`
decodes them, and `pitch.spec.ts`'s Lua differential pins them, so pruning is a
wire-format change across both languages rather than a TypeScript tidy-up. The
paragraph above stays accurate for the LÖVE tree, where the billboard is still
the fallback it describes.

The mechanics do reach the rig — through `pose_id`, which `player_pose.select`
derives from the same timers (`slide_timer`, `grab_timer`, `aerial_timer`,
`tackle_timer`, `stun_timer`, ...). Three separate mechanisms consume it, and
which one owns an id is not arbitrary; `player_renderer_3d.ts`'s `POSE_CLIP`
comment sets out the split. Measured over a full 7,200-tick match, the renderer
receives 13 distinct pose ids, `Locomotion` 69.8%, including `KeeperGrab`,
`KeeperStretch`, `AerialAction`, `Tackle` and `Stumble`.

`Tackle` and `Stumble` resolve to the `idle` clip: neither appears in
`POSE_CLIP`, and `action_pose` has no whole-body transform for them. That is
also true of the Lua, whose `POSE_CLIP` this port matches entry for entry — so
it is a gap in the GAME, not in the port, and worth a separate issue rather
than a divergence to fix here. Between them they account for 0.05% of
player-ticks.

`crates/gc-render/tests/pose_pipeline.rs` now pins this end to end: every link
in the chain was already unit-tested, which is precisely why nothing would have
noticed the chain going dead. Confirmed red by making `frame.rs` push a fixed
pose id ("pose vocabulary collapsed to 1 ids").
