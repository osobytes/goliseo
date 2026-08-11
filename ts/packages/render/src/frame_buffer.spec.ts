// Differential test for the RenderFrame wire format, against a pinned
// reference vector -- per ARCHITECTURE.md §3 rule 7 and this package's task
// brief, NOT a round trip. This package has no `encode`, so a round trip
// through this module alone is not even possible; the fixture is the only
// way to know `decode` agrees with the actual wire
// `crates/gc-render::frame_buffer::encode`/`encode_roster` produce, which is
// what a real client receives.
//
// The reference data (`./fixtures/frame_buffer_lua_reference.ts`) is a
// byte-for-byte copy of `rust/crates/gc-render/tests/fixtures/
// frame_buffer_lua_reference.txt`, embedded as a TS string constant -- see
// that file's header for why it is not read from disk with `node:fs` -- plus
// an untouched copy of the original `.txt` at
// `../tests/fixtures/frame_buffer_lua_reference.txt` for provenance/diffing
// against the Rust copy. Both were captured from the original renderer's
// implementation before it was retired -- see `tools/lua_reference/
// README.md` for the capture methodology; that implementation no longer
// exists in this repository, so these vectors are frozen and cannot be
// regenerated. Its rows are `label<TAB>word_count<TAB>comma-separated %.17g
// words`, covering the roster encoding and three frames: kickoff, and after
// 37 and 200 stepped ticks, so both a pristine and a moved state are
// compared. `%.17g` round-trips a binary64 exactly, so `parseFloat` recovers
// the identical bits the reference implementation held -- no
// `toPrecision`/formatting involved on the way in.
//
// Expected decoded values below are computed BY HAND from the raw fixture
// words (offsets and enum numberings copied from `crates/gc-render/src/
// frame_buffer.rs`, independent of this module's own logic) rather than by
// calling `decode` and asserting round-trip consistency -- the whole point
// of testing against a vector instead of a round trip is that a symmetric
// bug in `decode`'s own offset math would round-trip against itself
// perfectly and prove nothing.

import { describe, expect, it } from "vitest";
import {
  decode,
  decodeRoster,
  frameWords,
  toRenderFrame,
  type DecodedRenderFrame,
  EVENT_FIELD_COUNT,
  HEADER_WORDS,
  LAYOUT_VERSION,
  MAGIC,
  PLAYER_FIELD_COUNT,
  RENDER_FRAME_VERSION,
  ROSTER_FIELD_COUNT,
  ROSTER_HEADER_WORDS,
  ROSTER_MAGIC,
  ROSTER_STRING_FIELD_COUNT,
  SCALAR_FIELD_COUNT,
} from "./frame_buffer.ts";
import { FRAME_BUFFER_LUA_REFERENCE } from "./fixtures/frame_buffer_lua_reference.ts";

function loadFixture(): ReadonlyMap<string, readonly number[]> {
  const text = FRAME_BUFFER_LUA_REFERENCE;
  const rows = new Map<string, readonly number[]>();
  for (const line of text.split("\n")) {
    if (line.trim() === "") {
      continue;
    }
    const [label, countText, data] = line.split("\t");
    if (label === undefined || countText === undefined || data === undefined) {
      throw new Error(`malformed fixture line: ${line}`);
    }
    const count = Number.parseInt(countText, 10);
    const words = data.split(",").map((w) => Number.parseFloat(w));
    if (words.length !== count) {
      throw new Error(`${label}: declared count ${count} but parsed ${words.length} words`);
    }
    rows.set(label, words);
  }
  return rows;
}

const fixture = loadFixture();

function row(label: string): readonly number[] {
  const words = fixture.get(label);
  if (words === undefined) {
    throw new Error(`no reference row labelled ${label}`);
  }
  return words;
}

// Builds a synthetic string blob matching `count * ROSTER_STRING_FIELD_COUNT`
// newline-delimited parts, in the shape `decode_roster` expects. The
// reference fixture never captured the string blob (the Rust differential
// test discards it too -- `encode_roster`'s second return is
// `_digest`-ignored there), so this module's roster-string handling is
// exercised structurally rather than against a vector; the NUMERIC roster
// fields below are checked against the fixture.
//
// #447 grew this from two parts per slot to four (`id`, `name`,
// `presentation_id`, `loadout_id`). EVERY OTHER SLOT IS GIVEN NO LOADOUT so
// the absence encoding -- an empty part decoding back to `undefined` -- is
// exercised alongside the present case rather than only in passing.
function syntheticRosterBlob(count: number): string {
  const parts: string[] = [];
  for (let i = 0; i < count; i += 1) {
    parts.push(`player_${i}`, `Player ${i}`, `presentation_${i}`, i % 2 === 0 ? `loadout_${i}` : "");
  }
  return parts.join("\n");
}

describe("frame_buffer layout constants", () => {
  it("match crates/gc-render/src/frame_buffer.rs exactly", () => {
    expect(MAGIC).toBe(0x474f4c46);
    expect(ROSTER_MAGIC).toBe(0x474f4c52);
    expect(LAYOUT_VERSION).toBe(1);
    expect(RENDER_FRAME_VERSION).toBe(1);
    expect(HEADER_WORDS).toBe(12);
    expect(SCALAR_FIELD_COUNT).toBe(44);
    expect(PLAYER_FIELD_COUNT).toBe(21);
    expect(EVENT_FIELD_COUNT).toBe(15);
    expect(ROSTER_HEADER_WORDS).toBe(7);
    expect(ROSTER_FIELD_COUNT).toBe(7);
    // #447. `ROSTER_FIELD_COUNT` (the NUMERIC block) is deliberately
    // unchanged and `LAYOUT_VERSION` is deliberately still 1: the two new
    // columns are strings and went into the blob, which the pinned reference
    // fixture the rest of this file compares against never captured. See
    // `ROSTER_STRING_FIELD_COUNT`'s own doc and the Rust constant it mirrors.
    expect(ROSTER_STRING_FIELD_COUNT).toBe(4);
  });

  it("frameWords matches the fixture's declared total_words", () => {
    for (const label of ["t0", "t37", "t200"]) {
      const words = row(label);
      const count = words[6] ?? -1;
      const eventCount = words[8] ?? -1;
      expect(frameWords(count, eventCount)).toBe(words[3]);
    }
  });
});

describe("decodeRoster against the Lua reference vector", () => {
  const words = row("roster");
  const count = 10;
  const decoded = decodeRoster(words, syntheticRosterBlob(count));

  it("reads the header", () => {
    expect(decoded.layoutVersion).toBe(1);
    expect(decoded.renderFrameVersion).toBe(1);
    expect(decoded.count).toBe(count);
  });

  it("decodes team, is_keeper, radius, species_shape and species_color per slot exactly as the fixture words", () => {
    expect(decoded.teams).toEqual([
      "home",
      "home",
      "home",
      "home",
      "home",
      "away",
      "away",
      "away",
      "away",
      "away",
    ]);
    expect(decoded.is_keeper).toEqual([true, false, false, false, false, true, false, false, false, false]);
    expect(decoded.radius).toEqual(Array<number>(10).fill(12));
    expect(decoded.species_shape).toEqual([
      "round",
      "broad",
      "broad",
      "round",
      "angular",
      "broad",
      "broad",
      "round",
      "broad",
      "angular",
    ]);
    expect(decoded.species_color).toEqual([
      [0.35, 0.75, 1],
      [1, 0.55, 0.25],
      [1, 0.55, 0.25],
      [0.35, 0.75, 1],
      [0.9, 0.85, 0.2],
      [1, 0.55, 0.25],
      [1, 0.55, 0.25],
      [0.35, 0.75, 1],
      [1, 0.55, 0.25],
      [0.9, 0.85, 0.2],
    ]);
  });

  it("round-trips the synthetic id/name blob structurally", () => {
    expect(decoded.ids).toEqual(["player_0", "player_1", "player_2", "player_3", "player_4", "player_5", "player_6", "player_7", "player_8", "player_9"]);
    expect(decoded.names[3]).toBe("Player 3");
  });

  // #447. The two new string columns, and specifically the difference
  // between them: a presentation is always present, a loadout may genuinely
  // be absent, and the absence must arrive as `undefined` rather than as the
  // empty string it travelled as. Both branches are asserted, so neither can
  // pass by never occurring.
  it("recovers presentation ids, and a missing loadout as an absence rather than an empty string", () => {
    expect(decoded.presentation_ids).toEqual(["presentation_0", "presentation_1", "presentation_2", "presentation_3", "presentation_4", "presentation_5", "presentation_6", "presentation_7", "presentation_8", "presentation_9"]);
    expect(decoded.loadout_ids[0]).toBe("loadout_0");
    expect(decoded.loadout_ids[1]).toBeUndefined();
    expect(decoded.loadout_ids.filter((id) => id !== undefined)).toHaveLength(5);
    expect(decoded.loadout_ids.filter((id) => id === undefined)).toHaveLength(5);
    expect(decoded.loadout_ids).not.toContain("");
  });

  it("rejects a bad magic word", () => {
    const bad = words.slice();
    bad[0] = 0;
    expect(() => decodeRoster(bad, syntheticRosterBlob(count))).toThrow(/magic/);
  });

  it("rejects a string blob with the wrong part count", () => {
    expect(() => decodeRoster(words, "only_one_part")).toThrow(/roster blob holds/);
  });

  // THE CONSUMER HALF OF THE NON-EMPTY GUARANTEE (#447). `encode_roster`
  // asserts a non-empty presentation id on the producer side; without the
  // same check here the whole guarantee would rest on one assert written in
  // the other language, and what an empty id costs downstream is specific --
  // `variantFor` would once have read it as "nothing was wired" and handed
  // the player `themes.LIST[0]`'s sword and shield, keeper included.
  it("rejects an empty presentation id, which downstream would read as an unwired player", () => {
    const parts: string[] = [];
    for (let i = 0; i < count; i += 1) {
      parts.push(`player_${i}`, `Player ${i}`, i === 4 ? "" : `presentation_${i}`, "");
    }
    expect(() => decodeRoster(words, parts.join("\n"))).toThrow(/slot 4 carries an empty presentation id/);

    // NON-VACUOUS: the same blob with slot 4 filled in decodes cleanly, so
    // the throw is about the empty id rather than about the blob's shape.
    const fixed = parts.slice();
    fixed[4 * ROSTER_STRING_FIELD_COUNT + 2] = "presentation_4";
    expect(() => decodeRoster(words, fixed.join("\n"))).not.toThrow();
  });
});

describe("decode against the Lua reference vector: t0 (kickoff)", () => {
  const decoded = decode(row("t0"));

  it("reads the header", () => {
    expect(decoded.layoutVersion).toBe(1);
    expect(decoded.renderFrameVersion).toBe(1);
    expect(decoded.combatPresent).toBe(false);
  });

  it("decodes field geometry", () => {
    expect(decoded.field).toEqual({
      w: 960,
      h: 540,
      crossbar_h: 70,
      penalty_box_depth: 95,
      penalty_box_h: 200,
      goal_home: { x: -30, y: 215, w: 30, h: 110 },
      goal_away: { x: 960, y: 215, w: 30, h: 110 },
    });
  });

  it("decodes the ball, with no landing reticle at kickoff", () => {
    expect(decoded.ball).toEqual({ x: 450, y: 270, z: 0, vx: 0, vy: 0, vz: 0, visible: true });
    expect(decoded.ball.landing_x).toBeUndefined();
    expect(decoded.ball.landing_y).toBeUndefined();
  });

  it("decodes possession: slot 4, home, not held in the hands", () => {
    expect(decoded.possession).toEqual({ owner: 4, owner_team: "home", keeper_holds: false });
  });

  it("decodes control: slot 4 controlled, no pass target, no charge", () => {
    expect(decoded.control).toEqual({ controlled: 4, charge: 0 });
    expect(decoded.control.pass_target).toBeUndefined();
    expect(decoded.control.charge_kind).toBeUndefined();
  });

  it("decodes the HUD", () => {
    expect(decoded.hud).toEqual({
      home_score: 0,
      away_score: 0,
      time_left: 120,
      finished: false,
      possession_team: "home",
      controlled: 4,
      controlled_team: "home",
      controlled_is_keeper: false,
      controlled_owns_ball: true,
      controlled_stamina: 1,
      species_shape: "round",
      species_color: [0.35, 0.75, 1],
    });
  });

  it("decodes the players' structure-of-arrays exactly as the fixture words", () => {
    const p = decoded.players;
    expect(p.count).toBe(10);
    expect(p.x).toEqual([
      57.599999999999994, 268.8, 268.8, 432, 468, 902.4, 710.4, 570, 493.4935513385938, 493.4935513385938,
    ]);
    expect(p.y).toEqual([270, 162, 378, 270, 270, 270, 270, 270, 158.15943941504452, 381.8405605849555]);
    expect(p.facing_x).toEqual([1, 1, 1, 1, 1, -1, -1, -1, -1, -1]);
    expect(p.facing_y).toEqual(Array<number>(10).fill(0));
    expect(p.speed).toEqual(Array<number>(10).fill(0));
    expect(p.pose_id).toEqual([
      "keeper_ready_tall",
      "locomotion",
      "locomotion",
      "locomotion",
      "locomotion",
      "keeper_ready_tall",
      "locomotion",
      "locomotion",
      "locomotion",
      "locomotion",
    ]);
    expect(p.pose_priority).toEqual([13, 0, 0, 0, 0, 13, 0, 0, 0, 0]);
    expect(p.pose_source).toEqual(["soccer", "locomotion", "locomotion", "locomotion", "locomotion", "soccer", "locomotion", "locomotion", "locomotion", "locomotion"]);
    expect(p.controlled).toEqual([false, false, false, true, false, false, false, false, false, false]);
    expect(p.dashing).toEqual(Array<boolean>(10).fill(false));
    expect(p.holding).toEqual(Array<boolean>(10).fill(false));
    expect(p.dive).toEqual(Array<number>(10).fill(0));
    expect(p.aerial_style).toEqual(Array<undefined>(10).fill(undefined));
    expect(p.aerial_outcome).toEqual(Array<undefined>(10).fill(undefined));
  });

  it("decodes zero events at kickoff", () => {
    expect(decoded.events).toEqual({
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
    });
  });
});

describe("decode against the Lua reference vector: t37 (after 37 stepped ticks)", () => {
  const decoded = decode(row("t37"));

  it("advances the clock and moves players, leaving geometry and possession alone", () => {
    expect(decoded.hud.time_left).toBe(119.38333333333337);
    expect(decoded.field.w).toBe(960);
    expect(decoded.possession).toEqual({ owner: 4, owner_team: "home", keeper_holds: false });
  });

  it("decodes moved player positions and nonzero locomotion speed", () => {
    const p = decoded.players;
    expect(p.x[0]).toBe(13.842947421854188);
    expect(p.y[0]).toBe(270);
    expect(p.speed).toEqual([
      11.44229611018636, 64.15351216477657, 64.5659164374206, 0, 173.2085206525491, 11.846289650764119,
      41.46628655009226, 0, 180, 200.00000000000003,
    ]);
    // Poses are unchanged from kickoff at this tick.
    expect(p.pose_id[0]).toBe("keeper_ready_tall");
    expect(p.pose_id[5]).toBe("keeper_ready_tall");
    expect(p.controlled).toEqual([false, false, false, true, false, false, false, false, false, false]);
  });
});

describe("decode against the Lua reference vector: t200 (after 200 stepped ticks)", () => {
  const decoded = decode(row("t200"));

  it("advances the clock further", () => {
    expect(decoded.hud.time_left).toBe(116.66666666666686);
  });

  it("decodes a pose change: slot 8 (index 7) now shows contain, sourced from soccer", () => {
    const p = decoded.players;
    expect(p.pose_id[7]).toBe("contain");
    expect(p.pose_priority[7]).toBe(30);
    expect(p.pose_source[7]).toBe("soccer");
    // Every other slot is unchanged from kickoff.
    expect(p.pose_id[0]).toBe("keeper_ready_tall");
    expect(p.pose_id[3]).toBe("locomotion");
  });

  it("decodes the moved ball-carrier's position", () => {
    expect(decoded.players.x[3]).toBe(432);
    expect(decoded.players.controlled[3]).toBe(true);
  });
});

// `t0`, `t37` and `t200` above are all `event_count = 0` -- a kickoff and two
// early ticks of a seed-17 match that never produces a discrete `MatchEvent`
// in that window. So none of `RenderFrameEvents`' fields beyond the empty
// array itself (`kind`, `save_style`, `style`, `outcome`, `difficulty`,
// `shot_type`, `keeper_state`, `keeper_depth`, `jumping`, `on_target`) had
// differential coverage: a defect anywhere in this module's event decoding
// would have passed every test in this file.
//
// The `s1_t*` rows are a SEPARATE match -- same teams, same field, same
// `slot_input.neutral_match_input()` stepping, but seed 1 instead of 17,
// chosen because it produces a livelier match (seed 17 stalls after tick
// ~1083 and never generates another event). See
// `crates/gc-render/tests/frame_buffer_differential.rs`'s module doc and
// `tools/lua_reference/capture_frame_buffer_events.lua` for the full
// account of how these eight ticks were chosen and what they cover.
describe("decode against the Lua reference vector: the event section (seed-1 match, s1_t*)", () => {
  it("s1_t226: two events on one tick, both kind-only (press_commit_cover, tackle), same actor", () => {
    const decoded = decode(row("s1_t226"));
    expect(decoded.events).toEqual({
      count: 2,
      kind: ["press_commit_cover", "tackle"],
      x: [468.10109073622164, 430.59774200334988],
      y: [261.25582630265188, 270.08696473417865],
      slot: [8, 8],
      save_style: [undefined, undefined],
      style: [undefined, undefined],
      outcome: [undefined, undefined],
      difficulty: [undefined, undefined],
      shot_type: [undefined, undefined],
      keeper_state: [undefined, undefined],
      keeper_depth: [undefined, undefined],
      jumping: [undefined, undefined],
      on_target: [undefined, undefined],
    });
  });

  it("s1_t294: a reception carries the aerial field group (style, outcome, jumping = false, difficulty)", () => {
    const decoded = decode(row("s1_t294"));
    expect(decoded.events).toEqual({
      count: 1,
      kind: ["reception"],
      x: [317.25689487567922],
      y: [320.25990219214299],
      slot: [10],
      save_style: [undefined],
      style: ["leg_control"],
      outcome: ["clean"],
      difficulty: [0.34362326583817743],
      shot_type: [undefined],
      keeper_state: [undefined],
      keeper_depth: [undefined],
      jumping: [false],
      on_target: [undefined],
    });
  });

  it("s1_t374: a shot carries the keeper field group (shot_type, keeper_state, keeper_depth, on_target = false) and no aerial fields", () => {
    const decoded = decode(row("s1_t374"));
    expect(decoded.events).toEqual({
      count: 1,
      kind: ["shot"],
      x: [150.43886345831467],
      y: [290.45850091974103],
      slot: [10],
      save_style: [undefined],
      style: [undefined],
      outcome: [undefined],
      difficulty: [undefined],
      shot_type: ["ground"],
      keeper_state: ["set"],
      keeper_depth: [17.453016776202638],
      jumping: [undefined],
      on_target: [false],
    });
  });

  it("s1_t393: a claim is kind-only, like a tackle or touch", () => {
    const decoded = decode(row("s1_t393"));
    expect(decoded.events.kind).toEqual(["claim"]);
    expect(decoded.events.slot).toEqual([1]);
    expect(decoded.events.save_style).toEqual([undefined]);
    expect(decoded.events.keeper_state).toEqual([undefined]);
  });

  it("s1_t1236: a juke and a touch on one tick, both kind-only, same actor", () => {
    const decoded = decode(row("s1_t1236"));
    expect(decoded.events.kind).toEqual(["juke", "touch"]);
    expect(decoded.events.slot).toEqual([8, 8]);
    expect(decoded.events.style).toEqual([undefined, undefined]);
  });

  it("s1_t1466: a parry carries save_style, keeper_state and keeper_depth, but not on_target", () => {
    const decoded = decode(row("s1_t1466"));
    expect(decoded.events).toEqual({
      count: 1,
      kind: ["parry"],
      x: [46.41309480175785],
      y: [282.17909788915597],
      slot: [1],
      save_style: ["central"],
      style: [undefined],
      outcome: [undefined],
      difficulty: [undefined],
      shot_type: [undefined],
      keeper_state: ["set"],
      keeper_depth: [16.515295827250299],
      jumping: [undefined],
      on_target: [undefined],
    });
  });

  it("s1_t2012: a catch where keeper_depth is exactly 0 -- present, not absent -- while keeper_state is genuinely absent on the same event", () => {
    const decoded = decode(row("s1_t2012"));
    expect(decoded.events).toEqual({
      count: 1,
      kind: ["catch"],
      x: [44.81287651599969],
      y: [286.89927505483251],
      slot: [1],
      save_style: ["stretch"],
      style: [undefined],
      outcome: [undefined],
      difficulty: [undefined],
      shot_type: [undefined],
      keeper_state: [undefined],
      keeper_depth: [0],
      jumping: [undefined],
      on_target: [undefined],
    });
    // The presence flag, not the value, is what distinguishes this from
    // "keeper_depth absent": both would otherwise decode as falsy.
    expect(decoded.events.keeper_depth[0]).not.toBeUndefined();
  });

  it("s1_t2356: a header carries the aerial field group with jumping = true", () => {
    const decoded = decode(row("s1_t2356"));
    expect(decoded.events).toEqual({
      count: 1,
      kind: ["header"],
      x: [191.24980382674741],
      y: [341.00801028559846],
      slot: [10],
      save_style: [undefined],
      style: ["header"],
      outcome: ["clean"],
      difficulty: [0.48673093838590087],
      shot_type: [undefined],
      keeper_state: [undefined],
      keeper_depth: [undefined],
      jumping: [true],
      on_target: [undefined],
    });
  });
});

describe("decode: header validation (not exercised by the reference vector, which is always well-formed)", () => {
  const words = row("t0").slice();

  it("rejects a bad magic word", () => {
    const bad = words.slice();
    bad[0] = 0;
    expect(() => decode(bad)).toThrow(/magic/);
  });

  it("rejects a layout version mismatch", () => {
    const bad = words.slice();
    bad[1] = 2;
    expect(() => decode(bad)).toThrow(/layout version/);
  });

  it("rejects a render frame version mismatch", () => {
    const bad = words.slice();
    bad[2] = 999;
    expect(() => decode(bad)).toThrow(/render frame version/);
  });

  it("rejects a total_words that disagrees with the header's own counts", () => {
    const bad = words.slice();
    bad[3] = words[3] === undefined ? 0 : words[3] + 1;
    expect(() => decode(bad)).toThrow(/total words/);
  });

  it("rejects an unrecognised nonzero enum code", () => {
    const bad = words.slice();
    // hud_species_shape, scalar field index 40 -> absolute word HEADER_WORDS + 40.
    bad[HEADER_WORDS + 40] = 99;
    expect(() => decode(bad)).toThrow(/species shape/);
  });

  it("rejects a nonzero pose id this numbering does not recognise", () => {
    const bad = words.slice();
    const count = bad[6] ?? 0;
    const playersAt = HEADER_WORDS + SCALAR_FIELD_COUNT;
    const poseIdField = 5;
    bad[playersAt + poseIdField * count] = 33; // one past the real range 1..32
    expect(() => decode(bad)).toThrow(/pose id/);
  });
});

describe("toRenderFrame", () => {
  it("combines a decoded frame and a decoded roster into pitch.ts's RenderFrame shape", () => {
    const frame: DecodedRenderFrame = decode(row("t0"));
    const roster = decodeRoster(row("roster"), syntheticRosterBlob(10));

    const renderFrame = toRenderFrame(frame, roster);

    expect(renderFrame.field).toEqual(frame.field);
    expect(renderFrame.players).toEqual(frame.players);
    expect(renderFrame.ball).toEqual(frame.ball);
    expect(renderFrame.control).toEqual(frame.control);
    expect(renderFrame.roster).toEqual(roster);
    expect(renderFrame.combat).toBeUndefined();
  });

  it("attaches a combat model only when the caller supplies one", () => {
    const frame = decode(row("t0"));
    const roster = decodeRoster(row("roster"), syntheticRosterBlob(10));
    const combat = { enabled: true, players: [], projectiles: [] };

    const renderFrame = toRenderFrame(frame, roster, combat);

    expect(renderFrame.combat).toBe(combat);
  });
});
