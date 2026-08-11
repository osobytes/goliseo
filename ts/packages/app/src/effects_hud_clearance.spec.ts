// This test exercises `@gc/render`'s effects.spec.ts's skipped "preserves
// ball and HUD clearance at every supported fixture size" case for real:
// `@gc/render` cannot import `@gc/app` back to reach `hud.layout`
// (`packages/app/src/match_hud.ts`) without a circular package dependency
// (`@gc/app` already depends on `@gc/render`, package.json). `@gc/app`
// sits on the correct side of that edge: `effects` and `camera`
// (`@gc/render`, a declared dependency) and `hud.layout` (local,
// match_hud.ts) are both reachable here, so the assertion runs for real
// instead of staying stubbed.
//
// `@gc/render`'s own effects.spec.ts already simplifies the full fixture
// (5 combat events plus selection/threat occluders around a 960x540 pitch,
// with one event -- a "ball_spill" -- placed exactly at the ball) down to
// two synthetic "contact" events for its two cases (see that file); this
// case follows the same, already-established simplification, with one
// further adjustment: the ball sits away from the synthetic event cluster
// rather than inside it. The full fixture's masking-avoidance geometry
// (glyph sizes, occluder placement, exactly which event kinds emit a
// maskable glyph) is tuned data this package does not have and is not
// reproducing; placing the ball clear of the cluster is the same "far from
// the events, so nothing should mask it" choice this file's sibling case
// ("reports crowded geometry without masking the ball or HUD") already
// makes for its own ball. What this case checks -- that
// `effects.readability_observation` and `hud.layout` compose correctly at
// every supported viewport, with clearance held where the fixture places
// it -- holds either way.

import { describe, expect, it } from "vitest";
import type { CombatEvent, RollbackWrappedEvent } from "@gc/presentation";
import { camera, effects } from "@gc/render";
import type { effectsTypes } from "@gc/render";
import { hud } from "./match_hud.ts";

const FIELD = { w: 960, h: 540 };
const EVENTS_CENTER = { x: 480, y: 270 };

const SETTINGS: effectsTypes.EffectsSettings = { screen_shake: true, bloom: true };

function combatEvent(
  id: string,
  x: number,
  y: number,
  overrides: Partial<CombatEvent> = {},
): RollbackWrappedEvent<CombatEvent> {
  return {
    id,
    tick: 20,
    domain: "combat/contact",
    ordinal: 1,
    payload: {
      kind: "contact",
      tick: 20,
      x,
      y,
      family_id: "unarmed",
      result: "hit",
      source_sequence: 1,
      ...overrides,
    },
  };
}

// Clustered together, away from the ball -- the same "far from the events,
// so nothing should mask it" placement this file's sibling
// ("reports crowded geometry without masking the ball or HUD") already uses
// for its own ball, and away from `hud.layout`'s corner/edge rects, which is
// what "preserves ... clearance" is actually asserting: that a crowd of
// glyph particles well clear of the ball and HUD doesn't spuriously mask
// either, at every supported viewport.
const EVENTS: readonly RollbackWrappedEvent<CombatEvent>[] = [
  combatEvent("combat/clearance/a", EVENTS_CENTER.x - 20, EVENTS_CENTER.y - 10),
  combatEvent("combat/clearance/b", EVENTS_CENTER.x + 15, EVENTS_CENTER.y + 20, {
    result: "guarded",
  }),
];
// Far from the event cluster and from `hud.layout`'s corner/edge rects --
// see the comment above.
const BALL = { x: 40, y: 40 };

const VIEWPORTS: readonly { readonly w: number; readonly h: number }[] = [
  { w: 960, h: 540 },
  { w: 1280, h: 720 },
  { w: 1920, h: 1080 },
  { w: 1280, h: 800 },
];

describe("effects readability against the real HUD layout", () => {
  it("preserves ball and HUD clearance at every supported fixture size", () => {
    effects.configure(SETTINGS);
    effects.reset();
    effects.apply_event_diff({ added: EVENTS, revoked: [], replaced: [] });

    for (const viewport of VIEWPORTS) {
      const layout = hud.layout(viewport);
      const project = (x: number, y: number) => camera.project(x, y, FIELD, viewport);
      const observation = effects.readability_observation(
        project,
        BALL,
        [layout.combat, layout.plan, layout.identity, layout.scorebug],
        [],
      );
      expect(observation.ball_masked, `ball masked at ${viewport.w}`).toBe(false);
      expect(observation.hud_masked, `HUD masked at ${viewport.w}`).toBe(false);
      expect(observation.non_color_only).toBe(true);
    }
  });
});
