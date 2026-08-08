// Ported from game/render/effects.lua.
//
// The "juice" layer: turns one-frame sim events (shots, passes, traps) into
// short-lived particles, and samples a fading trail behind a fast loose
// ball. Renderer-side state only -- the sim stays pure. Rollback actions use
// stable event keys; `update` remains the offline adapter. Drawing (the
// `*Commands` functions) projects and returns paintable content -- see
// draw2d.ts.
//
// Almost the entire module is pure bookkeeping (particle/trail arrays keyed
// by event id) exactly like `replay.ts`'s buffer, so it is fully testable
// headless; only `drawTrail`/`drawOver` (the three.js materialization) are
// impure, and they are thin wrappers around the pure `*Commands` builders.
//
// Boundary notes (v2/README.md rule 6.7): `MatchEvent`/`MatchEventKind` are
// reused from `replay.ts` (same package, already declares the sim-shaped
// slice this module needs) rather than redeclared. `CombatEvent`,
// `RollbackWrappedEvent`, `RollbackEventDiff` are `@gc/presentation`'s
// (a declared dependency). `GameSettings` (`game/settings.lua`) would be
// `@gc/screens`, which `@gc/render`'s package.json does not depend on, so
// only the two fields this module reads are declared locally and injected,
// matching `arena.ts`'s treatment of `game/ui/theme.lua`.

import * as THREE from "three";
import type { CombatEvent, RollbackEventDiff, RollbackWrappedEvent } from "@gc/presentation";
import { combatFeedback } from "@gc/presentation";
import type { MatchEvent } from "./replay.ts";
import { DrawList, paint, type DrawCommand, type Project, type RGB } from "./draw2d.ts";

// Match the ball's billboard lift in pitch.lua so flashes/trails sit on the ball.
const BALL_LIFT = 4;
const TRAIL_LIFE = 0.32; // base seconds a trail dot lingers (hot dots last longer)
const TRAIL_SPACING = 7; // world units between samples (fps-independent spacing)
const TRAIL_MIN_SPEED = 80; // only trail a ball moving faster than this
const TRAIL_HOT_SPEED = 900; // ball speed that reads as full "heat" (charged shot)

type ParticleKind = "spark" | "ring" | "glyph";

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
  max: number;
  size: number;
  color: RGB;
  kind: ParticleKind;
  event_id?: string;
  glyph?: string;
}

interface TrailDot {
  x: number;
  y: number;
  z: number;
  heat: number;
  life: number;
  max: number;
}

export interface EffectsSettings {
  readonly screen_shake: boolean;
  readonly bloom: boolean;
}

export interface EffectsDiagnostics {
  readonly particle_count: number;
  readonly trail_count: number;
  readonly speculative_ids: readonly string[];
  readonly suppressed_ids: readonly string[];
  readonly confirmed_ids: readonly string[];
}

export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

export interface ReadabilityObservation {
  readonly active_feedback_count: number;
  readonly concurrent_event_count: number;
  readonly overlap_pair_count: number;
  readonly occluded_count: number;
  readonly ball_masked: boolean;
  readonly hud_masked: boolean;
  readonly non_color_only: boolean;
}

/** The slice of `MatchState` (`sim/match.lua`) `sample_ball`/`consume` read. */
export interface EffectsMatchState {
  readonly ball: { readonly x: number; readonly y: number };
  readonly ball_vel: { length(): number };
  readonly ball_z: number;
  readonly owner?: number;
  readonly events: readonly MatchEvent[];
}

let particles: Particle[] = [];
let trail: TrailDot[] = [];
let lastSample: { x: number; y: number } | undefined;
let speculativeEvents = new Set<string>();
let suppressedEvents = new Set<string>();
let confirmedEvents = new Set<string>();
let reducedMotion = false;
let reducedFlash = false;

function add(
  x: number,
  y: number,
  vx: number,
  vy: number,
  life: number,
  size: number,
  color: RGB,
  kind: ParticleKind,
  eventId?: string,
  glyphName?: string,
): void {
  particles.push({
    x,
    y,
    vx,
    vy,
    life,
    max: life,
    size,
    color,
    kind,
    ...(eventId !== undefined ? { event_id: eventId } : {}),
    ...(glyphName !== undefined ? { glyph: glyphName } : {}),
  });
}

function stableSeed(id: string): number {
  let value = 2166136261;
  for (let index = 0; index < id.length; index += 1) {
    const byte = id.codePointAt(index) ?? 0;
    value = (value * 16777619 + byte) % 2147483647;
  }
  return value;
}

function deterministicUnit(seed: number): readonly [number, number] {
  const nextSeed = (seed * 48271) % 2147483647;
  return [nextSeed, nextSeed / 2147483647];
}

function combatBurst(x: number, y: number, n: number, speed: number, life: number, size: number, color: RGB, eventId: string): void {
  const count = reducedFlash ? Math.min(2, n) : n;
  let seed = stableSeed(eventId);
  for (let i = 0; i < count; i += 1) {
    let angleUnit: number;
    let speedUnit: number;
    [seed, angleUnit] = deterministicUnit(seed);
    [seed, speedUnit] = deterministicUnit(seed);
    const angle = angleUnit * Math.PI * 2;
    let scalar = speed * (0.45 + speedUnit * 0.55);
    if (reducedMotion) {
      scalar = scalar * 0.25;
    }
    add(x, y, Math.cos(angle) * scalar, Math.sin(angle) * scalar, reducedFlash ? life * 0.55 : life, size, color, "spark", eventId);
  }
}

function glyphParticle(x: number, y: number, life: number, size: number, color: RGB, eventId: string, glyphName: string): void {
  add(x, y, 0, 0, life, size, color, "glyph", eventId, glyphName);
}

// A radial spray of sparks. Direction is randomized -- presentation-only
// "juice", never fed back into simulation state, so `Math.random()` is fine
// here (unlike anything on the determinism path, v2/README.md #2).
function burst(x: number, y: number, n: number, speed: number, life: number, size: number, color: RGB, eventId?: string): void {
  for (let i = 0; i < n; i += 1) {
    const a = Math.random() * Math.PI * 2;
    const sp = speed * (0.45 + Math.random() * 0.55);
    add(x, y, Math.cos(a) * sp, Math.sin(a) * sp, life * (0.6 + Math.random() * 0.4), size, color, "spark", eventId);
  }
}

function ring(x: number, y: number, life: number, size: number, color: RGB, eventId?: string): void {
  add(x, y, 0, 0, life, size, color, "ring", eventId);
}

const FAMILY_COLORS: Readonly<Record<string, RGB>> = {
  unarmed: [0.55, 0.95, 1],
  guard: [0.75, 0.9, 1],
  light_melee: [1, 0.72, 0.28],
  ranged: [1, 0.45, 0.78],
};

function spawnCombatEvent(event: CombatEvent, eventId: string): void {
  const link = combatFeedback.link(event, eventId);
  const disposition = link.disposition;
  const color = (event.family_id !== undefined ? FAMILY_COLORS[event.family_id] : undefined) ?? [1, 0.9, 0.55];
  const life = reducedFlash ? 0.16 : 0.32;
  const size = reducedFlash ? 8 : 13;
  const glyphName = disposition.glyph;

  switch (disposition.vfx) {
    case "none":
      return;
    case "commit":
      ring(event.x, event.y, life, size, color, eventId);
      glyphParticle(event.x, event.y, life, size, color, eventId, glyphName);
      return;
    case "projectile_spawn":
      glyphParticle(event.x, event.y, life, size, color, eventId, glyphName);
      combatBurst(event.x, event.y, 4, 120, life, 2, color, eventId);
      return;
    case "projectile_expire":
      glyphParticle(event.x, event.y, life, size, color, eventId, glyphName);
      return;
    case "contact_hit":
    case "contact_extended":
      ring(event.x, event.y, life, size + 5, color, eventId);
      glyphParticle(event.x, event.y, life, size + 2, color, eventId, glyphName);
      combatBurst(event.x, event.y, 8, 230, life, 3, color, eventId);
      return;
    case "contact_guarded":
    case "guard_recoil":
      ring(event.x, event.y, life, size + 2, color, eventId);
      ring(event.x, event.y, life * 1.15, size + 8, color, eventId);
      glyphParticle(event.x, event.y, life, size + 2, color, eventId, glyphName);
      return;
    case "contact_immune":
    case "contact_superseded":
      glyphParticle(event.x, event.y, life, size + 2, color, eventId, glyphName);
      return;
    case "ball_spill":
      ring(event.x, event.y, life, size + 6, [1, 0.95, 0.7], eventId);
      // Keep the authoritative event anchored to the ball while placing the
      // semantic glyph just above it so the ball remains readable.
      glyphParticle(event.x, event.y - 32, life, size + 3, [1, 0.95, 0.7], eventId, glyphName);
      combatBurst(event.x, event.y, 6, 180, life, 2, [1, 0.95, 0.7], eventId);
      return;
    case "forced":
      glyphParticle(event.x, event.y, life, size + 3, color, eventId, glyphName);
      combatBurst(event.x, event.y, 5, 150, life, 2, color, eventId);
      return;
    case "rejected":
      return;
  }
}

function spawnEvent(e: MatchEvent, eventId?: string): void {
  switch (e.kind) {
    case "shot":
      ring(e.x, e.y, 0.3, 16, [1, 0.95, 0.7], eventId);
      burst(e.x, e.y, 10, 230, 0.35, 3, [1, 0.9, 0.55], eventId);
      return;
    case "pass":
      ring(e.x, e.y, 0.24, 11, [0.8, 0.95, 1], eventId);
      burst(e.x, e.y, 5, 150, 0.26, 2, [0.8, 0.95, 1], eventId);
      return;
    case "touch":
      ring(e.x, e.y, 0.28, 13, [1, 1, 1], eventId);
      return;
    case "tackle":
      // A hard hit: punchier and warmer than the rest.
      ring(e.x, e.y, 0.34, 18, [1, 0.6, 0.3], eventId);
      burst(e.x, e.y, 12, 260, 0.4, 3, [1, 0.7, 0.4], eventId);
      return;
    case "catch":
    case "claim":
      // Clean grab/gather: a tight, confident double ring, no spray.
      ring(e.x, e.y, 0.3, 12, [0.8, 1, 0.9], eventId);
      ring(e.x, e.y, 0.38, 20, [0.6, 1, 0.8], eventId);
      return;
    case "parry":
      // Deflection: a sharp cool spark fan.
      ring(e.x, e.y, 0.3, 16, [0.7, 0.9, 1], eventId);
      burst(e.x, e.y, 9, 220, 0.34, 3, [0.7, 0.9, 1], eventId);
      return;
    case "header":
      // Aerial flick: a crisp high ring.
      ring(e.x, e.y, 0.28, 15, [0.9, 1, 1], eventId);
      burst(e.x, e.y, 5, 170, 0.28, 2, [0.9, 1, 1], eventId);
      return;
    case "volley":
      // A volley is violence: big hot flash.
      ring(e.x, e.y, 0.34, 20, [1, 0.8, 0.45], eventId);
      burst(e.x, e.y, 12, 280, 0.4, 3, [1, 0.8, 0.45], eventId);
      return;
    case "bicycle":
      // Acrobatic strike: a larger double flash, whether clean or wild.
      ring(e.x, e.y, 0.38, 24, [1, 0.65, 0.35], eventId);
      ring(e.x, e.y, 0.28, 14, [0.85, 0.95, 1], eventId);
      burst(e.x, e.y, 15, 310, 0.44, 3, [1, 0.7, 0.4], eventId);
      return;
    case "reception": {
      // First touch: clean is compact; a heavy or missed touch splashes wider.
      const clean = e.outcome === "clean";
      ring(e.x, e.y, clean ? 0.2 : 0.3, clean ? 9 : 16, [0.65, 1, 0.85], eventId);
      if (!clean) {
        burst(e.x, e.y, 5, 150, 0.26, 2, [0.65, 1, 0.85], eventId);
      }
      return;
    }
    case "block":
      // Body block: a blunt thud off a defender, warmer and smaller than a parry.
      ring(e.x, e.y, 0.26, 14, [1, 0.85, 0.6], eventId);
      burst(e.x, e.y, 6, 180, 0.3, 2, [1, 0.85, 0.6], eventId);
      return;
    default:
      return;
  }
}

function revokeEvent(id: string): void {
  speculativeEvents.delete(id);
  suppressedEvents.delete(id);
  for (let index = particles.length - 1; index >= 0; index -= 1) {
    if (particles[index]?.event_id === id) {
      particles.splice(index, 1);
    }
  }
}

function addSpeculative(event: RollbackWrappedEvent): void {
  const isMatch = event.domain.startsWith("match/");
  const isCombat = event.domain.startsWith("combat/");
  if ((!isMatch && !isCombat) || speculativeEvents.has(event.id) || suppressedEvents.has(event.id) || confirmedEvents.has(event.id)) {
    return;
  }
  speculativeEvents.add(event.id);
  if (isMatch) {
    spawnEvent(event.payload as MatchEvent, event.id);
  } else {
    spawnCombatEvent(event.payload as CombatEvent, event.id);
  }
}

/** The "juice" layer: renderer-side particles and ball trail. See file header. */
export const effects = {
  /** Apply a complete stable-event delta. Speculative visuals are keyed by
   * event ID, so a correction can revoke or atomically replace their particles. */
  apply_event_diff(diff: RollbackEventDiff): void {
    for (const event of diff.revoked) {
      revokeEvent(event.id);
    }
    for (const replacement of diff.replaced) {
      const wasSuppressed = suppressedEvents.has(replacement.before.id) || suppressedEvents.has(replacement.after.id);
      revokeEvent(replacement.before.id);
      if (wasSuppressed) {
        suppressedEvents.add(replacement.after.id);
      } else {
        addSpeculative(replacement.after);
      }
    }
    for (const event of diff.added) {
      addSpeculative(event);
    }
  },

  /** Consume live/corrected event deltas without creating drawable state
   * while a confirmed replay owns the screen. Suppressed IDs remain
   * reversible and keep replacements from surfacing late after the replay
   * transition. */
  discard_event_diff(diff: RollbackEventDiff): void {
    for (const event of diff.revoked) {
      revokeEvent(event.id);
    }
    for (const replacement of diff.replaced) {
      revokeEvent(replacement.before.id);
      if (replacement.after.domain.startsWith("match/") || replacement.after.domain.startsWith("combat/")) {
        suppressedEvents.add(replacement.after.id);
      }
    }
    for (const event of diff.added) {
      revokeEvent(event.id);
      if (event.domain.startsWith("match/") || event.domain.startsWith("combat/")) {
        suppressedEvents.add(event.id);
      }
    }
  },

  /** Once an event is confirmed it cannot be revoked. Existing particles
   * keep aging naturally, while the rollback key is retired from bounded
   * state. */
  confirm_event(event: RollbackWrappedEvent, suppressSpawn?: boolean): boolean {
    if (confirmedEvents.has(event.id)) {
      return false;
    }
    confirmedEvents.add(event.id);
    if (
      event.domain.startsWith("combat/") &&
      suppressSpawn !== true &&
      !speculativeEvents.has(event.id) &&
      !suppressedEvents.has(event.id)
    ) {
      spawnCombatEvent(event.payload as CombatEvent, event.id);
    }
    speculativeEvents.delete(event.id);
    suppressedEvents.delete(event.id);
    return true;
  },

  consume_combat(events: readonly CombatEvent[], stableId: (event: CombatEvent, ordinal: number) => string): void {
    events.forEach((event, zeroBasedIndex) => {
      spawnCombatEvent(event, stableId(event, zeroBasedIndex + 1));
    });
  },

  configure(settings: EffectsSettings): void {
    reducedMotion = !settings.screen_shake;
    reducedFlash = !settings.bloom;
  },

  /** Drop drawable state while retaining stable-event bookkeeping. Replay
   * exit uses this so an action suppressed while past footage was visible
   * cannot reappear late if that same speculative ID is subsequently
   * replaced. */
  reset_visuals(): void {
    particles = [];
    trail = [];
    lastSample = undefined;
  },

  /** Drop all live effects (call on a fresh match / kickoff so nothing
   * carries over). */
  reset(): void {
    effects.reset_visuals();
    speculativeEvents = new Set();
    suppressedEvents = new Set();
    confirmedEvents = new Set();
  },

  /** A corrected authoritative boundary invalidates renderer-owned loose-ball
   * path samples. Action particles remain reconciled separately by stable ID. */
  reset_trail(): void {
    trail = [];
    lastSample = undefined;
  },

  sample_ball(s: EffectsMatchState): void {
    // Sample the ball trail only when it's a fast loose ball, spaced by
    // distance so the trail looks the same regardless of framerate and
    // never blobs at rest. Each dot remembers the ball's speed ("heat") and
    // height, so a charged shot streaks hotter, longer and along its true
    // flight path.
    const speed = s.ball_vel.length();
    if (s.owner === undefined && speed > TRAIL_MIN_SPEED) {
      const dx = lastSample !== undefined ? s.ball.x - lastSample.x : Number.POSITIVE_INFINITY;
      const dy = lastSample !== undefined ? s.ball.y - lastSample.y : Number.POSITIVE_INFINITY;
      if (dx * dx + dy * dy >= TRAIL_SPACING * TRAIL_SPACING) {
        const heat = Math.min(1, speed / TRAIL_HOT_SPEED);
        const life = TRAIL_LIFE * (0.7 + 0.9 * heat);
        trail.push({ x: s.ball.x, y: s.ball.y, z: s.ball_z, heat, life, max: life });
        lastSample = { x: s.ball.x, y: s.ball.y };
      }
    } else {
      lastSample = undefined;
    }
  },

  /** Consume one simulated state's events and position sample. The
   * compatibility adapter used by the non-rollback match path. */
  consume(s: EffectsMatchState): void {
    for (const event of s.events) {
      spawnEvent(event, undefined);
    }
    effects.sample_ball(s);
  },

  diagnostics(): EffectsDiagnostics {
    return {
      particle_count: particles.length,
      trail_count: trail.length,
      speculative_ids: [...speculativeEvents].sort(),
      suppressed_ids: [...suppressedEvents].sort(),
      confirmed_ids: [...confirmedEvents].sort(),
    };
  },

  /** Observational hook for #148/#150: this reports the currently presented
   * geometry only. It does not create an authoritative outcome or timestamp. */
  readability_observation(project: Project, ball: { x: number; y: number }, hudRects: readonly Rect[], occluders: readonly Rect[]): ReadabilityObservation {
    const rows: { x: number; y: number; radius: number }[] = [];
    const ids = new Set<string>();
    let nonColorOnly = true;
    for (const particle of particles) {
      if (particle.kind === "glyph" && particle.event_id !== undefined) {
        const [x, y, scale] = project(particle.x, particle.y);
        rows.push({ x, y: y - BALL_LIFT * scale, radius: particle.size * scale });
        ids.add(particle.event_id);
        nonColorOnly = nonColorOnly && particle.glyph !== undefined;
      }
    }

    const [ballX, ballY, ballScale] = project(ball.x, ball.y);
    const ballRadius = Math.max(4, 7 * ballScale);
    let overlapPairCount = 0;
    let occludedCount = 0;
    let ballMasked = false;
    let hudMasked = false;
    rows.forEach((row, index) => {
      const dx = row.x - ballX;
      const dy = row.y - ballY;
      ballMasked = ballMasked || dx * dx + dy * dy <= (row.radius + ballRadius) * (row.radius + ballRadius);
      for (const rect of hudRects) {
        hudMasked = hudMasked || circleIntersectsRect(row.x, row.y, row.radius, rect);
      }
      for (const rect of occluders) {
        if (circleIntersectsRect(row.x, row.y, row.radius, rect)) {
          occludedCount += 1;
          break;
        }
      }
      for (let otherIndex = index + 1; otherIndex < rows.length; otherIndex += 1) {
        const other = rows[otherIndex];
        if (other === undefined) {
          continue;
        }
        const otherDx = row.x - other.x;
        const otherDy = row.y - other.y;
        if (otherDx * otherDx + otherDy * otherDy <= (row.radius + other.radius) * (row.radius + other.radius)) {
          overlapPairCount += 1;
        }
      }
    });

    return {
      active_feedback_count: rows.length,
      concurrent_event_count: ids.size,
      overlap_pair_count: overlapPairCount,
      occluded_count: occludedCount,
      ball_masked: ballMasked,
      hud_masked: hudMasked,
      non_color_only: nonColorOnly,
    };
  },

  /** Advance renderer-owned particles at display cadence. */
  tick(dt: number): void {
    for (let i = trail.length - 1; i >= 0; i -= 1) {
      const dot = trail[i];
      if (dot === undefined) {
        continue;
      }
      dot.life -= dt;
      if (dot.life <= 0) {
        trail.splice(i, 1);
      }
    }

    for (let i = particles.length - 1; i >= 0; i -= 1) {
      const p = particles[i];
      if (p === undefined) {
        continue;
      }
      p.life -= dt;
      p.x += p.vx * dt;
      p.y += p.vy * dt;
      const drag = Math.max(0, 1 - 4 * dt);
      p.vx *= drag;
      p.vy *= drag;
      if (p.life <= 0) {
        particles.splice(i, 1);
      }
    }
  },

  /** Compatibility helper for callers that have one simulation step per
   * render frame. Fixed-clock callers consume each tick then animate once
   * per render. */
  update(s: EffectsMatchState, dt: number): void {
    effects.consume(s);
    effects.tick(dt);
  },

  /** Ball trail. Draw under the ball (call before the depth-sorted
   * entities). Heat (sample-time ball speed) drives size, warmth and
   * brightness: a tap-pass leaves a faint wisp, a charged shot a hot comet
   * tail the bloom pass lights up. */
  drawTrailCommands(project: Project): DrawCommand[] {
    const dl = new DrawList();
    for (const t of trail) {
      const [sx, sy, scale] = project(t.x, t.y);
      const a = t.life / t.max;
      const heat = t.heat;
      const color: RGB = [1, 0.95 - 0.35 * heat, 0.7 - 0.45 * heat];
      const alpha = a * (0.35 + 0.4 * heat);
      const r = (2.5 + 3 * a) * (1 + 0.8 * heat) * scale;
      dl.circle("fill", sx, sy - (BALL_LIFT + t.z) * scale, r, color, { alpha });
    }
    return dl.commands;
  },

  /** Bursts and rings. Draw on top (call after the entities). */
  drawOverCommands(project: Project): DrawCommand[] {
    const dl = new DrawList();
    for (const p of particles) {
      const [sx, sy, scale] = project(p.x, p.y);
      const t = p.life / p.max;
      const c = p.color;
      if (p.kind === "ring") {
        // Expands as it fades.
        const rad = p.size * scale * (1.0 + (1 - t) * 0.6);
        dl.circle("line", sx, sy - BALL_LIFT * scale, rad, c, { alpha: t * 0.8, lineWidth: Math.max(1, 2 * scale) });
      } else if (p.kind === "glyph") {
        const radius = p.size * scale * (0.85 + 0.15 * t);
        const x = sx;
        const y = sy - BALL_LIFT * scale;
        const lineWidth = Math.max(1, 2 * scale);
        drawGlyph(dl, p.glyph, x, y, radius, c, t, lineWidth);
      } else {
        dl.circle("fill", sx, sy - BALL_LIFT * scale, p.size * scale * t + 0.5, c, { alpha: t });
      }
    }
    return dl.commands;
  },

  /** Impure: paints the ball trail into `group`. Untested -- see draw2d.ts. */
  drawTrail(group: THREE.Group, project: Project): void {
    paint(group, effects.drawTrailCommands(project));
  },

  /** Impure: paints bursts/rings into `group`. Untested -- see draw2d.ts. */
  drawOver(group: THREE.Group, project: Project): void {
    paint(group, effects.drawOverCommands(project));
  },
};

function drawGlyph(dl: DrawList, glyphName: string | undefined, x: number, y: number, radius: number, color: RGB, t: number, lineWidth: number): void {
  const alpha = t;
  if (glyphName === "chevron") {
    dl.line([x - radius, y + radius * 0.5, x, y - radius * 0.5], color, { alpha, lineWidth });
    dl.line([x, y - radius * 0.5, x + radius, y + radius * 0.5], color, { alpha, lineWidth });
  } else if (glyphName === "diamond" || glyphName === "hollow_diamond") {
    dl.polygon("line", [x, y - radius, x + radius, y, x, y + radius, x - radius, y], color, { alpha, lineWidth });
  } else if (glyphName === "shield") {
    dl.arc("line", x, y, radius, (190 * Math.PI) / 180, (350 * Math.PI) / 180, color, { alpha, lineWidth });
    dl.line([x - radius, y - radius * 0.2, x, y + radius], color, { alpha, lineWidth });
    dl.line([x, y + radius, x + radius, y - radius * 0.2], color, { alpha, lineWidth });
  } else if (glyphName === "broken_ring") {
    dl.arc("line", x, y, radius, 0, (135 * Math.PI) / 180, color, { alpha, lineWidth });
    dl.arc("line", x, y, radius, (180 * Math.PI) / 180, (315 * Math.PI) / 180, color, { alpha, lineWidth });
  } else if (glyphName === "split") {
    dl.line([x - radius, y - radius, x - radius * 0.2, y], color, { alpha, lineWidth });
    dl.line([x + radius, y + radius, x + radius * 0.2, y], color, { alpha, lineWidth });
  } else if (glyphName === "burst" || glyphName === "stagger") {
    for (let index = 0; index <= 3; index += 1) {
      const angle = (Math.PI * index) / 2;
      dl.line(
        [x + Math.cos(angle) * radius * 0.3, y + Math.sin(angle) * radius * 0.3, x + Math.cos(angle) * radius, y + Math.sin(angle) * radius],
        color,
        { alpha, lineWidth },
      );
    }
  } else if (glyphName === "double_cross") {
    dl.line([x - radius, y - radius, x + radius, y + radius], color, { alpha, lineWidth });
    dl.line([x - radius, y + radius, x + radius, y - radius], color, { alpha, lineWidth });
    dl.circle("line", x, y, radius * 0.55, color, { alpha, lineWidth });
  } else {
    dl.line([x - radius, y - radius, x + radius, y + radius], color, { alpha, lineWidth });
    dl.line([x - radius, y + radius, x + radius, y - radius], color, { alpha, lineWidth });
  }
}

function circleIntersectsRect(x: number, y: number, radius: number, rect: Rect): boolean {
  const nearestX = Math.max(rect.x, Math.min(x, rect.x + rect.w));
  const nearestY = Math.max(rect.y, Math.min(y, rect.y + rect.h));
  const dx = x - nearestX;
  const dy = y - nearestY;
  return dx * dx + dy * dy <= radius * radius;
}
