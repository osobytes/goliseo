// Every terminal reason the coordinator can emit has to land somewhere
// readable. The sweep below is driven by `lobby_model.ts`'s own
// `TERMINAL_TEXT` table rather than a hand-written list, so a reason added to
// the model without a thought for this screen fails here instead of rendering
// blank.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { sessionEnded } from "./session_ended.ts";
import { TERMINAL_TEXT, type CoordinatorTerminalReason } from "./lobby_model.ts";

const VP = { w: 960, h: 540 };

type State = Parameters<typeof sessionEnded.update>[0];

function click(state: State, id: string) {
  const widget = hit.find(sessionEnded.layout(state), id);
  expect(widget, `missing widget ${id}`).not.toBeNull();
  const rect = widget?.rect;
  return sessionEnded.update(state, {
    kind: "click",
    x: (rect?.x ?? 0) + (rect?.w ?? 0) / 2,
    y: (rect?.y ?? 0) + (rect?.h ?? 0) / 2,
    button: 1,
  });
}

const REASONS = Object.keys(TERMINAL_TEXT) as CoordinatorTerminalReason[];

describe("session ended", () => {
  it("gives every terminal reason the model can emit a headline and a consequence", () => {
    expect(REASONS.length, "expected the model to define terminal reasons").toBeGreaterThan(0);
    for (const reason of REASONS) {
      const state = sessionEnded.newState(VP, { reason });
      expect(state.headline, `no headline for ${reason}`).toBe(TERMINAL_TEXT[reason]);
      expect(state.consequence.length, `no consequence for ${reason}`).toBeGreaterThan(0);
      // The typed reason survives into the strip, verbatim.
      expect(state.detail).toContain(reason);
    }
  });

  it("prefers the text the caller supplies over its own table", () => {
    const state = sessionEnded.newState(VP, {
      reason: "guest_left",
      text: "A guest left the lobby.",
    });
    expect(state.headline).toBe("A guest left the lobby.");
  });

  it("renders a reason it has never heard of rather than throwing", () => {
    const state = sessionEnded.newState(VP, { reason: "some_future_reason" });
    expect(state.headline).toBe("The online session ended.");
    expect(state.detail).toContain("some_future_reason");
    expect(state.consequence.length).toBeGreaterThan(0);
  });

  it("says results were not recorded only when the session was abandoned", () => {
    expect(sessionEnded.newState(VP, { reason: "peer_abort" }).consequence).toContain(
      "not recorded",
    );
    expect(sessionEnded.newState(VP, { reason: "completed" }).consequence).toContain(
      "finished cleanly",
    );
    expect(sessionEnded.newState(VP, { reason: "completed" }).consequence).not.toContain(
      "not recorded",
    );
  });

  it("carries transport measurements into the detail strip when they exist", () => {
    const state = sessionEnded.newState(VP, {
      reason: "transport_lost",
      detail: "ice failed",
      tick: 4440,
      rttMs: 51,
    });
    expect(state.detail).toContain("transport_lost");
    expect(state.detail).toContain("ice failed");
    expect(state.detail).toContain("tick 4440");
    expect(state.detail).toContain("last rtt 51 ms");
  });

  it("omits measurements it was not given, rather than inventing zeroes", () => {
    const state = sessionEnded.newState(VP, { reason: "host_left" });
    expect(state.detail).toBe("host_left");
  });

  it("offers both exits, one keypress away", () => {
    const s = sessionEnded.newState(VP, { reason: "host_left" });
    expect(click(s, "main_menu")[1]).toEqual({ go: "main_menu" });
    expect(click(s, "new_lobby")[1]).toEqual({ go: "multiplayer" });
    expect(sessionEnded.update(s, { kind: "action", action: "back" })[1]).toEqual({
      go: "main_menu",
    });
  });

  it("keeps every widget inside the virtual canvas, for every reason", () => {
    for (const reason of REASONS) {
      for (const widget of sessionEnded.layout(sessionEnded.newState(VP, { reason }))) {
        const rect = widget.rect;
        expect(rect, `widget ${widget.id} has no rect`).toBeDefined();
        expect(rect?.x ?? -1).toBeGreaterThanOrEqual(0);
        expect(rect?.y ?? -1).toBeGreaterThanOrEqual(0);
        expect((rect?.x ?? 0) + (rect?.w ?? 0)).toBeLessThanOrEqual(VP.w);
        expect((rect?.y ?? 0) + (rect?.h ?? 0)).toBeLessThanOrEqual(VP.h);
      }
    }
  });
});
