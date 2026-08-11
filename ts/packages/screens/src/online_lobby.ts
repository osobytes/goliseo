// The impure half of the online lobby: it owns the star transport, the
// clipboard, and the fixed-rate lobby clock, and it draws. Every decision
// it makes is delegated to the pure screen in `lobby.ts`; this file only
// translates input, executes effects, and feeds transport facts back in.
//
// `@gc/online`'s `lobby_link.ts` (see ARCHITECTURE.md's directory table) and
// the star transport (`@gc/transport`) are both TypeScript-owned, but
// neither is a declared dependency of this package (only `@gc/core`,
// `@gc/ui`, `@gc/presentation` are, and this task may not edit
// package.json). Both are threaded through as injected ports --
// `LobbyLinkPort`/`starFactory` -- following the same pattern as every
// Rust-owned dependency elsewhere in this package. `@gc/ui`'s `draw` and
// `motion` *are* a declared dependency, so `draw()` and the transition wipe
// use the real modules.

import { draw, motion, type GraphicsBackend } from "@gc/ui";
import { lobby, type LobbyEffect, type LobbyScreenEvent, type LobbyScreenState } from "./lobby.ts";
import type { LobbyCommand, LobbyModelOptions, LobbyModelPorts, LobbyRole } from "./lobby_model.ts";

export interface LobbyClipboard {
  read(): string | undefined;
  write(text: string): void;
}

/** `@gc/online`'s `lobby_link.ts`'s `LobbyLink` instance, injected -- see this module's header. */
export interface LobbyLinkInstance<TStar, TEvent extends LobbyCommand> {
  readonly star: TStar;
  send(linkId: string, wire: string): void;
  apply(effect: LobbyEffect): readonly [boolean, string | undefined];
  poll(): readonly TEvent[];
}

export interface OnlineLobbyOptions<TStar, TEvent extends LobbyCommand> {
  readonly starFactory: (role: LobbyRole, peerId: string) => TStar | undefined;
  readonly newLink: (star: TStar) => LobbyLinkInstance<TStar, TEvent>;
  readonly clipboard?: LobbyClipboard;
  readonly modelPorts: LobbyModelPorts;
  readonly modelOptions?: LobbyModelOptions;
}

export type OnlineLobbyAction = { readonly go: string; readonly [key: string]: unknown };

const TICK_SECONDS = 1 / 60;

export class OnlineLobby<TStar, TEvent extends LobbyCommand> {
  state: LobbyScreenState;
  link: LobbyLinkInstance<TStar, TEvent> | undefined;
  transition = 0;
  private accumulator = 0;
  private readonly onAction: ((action: OnlineLobbyAction) => void) | undefined;
  private readonly clipboard: LobbyClipboard;
  private readonly starFactory: (role: LobbyRole, peerId: string) => TStar | undefined;
  private readonly newLink: (star: TStar) => LobbyLinkInstance<TStar, TEvent>;

  constructor(
    viewport: { readonly w: number; readonly h: number },
    onAction: ((action: OnlineLobbyAction) => void) | undefined,
    options: OnlineLobbyOptions<TStar, TEvent>
  ) {
    this.state = lobby.newState(viewport, options.modelPorts, {
      ...(options.modelOptions !== undefined ? { options: options.modelOptions } : {}),
    });
    this.onAction = onAction;
    this.clipboard = options.clipboard ?? { read: () => undefined, write: () => undefined };
    this.starFactory = options.starFactory;
    this.newLink = options.newLink;
  }

  dispatch(command: LobbyCommand): void {
    const [state, action] = lobby.update(this.state, { kind: "lobby", command });
    this.state = state;
    this.run(state.effects);
    if (action && this.onAction) {
      this.onAction(action);
    }
  }

  private run(effects: readonly LobbyEffect[]): void {
    for (const effect of effects) {
      if (effect.kind === "open_star") {
        const star = this.starFactory(effect.role, effect.peer_id);
        if (star !== undefined) {
          this.link = this.newLink(star);
        }
      } else if (effect.kind === "clipboard") {
        this.clipboard.write(effect.text);
      } else if (effect.kind === "paste_request") {
        const text = this.clipboard.read();
        // Straight back into the pure model, which keeps only a digest.
        this.dispatch({ kind: "paste", text: text ?? "" });
      } else if (effect.kind === "shutdown") {
        if (this.link) {
          this.link.apply(effect);
          this.link = undefined;
        }
      } else if (effect.kind !== "leave" && effect.kind !== "start_match") {
        if (this.link) {
          const [ok, err] = this.link.apply(effect);
          if (!ok && err) {
            this.dispatch({ kind: "link_error", detail: err });
          }
        }
      }
    }
  }

  update(dt: number): void {
    this.transition = motion.advance(this.transition, dt);
    if (this.link) {
      for (const event of this.link.poll()) {
        this.dispatch(event);
      }
    }
    this.accumulator += dt;
    while (this.accumulator >= TICK_SECONDS) {
      this.accumulator -= TICK_SECONDS;
      this.dispatch({ kind: "tick" });
    }
  }

  event(evt: LobbyScreenEvent): void {
    const [state, action] = lobby.update(this.state, evt);
    this.state = state;
    this.run(state.effects);
    if (action && this.onAction) {
      this.onAction(action);
    }
  }

  draw(backend: GraphicsBackend): void {
    draw.layout(backend, lobby.layout(this.state), this.state.viewport, this.transition);
  }

  teardown(): void {
    if (this.link) {
      this.link.apply({ kind: "shutdown" });
      this.link = undefined;
    }
  }
}
