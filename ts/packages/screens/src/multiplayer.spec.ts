// The multiplayer front door is pure: no ports, no transport, no clock, so
// every case here runs with zero display. See AGENTS.md §9 tier 2.

import { describe, expect, it } from "vitest";
import { hit } from "@gc/ui";
import { multiplayer } from "./multiplayer.ts";
import { DEFAULT_MODE, MODES } from "./lobby_model.ts";

const VP = { w: 960, h: 540 };

type State = Parameters<typeof multiplayer.update>[0];

function click(state: State, id: string) {
  const widget = hit.find(multiplayer.layout(state), id);
  expect(widget, `missing widget ${id}`).not.toBeNull();
  const rect = widget?.rect;
  expect(rect).toBeDefined();
  return multiplayer.update(state, {
    kind: "click",
    x: (rect?.x ?? 0) + (rect?.w ?? 0) / 2,
    y: (rect?.y ?? 0) + (rect?.h ?? 0) / 2,
    button: 1,
  });
}

describe("multiplayer front door", () => {
  it("offers exactly the modes the lobby model defines, and defaults to its default", () => {
    const s = multiplayer.newState(VP);
    expect(s.mode).toBe(DEFAULT_MODE);
    const offered = multiplayer.layout(s).flatMap((w) => /^mode_(.+)$/.exec(w.id)?.[1] ?? []);
    expect(offered).toEqual([...MODES]);
    expect(hit.find(multiplayer.layout(s), `mode_${DEFAULT_MODE}`)?.selected).toBe(true);
  });

  it("selects a lobby size without leaving the screen", () => {
    const s = multiplayer.newState(VP);
    const [s2, action] = click(s, "mode_2v2");
    expect(s2.mode).toBe("2v2");
    expect(action, "choosing a size should not navigate").toBeUndefined();
    expect(s.mode, "update should not mutate its input state").toBe(DEFAULT_MODE);
  });

  it("sends the host into the lobby with a room-hosting intent, carrying the chosen size", () => {
    let s = multiplayer.newState(VP);
    [s] = click(s, "mode_2v2");
    const [, action] = click(s, "host");
    expect(action).toEqual({ go: "lobby", intent: "host", mode: "2v2" });
  });

  it("sends a guest into the lobby with a room-joining intent and no size, because the host owns it", () => {
    const [, action] = click(multiplayer.newState(VP), "join");
    expect(action).toEqual({ go: "lobby", intent: "guest" });
  });

  it("says out loud that this is peer to peer, so nobody waits for matchmaking", () => {
    const layout = multiplayer.layout(multiplayer.newState(VP));
    expect(hit.find(layout, "brand")?.text).toContain("PEER TO PEER");
    expect(hit.find(layout, "note")?.text).toContain("directly between the two browsers");
  });

  it("returns to the title on Back and on the back action", () => {
    const s = multiplayer.newState(VP);
    expect(click(s, "back")[1]).toEqual({ go: "title" });
    expect(multiplayer.update(s, { kind: "action", action: "back" })[1]).toEqual({ go: "title" });
  });

  it("keeps every widget inside the virtual canvas", () => {
    for (const widget of multiplayer.layout(multiplayer.newState(VP))) {
      const rect = widget.rect;
      expect(rect, `widget ${widget.id} has no rect`).toBeDefined();
      expect(rect?.x ?? -1).toBeGreaterThanOrEqual(0);
      expect(rect?.y ?? -1).toBeGreaterThanOrEqual(0);
      expect((rect?.x ?? 0) + (rect?.w ?? 0)).toBeLessThanOrEqual(VP.w);
      expect((rect?.y ?? 0) + (rect?.h ?? 0)).toBeLessThanOrEqual(VP.h);
    }
  });
});
