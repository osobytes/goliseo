// Ported from game/screens/menu.lua — the adapter that drives a pure screen
// definition (layout/update) as a screen in the stack. All logic stays pure
// in the screen def; this class only gathers events, routes returned
// actions, and renders the layout. See AGENTS.md §9.
//
// Deviation from the Lua original: `Menu.new(def, viewport, on_action,
// context)` constructs the screen's initial state itself, by calling
// `def.new_state(viewport, context)`. Every screen in this package now
// takes its content (players, teams, formations, ...) as an explicit
// `newState` parameter beyond `viewport`/`context` (content.ts's header —
// v2/README.md rule 6.7), so a single uniform `newState(viewport, context)`
// signature no longer fits every screen. `Menu`'s own stated job — gathering
// events, routing actions, rendering the layout — never actually needed
// state *construction*; only `layout`/`update` do. This port therefore
// takes an already-constructed `initialState` instead of building one, and
// `ScreenDef` only requires `layout`/`update`. Composing a screen's
// `newState` with its injected content into that initial state is the same
// later milestone that wires screens to real content (v2/README.md §1).

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
