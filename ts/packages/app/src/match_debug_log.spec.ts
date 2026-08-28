// Tier-2 tests for the dev match debug log's pure entry builder. The impure
// shell (module state, fetch batching, the `import.meta.env.DEV` gate) is
// deliberately unspecced -- it is dev-only plumbing with no sink in tests.

import { describe, expect, it } from "vitest";

import type { frameBufferTypes } from "@gc/render";

import { SAMPLE_EVERY_TICKS, entriesForFrame, inputEntry } from "./match_debug_log.ts";

type RenderFrame = frameBufferTypes.RenderFrame;

const IDS = ["kael", "veil_nyx", "ostra"] as const;

function fakeFrame(overrides?: {
  events?: Partial<frameBufferTypes.RenderFrameEvents>;
}): RenderFrame {
  const events: frameBufferTypes.RenderFrameEvents = {
    count: 0,
    kind: [],
    x: [],
    y: [],
    slot: [],
    save_style: [],
    style: [],
    outcome: [],
    difficulty: [],
    shot_type: [],
    keeper_state: [],
    keeper_depth: [],
    jumping: [],
    on_target: [],
    ...overrides?.events,
  };
  return {
    field: {} as RenderFrame["field"],
    roster: {} as RenderFrame["roster"],
    players: {} as RenderFrame["players"],
    ball: { x: 800, y: 460, z: 12.4, visible: true },
    control: {} as RenderFrame["control"],
    possession: { owner: 2, owner_team: "home", keeper_holds: false },
    hud: {
      home_score: 1,
      away_score: 0,
      time_left: 93.27,
      finished: false,
      possession_team: "home",
      controlled: 2,
      controlled_team: "home",
      controlled_is_keeper: false,
      controlled_owns_ball: true,
      controlled_stamina: 0.8,
      species_shape: "round",
      species_color: [1, 1, 1],
    },
    events,
  };
}

describe("match_debug_log.entriesForFrame", () => {
  it("serializes an event with the actor resolved to a roster id", () => {
    const frame = fakeFrame({
      events: {
        count: 1,
        kind: ["first_touch_shot"],
        x: [1120.6],
        y: [460.2],
        slot: [2],
        save_style: [undefined],
        style: ["volley"],
        outcome: ["clean"],
        difficulty: [0.2345],
        shot_type: [undefined],
        keeper_state: [undefined],
        keeper_depth: [undefined],
        jumping: [false],
        on_target: [undefined],
      },
    });
    const lines = entriesForFrame(100, frame, IDS, 100);
    expect(lines).toHaveLength(1);
    const entry = JSON.parse(lines[0] ?? "") as Record<string, unknown>;
    expect(entry).toMatchObject({
      t: "ev",
      tick: 100,
      kind: "first_touch_shot",
      p: "veil_nyx",
      x: 1121,
      y: 460,
      outcome: "clean",
      style: "volley",
      difficulty: 0.234, // 0.2345 is 0.23449999... in binary; toFixed(3) rounds down
    });
  });

  it("emits a state sample only when the cadence has elapsed", () => {
    const frame = fakeFrame();
    expect(entriesForFrame(10, frame, IDS, 0)).toHaveLength(0);
    const lines = entriesForFrame(SAMPLE_EVERY_TICKS, frame, IDS, 0);
    expect(lines).toHaveLength(1);
    const entry = JSON.parse(lines[0] ?? "") as Record<string, unknown>;
    expect(entry).toMatchObject({
      t: "state",
      tick: SAMPLE_EVERY_TICKS,
      ball: [800, 460, 12],
      owner: "veil_nyx",
      controlled: "veil_nyx",
      score: [1, 0],
      time_left: 93.3,
    });
  });

  it("records an unattributed event actor as null", () => {
    const frame = fakeFrame({
      events: {
        count: 1,
        kind: ["touch"],
        x: [1],
        y: [2],
        slot: [undefined],
        save_style: [undefined],
        style: [undefined],
        outcome: [undefined],
        difficulty: [undefined],
        shot_type: [undefined],
        keeper_state: [undefined],
        keeper_depth: [undefined],
        jumping: [undefined],
        on_target: [undefined],
      },
    });
    const lines = entriesForFrame(5, frame, IDS, 5);
    const entry = JSON.parse(lines[0] ?? "") as Record<string, unknown>;
    expect(entry["p"]).toBeNull();
  });
});

describe("match_debug_log.inputEntry", () => {
  it("emits on a held-set transition and decodes the bit names", () => {
    const sample = { move_x: 0, move_y: -90, held: 8 + 32, edges: 0 };
    const entry = inputEntry(50, sample, "");
    expect(entry).toBeDefined();
    const parsed = JSON.parse(entry?.line ?? "") as Record<string, unknown>;
    expect(parsed).toMatchObject({
      t: "input",
      tick: 50,
      held: ["jockey", "aerial_strike"],
      move: [0, -90],
    });
  });

  it("suppresses a repeat of the same transition key (stick wiggle)", () => {
    const a = { move_x: 10, move_y: 0, held: 32, edges: 0 };
    const first = inputEntry(50, a, "");
    expect(first).toBeDefined();
    const wiggled = { move_x: 60, move_y: -40, held: 32, edges: 0 };
    expect(inputEntry(51, wiggled, first?.key ?? "")).toBeUndefined();
  });

  it("emits again when the stick crosses to neutral", () => {
    const a = { move_x: 10, move_y: 0, held: 32, edges: 0 };
    const first = inputEntry(50, a, "");
    const neutral = { move_x: 0, move_y: 0, held: 32, edges: 0 };
    expect(inputEntry(51, neutral, first?.key ?? "")).toBeDefined();
  });
});
