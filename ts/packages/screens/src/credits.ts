// The credits & source screen. `@gc/app`'s `build_info.ts` is injected as
// a `BuildInfo` parameter; see content.ts's header and squad.ts's for the
// rationale.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { BuildInfo } from "./content.ts";

export interface CreditsScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly buildInfo: BuildInfo;
  readonly focus: string;
}

export type CreditsAction = { readonly go: "back" };

function newState(
  viewport: { readonly w: number; readonly h: number },
  buildInfo: BuildInfo,
): CreditsScreenState {
  return { viewport, buildInfo, focus: "back" };
}

function creditLines(buildInfo: BuildInfo): readonly string[] {
  const lines = [
    "GOLISEO",
    "Designed and developed by the GOLISEO contributors",
    "",
    "Third-party runtime: LÖVE 11.5 and LuaJIT",
    "Third-party media: none bundled",
    "Code, documentation, and repository-owned assets:",
    "PolyForm Noncommercial License 1.0.0",
    "",
    `Version ${buildInfo.version} • ${buildInfo.channel}`,
  ];
  if (buildInfo.source_url !== undefined) {
    lines.push(`Source: ${buildInfo.source_url}`);
  }
  lines.push("");
  lines.push("No warranty. See LICENSE for complete terms.");
  return lines;
}

function layout(state: CreditsScreenState): Layout {
  return [
    {
      id: "title",
      kind: "title",
      text: "CREDITS & SOURCE",
      rect: { x: 64, y: 54, w: 832, h: 36 },
      data: { align: "center", focusable: false },
    },
    {
      id: "credits",
      kind: "card",
      text: creditLines(state.buildInfo).join("\n"),
      rect: { x: 180, y: 118, w: 600, h: 300 },
      data: { align: "center", focusable: false },
    },
    {
      id: "back",
      kind: "button",
      text: "BACK",
      focused: true,
      rect: { x: 380, y: 458, w: 200, h: 42 },
    },
  ];
}

function update(
  state: CreditsScreenState,
  event: FocusEvent,
): readonly [CreditsScreenState, CreditsAction | undefined] {
  if (event.kind === "action" && event.action === "back") {
    return [state, { go: "back" }];
  }
  const id = focus.activated(layout(state), state.focus, event);
  return [state, id === "back" ? { go: "back" } : undefined];
}

export const credits = { newState, layout, update };
