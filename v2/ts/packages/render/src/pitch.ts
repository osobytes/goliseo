// Ported from game/render/pitch.lua.
//
// Draws a `RenderFrame` through the camera projection as a perspective pitch
// with depth-sorted billboard players. This module never sees a
// `MatchState` -- everything it draws comes off the versioned payload built
// by `render.frame` (Rust, see below), which is the whole point: the same
// payload can be handed to a renderer not written in Lua. The only things
// read from outside it are renderer-owned presentation state (`view_state`
// gait/lean, the particle systems) and per-match theming.
//
// `pitchDrawCommands` is the PURE, tested path: it produces the exact
// depth-sorted `DrawCommand[]` (draw2d.ts) the Lua original would have drawn
// for a frame where every player uses the procedural 2.5D renderer
// (`player_renderer.ts`). `pitch.draw` is the impure orchestrator that also
// honors `pitch.rigged_players`, calling into `player_renderer_3d.ts`'s
// genuine `THREE.SkinnedMesh` pass per player where available.
//
// SCOPE NOTE. Compositing a per-player 3D rigged pass with the flat
// screen-space 2D content at the correct point in the painter's-algorithm
// depth sort (so a rigged player occludes/is occluded exactly like its
// procedural billboard would) is real GPU-integration work: it needs a live
// `WebGLRenderer` to verify at all, and v2/README.md #1 scopes "a running
// app" / the wasm-bindings glue out of this milestone. `pitch.draw` below
// wires the two renderers together as faithfully as the Lua original (same
// per-player choice, same call sites), but is -- like the rest of the
// GPU-adjacent surface in this package -- untested. `pitchDrawCommands`
// stays the fully-tested reference for the procedural path.
//
// Boundary note (v2/README.md rule 6.7): `RenderFrame` is the (Rust)
// `render/frame.lua` producer's output (`render/**` -> Rust
// `crates/gc-render`) -- only the fields this module reads are declared
// locally. `ArenaData` (`data/arenas.lua`) is Rust-owned (`crates/gc-data`);
// `DEFAULT_ARENA` below mirrors its only current entry (`helios_crown`) as
// the same "no arena supplied" fallback the Lua original had, without
// importing Rust-owned content.

import * as THREE from "three";
import { Vec2 } from "@gc/core";
import type { CombatPresentationModel } from "@gc/presentation";
import { camera, type CameraField, type CameraView, type CameraViewport } from "./camera.ts";
import { cameraFollow } from "./camera_follow.ts";
import * as arenaRender from "./arena.ts";
import type { ArenaColors, ArenaThemeColors } from "./arena.ts";
import * as combatRender from "./combat.ts";
import * as effectsModule from "./effects.ts";
import * as playerRenderer from "./player_renderer.ts";
import type { AerialOutcome, AerialStyle, PlayerRenderOptions, SpeciesShape } from "./player_renderer.ts";
import * as playerRenderer3d from "./player_renderer_3d.ts";
import { viewState } from "./view_state.ts";
import { DrawList, paint, type DrawCommand, type Project, type RGB } from "./draw2d.ts";

const HEX_RADIUS = 26; // world units, centre to corner
const NET_BACK_FRAC = 0.55; // back frame height as a fraction of the crossbar

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
    ...(combatModel !== undefined && combatModel.players[index] !== undefined ? { combat: combatModel.players[index] } : {}),
    pose: {
      ...(players.pose_id[index] !== undefined ? { id: players.pose_id[index] } : {}),
      ...(players.pose_priority[index] !== undefined ? { priority: players.pose_priority[index] } : {}),
      ...(players.pose_source[index] !== undefined ? { source: players.pose_source[index] } : {}),
    },
  };
}

/**
 * Render the whole pitch + entities for one frame, using the procedural 2.5D
 * player renderer throughout. Pure, tested reference -- see file header for
 * why the rigged-3D path is not folded in here.
 */
export function pitchDrawCommands(frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, now = 0): DrawCommand[] {
  const dl = new DrawList();
  const field = frame.field;
  const roster = frame.roster;
  const players = frame.players;
  const ball = frame.ball;
  const arena = opts.arena ?? DEFAULT_ARENA;
  const theme = opts.ui_theme ?? DEFAULT_UI_THEME;

  const view: CameraView | undefined = pitch.follow_camera ? cameraFollow.view(field) : undefined;
  const cameraField: CameraField = field;
  const cameraViewport: CameraViewport = vp;
  const project: Project = (wx, wy) => {
    const [sx, sy, scale] = camera.project(wx, wy, cameraField, cameraViewport, undefined, view);
    const offset = opts.camera_offset;
    return [sx + (offset?.x ?? 0), sy + (offset?.y ?? 0), scale];
  };

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
  dl.extend(arenaRender.frameCommands(arena, { ax, ay, bx, by, cx, cy, dx, dy }, opts.arena_pulse));

  // Real goals standing behind the goal line, outside the field.
  const goalHome = field.goal_home;
  const goalAway = field.goal_away;
  drawGoal(dl, project, field, goalHome, opts.home_color, goalHome.x + goalHome.w, goalHome.x);
  drawGoal(dl, project, field, goalAway, opts.away_color, goalAway.x, goalAway.x + goalAway.w);

  // Ball trail sits on the ground, under the entities.
  dl.extend(effectsModule.effects.drawTrailCommands(project));
  dl.extend(combatRender.drawUnderCommands(frame, project));

  // Depth-sorted drawables (far first). `index === undefined` is the ball.
  interface Drawable {
    readonly index?: number;
    readonly depth: number;
  }
  const items: Drawable[] = [];
  for (let index = 0; index < players.count; index += 1) {
    items.push({ index, depth: players.y[index] ?? 0 });
  }
  items.push({ depth: ball.y });
  items.sort((a, b) => a.depth - b.depth);

  for (const item of items) {
    const index = item.index;
    if (index !== undefined) {
      const px = players.x[index] ?? 0;
      const py = players.y[index] ?? 0;
      const [sx, sy, scale] = project(px, py);
      const r = (roster.radius[index] ?? 0) * scale;
      const color = roster.teams[index] === "home" ? opts.home_color : opts.away_color;
      const v = viewState.get(roster.ids[index] ?? "");
      dl.extend(playerRenderer.playerDrawCommands(sx, sy, r, color, v, playerOptions(frame, index)));
    } else if (ball.visible) {
      // Loose / dribbled ball. (A keeper-held ball is drawn in its hands by
      // the keeper avatar, so skip the ground ball then.) The shadow stays
      // on the ground and shrinks/fades with height; the ball lifts by its
      // height.
      const [sx, sy, scale] = project(ball.x, ball.y);
      const z = ball.z;
      const hk = 1 / (1 + z / 80);
      dl.ellipse("fill", sx, sy, 6 * scale * hk, 3 * scale * hk, [0, 0, 0], { alpha: 0.3 * hk });
      dl.circle("fill", sx, sy - (z + 4) * scale, 5 * scale, [1, 0.95, 0.7]);
    }
  }

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
    const tx = players.x[target] ?? 0;
    const ty = players.y[target] ?? 0;
    const [tsx, tsy, tscale] = project(tx, ty);
    const pulse = 0.65 + 0.35 * Math.abs(Math.sin(now * 5));
    const teamColor = roster.teams[target] === "home" ? opts.home_color : opts.away_color;
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
    const controlled = frame.control.controlled;
    const px = players.x[controlled] ?? 0;
    const py = players.y[controlled] ?? 0;
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

  return dl.commands;
}

/** Pitch rendering configuration and the impure orchestrator. See file header. */
export const pitch = {
  // Rigged 3D players default to on, matching the Lua original's direction.
  // See `player_renderer_3d.ts`'s header for how the love.js-specific
  // "unavailable, not crashed" contract simplifies once a real browser JS
  // exception model replaces love.js's non-catchable runtime abort.
  rigged_players: true,

  // Opt-in broadcast-style following camera. Off by default: it reframes
  // the whole match, so it stays behind a flag until it has been played.
  follow_camera: false,

  /**
   * Impure: paints one frame into `group` (all screen-space 2D content, via
   * draw2d.ts), and -- when `renderer` is supplied and the rigged pass is
   * available -- composites each player's rigged 3D character on top of it
   * through `player_renderer_3d.ts`'s own per-character render pass,
   * exactly as the Lua original's per-player renderer choice does. Without
   * a `renderer` (or when the rigged pass is unavailable/disabled), every
   * player falls back to the procedural 2.5D renderer via `group`. Untested
   * -- see file header's scope note.
   */
  draw(group: THREE.Group, frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, renderer?: THREE.WebGLRenderer, now = 0): void {
    const riggedActive = pitch.rigged_players && renderer !== undefined && playerRenderer3d.available();
    if (!riggedActive) {
      paint(group, pitchDrawCommands(frame, vp, opts, now));
      return;
    }
    // At least one player may use the rigged pass: draw everything except
    // players via the pure command list, then draw players individually so
    // each can pick its renderer, preserving the original depth sort.
    const field = frame.field;
    const roster = frame.roster;
    const players = frame.players;
    const view = pitch.follow_camera ? cameraFollow.view(field) : undefined;
    const project: Project = (wx, wy) => {
      const [sx, sy, scale] = camera.project(wx, wy, field, vp, undefined, view);
      const offset = opts.camera_offset;
      return [sx + (offset?.x ?? 0), sy + (offset?.y ?? 0), scale];
    };
    paint(group, pitchWithoutPlayersCommands(frame, vp, opts, now));
    for (let index = 0; index < players.count; index += 1) {
      const px = players.x[index] ?? 0;
      const py = players.y[index] ?? 0;
      const [sx, sy, scale] = project(px, py);
      const r = (roster.radius[index] ?? 0) * scale;
      const color = roster.teams[index] === "home" ? opts.home_color : opts.away_color;
      const v = viewState.get(roster.ids[index] ?? "");
      const options = playerOptions(frame, index);
      if (playerRenderer3d.available()) {
        playerRenderer3d.draw(renderer, sx, sy, r, vp.w, vp.h, v, options, now);
      } else {
        paint(group, playerRenderer.playerDrawCommands(sx, sy, r, color, v, options));
      }
    }
  },
};

// Everything `pitchDrawCommands` draws except the depth-sorted players
// (and the ball, drawn in its usual depth slot). Used only by `pitch.draw`'s
// mixed-renderer path above.
function pitchWithoutPlayersCommands(frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, now: number): DrawCommand[] {
  const full = pitchDrawCommandsInternal(frame, vp, opts, now, true);
  return full;
}

// Shares its body with `pitchDrawCommands`; `skipPlayers` is only set by the
// mixed-renderer path in `pitch.draw`.
function pitchDrawCommandsInternal(frame: RenderFrame, vp: PitchViewport, opts: PitchDrawOptions, now: number, skipPlayers: boolean): DrawCommand[] {
  if (!skipPlayers) {
    return pitchDrawCommands(frame, vp, opts, now);
  }
  const framePlayerless: RenderFrame = { ...frame, players: { ...frame.players, count: 0 } };
  return pitchDrawCommands(framePlayerless, vp, opts, now);
}
