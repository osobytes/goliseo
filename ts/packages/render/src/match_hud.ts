// Draws the product match HUD (scorebug, clock, possession, player identity
// + stamina, combat readiness, plan banner, onboarding prompt, full-time/goal
// announcement) from an already-computed `MatchHudModel`/`MatchHudLayout`.
// Pure content: `matchHudCommands` returns a `DrawCommand[]` (draw2d.ts).
//
// Boundary notes (ARCHITECTURE.md §4 rule 6):
//   - `MatchHudModel`/`MatchHudLayout` come from the HUD *model*
//     (`packages/app/src/match_hud.ts`), not this drawing module -- a
//     different module this package does not own. Only the fields this
//     module reads are declared locally, and both are threaded through as
//     parameters rather than computed here, exactly as `combat.ts` threads
//     `CombatDrawFrame`.
//   - `theme.colors`/`theme.radius`/`theme.border_width`/`theme.fonts` would
//     be `@gc/ui`, which `@gc/render`'s package.json does not depend on (see
//     `arena.ts`'s identical note), so the theme is threaded through as an
//     explicit `MatchHudTheme` parameter.

import * as THREE from "three";
import { DrawList, paint, type DrawCommand, type PaintOptions, type RGB } from "./draw2d.ts";

export type BroadcastPhase = "kickoff" | "goal" | "replay" | "full_time";
export type SpeciesShape = "round" | "broad" | "angular" | "cluster";
export type FontKind = "body" | "eyebrow" | "title" | "hero";

export interface OnboardingPrompt {
  readonly title: string;
  readonly body: string;
}

/** The slice of the HUD model's `MatchHudModel` this module reads. */
export interface MatchHudModel {
  readonly home_name: string;
  readonly away_name: string;
  readonly home_score: number;
  readonly away_score: number;
  readonly clock: string;
  readonly venue: string;
  readonly possession: string;
  readonly possession_marker: "filled" | "outline";
  readonly player_name: string;
  readonly player_detail: string;
  readonly player_state: string;
  readonly species_shape: SpeciesShape;
  readonly species_color: RGB;
  readonly stamina: number;
  readonly plan: string;
  readonly prompt?: OnboardingPrompt;
  readonly announcement_title?: string;
  readonly announcement_detail?: string;
  readonly announcement_kind?: BroadcastPhase;
  readonly equipment_label?: string;
  readonly equipment_state?: string;
  readonly equipment_progress: number;
  readonly feedback_text?: string;
  readonly feedback_glyph?: string;
}

interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/** The slice of the HUD model's `MatchHudLayout` this module reads. */
export interface MatchHudLayout {
  readonly venue: Rect;
  readonly scorebug: Rect;
  readonly clock: Rect;
  readonly status: Rect;
  readonly identity: Rect;
  readonly plan: Rect;
  readonly prompt: Rect;
  readonly announcement: Rect;
  readonly combat: Rect;
  readonly scale: number;
}

/** The slice of `@gc/ui`'s theme this module reads. */
export interface MatchHudTheme {
  readonly colors: {
    readonly void: RGB;
    readonly cyan: RGB;
    readonly amber: RGB;
    readonly text: RGB;
    readonly text_muted: RGB;
    readonly panel: RGB;
    readonly panel_raised: RGB;
    readonly border: RGB;
    readonly border_soft: RGB;
  };
  readonly radius: number;
  readonly border_width: number;
  readonly fonts: Readonly<Record<FontKind, number>>;
}

export interface MatchHudViewport {
  readonly w: number;
  readonly h: number;
}

function fontPx(theme: MatchHudTheme, kind: FontKind, scale: number): number {
  return Math.max(10, Math.floor(theme.fonts[kind] * scale + 0.5));
}

function panel(
  dl: DrawList,
  rect: Rect,
  theme: MatchHudTheme,
  fill: RGB,
  border: RGB,
  alpha?: number,
): void {
  dl.rect("fill", rect.x, rect.y, rect.w, rect.h, fill, {
    alpha: alpha ?? 0.94,
    rx: theme.radius,
    ry: theme.radius,
  });
  dl.rect("line", rect.x, rect.y, rect.w, rect.h, border, {
    alpha: 0.86,
    rx: theme.radius,
    ry: theme.radius,
    lineWidth: theme.border_width,
  });
}

function speciesMark(
  dl: DrawList,
  shape: SpeciesShape,
  x: number,
  y: number,
  color: RGB,
  size: number,
): void {
  if (shape === "broad") {
    dl.rect("fill", x - size, y - size * 0.72, size * 2, size * 1.44, color, { rx: 2, ry: 2 });
  } else if (shape === "angular") {
    dl.polygon("fill", [x, y - size, x + size, y + size * 0.86, x - size, y + size * 0.86], color);
  } else if (shape === "cluster") {
    dl.circle("fill", x - size * 0.5, y + size * 0.25, size * 0.5, color);
    dl.circle("fill", x + size * 0.5, y + size * 0.25, size * 0.5, color);
    dl.circle("fill", x, y - size * 0.48, size * 0.5, color);
  } else {
    dl.circle("fill", x, y, size, color);
  }
}

function stamina(
  dl: DrawList,
  rect: Rect,
  amount: number,
  scale: number,
  theme: MatchHudTheme,
): void {
  const x = rect.x + 64 * scale;
  const y = rect.y + 46 * scale;
  const w = rect.w - 78 * scale;
  const h = Math.max(6, 7 * scale);
  dl.rect("fill", x, y, w, h, theme.colors.void, { alpha: 0.9 });
  dl.rect("fill", x, y, w * amount, h, amount <= 0.2 ? theme.colors.amber : theme.colors.cyan, {
    alpha: 0.95,
  });
  dl.rect("line", x, y, w, h, theme.colors.text, { alpha: 0.7 });
  for (let i = 1; i <= 4; i += 1) {
    const tickX = x + (w * i) / 5;
    dl.line([tickX, y, tickX, y + h], theme.colors.text, { alpha: 0.7 });
  }
}

function readiness(
  dl: DrawList,
  rect: Rect,
  amount: number,
  scale: number,
  theme: MatchHudTheme,
): void {
  const segments = 5;
  const gap = Math.max(1, 2 * scale);
  const width = (rect.w - 16 * scale - gap * (segments - 1)) / segments;
  const y = rect.y + rect.h - 6 * scale;
  for (let index = 1; index <= segments; index += 1) {
    const threshold = index / segments;
    const color = amount + 1e-9 >= threshold ? theme.colors.cyan : theme.colors.border_soft;
    dl.rect(
      "fill",
      rect.x + 8 * scale + (index - 1) * (width + gap),
      y,
      width,
      Math.max(2, 3 * scale),
      color,
      { alpha: 0.9 },
    );
  }
}

/** The product match HUD, drawn from an already-computed model/layout. See file header. */
export function matchHudCommands(
  model: MatchHudModel,
  layout: MatchHudLayout,
  theme: MatchHudTheme,
  viewport: MatchHudViewport,
): DrawCommand[] {
  const dl = new DrawList();
  const scale = layout.scale;
  const C = theme.colors;

  dl.text(model.venue, layout.venue.x, layout.venue.y, layout.venue.w, "center", C.text_muted, {
    fontPx: fontPx(theme, "eyebrow", scale),
  });

  panel(dl, layout.scorebug, theme, C.panel, C.border);
  dl.rect("fill", layout.scorebug.x, layout.scorebug.y, layout.scorebug.w / 2, 3 * scale, C.cyan);
  dl.rect(
    "fill",
    layout.scorebug.x + layout.scorebug.w / 2,
    layout.scorebug.y,
    layout.scorebug.w / 2,
    3 * scale,
    C.amber,
  );
  dl.text(
    model.home_name,
    layout.scorebug.x + 14 * scale,
    layout.scorebug.y + 17 * scale,
    170 * scale,
    "left",
    C.text,
    {
      fontPx: fontPx(theme, "body", scale),
    },
  );
  dl.text(
    model.away_name,
    layout.scorebug.x + layout.scorebug.w - 184 * scale,
    layout.scorebug.y + 17 * scale,
    170 * scale,
    "right",
    C.text,
    {
      fontPx: fontPx(theme, "body", scale),
    },
  );
  dl.text(
    `${model.home_score}  —  ${model.away_score}`,
    layout.scorebug.x + 170 * scale,
    layout.scorebug.y + 9 * scale,
    160 * scale,
    "center",
    C.text,
    { fontPx: fontPx(theme, "title", scale) },
  );

  panel(dl, layout.clock, theme, C.panel_raised, C.border);
  dl.text(
    model.clock,
    layout.clock.x,
    layout.clock.y + 8 * scale,
    layout.clock.w,
    "center",
    C.text,
    {
      fontPx: fontPx(theme, "body", scale),
    },
  );

  dl.text(
    model.possession,
    layout.status.x + 18 * scale,
    layout.status.y + 2 * scale,
    layout.status.w - 18 * scale,
    "center",
    C.text,
    {
      fontPx: fontPx(theme, "eyebrow", scale),
    },
  );
  const diamondX = layout.status.x + 20 * scale;
  const diamondY = layout.status.y + 8 * scale;
  const diamondR = 5 * scale;
  dl.polygon(
    model.possession_marker === "filled" ? "fill" : "line",
    [
      diamondX,
      diamondY - diamondR,
      diamondX + diamondR,
      diamondY,
      diamondX,
      diamondY + diamondR,
      diamondX - diamondR,
      diamondY,
    ],
    C.cyan,
  );

  panel(dl, layout.identity, theme, C.panel, C.border);
  dl.rect(
    "fill",
    layout.identity.x,
    layout.identity.y,
    5 * scale,
    layout.identity.h,
    model.species_color,
    {
      rx: theme.radius,
      ry: theme.radius,
    },
  );
  speciesMark(
    dl,
    model.species_shape,
    layout.identity.x + 27 * scale,
    layout.identity.y + 24 * scale,
    model.species_color,
    8 * scale,
  );
  dl.text(
    `${model.player_name} · ${model.player_detail}`,
    layout.identity.x + 46 * scale,
    layout.identity.y + 10 * scale,
    layout.identity.w - 58 * scale,
    "left",
    C.text,
    {
      fontPx: fontPx(theme, "body", scale),
    },
  );
  dl.text(
    "STAMINA",
    layout.identity.x + 10 * scale,
    layout.identity.y + 43 * scale,
    52 * scale,
    "left",
    C.text_muted,
    {
      fontPx: fontPx(theme, "eyebrow", scale),
    },
  );
  dl.text(
    model.player_state,
    layout.identity.x + layout.identity.w - 100 * scale,
    layout.identity.y + 29 * scale,
    88 * scale,
    "right",
    model.possession_marker === "filled" ? C.cyan : C.text_muted,
    { fontPx: fontPx(theme, "eyebrow", scale) },
  );
  stamina(dl, layout.identity, model.stamina, scale, theme);

  if (model.equipment_label !== undefined) {
    panel(
      dl,
      layout.combat,
      theme,
      C.panel,
      model.feedback_text !== undefined ? C.amber : C.border_soft,
    );
    const equipmentState = model.equipment_state;
    if (equipmentState === undefined) {
      throw new Error("match_hud.ts: equipment_state must be set alongside equipment_label");
    }
    const text =
      model.feedback_text !== undefined
        ? `[${(model.feedback_glyph ?? "FEEDBACK").toUpperCase()}] ${model.feedback_text}`
        : `EQUIPMENT · ${model.equipment_label} · ${equipmentState}`;
    dl.text(
      text,
      layout.combat.x + 8 * scale,
      layout.combat.y + 4 * scale,
      layout.combat.w - 16 * scale,
      "center",
      model.feedback_text !== undefined ? C.amber : C.text,
      {
        fontPx: fontPx(theme, "eyebrow", scale),
      },
    );
    readiness(dl, layout.combat, model.equipment_progress, scale, theme);
  }

  panel(dl, layout.plan, theme, C.panel, C.border_soft);
  dl.text(
    model.plan,
    layout.plan.x + 8 * scale,
    layout.plan.y + 15 * scale,
    layout.plan.w - 16 * scale,
    "center",
    C.amber,
    {
      fontPx: fontPx(theme, "eyebrow", scale),
    },
  );

  if (model.prompt !== undefined) {
    panel(dl, layout.prompt, theme, C.panel_raised, C.cyan);
    dl.text(
      model.prompt.title,
      layout.prompt.x + 10 * scale,
      layout.prompt.y + 8 * scale,
      layout.prompt.w - 20 * scale,
      "center",
      C.cyan,
      {
        fontPx: fontPx(theme, "eyebrow", scale),
      },
    );
    dl.text(
      model.prompt.body,
      layout.prompt.x + 10 * scale,
      layout.prompt.y + 27 * scale,
      layout.prompt.w - 20 * scale,
      "center",
      C.text,
      {
        fontPx: fontPx(theme, "body", scale),
      },
    );
  }

  if (model.announcement_title !== undefined) {
    if (model.announcement_kind === "full_time") {
      dl.rect("fill", 0, 0, viewport.w, viewport.h, C.void, { alpha: 0.78 });
    }
    const highlight = model.announcement_kind === "replay" ? C.cyan : C.amber;
    panel(dl, layout.announcement, theme, C.panel_raised, highlight, 0.96);
    dl.text(
      model.announcement_title,
      layout.announcement.x + 12 * scale,
      layout.announcement.y + 24 * scale,
      layout.announcement.w - 24 * scale,
      "center",
      highlight,
      {
        fontPx: fontPx(theme, "title", scale),
      },
    );
    dl.text(
      model.announcement_detail ?? "",
      layout.announcement.x + 12 * scale,
      layout.announcement.y + 62 * scale,
      layout.announcement.w - 24 * scale,
      "center",
      C.text,
      {
        fontPx: fontPx(theme, "body", scale),
      },
    );
  }

  return dl.commands;
}

/**
 * Impure: paints the HUD into `group`. `paintOptions` is forwarded verbatim
 * to `paint` -- production callers never pass it (the default `buildText`
 * needs a real DOM, which `game/`'s eventual canvas host provides); tests use
 * it to substitute the text path so HUD *population* -- every command this
 * module emits actually reaching `group` as a three.js object, in order --
 * is exercised without a DOM polyfill. See match_hud.spec.ts and
 * draw2d.ts's `PaintOptions` doc comment.
 */
export function drawMatchHud(
  group: THREE.Group,
  model: MatchHudModel,
  layout: MatchHudLayout,
  theme: MatchHudTheme,
  viewport: MatchHudViewport,
  paintOptions?: PaintOptions,
): void {
  paint(group, matchHudCommands(model, layout, theme, viewport), paintOptions);
}
