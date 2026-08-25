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

// ---------------------------------------------------------------------------
// Inline code entry (#610): typing a friend's code should not need the
// "USE AN INVITE" click at all -- a whole screen change to type six
// characters. This front door now carries its own composer, sharing the
// exact editing primitives (`room_code_entry.ts`) the lobby's own guest
// composer uses.
// ---------------------------------------------------------------------------

describe("multiplayer front door: inline code entry (#610)", () => {
  function key(state: State, k: string) {
    return multiplayer.update(state, { kind: "key", key: k });
  }

  function typeCode(state: State, code: string): State {
    let next = state;
    for (const ch of code) {
      [next] = key(next, ch);
    }
    return next;
  }

  it("renders an inline six-character composer on the front door itself", () => {
    const layout = multiplayer.layout(multiplayer.newState(VP));
    const widget = hit.find(layout, "code_entry");
    expect(widget, "the inline code composer must be on screen").not.toBeNull();
  });

  it("typing a key anywhere on the screen focuses the composer and starts filling it in -- no prior click", () => {
    const s = multiplayer.newState(VP);
    expect(s.focus).not.toBe("code_entry");
    const [s2] = key(s, "7");
    expect(s2.focus).toBe("code_entry");
    expect(hit.find(multiplayer.layout(s2), "code_entry")?.text).toContain("7");
    // update() never mutates its input state.
    expect(s.focus).not.toBe("code_entry");
  });

  it("sends a completed code into the lobby with a room-joining intent, once confirmed", () => {
    const s = typeCode(multiplayer.newState(VP), "7F3K9Q");
    const [, action] = multiplayer.update(s, { kind: "action", action: "confirm" });
    expect(action).toEqual({ go: "lobby", intent: "guest", code: "7F3K9Q" });
  });

  it("does nothing on confirm while the code is incomplete", () => {
    let s = multiplayer.newState(VP);
    s = typeCode(s, "7F3");
    const [, action] = multiplayer.update(s, { kind: "action", action: "confirm" });
    expect(action).toBeUndefined();
  });

  it("still leaves BACK reachable while the composer is focused", () => {
    let s = multiplayer.newState(VP);
    [s] = key(s, "7");
    expect(s.focus).toBe("code_entry");
    const [, action] = multiplayer.update(s, { kind: "action", action: "back" });
    expect(action).toEqual({ go: "title" });
  });

  it("still reaches the lobby with a room-hosting intent when the composer was never touched", () => {
    let s = multiplayer.newState(VP);
    [s] = click(s, "mode_2v2");
    const [, action] = click(s, "host");
    expect(action).toEqual({ go: "lobby", intent: "host", mode: "2v2" });
  });
});
