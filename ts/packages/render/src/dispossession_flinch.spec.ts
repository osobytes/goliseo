// Tests for dispossession_flinch.ts -- the presentation-only victim
// reaction window for the standing-poke tackle (#591). Structured after
// `release_follow.spec.ts`, with the extra `ownerId` dimension this module's
// victim resolution needs -- see the module header's "owner just before
// this batch" section.
//
// `remaining`/`lastOwnerId` are module state, so every case resets first.

import { beforeEach, describe, expect, it } from "vitest";
import { dispossessionFlinch } from "./dispossession_flinch.ts";

beforeEach(() => {
  dispossessionFlinch.reset();
});

describe("dispossession_flinch.update", () => {
  it("latches a window on the previous owner when a tackle event fires", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.active("victim")).toBe(true);
  });

  it("ignores every other event kind", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update(
      [
        { kind: "goal", player: "challenger" },
        { kind: "tackle_miss", player: "challenger" },
        { kind: "save", player: "keeper" },
        { kind: "kickoff", player: "challenger" },
      ],
      0,
      "victim",
    );
    expect(dispossessionFlinch.active("victim")).toBe(false);
    expect(dispossessionFlinch.windows()).toEqual({});
  });

  it("opens no window when nobody owned the ball before the tackle", () => {
    // No prior `update` call at all -- `lastOwnerId` starts undefined.
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.windows()).toEqual({});
  });

  it("never blames the challenger for their own tackle", () => {
    // A defensive guard against a same-id producer bug upstream: the
    // remembered owner and the tackle's own `player` must never coincide in
    // real data (a player cannot tackle themselves), but this proves the
    // module does not open a window even if they did.
    dispossessionFlinch.update([], 0, "same_id");
    dispossessionFlinch.update([{ kind: "tackle", player: "same_id" }], 0, undefined);
    expect(dispossessionFlinch.active("same_id")).toBe(false);
  });

  it("resolves the victim as the owner BEFORE this batch, not the owner this call reports", () => {
    // Owner is "victim" going into this call; the tackle inside it must
    // still blame "victim", even though `ownerId` reports the ball now
    // belongs to nobody (`win_ball` clears `owner` the same tick).
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.active("victim")).toBe(true);

    // The NEXT tackle (a different victim, ball having changed hands again)
    // must not still blame the first victim.
    dispossessionFlinch.update([], 0, "second_victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "someone_else" }], 0, undefined);
    expect(dispossessionFlinch.active("second_victim")).toBe(true);
  });

  it("ages first, so a tackle arriving in this frame's batch keeps its FULL duration", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0.1, undefined);
    dispossessionFlinch.update([], dispossessionFlinch.DURATION - 0.001, undefined);
    expect(dispossessionFlinch.active("victim")).toBe(true);
  });

  it("ages a window out after exactly DURATION seconds", () => {
    expect(dispossessionFlinch.DURATION).toBe(0.3);
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    dispossessionFlinch.update([], 0.25, undefined);
    expect(dispossessionFlinch.active("victim")).toBe(true);
    dispossessionFlinch.update([], 0.05, undefined);
    expect(dispossessionFlinch.active("victim")).toBe(false);
    expect(dispossessionFlinch.windows()).toEqual({});
  });

  it("does not age anything on a zero or negative dt", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    dispossessionFlinch.update([], 0, undefined);
    dispossessionFlinch.update([], -1, undefined);
    expect(dispossessionFlinch.active("victim")).toBe(true);
  });
});

describe("dispossession_flinch.windows", () => {
  it("is an id -> true map of the OPEN windows only", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.windows()).toEqual({ victim: true });
  });

  it("is a snapshot: mutating it cannot reach back into the module's own state", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    const snapshot = dispossessionFlinch.windows() as Record<string, true>;
    delete snapshot["victim"];
    expect(dispossessionFlinch.active("victim")).toBe(true);
    expect(dispossessionFlinch.windows()).toEqual({ victim: true });
  });

  it("is empty with nothing open", () => {
    expect(dispossessionFlinch.windows()).toEqual({});
  });
});

describe("dispossession_flinch.slotMask", () => {
  const roster = ["keeper", "back", "victim", "playmaker", "winger"];

  it("is zero with no window open", () => {
    expect(dispossessionFlinch.slotMask(roster)).toBe(0);
  });

  it("sets the bit for the flinching player's roster slot", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.slotMask(roster)).toBe(0b00100);
  });

  it("clears the bit once the window ages out", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    dispossessionFlinch.update([], dispossessionFlinch.DURATION, undefined);
    expect(dispossessionFlinch.slotMask(roster)).toBe(0);
  });

  it("drops slots past 32 rather than aliasing them onto low bits", () => {
    const overlong = Array.from({ length: 40 }, (_, index) => `p${index}`);
    dispossessionFlinch.update([], 0, "p33");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.slotMask(overlong)).toBe(0);
  });
});

describe("dispossession_flinch.reset", () => {
  it("drops every window and the remembered owner", () => {
    dispossessionFlinch.update([], 0, "victim");
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    dispossessionFlinch.reset();
    expect(dispossessionFlinch.active("victim")).toBe(false);
    expect(dispossessionFlinch.windows()).toEqual({});
    expect(dispossessionFlinch.slotMask(["victim"])).toBe(0);

    // The remembered owner is gone too: a tackle right after reset (no
    // fresh `update([], _, ownerId)` call first) opens no window.
    dispossessionFlinch.update([{ kind: "tackle", player: "challenger" }], 0, undefined);
    expect(dispossessionFlinch.windows()).toEqual({});
  });
});
