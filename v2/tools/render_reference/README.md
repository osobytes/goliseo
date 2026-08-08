# Render differential: `game/render/pitch.lua` vs `packages/render/src/pitch.ts`

Every determinism-critical module in this migration got a Lua differential
(`v2/tools/lua_reference/`). Rendering never did -- there is no
`crates/gc-render/tests/fixtures/frame_buffer_lua_reference.txt` equivalent for
what actually gets drawn. This directory is that gate for the pitch renderer.

## Why rendering needed a DIFFERENT harness than `lua_reference/`

`lua_reference/`'s pattern (capture `%.17g` scalar values, compare
`f64::to_bits()`) fits values on the determinism path: RNG draws, hashes,
snapshot fields. `pitch.lua`/`pitch.ts` do not return a value at all -- they
issue an ordered sequence of `love.graphics` calls (Lua) or build an ordered
`DrawCommand[]` (TypeScript, `pitch.ts`'s `pitchDrawCommands`, which the file's
own header calls "the PURE, tested reference path for the procedural side").
The comparable unit here is a *sequence of draw calls*, not a scalar, so the
capture script (`capture_pitch_reference.lua`) replaces `love.graphics` with a
RECORDING backend instead of stubbing it to a no-op, and normalizes each call
into the same shape as a `DrawCommand` (`kind`, `mode`, coordinates, `color`,
`alpha`, `blend`, `lineWidth`).

## What is compared, and what is deliberately narrowed away

**In scope, full command-level diff:** the static pitch (arena backdrop, floor
trapezoid, floor glow, hex tiling, markings, goal nets/frames, outline, arena
frame chevrons), the loose ball (position, shadow, height lift), and the
overlay layer (landing reticle, pass-target preview, charge meter bar/ticks/
label). This is everything `pitch.lua`/`pitch.ts` compute and draw *without*
delegating to a player renderer.

**In scope, per-player anchor + full options diff:** the `(sx, sy, r, color)`
screen anchor pitch hands off per player, AND the complete
`PlayerRenderOptions` payload (pose id/priority/source, windup, aerial*,
dive*, grab, throw, holding, dashing, controlled, team, species*, facing) --
this is the exact boundary `player_renderer_3d.ts`'s rigged pass consumes
(`pitch.ts`'s `playerOptions()` builds this SAME struct for both the
procedural and rigged branches), so verifying it is a direct, in-scope check
of "does pose/windup/aerial/dive actually reach the rig" without touching
`player_renderer_3d.ts`/`rig3d/**` (off limits to this task).

**Narrowed away, and why:** the polygon/line/circle soup `game/render/
player_renderer.lua` / `player_renderer.ts` draw INSIDE a player silhouette
(limbs, gait pose, equipment) is not captured here. `pitch.lua` delegates to
`game.render.player_renderer` for that, and this harness replaces that module
entirely with a recording stand-in (see `capture_pitch_reference.lua`'s
header) rather than letting it run — reproducing LÖVE's `push`/`translate`/
`rotate` transform-stack semantics faithfully enough to compare limb polygons
one-for-one is a second, much larger porting-fidelity project, and
`player_renderer.ts` already has its own spec. If the anchor/options payload
into that module matches, the "same game" question for a given player's
silhouette is that module's own port-fidelity question, not this harness's.

**Also narrowed away:** the relative depth-sort order BETWEEN a player and the
ball. Both `pitch.lua`'s `table.sort` and `pitch.ts`'s `depthSortedItems`
(`Array.prototype.sort`) implement the identical `{index, depth}` comparator
over the identical input -- verified by direct side-by-side reading, not
worth a second capture. What IS captured is the depth-sort order among the
non-player-anchored draws (arena before entities, overlay after), which this
harness's ordered record list preserves.

## Files

- `capture_pitch_reference.lua` -- the capture script. Checked in here so it
  can be re-run whenever `pitch.lua` changes; not runnable in place (it needs
  `love` and a copy of `game/`/`core`/`data/` -- see its own header for the
  exact steps, matching `v2/tools/lua_reference/README.md`'s pattern).
- The captured JSON output is NOT checked in as a separate fixture file.
  `packages/render/src/pitch.spec.ts` cannot read files from disk (see
  `packages/render/src/fixtures/frame_buffer_lua_reference.ts`'s header for
  why -- no `@types/node` in this package, and that package's own task scope
  forbids touching `package.json` to add it), so the captured JSON is embedded
  as a string literal directly in the differential `describe` block instead of
  a sibling fixture module, matching that same constraint.

## How the fixture was chosen

`game/render/pitch.lua`'s hex floor tiles the WHOLE field at a fixed 26-world-
unit radius regardless of field size. At the product's real 960x540 pitch that
is ~300 hex polygon commands alone -- correct for a live game, unusable as a
literal embedded in a spec file. The fixture below shrinks the field to
200x120 (proportions otherwise arbitrary) purely to bring the hex count down
to ~15 cells; every other code path (backdrop, markings, goals, depth sort,
overlay) is exercised identically regardless of field size. Three players
(one controlled/dashing/windup outfielder, one diving/holding keeper, one
aerial-heading outfielder) plus a lofted ball (nonzero `z`, a landing spot)
and an active charge meter + pass target exercise every optional
`PlayerRenderOptions`/overlay field at once. See `capture_pitch_reference.lua`
for the exact literal.

`control.controlled = 1` and `control.pass_target = 3` are deliberately
1-based Lua roster slots that do NOT equal their own zero-based array position
(player 1 is roster slot 1, at TS array index 0; player 3 is roster slot 3, at
TS array index 2) -- see the port report for what this exposed.
