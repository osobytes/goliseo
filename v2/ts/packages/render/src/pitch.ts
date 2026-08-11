// Ported from game/render/pitch.lua.
//
// Draws a `RenderFrame` through the camera projection as a perspective pitch
// with depth-sorted players. This module never sees a
// `MatchState` -- everything it draws comes off the versioned payload built
// by `render.frame` (Rust, see below), which is the whole point: the same
// payload can be handed to a renderer not written in Lua. The only things
// read from outside it are renderer-owned presentation state (`view_state`
// gait/lean, the particle systems) and per-match theming.
//
// TWO EXPORTS, ONE FRAME. `pitchDrawCommands` is the PURE, tested path: it
// produces the exact `DrawCommand[]` (draw2d.ts) for everything that is not a
// player -- arena, floor, markings, goals, the loose ball, the overlay layer.
// `playerAnchors` is the pure, tested per-player half: the depth-sorted screen
// anchor and `PlayerRenderOptions` payload each player is drawn from.
// `pitch.draw` is the impure orchestrator that assembles both into `group` and
// calls `player_renderer_3d.ts`'s genuine `THREE.SkinnedMesh` pass per player.
//
// THERE IS EXACTLY ONE PLAYER RENDERER (#415). A procedural 2D billboard
// avatar (`player_renderer.ts`) used to sit behind three fallback conditions --
// no `THREE.WebGLRenderer` passed, `player_renderer_3d.build()` having failed,
// or a `pitch.rigged_players` module flag -- and the middle one was re-checked
// PER PLAYER, so a rig build that failed partway through a roster silently
// downgraded the rest of the team mid-frame. `build()` fails on invalid vertex
// indices and missing rig3d content: programmer errors, which AGENTS.md §7 says
// fail loud rather than degrade quietly. All three conditions, the flag, the
// `renderer` parameter that gated them and the billboard itself are gone; a
// character that cannot be built now throws (see `pitch.draw`).
//
// SCOPE NOTE. Compositing a per-player 3D rigged pass with the flat
// screen-space 2D content at the correct point in the painter's-algorithm
// depth sort (so a rigged player occludes/is occluded exactly like a flat
// screen-space entity at the same world position would) is real GPU-integration work: it needs a live
// `WebGLRenderer` to verify pixel-for-pixel, and v2/README.md #1 scopes "a
// running app" / the wasm-bindings glue out of this milestone. What follows
// was nonetheless verified against a live GL context outside this milestone's
// test harness (screenshots, draw-call counts -- see this port's report);
// the automated suite still only covers what does not need one (below).
//
// `pitch.draw`'s rigged pass CONTRIBUTES TO `group`'s `Object3D` GRAPH rather
// than rasterizing immediately: each rigged character is a real, posed,
// depth-tested `THREE.SkinnedMesh` (`player_renderer_3d.ts`'s
// `characterMesh`, pooled per player), wrapped by `riggedCharacterObject`
// below with its screen position/scale/elevation tilt, and added to `group`
// at its own point in the depth-sorted iteration -- interleaved with the ball
// and every other player via `depthSortedItems` below, the same ordering
// `playerAnchors` returns and `pitchDrawCommands` places the ball by, shared
// through `drawPitchBeforeItems`/`drawPitchAfterItems`/`drawLooseBallCommands`/
// `playerAnchor` so the pure and impure paths cannot silently diverge on either
// content or order. This is what lets exactly ONE full-scene
// render (`SceneRoot.render`, see scene.ts) draw the whole frame -- flat
// content and every rigged character together, ONE `renderer.render()` call
// total instead of one per character -- in the right order and with real
// depth testing between them, rather than N off-screen full-viewport passes
// each composited as a `depthTest: false` quad (the shape this had before
// this port; see `player_renderer_3d.ts`'s `renderToSprite`, kept for
// parity/diagnostics but no longer on this call path). `pitchDrawCommands` and
// `playerAnchors` stay the fully-tested reference for everything that needs no
// GL context; `pitch.draw`'s rigged branch is -- like the rest of the
// GPU-adjacent surface in this package -- untested beyond what does not need a
// live GL context (object-graph shape, ordering, disposal).
//
// Boundary note (v2/README.md rule 6.7): `RenderFrame` is the (Rust)
// `render/frame.lua` producer's output (`render/**` -> Rust
// `crates/gc-render`) -- only the fields this module reads are declared
// locally. `ArenaData` (`data/arenas.lua`) is Rust-owned (`crates/gc-data`);
// `DEFAULT_ARENA` below mirrors its only current entry (`helios_crown`) as
// the same "no arena supplied" fallback the Lua original had, without
// importing Rust-owned content.
//
// ONE PASS, ONE DEPTH BUFFER -- reconciling 2D screen space with 3D character
// space. `pitchGroup`'s flat content (this file, arena.ts, effects.ts,
// combat.ts) lives entirely in draw2d.ts's screen-pixel convention: an
// orthographic camera with `top=0`/`bottom=h` (Y increasing DOWNWARD, unlike
// three.js's own default), everything at z=0, ordering from `group.children`
// insertion order. `player_renderer_3d.ts`'s rigged characters are real,
// posed 3D geometry sized in METRES with their own facing yaw already baked
// in (see that file's `characterMesh`). Getting both into the SAME single
// full-scene render under the SAME shared camera (see scene.ts's
// `SceneRoot`) means every rigged character's own transform -- not a second
// camera -- has to carry everything that used to live in
// `player_renderer_3d.ts`'s per-character `characterCameraParams`. This is
// implemented below as `riggedCharacterObject`, a small wrapper `THREE.Group`
// built fresh per player per frame around `characterMesh`'s returned mesh:
//
//   - SCALE: `ppm` (pixels-per-metre) varies per player -- it comes from
//     `r = radius * scale`, where `scale` is `camera.ts`'s own fake-
//     perspective falloff for that player's world position. A shared camera
//     cannot reproduce a PER-OBJECT scale by zooming itself, so this becomes
//     `riggedCharacterObject`'s own `wrapper.scale`'s X/Y (sign note below),
//     exactly mirroring how `characterCameraParams` already derives `ppm`
//     today (see `player_renderer_3d.ts`'s `ppmForRadius`). Z is NOT `ppm`
//     -- see `CHARACTER_DEPTH_SCALE`'s own comment for why applying the
//     same pixel-scale factor to Z (a defect this port's report found live)
//     clips characters against the camera's near plane instead of sizing
//     them.
//   - POSITION: `(sx, sy)` (screen pixels) becomes `wrapper.position`'s
//     world `(x, y)` directly, since the shared camera's frustum already
//     spans `[0, vw] x [0, vh]` in exactly that space -- no separate
//     placement math needed, unlike the old per-character asymmetric
//     frustum.
//   - DEPTH: the world `z` a character/ball is placed at for real depth
//     testing against everything else in `pitchGroup` -- see `depthToZ` and
//     the DEPTH ZONES block below. This is genuinely NEW relative to the
//     Lua: the old per-character camera had no notion of a character's depth
//     relative to ANOTHER character (see DEPTH ZONES' note on the Lua's own
//     `beginPass`-clears-depth-every-call behaviour).
//   - THE Y-INVERSION: the shared camera's `top=0`/`bottom=h` convention
//     means world +Y renders toward the BOTTOM of the screen, but a
//     character's own local +Y is "up" (toward the head). Left alone, a
//     character would render feet-up. `player_renderer_3d.ts`'s OLD
//     `renderToSprite` hit the exact same mismatch one level removed (its
//     composited PLANE, not the character, needed the flip) and fixed it
//     with `mesh.scale.y = -1` -- see that function's doc comment. The same
//     correction is still needed here, just moved: `wrapper.scale.set(ppm,
//     -ppm, CHARACTER_DEPTH_SCALE)` folds the negative Y in alongside the
//     same `ppm` used for X. This is NOT something `WebGLRenderTarget.texture.flipY` could
//     ever fix (confirmed a no-op for render-target textures -- see
//     `renderToSprite`'s doc comment, now moot here anyway: there is no
//     render target in this path at all) -- it is a property of the SHARED
//     CAMERA's convention, which both the old sprite plane and a real
//     character mesh sit under identically.
//   - ELEVATION: the old per-character camera looked at the character from
//     slightly above and behind (`ELEVATION`, see `characterCameraParams`'s
//     `eye`/`target`) rather than straight on. A shared, un-tilted camera
//     cannot reproduce that by moving itself per character, but an
//     orthographic projection commutes with rotating camera-and-subject
//     together, so the SAME apparent view is reproducible by tilting the
//     character instead: a rotation of `+ELEVATION` around the shared
//     camera's local X (screen-horizontal) axis, composed AFTER the
//     character's own yaw (i.e. `finalQuat = tiltX(ELEVATION).multiply(
//     yawY(yaw))` in three.js's quaternion convention) -- yaw happens in the
//     character's own frame (which way it faces on the pitch), the
//     elevation tilt happens in the CAMERA's frame (how the fixed camera
//     sees every character alike), so composition order matters and cannot
//     be collapsed into one axis-angle call. `riggedCharacterObject`
//     PREMULTIPLIES the tilt onto `mesh`'s own already-baked yaw quaternion
//     (not the wrapper's) -- see that function's own doc comment for why
//     both rotations have to live on the SAME object as each other, and a
//     DIFFERENT object than the Y-mirroring scale: the mirror commutes with
//     the yaw (both act on/around the same Y axis) but not with a tilt about
//     a different axis, so the vertex pipeline has to be rotate (yaw, then
//     tilt) -> scale (mirror) -> translate, not some other order that would
//     look identical for yaw=0 and visibly wrong once a character turns.
//
// `player_renderer_3d.ts`'s `characterMesh(playerId: string, view:
// PlayerView | undefined, opts: PlayerRenderOptions, now: number):
// THREE.Object3D | undefined` returns the POSED, COLOURED, YAWED character
// mesh in its own local metre space (i.e. exactly the old shared singleton
// `character.mesh`, MINUS the per-character camera/scene/render-target work
// `renderToSprite` used to layer on top). `playerId` (a stable roster-slot
// key, matching how `viewState.get(roster.ids[index])` already keys
// per-player presentation state in this file) is why it can return a
// DIFFERENT mesh instance per player rather than the old shared singleton
// reused sequentially: `characterMesh` pools one skeleton/mesh per
// `playerId`, reused frame-to-frame (posed afresh each call, not rebuilt),
// because every character now coexists simultaneously in one scene graph for
// one shared render rather than being rendered and composited one at a time.
// `riggedCharacterObject` below owns everything the bullets above describe
// (scale, position, the Y-flip, `depthToZ`'s z, and the elevation tilt),
// wrapping the returned mesh in a `THREE.Group` added directly to `group` in
// `depthSortedItems`' loop in `pitch.draw`, in place of the old
// `renderToSprite` call.

import * as THREE from "three";
import { Vec2 } from "@gc/core";
import type { CombatPresentationModel } from "@gc/presentation";
import { camera, type CameraField, type CameraView, type CameraViewport } from "./camera.ts";
import { cameraFollow } from "./camera_follow.ts";
import * as arenaRender from "./arena.ts";
import type { ArenaColors, ArenaThemeColors } from "./arena.ts";
import * as combatRender from "./combat.ts";
import * as effectsModule from "./effects.ts";
import type { AerialOutcome, AerialStyle, PlayerRenderOptions, SpeciesShape } from "./player_render_options.ts";
import * as playerRenderer3d from "./player_renderer_3d.ts";
import { viewState } from "./view_state.ts";
import { DrawList, appendCommands, paint, type DrawCommand, type Project, type RGB } from "./draw2d.ts";

const HEX_RADIUS = 26; // world units, centre to corner
const NET_BACK_FRAC = 0.55; // back frame height as a fraction of the crossbar

// DEPTH ZONES.
//
// `pitchGroup` shares ONE `THREE.Scene` (and therefore one real depth buffer)
// with everything else `SceneRoot` draws -- see scene.ts's class doc comment.
// That is what lets a rigged `THREE.SkinnedMesh` character depth-test
// correctly against the ball and against another character: real per-pixel
// occlusion, not the quad/
// array-insertion-order painter's algorithm this file used before. Studying
// `game/render/rig3d/renderer.lua`'s `characterCamera`/`beginPass`/`endPass`
// for this port surfaced a nuance worth recording, because it changes what
// "matching the Lua" means here: the Lua's OWN cross-character ordering was
// ALSO painter's-algorithm, not a shared depth buffer -- `beginPass` clears
// ONLY the depth buffer on every call (`love.graphics.clear(false, false,
// true)`), once per character, so a character's depth never persists to the
// next one, and `renderer.lua`'s own "WHY THERE IS NO BACK-TO-FRONT PART
// SORTING" comment is about self-occlusion of ONE character's parts, not
// occlusion BETWEEN characters. What the Lua's real depth buffer bought was
// exclusively intra-character correctness (an arm in front of a torso). This
// port intentionally goes further than the Lua did -- a genuine improvement,
// not a re-derivation -- by giving depth-sorted entities REAL, PERSISTENT z
// so cross-character/cross-ball occlusion is correct too, per this task's
// brief. Three zones, all inside `SceneRoot`'s camera frustum (position z=1,
// near=0.1/far=10 -> visible world z roughly (0.9, -9), see scene.ts):
//
//   BACKDROP_Z (0)     -- arena/floor/markings/goals/chevrons/ball trail/
//                          combat "under" (`drawPitchBeforeItems`). Always
//                          farther from the camera than any entity standing
//                          on it, so it can never win a depth test against
//                          one -- see `depthToZ`, which only ever returns
//                          values > 0.
//   ENTITY_Z_NEAR/FAR  -- depth-sorted players + the ball
//                          (`depthSortedItems`), mapped from the SAME
//                          world-y depth key that already drives the
//                          painter's-algorithm array order, so real depth
//                          testing reproduces (and, for rigged geometry,
//                          extends beyond) that order rather than
//                          contradicting it -- see `depthToZ`.
//   overlay            -- combat "over"/landing reticle/pass preview/charge
//                          meter/effects "over" (`drawPitchAfterItems`).
//                          These must stay visually on top of every entity
//                          regardless of world depth (a charge meter always
//                          reads above the player it belongs to), so rather
//                          than out-z every possible entity they opt OUT of
//                          depth testing entirely via draw2d.ts's
//                          `PaintOptions.depthTest`.
//
// WHAT THIS NOW DOES. `ENTITY_Z_NEAR`/`ENTITY_Z_FAR` take effect for the ball
// and the rigged characters alike: a
// rigged character (`riggedCharacterObject`, below `pitch.draw`) is a real
// `THREE.SkinnedMesh` positioned at `depthToZ`'s z with ordinary depth
// testing/writing (no `depthTest: false` anywhere on it), not the old
// offscreen-rendered, `depthTest: false` quad `player_renderer_3d.ts`'s
// `renderToSprite` produced (still present, kept for parity/diagnostics, but
// no longer on this call path -- see that file's header). Two entities'
// world z, not their `group.children` array position, decides who wins the
// depth test now -- for player-vs-ball this was
// already the design's goal (a real single-pass render instead of N
// full-viewport passes; see file header). Rigged-character-vs-rigged-
// character per-pixel occlusion falls out of the SAME mechanism for free --
// worth naming as a genuine bonus (the Lua original never had it either, see
// this section's own note above on the Lua's `beginPass`-clears-depth-every-
// call behaviour), but it was never the thing this change set out to fix,
// and nothing here was contorted to guarantee it.
// Exported for tests (pitch.spec.ts) -- same rationale as
// `resetStaticSceneCache`/`staticSceneBuildCount` below: a deep-equality
// check on drawn objects would not catch these invariants drifting.
export const BACKDROP_Z = 0;
export const ENTITY_Z_NEAR = 0.02;
export const ENTITY_Z_FAR = 0.6;
// Applied to `drawPitchAfterItems`' output alongside `depthTest: false` (see
// above) so it also wins three.js's internal opaque-list ordering, not only
// the GL depth test -- belt and suspenders for the same "always on top"
// guarantee.
export const OVERLAY_RENDER_ORDER = 10;

// Maps a `depthSortedItems()` depth key (world y; larger = nearer the
// viewer, see that function's doc comment) into a world z the shared camera
// can depth-test, monotonic in the SAME direction `depthSortedItems` already
// sorts by: larger y (nearer) -> larger z (nearer the camera, which sits at
// world z=1 looking toward -Z -- see scene.ts's `SceneRoot`). `fieldH`
// normalises so the mapping does not depend on one specific arena's pitch
// height. Always `> BACKDROP_Z` (0), by construction, so an entity never
// loses a depth test to the ground it stands on.
export function depthToZ(depth: number, fieldH: number): number {
  const t = fieldH > 0 ? Math.min(1, Math.max(0, depth / fieldH)) : 0;
  return ENTITY_Z_NEAR + t * (ENTITY_Z_FAR - ENTITY_Z_NEAR);
}

// Mirrors `data/arenas.lua`'s only current entry. See file header.
const DEFAULT_ARENA: ArenaColors = {
  floor_color: [0.025, 0.16, 0.17],
  rail_color: [0.25, 0.88, 1.0],
  highlight_color: [1.0, 0.66, 0.24],
};

// Mirrors `game/ui/theme.lua`'s `theme.colors.void`/`text`. See `arena.ts`'s
// header note on why `@gc/ui` cannot be imported here.
const DEFAULT_UI_THEME: ArenaThemeColors = {
  void: [0.015, 0.022, 0.055],
  text: [0.91, 0.96, 1.0],
};

export interface RenderFrameField {
  readonly w: number;
  readonly h: number;
  readonly penalty_box_depth: number;
  readonly penalty_box_h: number;
  readonly crossbar_h: number;
  readonly goal_home: Rect;
  readonly goal_away: Rect;
}

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface RenderFrameRoster {
  readonly radius: readonly number[];
  readonly teams: readonly ("home" | "away")[];
  readonly is_keeper: readonly boolean[];
  readonly species_shape: readonly SpeciesShape[];
  readonly species_color: readonly RGB[];
  readonly ids: readonly string[];
  /**
   * The authored `gc-data` character presentation id per slot (#447). What
   * decides which theme's geometry the player renderer builds this player
   * from -- before this existed, every player on the pitch was drawn from
   * `themes.LIST[0]`.
   */
  readonly presentation_ids: readonly string[];
  /**
   * The authored `gc-data` loadout id per slot, `undefined` where the player
   * carries nothing (#447).
   *
   * `undefined` IS THE KEEPER RULE, not a missing value. `gc-data` states it
   * and its own tests enforce it in both directions; a slot with no loadout
   * must reach the renderer as an absence rather than being defaulted into
   * whatever the theme happens to carry, which is exactly the defect #447
   * records -- keepers diving with a shield.
   */
  readonly loadout_ids: readonly (string | undefined)[];
}

export interface RenderFramePlayers {
  readonly count: number;
  readonly x: readonly number[];
  readonly y: readonly number[];
  readonly facing_x: readonly number[];
  readonly facing_y: readonly number[];
  readonly controlled: readonly boolean[];
  readonly dashing: readonly (boolean | undefined)[];
  readonly dive: readonly (number | undefined)[];
  readonly dive_dir_x: readonly (number | undefined)[];
  readonly dive_dir_y: readonly (number | undefined)[];
  readonly holding: readonly (boolean | undefined)[];
  readonly grab: readonly (number | undefined)[];
  readonly throw: readonly (number | undefined)[];
  readonly windup: readonly (number | undefined)[];
  readonly aerial: readonly (number | undefined)[];
  readonly aerial_style: readonly (AerialStyle | undefined)[];
  readonly aerial_outcome: readonly (AerialOutcome | undefined)[];
  readonly aerial_jump: readonly (number | undefined)[];
  readonly pose_id: readonly (string | undefined)[];
  readonly pose_priority: readonly (number | undefined)[];
  readonly pose_source: readonly (string | undefined)[];
}

export interface RenderFrameBall {
  readonly x: number;
  readonly y: number;
  readonly z: number;
  readonly visible: boolean;
  readonly landing_x?: number;
  readonly landing_y?: number;
}

export interface RenderFrameControl {
  readonly pass_target?: number;
  readonly charge_kind?: "shot" | "pass";
  readonly charge: number;
  readonly controlled: number;
}

/** The slice of `render/frame.lua`'s `RenderFrame` this module reads. */
export interface RenderFrame {
  readonly field: RenderFrameField;
  readonly roster: RenderFrameRoster;
  readonly players: RenderFramePlayers;
  readonly ball: RenderFrameBall;
  readonly control: RenderFrameControl;
  readonly combat?: CombatPresentationModel;
}

export interface PitchViewport {
  readonly w: number;
  readonly h: number;
}

export interface PitchDrawOptions {
  readonly home_color: RGB;
  readonly away_color: RGB;
  readonly arena?: ArenaColors;
  readonly arena_pulse?: number;
  readonly camera_offset?: { readonly x: number; readonly y: number };
  readonly ui_theme?: ArenaThemeColors;
}

function projectedCircle(project: Project, cx: number, cy: number, r: number, segs: number): number[] {
  const pts: number[] = [];
  for (let i = 0; i <= segs; i += 1) {
    const ang = (i / segs) * 2 * Math.PI;
    const [sx, sy] = project(cx + r * Math.cos(ang), cy + r * Math.sin(ang));
    pts.push(sx, sy);
  }
  return pts;
}

// Soft additive luminance toward the pitch centre so the floor reads as lit.
function drawFloorGlow(dl: DrawList, project: Project, field: RenderFrameField): void {
  const [cx, cy] = project(field.w / 2, field.h / 2);
  for (let i = 4; i >= 1; i -= 1) {
    dl.ellipse("fill", cx, cy, 130 * i, 64 * i, [0.05, 0.16, 0.2], { alpha: 0.06, blend: "add" });
  }
}

// Bright, blooming pitch markings: halfway line + circle + spot, and goal boxes.
function drawMarkings(dl: DrawList, project: Project, field: RenderFrameField): void {
  const markingColor: RGB = [0.35, 0.72, 1.0];
  const [x1, y1] = project(field.w / 2, 0);
  const [x2, y2] = project(field.w / 2, field.h);
  dl.line([x1, y1, x2, y2], markingColor, { alpha: 0.85, lineWidth: 2 });

  dl.polygon("line", projectedCircle(project, field.w / 2, field.h / 2, 70, 36), markingColor, { alpha: 0.85, lineWidth: 2 });

  const [sx, sy] = project(field.w / 2, field.h / 2);
  dl.circle("fill", sx, sy, 3, markingColor, { alpha: 0.85 });

  const depth = field.penalty_box_depth;
  const boxH = field.penalty_box_h;
  const top = field.h / 2 - boxH / 2;
  const bot = field.h / 2 + boxH / 2;
  const box = (xa: number, xb: number): void => {
    const [p1x, p1y] = project(xa, top);
    const [p2x, p2y] = project(xb, top);
    const [p3x, p3y] = project(xb, bot);
    const [p4x, p4y] = project(xa, bot);
    dl.polygon("line", [p1x, p1y, p2x, p2y, p3x, p3y, p4x, p4y], markingColor, { alpha: 0.85, lineWidth: 2 });
  };
  box(0, depth);
  box(field.w - depth, field.w);
}

// Draw a pointy-top hex tiling over the pitch, projected per-corner so the
// cells follow the perspective. Corners are clamped to the field so edge
// cells meet the touchlines instead of spilling onto the space backdrop.
function drawHexFloor(dl: DrawList, project: Project, field: RenderFrameField): void {
  const r = HEX_RADIUS;
  const colStep = Math.sqrt(3) * r;
  const rowStep = 1.5 * r;
  const hexColor: RGB = [0.16, 0.5, 0.6];

  let row = 0;
  let cy = 0;
  while (cy <= field.h + r) {
    const xOff = row % 2 === 1 ? colStep / 2 : 0;
    let cx = xOff;
    while (cx <= field.w + r) {
      const pts: number[] = [];
      for (let i = 0; i <= 5; i += 1) {
        const ang = ((60 * i - 30) * Math.PI) / 180;
        const wx = Math.min(field.w, Math.max(0, cx + r * Math.cos(ang)));
        const wy = Math.min(field.h, Math.max(0, cy + r * Math.sin(ang)));
        const [sx, sy] = project(wx, wy);
        pts.push(sx, sy);
      }
      dl.polygon("line", pts, hexColor, { alpha: 0.1 });
      cx += colStep;
    }
    row += 1;
    cy += rowStep;
  }
}

function drawGoal(dl: DrawList, project: Project, field: RenderFrameField, g: Rect, color: RGB, lineX: number, backX: number): void {
  const bar = field.crossbar_h;
  const [lfx, lfy, lfs] = project(lineX, g.y); // far post base (on the line)
  const [lnx, lny, lns] = project(lineX, g.y + g.h); // near post base
  const [bfx, bfy, bfs] = project(backX, g.y); // back frame, far
  const [bnx, bny, bns] = project(backX, g.y + g.h); // back frame, near
  const backH = bar * NET_BACK_FRAC;

  // The Lua original's screen-space grid shader (a diagonal mesh pattern
  // texturing the net polygons) is dropped here: it is a shading detail
  // with no bearing on the net's shape or position, and three.js has no
  // equivalent to a LÖVE pixel shader stamped over an arbitrary polygon
  // without a bespoke canvas-texture material. The net panels themselves
  // (their exact screen-space quads, in the same draw order) are kept.
  dl.polygon("fill", [lfx, lfy, bfx, bfy, bfx, bfy - backH * bfs, lfx, lfy - bar * lfs], color, { alpha: 0.3 });
  dl.polygon("fill", [lnx, lny, bnx, bny, bnx, bny - backH * bns, lnx, lny - bar * lns], color, { alpha: 0.3 });
  // Back net.
  dl.polygon("fill", [bfx, bfy, bnx, bny, bnx, bny - backH * bns, bfx, bfy - backH * bfs], color, { alpha: 0.3 });
  // Roof net: crossbar down to the back frame.
  dl.polygon("fill", [lfx, lfy - bar * lfs, lnx, lny - bar * lns, bnx, bny - backH * bns, bfx, bfy - backH * bfs], color, { alpha: 0.22 });

  // The frame: two posts + crossbar, bright so the bloom pass lights it.
  const frameColor: RGB = [0.92, 0.97, 1.0];
  dl.line([lfx, lfy, lfx, lfy - bar * lfs], frameColor, { alpha: 0.95, lineWidth: 3 });
  dl.line([lnx, lny, lnx, lny - bar * lns], frameColor, { alpha: 0.95, lineWidth: 3 });
  dl.line([lfx, lfy - bar * lfs, lnx, lny - bar * lns], frameColor, { alpha: 0.95, lineWidth: 3 });
  // Back frame, thinner and dimmer.
  const backFrameColor: RGB = [0.7, 0.85, 1.0];
  dl.line([bfx, bfy, bfx, bfy - backH * bfs], backFrameColor, { alpha: 0.5, lineWidth: 1 });
  dl.line([bnx, bny, bnx, bny - backH * bns], backFrameColor, { alpha: 0.5, lineWidth: 1 });
  dl.line([bfx, bfy - backH * bfs, bnx, bny - backH * bns], backFrameColor, { alpha: 0.5, lineWidth: 1 });
}

/**
 * One player's screen anchor and presentation payload for one frame: exactly
 * what `pitch.draw` hands the rigged renderer, and nothing three.js-shaped.
 *
 * `sx`/`sy` are screen pixels (the player's feet), `r` the projected body
 * radius in pixels (`roster.radius[index] * scale`) that every derived size --
 * the rigged character's pixels-per-metre included -- scales off.
 */
export interface PlayerAnchor {
  /** Zero-based index into the frame's SoA player arrays. */
  readonly index: number;
  readonly player_id: string;
  readonly sx: number;
  readonly sy: number;
  readonly r: number;
  readonly options: PlayerRenderOptions;
}

/**
 * Every player's anchor for one frame, in the SAME far-to-near order
 * `pitch.draw` adds them to the scene graph (see `depthSortedItems`). Pure:
 * this is the whole sim-to-player-renderer boundary with no three.js in it, so
 * the payload can be asserted against the Lua original headless -- see
 * pitch.spec.ts's differential.
 */
export function playerAnchors(frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions): PlayerAnchor[] {
  const project = pitchProject(frame, vp, opts);
  const anchors: PlayerAnchor[] = [];
  for (const item of depthSortedItems(frame.players, frame.ball)) {
    const index = item.index;
    if (index !== undefined) {
      anchors.push(playerAnchor(frame, index, project));
    }
  }
  return anchors;
}

// The single derivation `playerAnchors` and `pitch.draw` share, so the pure
// path cannot drift from the drawn one.
function playerAnchor(frame: RenderFrame, index: number, project: Project): PlayerAnchor {
  const players = frame.players;
  const [sx, sy, scale] = project(players.x[index] ?? 0, players.y[index] ?? 0);
  return {
    index,
    player_id: frame.roster.ids[index] ?? "",
    sx,
    sy,
    r: (frame.roster.radius[index] ?? 0) * scale,
    options: playerOptions(frame, index),
  };
}

function playerOptions(frame: RenderFrame, index: number): PlayerRenderOptions {
  const roster = frame.roster;
  const players = frame.players;
  const facingX = players.facing_x[index] ?? 0;
  const facingY = players.facing_y[index] ?? 0;
  const diveDirX = players.dive_dir_x[index];
  const diveDirY = players.dive_dir_y[index];
  const combatModel = frame.combat;
  return {
    facing: new Vec2(facingX, facingY),
    is_keeper: roster.is_keeper[index] ?? false,
    controlled: players.controlled[index] ?? false,
    ...(players.dashing[index] !== undefined ? { dashing: players.dashing[index] } : {}),
    ...(players.dive[index] !== undefined ? { dive: players.dive[index] } : {}),
    ...(diveDirX !== undefined && diveDirY !== undefined ? { dive_dir: new Vec2(diveDirX, diveDirY) } : {}),
    ...(players.holding[index] !== undefined ? { holding: players.holding[index] } : {}),
    ...(players.grab[index] !== undefined ? { grab: players.grab[index] } : {}),
    ...(players.throw[index] !== undefined ? { throw: players.throw[index] } : {}),
    ...(players.windup[index] !== undefined ? { windup: players.windup[index] } : {}),
    ...(players.aerial[index] !== undefined ? { aerial: players.aerial[index] } : {}),
    ...(players.aerial_style[index] !== undefined ? { aerial_style: players.aerial_style[index] } : {}),
    ...(players.aerial_outcome[index] !== undefined ? { aerial_outcome: players.aerial_outcome[index] } : {}),
    ...(players.aerial_jump[index] !== undefined ? { aerial_jump: players.aerial_jump[index] } : {}),
    ...(roster.species_shape[index] !== undefined ? { species_shape: roster.species_shape[index] } : {}),
    ...(roster.species_color[index] !== undefined ? { species_color: roster.species_color[index] } : {}),
    ...(roster.teams[index] !== undefined ? { team: roster.teams[index] } : {}),
    // #447. Both are match-constant roster columns rather than per-frame
    // state, and both reach `player_renderer_3d.characterMesh` through
    // exactly this payload -- it is the only channel between the wire and
    // the mesh builder, which is why neither could be honoured before.
    // An EMPTY presentation id is dropped rather than forwarded: the
    // renderer treats "no presentation" as "not on the product path" and
    // falls back to its preview default, so passing `""` through would turn
    // a wiring failure into a silently plausible character.
    ...(roster.presentation_ids[index] ? { presentation_id: roster.presentation_ids[index] } : {}),
    ...(roster.loadout_ids[index] !== undefined ? { loadout_id: roster.loadout_ids[index] } : {}),
    ...(combatModel !== undefined && combatModel.players[index] !== undefined ? { combat: combatModel.players[index] } : {}),
    pose: {
      ...(players.pose_id[index] !== undefined ? { id: players.pose_id[index] } : {}),
      ...(players.pose_priority[index] !== undefined ? { priority: players.pose_priority[index] } : {}),
      ...(players.pose_source[index] !== undefined ? { source: players.pose_source[index] } : {}),
    },
  };
}

/**
 * The part of the pitch that cannot change while a match runs: the arena
 * backdrop, the floor trapezoid, its glow and hex tiling, the markings, the
 * neon outline and the goals.
 *
 * This split is carried over from the Lua (#398/#393), where re-deriving these
 * every frame cost roughly 2,000 interpreter-side projection calls and ~350
 * draw calls — about 2.3 ms of browser frame time, because love.js ships a plain
 * Lua 5.1 interpreter.
 *
 * The Lua's *mechanism* — rendering into two LÖVE canvases and blitting them —
 * did not carry over, and should not: caching a projected 3D scene into a 2D
 * bitmap cannot respond to camera movement, which is why that cache had to
 * bypass itself entirely whenever a follow view was active. Under three.js the
 * static scene is persistent geometry and the GPU redraws it, so the scene graph
 * *is* the cache. What carries over is the invariant — build it once — and the
 * membership of the static set, which is not obvious: the arena frame chevrons
 * pulse with the kickoff banner and are therefore dynamic, drawn over this.
 *
 * Memoised on one entry, like the Lua's single-key cache. The camera offset is
 * part of the key because it is baked into projected coordinates here; combat
 * shake therefore misses, which is the same trade the Lua made when it bypassed
 * on a follow view, and the common no-shake case still hits.
 */
let staticSceneCache: { key: string; commands: readonly DrawCommand[] } | undefined;
let staticSceneBuilds = 0;

function staticSceneCommands(
  field: RenderFrameField,
  vp: PitchViewport,
  opts: PitchDrawOptions,
  arena: ArenaColors,
  theme: ArenaThemeColors,
  project: Project,
): readonly DrawCommand[] {
  const key = JSON.stringify([
    field.w,
    field.h,
    field.goal_home,
    field.goal_away,
    vp.w,
    vp.h,
    arena,
    theme,
    opts.home_color,
    opts.away_color,
    opts.camera_offset ?? null,
    pitch.follow_camera,
  ]);
  const cached = staticSceneCache;
  if (cached !== undefined && cached.key === key) {
    return cached.commands;
  }

  const dl = new DrawList();
  dl.extend(arenaRender.backdropCommands(arena, vp, theme));

  // Pitch surface (projected trapezoid).
  const [ax, ay] = project(0, 0);
  const [bx, by] = project(field.w, 0);
  const [cx, cy] = project(field.w, field.h);
  const [dx, dy] = project(0, field.h);
  dl.polygon("fill", [ax, ay, bx, by, cx, cy, dx, dy], arena.floor_color);

  drawFloorGlow(dl, project, field);
  drawHexFloor(dl, project, field);
  drawMarkings(dl, project, field);

  // Pitch outline (bright neon border).
  dl.polygon("line", [ax, ay, bx, by, cx, cy, dx, dy], arena.rail_color, { alpha: 0.9, lineWidth: 2 });

  // Real goals standing behind the goal line, outside the field.
  const goalHome = field.goal_home;
  const goalAway = field.goal_away;
  drawGoal(dl, project, field, goalHome, opts.home_color, goalHome.x + goalHome.w, goalHome.x);
  drawGoal(dl, project, field, goalAway, opts.away_color, goalAway.x, goalAway.x + goalAway.w);

  const commands = dl.commands;
  staticSceneBuilds += 1;
  staticSceneCache = { key, commands };
  return commands;
}

/** Drop the memoised static scene and its build counter. Tests use this. */
export function resetStaticSceneCache(): void {
  staticSceneCache = undefined;
  staticSceneBuilds = 0;
}

/**
 * How many times the static scene has actually been rebuilt since the last
 * reset. Exported so a test can assert the memo works — a deep-equality check
 * would pass even if the scene were rebuilt on every frame, which is precisely
 * the cost this split exists to remove.
 */
export function staticSceneBuildCount(): number {
  return staticSceneBuilds;
}

// The view every projection in this file uses: `pitch.follow_camera`'s own
// broadcast-follow view when the flag is on, or `undefined` (the whole-pitch
// default). Factored out so `pitchProject` and `characterTilt` (below)
// derive it through the SAME call -- both need to agree on which view is
// "current" for a frame, and a second, independently-written copy of "check
// the flag, call cameraFollow.view" is exactly the kind of thing that drifts
// the moment one call site gets touched and the other does not.
function currentView(field: RenderFrameField): CameraView | undefined {
  return pitch.follow_camera ? cameraFollow.view(field) : undefined;
}

function pitchProject(frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions): Project {
  const field = frame.field;
  const view = currentView(field);
  const cameraField: CameraField = field;
  const cameraViewport: CameraViewport = vp;
  return (wx, wy) => {
    const [sx, sy, scale] = camera.project(wx, wy, cameraField, cameraViewport, undefined, view);
    const offset = opts.camera_offset;
    return [sx + (offset?.x ?? 0), sy + (offset?.y ?? 0), scale];
  };
}

// Everything drawn BEFORE the depth-sorted players/ball: the static scene
// (backdrop, floor, markings, goals -- memoised, see staticSceneCommands),
// the dynamic arena frame chevrons, the ball trail and combat's "under"
// layer. Shared by `pitchDrawCommands` and `pitch.draw`'s rigged branch so
// the two paths cannot silently diverge on ordering.
//
// STADIUM MODE (`pitch.stadium_mode`). When a world-space stadium layer owns
// the backdrop/floor/hex-grid/markings/goals/chevrons (scene.ts's
// `WorldLayer`, composited underneath via `bloom.ts`'s `draw_layers`), this
// screen-space pass must not ALSO draw a second, flat copy of the same
// content on top of it -- so `staticSceneCommands` and
// `arenaRender.frameCommands` are skipped ENTIRELY (not built and then
// discarded: `staticSceneCommands` has its own memoisation counter,
// `staticSceneBuildCount`, which tests assert against, so calling it and
// throwing the result away would still (wrongly) count as a build). Only the
// ball trail and combat's "under" layer -- content anchored to gameplay
// state, not the arena itself -- still draw here. See this file's own
// header (the ONE PASS / DEPTH ZONES sections) for how the remaining
// content still composes correctly with no opaque backdrop rect underneath
// it: `SceneRoot`'s `bloom.draw_layers` clears the canvas on its FIRST pass
// (the world layer) regardless of what pitch.ts draws afterward -- the
// backdrop rect was never what cleared the canvas even in non-stadium mode,
// it was always just the first thing painted onto an already-cleared
// target. Stadium mode simply stops painting a second, redundant backdrop
// over a target that is already correctly cleared and already has real
// stadium geometry behind it.
function drawPitchBeforeItems(dl: DrawList, frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, project: Project): void {
  const field = frame.field;

  if (!pitch.stadium_mode) {
    const arena = opts.arena ?? DEFAULT_ARENA;
    const theme = opts.ui_theme ?? DEFAULT_UI_THEME;

    const [ax, ay] = project(0, 0);
    const [bx, by] = project(field.w, 0);
    const [cx, cy] = project(field.w, field.h);
    const [dx, dy] = project(0, field.h);

    // The static scene: backdrop, floor trapezoid, glow, hex tiling, markings,
    // neon outline and goals. Built once per distinct projection and reused.
    dl.extend(staticSceneCommands(field, vp, opts, arena, theme, project));

    // The arena frame chevrons pulse with the kickoff banner, so they are NOT
    // part of the static scene and are issued live over it. They sit at the
    // trapezoid corners, clear of the goals, so emitting them after the goals
    // instead of before the outline is visually identical. This ordering comes
    // from the Lua's own static/dynamic split (#398) — getting it wrong freezes
    // the kickoff pulse.
    dl.extend(arenaRender.frameCommands(arena, { ax, ay, bx, by, cx, cy, dx, dy }, opts.arena_pulse));
  }

  // Ball trail sits on the ground, under the entities. Kept in BOTH modes:
  // it is gameplay-state content (the ball's own recent path), not arena
  // geometry, so a world-space stadium layer has no equivalent of it to
  // take over.
  dl.extend(effectsModule.effects.drawTrailCommands(project));
  dl.extend(combatRender.drawUnderCommands(frame, project));
}

// Depth-sorted drawables (far first). `index === undefined` is the ball.
// Shared by `playerAnchors` (the pure per-player half) and `pitch.draw` (which
// needs the ball interleaved with the players in that same order -- a rigged
// player must occlude/be occluded by the ball according to its world position,
// not always draw over or under it regardless).
interface Drawable {
  readonly index?: number;
  readonly depth: number;
}

function depthSortedItems(players: RenderFramePlayers, ball: RenderFrameBall): Drawable[] {
  const items: Drawable[] = [];
  for (let index = 0; index < players.count; index += 1) {
    items.push({ index, depth: players.y[index] ?? 0 });
  }
  items.push({ depth: ball.y });
  items.sort((a, b) => a.depth - b.depth);
  return items;
}

// Loose / dribbled ball. (A keeper-held ball is drawn in its hands by the
// keeper avatar, so skip the ground ball then.) The shadow stays on the
// ground and shrinks/fades with height; the ball lifts by its height.
function drawLooseBallCommands(dl: DrawList, project: Project, ball: RenderFrameBall): void {
  const [sx, sy, scale] = project(ball.x, ball.y);
  const z = ball.z;
  const hk = 1 / (1 + z / 80);
  dl.ellipse("fill", sx, sy, 6 * scale * hk, 3 * scale * hk, [0, 0, 0], { alpha: 0.3 * hk });
  dl.circle("fill", sx, sy - (z + 4) * scale, 5 * scale, [1, 0.95, 0.7]);
}

// Everything drawn AFTER the depth-sorted players/ball: combat's "over"
// layer, the landing reticle, the pass-target preview, the charge meter and
// the effects "over" layer (flashes/sparks). Shared for the same reason
// `drawPitchBeforeItems` is.
function drawPitchAfterItems(dl: DrawList, frame: RenderFrame, opts: PitchDrawOptions, project: Project, now: number): void {
  const players = frame.players;
  const roster = frame.roster;
  const ball = frame.ball;

  dl.extend(combatRender.drawOverCommands(frame, project));

  // Landing reticle: a lofted, loose ball projects where it will come down,
  // so a player can time a run to meet a cross.
  const landingX = ball.landing_x;
  const landingY = ball.landing_y;
  if (landingX !== undefined && landingY !== undefined) {
    const [sx, sy, scale] = project(landingX, landingY);
    const pulse = 0.6 + 0.4 * Math.abs(Math.sin(now * 6));
    dl.circle("line", sx, sy, 12 * scale * pulse, [1, 0.85, 0.35], { alpha: 0.85 * pulse, lineWidth: Math.max(1, 1.5 * scale) });
    dl.circle("line", sx, sy, 7 * scale, [1, 0.85, 0.35], { alpha: 0.4, lineWidth: Math.max(1, 1.5 * scale) });
  }

  // Pass-target preview: a small pulsing double-ring at the intended
  // receiver's feet while the pass button is held.
  const target = frame.control.pass_target;
  if (target !== undefined) {
    // ONE-BASED. frame_buffer.ts's own doc on this field says so outright --
    // "Roster slot, one-based ... recover it as roster.ids[hud.controlled - 1]"
    // -- and match.ts converts the identical field with `slot - 1`. Indexing
    // the zero-based SoA arrays directly drew this ring under the NEXT player,
    // or at the world origin for the last slot, and read that player's team
    // colour too. README rule 5.3: convert one-based to zero-based EXCEPT for
    // wire values, and this is a wire value being used as an index.
    const targetIndex = target - 1;
    const tx = players.x[targetIndex] ?? 0;
    const ty = players.y[targetIndex] ?? 0;
    const [tsx, tsy, tscale] = project(tx, ty);
    const pulse = 0.65 + 0.35 * Math.abs(Math.sin(now * 5));
    const teamColor = roster.teams[targetIndex] === "home" ? opts.home_color : opts.away_color;
    dl.circle("line", tsx, tsy, 10 * tscale * pulse, teamColor, { alpha: 0.85 * pulse, lineWidth: Math.max(1, 1.5 * tscale) });
    dl.circle("line", tsx, tsy, 16 * tscale * pulse, teamColor, { alpha: 0.45 * pulse, lineWidth: Math.max(1, 1.5 * tscale) });
  }

  // Charge meter under the controlled player (soccer-game power bar): warm
  // while charging a shot/punt, cool while charging a pass range.
  const chargeKind = frame.control.charge_kind;
  if (chargeKind !== undefined) {
    const amt = frame.control.charge;
    const ccol: RGB = chargeKind === "shot" ? [1, 0.72, 0.3] : [0.45, 0.85, 1];
    const label = chargeKind === "shot" ? "SHOT" : "PASS";
    // One-based, same as `pass_target` above -- see that comment. Without the
    // conversion the charge meter drew under the wrong player entirely, which
    // is what "the charging kick isn't there" looks like from the outside.
    const controlledIndex = frame.control.controlled - 1;
    const px = players.x[controlledIndex] ?? 0;
    const py = players.y[controlledIndex] ?? 0;
    const [sx, sy, scale] = project(px, py);
    const w = 34 * scale;
    const h = Math.max(3, 4 * scale);
    const y0 = sy + 12 * scale;
    dl.rect("fill", sx - w / 2, y0, w, h, [0, 0, 0], { alpha: 0.55 });
    dl.rect("fill", sx - w / 2, y0, w * amt, h, ccol, { alpha: 0.95 });
    dl.rect("line", sx - w / 2, y0, w, h, [1, 1, 1], { alpha: 0.35 });
    for (let i = 1; i <= 4; i += 1) {
      const tickX = sx - w / 2 + (w * i) / 5;
      dl.line([tickX, y0, tickX, y0 + h], [1, 1, 1], { alpha: 0.35 });
    }
    dl.text(label, sx - w / 2, y0 + h + 1, w, "center", ccol, { alpha: 0.95 });
  }

  // Flashes/sparks ride on top of everything.
  dl.extend(effectsModule.effects.drawOverCommands(project));
}

/**
 * Every flat, screen-space `DrawCommand` one frame of the pitch produces:
 * arena backdrop and frame, the projected floor and its markings, the goals,
 * the ball trail and combat's "under" layer, the loose ball, and the overlay
 * layer (landing reticle, pass preview, charge meter, effects "over").
 *
 * PLAYERS ARE NOT IN HERE, and that is the point rather than an omission. A
 * player is a rigged `THREE.SkinnedMesh` in metre space, not a draw command
 * (see file header); the pure, headless-testable half of drawing one is
 * `playerAnchors` above. Until #415 this function ALSO emitted a procedural 2D
 * billboard per player, which is what made "the whole frame as commands"
 * possible -- and what let a rig-build failure downgrade the visible product
 * silently.
 *
 * Pure, and the fully-tested reference for everything it does cover: the Lua
 * differential in pitch.spec.ts compares this output command-for-command
 * against a capture of `game.render.pitch`.
 *
 * It has no product caller of its own -- `pitch.draw` builds a scene graph,
 * not a command list -- but it is not a parallel implementation either: its
 * three parts (`drawPitchBeforeItems`, `drawLooseBallCommands`,
 * `drawPitchAfterItems`) are the SAME functions `pitch.draw` runs, in the same
 * order. That is what makes asserting on this output an assertion about the
 * frame the product actually draws.
 */
export function pitchDrawCommands(frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, now = 0): DrawCommand[] {
  const dl = new DrawList();
  const ball = frame.ball;
  const project = pitchProject(frame, vp, opts);

  drawPitchBeforeItems(dl, frame, vp, opts, project);

  if (ball.visible) {
    drawLooseBallCommands(dl, project, ball);
  }

  drawPitchAfterItems(dl, frame, opts, project, now);

  return dl.commands;
}

// The elevation tilt (see file header's ELEVATION bullet) as a fixed
// quaternion, computed once at module load: it is the SAME rotation for
// every character every frame under the FIXED trapezoid projection (a
// property of the shared camera's fixed viewing angle, not of any one
// player), so there is nothing to recompute per player. Rotation about the
// shared camera's local X axis (== world X here, since the camera has no
// roll/pitch of its own -- see scene.ts's `SceneRoot`), matching
// `player_renderer_3d.ts`'s exported `ELEVATION` constant.
const ELEVATION_TILT = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), playerRenderer3d.ELEVATION);

// CHARACTER TILT COHERENCE. `ELEVATION_TILT` above approximates the fixed
// trapezoid's implied viewing angle -- there is no REAL camera behind that
// projection to match exactly, just a screen-space approximation tuned to
// look right. Under `camera.perspective_mode` there IS a real camera
// (`camera.rigAngleRad`, derived from the exact same `PerspectiveRig` the
// mat4 projection and `scene.ts`'s `worldCamera` both use), and it looks at
// the pitch from a specific, computed angle that is not generally
// `playerRenderer3d.ELEVATION` -- using the fixed constant there would tilt
// every rigged character to an angle the actual camera is not at, which
// reads as visibly wrong (the character's own silhouette/shading implies a
// different camera than the one actually rendering it) the moment a
// character turns and its asymmetry becomes visible. `characterTilt` picks
// the RIGHT rotation for whichever camera is actually active this frame,
// computed ONCE per `pitch.draw` call (not per character) via `currentView`
// -- the same reasoning `ELEVATION_TILT` itself already applies (one shared
// camera pose, not a per-player one), just now conditional on the mode.
function characterTilt(field: RenderFrameField): THREE.Quaternion {
  if (!camera.perspective_mode) {
    return ELEVATION_TILT;
  }
  const angle = camera.rigAngleRad(field, currentView(field));
  return new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), angle);
}

// A rigged character's own local Z (front-to-back thickness) scale, applied
// by `riggedCharacterObject` -- see that function's SCALE comment for the
// live-GL-verified defect this constant fixes (uniform `ppm` on Z clips
// characters against the camera's near plane). Chosen small relative to the
// entity depth zone's own width (`ENTITY_Z_FAR - ENTITY_Z_NEAR` = 0.58) so a
// character's full depth extent (~1-2 local metres) stays a thin slice of
// that zone -- comfortably inside the frustum -- while staying nonzero so
// intra-character depth testing (an arm in front of a torso) still works.
const CHARACTER_DEPTH_SCALE = 0.05;

/**
 * Wraps a rigged character's own local-space mesh (`player_renderer_3d.ts`'s
 * `characterMesh` -- posed, coloured, yawed, in metres) into the world-space
 * object `pitch.draw` actually adds to `group`, applying everything that
 * used to live in `player_renderer_3d.ts`'s per-character camera -- see file
 * header's "ONE PASS, ONE DEPTH BUFFER" section for the full derivation of
 * each piece below. `r` is this player's projected on-screen body radius in
 * pixels (`playerAnchor`'s `r`); `sx`/`sy`/`z` are the
 * screen position and real depth-tested world z (see `depthToZ`).
 *
 * THROWS when `player_renderer_3d.ts`'s `ppmForRadius` returns `undefined`,
 * which happens only when `build()` failed -- see `pitch.draw`'s own throw for
 * why that is now loud (AGENTS.md §7) rather than a quiet downgrade.
 *
 * COMPOSITION ORDER, and why it is not arbitrary (verified against a live GL
 * context -- see this port's report for before/after screenshots):
 * `characterMesh` already baked the character's own YAW onto `mesh`'s local
 * quaternion. The elevation tilt is PREMULTIPLIED onto that SAME quaternion
 * -- `mesh.quaternion = tilt * yaw`, i.e. tilt composed after yaw, exactly
 * the `finalQuat` formula in the file header -- rather than living on the
 * wrapper, because the wrapper also carries the Y-inverting mirror (folded
 * into its negative-Y scale), and a mirror does not commute with a tilt
 * about a DIFFERENT axis (it commutes fine with the yaw, a rotation about
 * the mirrored axis itself, which is why yaw can sit on either object with
 * no visible difference -- but not with a tilt about X, which mixes Y and
 * Z). Putting BOTH rotations on `mesh` and leaving the wrapper
 * rotation-free guarantees the vertex pipeline is rotate (yaw, then tilt) ->
 * scale (the Y-mirror) -> translate, matching how the old two-step render-
 * then-flip-the-image process actually composed (the tilt was baked into the
 * rendered pixels before the image-space flip was ever applied).
 *
 * `tilt` is `characterTilt`'s return for the current frame (`ELEVATION_TILT`
 * under the fixed trapezoid, the real camera's own angle under
 * `perspective_mode`) -- see that function's CHARACTER TILT COHERENCE doc
 * comment. Passed in rather than read from a module constant here so the
 * caller computes it ONCE per `pitch.draw` call, not once per character.
 */
function riggedCharacterObject(mesh: THREE.Object3D, sx: number, sy: number, z: number, r: number, tilt: THREE.Quaternion): THREE.Group {
  const ppm = playerRenderer3d.ppmForRadius(r);
  if (ppm === undefined) {
    throw new Error("pitch.ts: player_renderer_3d.ppmForRadius() declined -- the rigged character build failed; see the preceding 'rigged 3D players disabled (build failed)' warning for the cause");
  }
  mesh.quaternion.premultiply(tilt);

  const wrapper = new THREE.Group();
  // SCALE: ppm (pixels-per-metre for THIS player, from `r = radius * scale`
  // in the caller -- see file header's SCALE bullet) for X/Y. Y is negated:
  // the Y-INVERSION the file header describes, now living on this wrapper
  // instead of `player_renderer_3d.ts`'s old composited plane -- still
  // required for the same reason (`SceneRoot`'s Y-inverted 2D camera
  // convention), just moved. NOT `WebGLRenderTarget.texture.flipY` territory
  // at all anymore -- there is no render target here.
  //
  // Z is DELIBERATELY NOT `ppm`, unlike X/Y -- a real defect this port's
  // report documents finding live: uniform (ppm, -ppm, ppm) was the file
  // header's original proposal, and it renders characters INVISIBLE (or as
  // thin near-plane-clipped fragments). `ppm` is calibrated for SCREEN-PIXEL
  // sizing (tens to low hundreds of units), but world Z in this scene is a
  // THIN bookkeeping axis, not a real spatial dimension at that scale -- the
  // WHOLE entity depth zone spans only `ENTITY_Z_FAR - ENTITY_Z_NEAR` (0.58
  // units, see DEPTH ZONES above), and the shared camera's own near plane
  // sits a mere 0.3 units beyond `ENTITY_Z_FAR`. Scaling a character's ~1-2
  // metre local front-to-back thickness (an outstretched shield, a raised
  // arm) by a pixel-scale `ppm` (25 for a normal player, over 150 for one
  // framed close/large) spreads its world Z across tens to hundreds of
  // units -- annihilated by the near/far clip almost immediately, which is
  // exactly the failure mode found live (VERIFIED WITH A LIVE GL CONTEXT --
  // see this port's report): most players rendered as nothing at all, and a
  // large centred character rendered as nothing rather than "big". Z instead
  // gets its own small, fixed scale, keeping a character's own depth extent
  // a tiny slice of the entity zone -- comfortably inside the frustum, with
  // enough non-zero thickness left for the SAME intra-character depth
  // testing (an arm in front of a torso) the Lua original had via its own
  // depth buffer (see this file's DEPTH ZONES section).
  wrapper.scale.set(ppm, -ppm, CHARACTER_DEPTH_SCALE);
  // POSITION: (sx, sy) screen pixels become this object's world (x, y)
  // directly, since the shared camera's frustum already spans [0, vw] x
  // [0, vh] in exactly that space. DEPTH: `z`, from `depthToZ` -- see DEPTH
  // ZONES above.
  wrapper.position.set(sx, sy, z);
  // Marks this subtree for tests -- same convention as
  // `renderToSprite`'s old `userData.ownedRenderTarget`, minus the
  // disposal implication: there is no owned GPU resource on this wrapper to
  // release (the pooled mesh inside is not this object's to dispose -- see
  // `pitch.draw`'s note on `paint()`/`disposeObject` above).
  wrapper.userData["riggedCharacter"] = true;
  wrapper.add(mesh);
  return wrapper;
}

/** Pitch rendering configuration and the impure orchestrator. See file header. */
export const pitch = {
  // Opt-in broadcast-style following camera. Off by default: it reframes
  // the whole match, so it stays behind a flag until it has been played.
  follow_camera: false,

  // Opt-in: a world-space `WorldLayer` (scene.ts) owns the backdrop, floor,
  // hex grid, markings, goals AND the arena frame chevrons, composited
  // UNDERNEATH this file's screen-space content instead of this file
  // drawing a flat copy of the same thing -- see `drawPitchBeforeItems`'s
  // own "STADIUM MODE" doc comment for exactly what gets skipped and why
  // that is safe with no backdrop rect to clear the canvas. Off by default,
  // matching `follow_camera`'s own reasoning: this changes what a frame
  // looks like, so it stays behind a flag until the app shell (not this
  // package) decides to flip it -- almost always alongside
  // `camera.perspective_mode`, since a flat trapezoid pitch with a 3D
  // stadium underneath it would look broken, but this file does not
  // enforce that pairing itself (see `characterTilt` below for the one
  // place this file DOES branch on `camera.perspective_mode` directly).
  stadium_mode: false,

  /**
   * Impure: paints one frame into `group` (all screen-space 2D content, via
   * draw2d.ts) and adds each player's rigged 3D character (a real,
   * depth-tested `THREE.SkinnedMesh`, from `player_renderer_3d.ts`'s
   * `characterMesh`) directly INTO `group`, wrapped in a small `THREE.Group`
   * (see `riggedCharacterObject` below) that carries its screen position/scale
   * and the elevation tilt, at its own point in the depth-sorted iteration.
   * This is what lets a
   * single later full-scene render (see scene.ts's `SceneRoot.render`) draw
   * the flat content and the rigged characters together in ONE pass, with
   * real depth testing between them and the ball -- see this file's "ONE
   * PASS, ONE DEPTH BUFFER" header section and the "DEPTH ZONES" section
   * above for the full reconciliation.
   *
   * NO `renderer` PARAMETER (#415). One used to be threaded in here purely as
   * a gate -- "is a live render coming at all?", answered so the frame could
   * fall back to the procedural billboard when it was not. `characterMesh`
   * needs no `THREE.WebGLRenderer` (there is no per-character render pass
   * anymore) and there is no longer anything to fall back TO, so the parameter
   * had exactly one remaining consumer and it was deleted with the billboard.
   * A useful side effect: this function now cannot rasterize even by accident,
   * which the spec used to have to assert with a call-counting stub.
   *
   * THROWS if a player's rigged character cannot be built -- see the throw
   * below. Untested beyond object-graph shape/ordering -- see file header's
   * scope note.
   */
  draw(group: THREE.Group, frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, now = 0): void {
    const players = frame.players;
    const ball = frame.ball;
    const project = pitchProject(frame, vp, opts);
    // Computed once for the whole frame, not per character -- see
    // `characterTilt`'s own CHARACTER TILT COHERENCE doc comment.
    const tilt = characterTilt(frame.field);

    // One `paint()` clears `group` and populates everything before the
    // depth-sorted items; everything from here on uses `appendCommands` (or
    // `group.add` directly for a rigged character) so it interleaves into the
    // SAME child list instead of wiping out what came before it. Left at
    // draw2d.ts's default z (0 == BACKDROP_Z, see DEPTH ZONES above) --
    // deliberately no `{ z: BACKDROP_Z }` here since that constant only
    // documents the invariant, it does not need to be applied.
    //
    // NOTE ON POOLED CHARACTERS SURVIVING THIS CLEAR: `paint()` disposes
    // every child it removes (`draw2d.ts`'s `disposeObject`), which would be
    // fatal for a pooled, frame-to-frame-REUSED character mesh -- except
    // `disposeObject` only inspects direct `Mesh`/`Line`/`Sprite` children,
    // never recursing into a plain `THREE.Group`'s own children, and every
    // rigged character below is added as exactly that: a fresh wrapper
    // `THREE.Group` holding the pooled mesh. The wrapper is discarded (and
    // GC'd) every frame; the pooled mesh it held onto is not -- see
    // `player_renderer_3d.ts`'s `characterPool` doc comment.
    const before = new DrawList();
    drawPitchBeforeItems(before, frame, vp, opts, project);
    // `batchStrokes`: this pass is the arena backdrop, the floor trapezoid and
    // its hex tiling, the markings and the goals -- overwhelmingly strokes,
    // and measured at 353 separate `THREE.Line` objects, one draw call each,
    // on a frame totalling ~455. Nothing here interleaves with the depth-
    // sorted entity pass below (that appends AFTER this paint), so merging
    // consecutive same-state runs cannot reorder anything visible. See
    // `PaintOptions.batchStrokes` for why it is opt-in rather than default.
    paint(group, before.commands, { batchStrokes: true });

    for (const item of depthSortedItems(players, ball)) {
      // Real depth-testable z for this entity -- see DEPTH ZONES above.
      const z = depthToZ(item.depth, frame.field.h);
      const index = item.index;
      if (index !== undefined) {
        const anchor = playerAnchor(frame, index, project);
        const v = viewState.get(anchor.player_id);
        const mesh = playerRenderer3d.characterMesh(anchor.player_id, v, anchor.options, now);
        // FAIL LOUD (#415, AGENTS.md §7). `characterMesh` declines only when
        // `player_renderer_3d.build()` failed, and `build()` fails on an
        // invalid vertex index or missing rig3d content -- programmer errors,
        // not environment conditions. This used to drop the player to a
        // procedural billboard, re-checked PER PLAYER, so half a roster could
        // quietly downgrade mid-frame and the match still looked like a match.
        // During #403 "the reporter may have been on the fallback without
        // realising" stayed a live hypothesis for hours precisely because that
        // could happen invisibly. Rendering nothing here, or continuing past a
        // missing player, would reproduce exactly that; a thrown error names
        // the player and stops the frame instead. `build()`'s own
        // `console.warn` (see that module) still carries the underlying cause.
        if (mesh === undefined) {
          throw new Error(`pitch.ts: no rigged character for player "${anchor.player_id}" (roster index ${String(index)}) -- player_renderer_3d.build() failed; see the preceding 'rigged 3D players disabled (build failed)' warning for the cause`);
        }
        group.add(riggedCharacterObject(mesh, anchor.sx, anchor.sy, z, anchor.r, tilt));
      } else if (ball.visible) {
        const ballCommands = new DrawList();
        drawLooseBallCommands(ballCommands, project, ball);
        appendCommands(group, ballCommands.commands, { z });
      }
    }

    // Always on top, regardless of any entity's real depth -- see DEPTH
    // ZONES above.
    const after = new DrawList();
    drawPitchAfterItems(after, frame, opts, project, now);
    appendCommands(group, after.commands, { depthTest: false, renderOrder: OVERLAY_RENDER_ORDER });
  },
};
