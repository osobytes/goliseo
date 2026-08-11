# Render differential: `game/render/pitch.lua` vs `packages/render/src/pitch.ts`

This directory documents a second, frozen piece of cross-language
determinism evidence, parallel to `tools/lua_reference/` but for *draw
sequences* rather than scalar values. The capture script that originally
produced it (`capture_pitch_reference.lua`) no longer exists in this
repository — the Lua implementation it targeted was retired, and there is
no interpreter left to run a Lua capture script against in any case. What
survives, and remains load-bearing, is the frozen evidence itself: a
captured JSON literal embedded directly in `packages/render/src/pitch.spec.ts`.
As with `tools/lua_reference/`, a failing differential against that embedded
evidence is a finding about `pitch.ts`, never a stale fixture to refresh.

## Why rendering needed a different harness than `lua_reference/`

`lua_reference/`'s pattern (capture `%.17g` scalar values, compare
`f64::to_bits()`) fits values on the determinism path: RNG draws, hashes,
snapshot fields. `pitch.lua`/`pitch.ts` do not return a value at all — they
issue an ordered sequence of `love.graphics` calls (Lua) or build an
ordered `DrawCommand[]` (TypeScript, `pitch.ts`'s `pitchDrawCommands`, which
the file's own header calls "the PURE, tested reference path for the
procedural side"). The comparable unit here is a *sequence of draw calls*,
not a scalar, so the original capture script replaced `love.graphics` with
a recording backend instead of stubbing it to a no-op, and normalized each
call into the same shape as a `DrawCommand` (`kind`, `mode`, coordinates,
`color`, `alpha`, `blend`, `lineWidth`).

## What is compared, and what is deliberately narrowed away

**In scope, full command-level diff:** the static pitch (arena backdrop,
floor trapezoid, floor glow, hex tiling, markings, goal nets/frames,
outline, arena frame chevrons), the loose ball (position, shadow, height
lift), and the overlay layer (landing reticle, pass-target preview, charge
meter bar/ticks/label). This is everything `pitch.lua`/`pitch.ts` computed
and drew *without* delegating to a player renderer.

**In scope, per-player anchor + full options diff:** the `(sx, sy, r)`
screen anchor pitch hands off per player, AND the complete
`PlayerRenderOptions` payload (pose id/priority/source, windup, aerial*,
dive*, grab, throw, holding, dashing, controlled, team, species*, facing) —
this is the exact boundary `player_renderer_3d.ts`'s rigged pass consumes,
so verifying it is a direct check of "does pose/windup/aerial/dive actually
reach the rig" without touching `player_renderer_3d.ts`/`rig3d/**`, which
this harness deliberately does not reach.

Since #415 the TypeScript side of that comparison is `pitch.ts`'s exported,
pure `playerAnchors(frame, vp, opts)` rather than a `vi.spyOn` on the
renderer module. The captured `color` is no longer compared: it was the
billboard's team tint, the rigged path resolves colour from rig3d's own
team palette, and the billboard is deleted. `options.team`, which it was
derived from, is still asserted.

**Narrowed away, and why:** the polygon/line/circle soup
`game/render/player_renderer.lua` drew *inside* a player silhouette (limbs,
gait pose, equipment) was never captured here. `pitch.lua` delegated to
that module for it, and this harness replaced that module entirely with a
recording stand-in rather than letting it run — reproducing `love`'s
`push`/`translate`/`rotate` transform-stack semantics faithfully enough to
compare limb polygons one-for-one would have been a second, much larger
fidelity project, and there is no billboard renderer in this codebase to
compare them against at all (#415). If the anchor/options payload into that
module matches, the "same game" question for a given player's appearance is
the rig's own question, not this harness's.

**Also narrowed away:** the relative depth-sort order BETWEEN a player and
the ball. Both `pitch.lua`'s `table.sort` and `pitch.ts`'s
`depthSortedItems` (`Array.prototype.sort`) implement the identical
`{index, depth}` comparator over the identical input — verified by direct
side-by-side reading at the time, not a second capture. What IS captured is
the depth-sort order among the non-player-anchored draws (arena before
entities, overlay after), which this harness's ordered record list
preserves.

## Files

- `capture_pitch_reference.lua` — gone. It was the capture script that
  produced the JSON below; its target (`pitch.lua`, and the `love`
  runtime it needed) no longer exists in this repository, so it cannot be
  re-run. It is not coming back, and the frozen JSON it already produced is
  now the only record of what it once measured.
- The captured JSON output is **not** checked in as a separate fixture
  file. `packages/render/src/pitch.spec.ts` cannot read files from disk
  (see `packages/render/src/fixtures/frame_buffer_lua_reference.ts`'s
  header for why — no `@types/node` in this package, and that package's
  scope forbids touching `package.json` to add it), so the captured JSON is
  embedded as a string literal (`LUA_REFERENCE_JSON`) directly in the
  differential `describe` block instead of a sibling fixture module. That
  string literal is the load-bearing evidence this whole directory exists
  to document; the frozen `LUA_RECORDS` array `pitch.spec.ts` parses from it
  is what every differential assertion in that file compares against.

## How the fixture was chosen

`game/render/pitch.lua`'s hex floor tiled the WHOLE field at a fixed
26-world-unit radius regardless of field size. At the product's real
960x540 pitch that was ~300 hex polygon commands alone — correct for a live
game, unusable as a literal embedded in a spec file. The fixture below
shrinks the field to 200x120 (proportions otherwise arbitrary) purely to
bring the hex count down to ~15 cells; every other code path (backdrop,
markings, goals, depth sort, overlay) is exercised identically regardless
of field size. Three players (one controlled/dashing/windup outfielder, one
diving/holding keeper, one aerial-heading outfielder) plus a lofted ball
(nonzero `z`, a landing spot) and an active charge meter + pass target
exercise every optional `PlayerRenderOptions`/overlay field at once. The
exact literal that produced this is lost along with the capture script; the
captured `LUA_REFERENCE_JSON` in `pitch.spec.ts` is what remains of it.

The **viewport** is pinned to the field's own size (200x120), and unlike
the field's proportions that is not arbitrary (#414). It previously ran at
1280x720, described at the time as "the product's actual viewport" — which
was never true of the LÖVE build (`conf.lua` pinned a non-resizable 960x540
window and `sim/env_config.lua`'s `DEFAULT_FIELD` was 960x540, so
`vp == field` in every frame the original build ever drew).

That does **not** make this fixture the shipping configuration, and it
cannot be: its field is the synthetic 200x120 above, and nothing in this
codebase renders at 200x120. What `vp == field` buys here is narrower.
`game/render/camera.lua` put the world-to-pixel factor into screen
positions only and left the depth scale (the sole input to every entity
size) a pure ratio; `packages/render/src/camera.ts` now carries one uniform
factor into both. The two therefore agreed exactly when that factor was 1
— i.e. at `vp == field` — and diverged otherwise, in a shape that depended
on the aspect ratios:

- **Same aspect** (960x540 field at 1280x720): positions stayed identical
  and only `scale` differed, by one constant. `camera.spec.ts`'s kept
  1280x720 rows characterize exactly that.
- **Different aspect**, which was this fixture's old case: 200x120 is 5:3
  while 1280x720 is 16:9, so the old per-axis `vp.w/field.w` (6.4) and the
  uniform fit `min(6.4, 6.0)` (6.0) disagreed — positions *and* sizes both
  diverged, and pinning the fixture at that aspect would have meant
  characterizing a two-part divergence across all 101 records.

Capturing at `vp == field` collapsed that to zero divergence, keeping this
a plain equality differential. Entity SIZES were unchanged by the switch —
the old scale had no viewport term at all — so only screen positions moved.

This capture is therefore **not** the regression guard for #414: at
`vp == field` the fit factor is 1 and the fixed formula is byte-identical
to the pre-fix one, so reverting `camera.ts` would leave every record here
passing. Viewport-safety coverage lives in `camera.spec.ts`'s 1280x720
differential rows and in `pitch.spec.ts`'s Lua-independent "pitch entity
sizes stay in proportion to the pitch at any viewport" block.

## The one-based roster-slot bug this fixture caught

`control.controlled = 1` and `control.pass_target = 3` in the frozen
evidence are deliberately one-based roster slots that do NOT equal their
own zero-based array position (player 1 is roster slot 1, at array index
0; player 3 is roster slot 3, at array index 2). `frame_buffer.ts`'s own
decoder, the Rust producer (`crates/gc-render/src/frame.rs`'s
`RenderFrameControl.controlled`, copied verbatim from `gc-sim`'s own
one-based `MatchState.controlled`), and `packages/screens/src/match.ts`'s
handling of the identical field all agree these fields are one-based on
the wire — matching ARCHITECTURE.md's rule that wire-format indices keep
their defined value rather than being renumbered to ordinary zero-based
indexing.

This fixture is not just descriptive evidence — it is what caught a real
bug. `pitch.ts`'s `drawPitchAfterItems` originally read both fields
straight into the zero-based `players.x`/`players.y` arrays with no `-1`
anywhere, so the charge meter rendered under the wrong player and the
pass-target preview rendered out of bounds (defaulting to world `(0, 0)`
via `?? 0`). The differential comparing against this fixture's captured
positions is what exposed the mismatch: the two assertions in
`pitch.spec.ts`'s "`pitch.ts` converts `frame.control.controlled`/
`pass_target` from one-based to zero-based" block were pinned as
`it.fails` by the failing differential, then flipped to ordinary passing
assertions once `drawPitchAfterItems` was fixed to subtract 1 before
indexing — the transition from expected-fail to pass is itself the proof
the fix was correct. This is the concrete version of the abstract claim at
the top of this file: a failing differential against frozen evidence is a
finding about the code, not the fixture, and here it found a real one.
