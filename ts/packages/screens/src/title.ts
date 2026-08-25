// The main title/menu screen. See AGENTS.md §9 for the pure/impure seam;
// this screen needs no injected content.
//
// Five entries, down from seven. What went, and why:
//
//   - COMBAT PROTOTYPE was a duplicate of PLAY distinguished only by
//     `session.setCombatEnabled`. Combat is a match option, not a second Play
//     button, so it is a toggle on the team sheet now.
//   - QUIT has been a no-op since native was dropped: the browser entry logs
//     a message and returns. The `back` action still emits `{go: "quit"}` —
//     that is the window-close gesture, which `App` forwards to an injected
//     `requestQuit` — but there is no longer a button for it.
//   - CREDITS folded into Settings -> About, where build info belongs.
//   - ONLINE LOBBY (DEV) became MULTIPLAYER, a real front door -- and #610
//     round-2 review folded that front door into the hosting screen
//     itself, so this entry is PLAY ONLINE now (still the `"multiplayer"`
//     action id -- only the player-facing label changed): it routes
//     straight to an already-live hosting attempt (a room code, mode
//     chips, START MATCH), no intermediate card screen. A friend who means
//     to join uses that screen's own inline code entry instead of a
//     separate JOIN card.
//
// TEAM is new: the team sheet used to be reachable only through PLAY, which
// meant "look at my team" and "start a match" were the same click. It is a
// destination now — `app.ts`'s `team_settings.ts` persists whatever is set
// here, so a visit that ends in BACK still sticks for the next PLAY.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";

export interface TitleScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly focus: string;
}

export type TitleAction = { readonly go: string };

function newState(viewport: { readonly w: number; readonly h: number }): TitleScreenState {
  return { viewport, focus: "play" };
}

function layout(state: TitleScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "brand",
      kind: "eyebrow",
      text: "ARCADE COMBAT SOCCER  •  5 V 5",
      rect: { x: 0, y: 72, w: state.viewport.w, h: 22 },
      data: { align: "center" },
    },
    {
      id: "title",
      kind: "hero_title",
      text: "GOLISEO",
      rect: { x: 0, y: 104, w: state.viewport.w, h: 58 },
      data: { align: "center" },
    },
    {
      id: "tagline",
      kind: "label",
      text: "COACH FOR THIRTY SECONDS  •  THEN PLAY IT YOURSELF",
      rect: { x: 0, y: 174, w: state.viewport.w, h: 24 },
      data: { align: "center", tone: "muted" },
    },
  ];

  const labels: readonly [string, string][] = [
    ["play", "PLAY"],
    ["team", "TEAM"],
    ["multiplayer", "PLAY ONLINE"],
    ["help", "HOW TO PLAY"],
    ["settings", "SETTINGS"],
  ];
  labels.forEach(([id, text], i) => {
    widgets.push({
      id,
      kind: "button",
      text,
      focused: state.focus === id,
      rect: { x: 330, y: 224 + i * 44, w: 300, h: 42 },
    });
  });

  widgets.push({
    id: "hint",
    kind: "label",
    text: "UP / DOWN TO NAVIGATE  •  ENTER TO SELECT",
    rect: { x: 0, y: 470, w: state.viewport.w, h: 20 },
    data: { align: "center", tone: "muted", focusable: false },
  });
  return widgets;
}

function update(
  state: TitleScreenState,
  event: FocusEvent,
): readonly [TitleScreenState, TitleAction | undefined] {
  const currentLayout = layout(state);
  const nextFocus = focus.navigate(currentLayout, state.focus, event) ?? state.focus;
  if (event.kind === "action" && event.action === "back") {
    return [{ viewport: state.viewport, focus: nextFocus }, { go: "quit" }];
  }
  const id = focus.activated(currentLayout, nextFocus, event);
  const nextState: TitleScreenState = { viewport: state.viewport, focus: id ?? nextFocus };
  return [nextState, id !== null ? { go: id } : undefined];
}

export const title = { newState, layout, update };

type Widget = Layout[number];
