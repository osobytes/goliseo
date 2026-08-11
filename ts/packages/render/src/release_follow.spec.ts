// Tests for release_follow.ts -- the presentation-only follow-through window
// for outfield ball releases. Every sibling in this package has a spec; this
// module shipped without one, which is how it stayed a dead end (no consumer
// AND no test) long enough for the pose it feeds to become structurally
// unreachable in a real match.
//
// `remaining` is module state, so every case resets first.

import { beforeEach, describe, expect, it } from "vitest";
import { releaseFollow } from "./release_follow.ts";

beforeEach(() => {
  releaseFollow.reset();
});

describe("release_follow.update", () => {
  it("latches a window on a shot and on a pass", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    expect(releaseFollow.active("striker")).toBe(true);

    releaseFollow.reset();
    releaseFollow.update([{ kind: "pass", player: "playmaker" }], 0);
    expect(releaseFollow.active("playmaker")).toBe(true);
  });

  it("ignores every other event kind", () => {
    releaseFollow.update(
      [
        { kind: "goal", player: "striker" },
        { kind: "tackle", player: "striker" },
        { kind: "save", player: "keeper" },
        { kind: "kickoff", player: "striker" },
      ],
      0,
    );
    expect(releaseFollow.active("striker")).toBe(false);
    expect(releaseFollow.active("keeper")).toBe(false);
    expect(releaseFollow.windows()).toEqual({});
  });

  it("ignores a release with no attributed player", () => {
    releaseFollow.update([{ kind: "shot" }, { kind: "pass" }], 0);
    expect(releaseFollow.windows()).toEqual({});
  });

  it("ages first, so a release arriving in this frame's batch keeps its FULL duration", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0.1);
    // Had the batch been latched before the ageing pass, this frame's own dt
    // would already have eaten 0.1s of the window.
    releaseFollow.update([], releaseFollow.DURATION - 0.001);
    expect(releaseFollow.active("striker")).toBe(true);
  });

  it("ages a window out after exactly DURATION seconds", () => {
    expect(releaseFollow.DURATION).toBe(0.15);
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    releaseFollow.update([], 0.14);
    expect(releaseFollow.active("striker")).toBe(true);
    releaseFollow.update([], 0.01);
    expect(releaseFollow.active("striker")).toBe(false);
    expect(releaseFollow.windows()).toEqual({});
  });

  it("re-latches a still-open window back to its full duration", () => {
    releaseFollow.update([{ kind: "pass", player: "playmaker" }], 0);
    releaseFollow.update([{ kind: "pass", player: "playmaker" }], 0.1);
    releaseFollow.update([], 0.1);
    expect(releaseFollow.active("playmaker")).toBe(true);
  });

  it("does not age anything on a zero or negative dt", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    releaseFollow.update([], 0);
    releaseFollow.update([], -1);
    expect(releaseFollow.active("striker")).toBe(true);
  });

  it("tracks each player's window independently", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    releaseFollow.update([{ kind: "pass", player: "playmaker" }], 0.1);
    // 0.1s in, the striker has 0.05s left and the playmaker a fresh 0.15s.
    releaseFollow.update([], 0.06);
    expect(releaseFollow.active("striker")).toBe(false);
    expect(releaseFollow.active("playmaker")).toBe(true);
  });
});

describe("release_follow.windows", () => {
  it("is an id -> true map of the OPEN windows only", () => {
    releaseFollow.update(
      [
        { kind: "shot", player: "striker" },
        { kind: "pass", player: "playmaker" },
      ],
      0,
    );
    expect(releaseFollow.windows()).toEqual({ striker: true, playmaker: true });
  });

  it("is a snapshot: mutating it cannot reach back into the module's own state", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    const snapshot = releaseFollow.windows() as Record<string, true>;
    delete snapshot["striker"];
    expect(releaseFollow.active("striker")).toBe(true);
    expect(releaseFollow.windows()).toEqual({ striker: true });
  });

  it("is empty with nothing open", () => {
    expect(releaseFollow.windows()).toEqual({});
  });
});

describe("release_follow.slotMask", () => {
  const roster = ["keeper", "back", "striker", "playmaker", "winger"];

  it("is zero with no window open", () => {
    expect(releaseFollow.slotMask(roster)).toBe(0);
  });

  it("sets the bit for each open window's roster slot", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    expect(releaseFollow.slotMask(roster)).toBe(0b00100);

    releaseFollow.update([{ kind: "pass", player: "keeper" }], 0);
    expect(releaseFollow.slotMask(roster)).toBe(0b00101);
  });

  it("ignores an open window whose id is not on the roster it was handed", () => {
    releaseFollow.update([{ kind: "shot", player: "someone_else" }], 0);
    expect(releaseFollow.slotMask(roster)).toBe(0);
    // ...but the window itself is still open for anyone reading by id.
    expect(releaseFollow.active("someone_else")).toBe(true);
  });

  it("clears the bit once the window ages out", () => {
    releaseFollow.update([{ kind: "shot", player: "striker" }], 0);
    releaseFollow.update([], releaseFollow.DURATION);
    expect(releaseFollow.slotMask(roster)).toBe(0);
  });

  it("stays an unsigned 32-bit value for a high slot rather than going negative", () => {
    const wide = Array.from({ length: 32 }, (_, index) => `p${index}`);
    releaseFollow.update([{ kind: "shot", player: "p31" }], 0);
    expect(releaseFollow.slotMask(wide)).toBe(0x8000_0000);
  });

  it("drops slots past 32 rather than aliasing them onto low bits", () => {
    const overlong = Array.from({ length: 40 }, (_, index) => `p${index}`);
    releaseFollow.update([{ kind: "shot", player: "p33" }], 0);
    expect(releaseFollow.slotMask(overlong)).toBe(0);
  });
});

describe("release_follow.reset", () => {
  it("drops every window", () => {
    releaseFollow.update(
      [
        { kind: "shot", player: "striker" },
        { kind: "pass", player: "playmaker" },
      ],
      0,
    );
    releaseFollow.reset();
    expect(releaseFollow.active("striker")).toBe(false);
    expect(releaseFollow.active("playmaker")).toBe(false);
    expect(releaseFollow.windows()).toEqual({});
    expect(releaseFollow.slotMask(["striker", "playmaker"])).toBe(0);
  });
});
