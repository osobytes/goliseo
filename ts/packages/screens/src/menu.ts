// The adapter that drives a pure screen definition (layout/update) as a
// screen in the stack. All logic stays pure in the screen def; this class
// only gathers events, routes returned actions, and renders the layout.
// See AGENTS.md §9.
//
// `Menu`'s own stated job — gathering events, routing actions, rendering
// the layout — never actually needs state *construction*; only
// `layout`/`update` do. Every screen in this package takes its content
// (players, teams, formations, ...) as an explicit `newState` parameter
// beyond `viewport`/`context` (content.ts's header — ARCHITECTURE.md §4
// rule 6), so a single uniform `newState(viewport, context)` signature does
// not fit every screen. `Menu` therefore takes an already-constructed
// `initialState` instead of building one itself, and `ScreenDef` only
// requires `layout`/`update`. Composing a screen's `newState` with its
// injected content into that initial state is the same later milestone
// that wires screens to real content.

import { draw, motion, type GraphicsBackend, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";

export interface Viewport {
  readonly w: number;
  readonly h: number;
}

interface HasViewport {
  readonly viewport: Viewport;
}

/** The pure half of a screen: `newState` is intentionally not part of this contract — see this file's header. */
export interface ScreenDef<State extends HasViewport, Action> {
  layout(state: State): Layout;
  update(state: State, event: FocusEvent): readonly [State, Action | undefined];
}

export class Menu<State extends HasViewport, Action> {
  private readonly def: ScreenDef<State, Action>;
  private readonly onAction: ((action: Action) => void) | undefined;
  private state: State;
  private transition = 0;

  constructor(def: ScreenDef<State, Action>, initialState: State, onAction?: (action: Action) => void) {
    this.def = def;
    this.onAction = onAction;
    this.state = initialState;
  }

  update(dt: number): void {
    this.transition = motion.advance(this.transition, dt);
  }

  event(evt: FocusEvent): void {
    const [state, action] = this.def.update(this.state, evt);
    this.state = state;
    if (action !== undefined && this.onAction) {
      this.onAction(action);
    }
  }

  draw(backend: GraphicsBackend): void {
    draw.layout(backend, this.def.layout(this.state), this.state.viewport, this.transition);
  }
}
