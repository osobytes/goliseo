// The "product flow laboratory" stand-in for a real match, used to
// exercise the squad → formation → tactic → result flow before the real
// match adapter lands (M9).
//
// Synthesizing a result from the request (a deterministic hash) is
// `@gc/app`'s `fake_result.ts`'s `forRequest` — not this package's to own
// (content.ts's header), and unlike the data lookups the other screens
// inject, it is a nontrivial *mechanism*, not a data table — injecting it
// as a callback would just move the same cross-layer dependency one level
// down. Instead this screen takes the already-computed `result` as part of
// its context, exactly as result.ts already does: computing the fake
// result is `@gc/app`'s job when it wires this screen up, not this
// screen's.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { ProductMatchRequest, ProductMatchResult } from "./content.ts";

export interface FakeMatchScreenContext {
  readonly request: ProductMatchRequest;
  readonly result: ProductMatchResult;
}

export interface FakeMatchScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly request: ProductMatchRequest;
  readonly result: ProductMatchResult;
  readonly focus: string;
}

export type FakeMatchAction =
  { readonly go: "cancel" } | { readonly go: "complete"; readonly result: ProductMatchResult };

function newState(
  viewport: { readonly w: number; readonly h: number },
  context: FakeMatchScreenContext,
): FakeMatchScreenState {
  return { viewport, request: context.request, result: context.result, focus: "complete" };
}

function layout(state: FakeMatchScreenState): Layout {
  return [
    {
      id: "eyebrow",
      kind: "eyebrow",
      text: "PRODUCT FLOW LABORATORY",
      rect: { x: 0, y: 88, w: state.viewport.w, h: 22 },
      data: { align: "center", focusable: false },
    },
    {
      id: "title",
      kind: "hero_title",
      text: "MATCH TRANSMISSION READY",
      rect: { x: 0, y: 124, w: state.viewport.w, h: 44 },
      data: { align: "center", focusable: false },
    },
    {
      id: "request",
      kind: "card",
      text: [
        "NEBULA FC  vs  ORION MINERS",
        `FORMATION  ${state.request.formation_id}`,
        `TACTIC  ${state.request.tactic_id.replaceAll("_", " ").toUpperCase()}`,
        "ARENA  HELIOS CROWN",
        "",
        "The real match adapter is intentionally disconnected until M9.",
      ].join("\n"),
      rect: { x: 230, y: 202, w: 500, h: 190 },
      data: { align: "center", focusable: false },
    },
    {
      id: "cancel",
      kind: "button",
      text: "CANCEL",
      focused: state.focus === "cancel",
      rect: { x: 264, y: 438, w: 190, h: 44 },
    },
    {
      id: "complete",
      kind: "button",
      text: "COMPLETE FIXTURE",
      focused: state.focus === "complete",
      rect: { x: 506, y: 438, w: 190, h: 44 },
    },
  ];
}

function update(
  state: FakeMatchScreenState,
  event: FocusEvent,
): readonly [FakeMatchScreenState, FakeMatchAction | undefined] {
  const currentLayout = layout(state);
  let next: FakeMatchScreenState = {
    ...state,
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "cancel" }];
  }
  const id = focus.activated(currentLayout, next.focus, event);
  if (id !== null) {
    next = { ...next, focus: id };
  }
  if (id === "complete") {
    return [next, { go: "complete", result: next.result }];
  } else if (id === "cancel") {
    return [next, { go: "cancel" }];
  }
  return [next, undefined];
}

export const fakeMatch = { newState, layout, update };
