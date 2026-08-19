// The impure half of the online lobby: it owns the star transport, the
// room-code signaling channel, the clipboard, and the fixed-rate lobby
// clock, and it draws. Every decision it makes is delegated to the pure
// screen in `lobby.ts`; this file only translates input, executes effects,
// and feeds transport/signaling facts back in.
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
//
// `@gc/online`'s `room_signaling.ts` (#552) is threaded through the exact
// same way, as `RoomSignalingFactory`/`RoomSignalingHandle` -- structural
// types this file declares itself, never imported. `@gc/app`'s
// `room_signaling_port.ts` supplies the real, WebSocket-backed
// implementation; a spec supplies a fake one, mirroring `starFactory`.

import { draw, motion, type GraphicsBackend } from "@gc/ui";
import {
  lobby,
  ROOM_CODE_ENTRY_WIDGET,
  type LobbyEffect,
  type LobbyScreenEvent,
  type LobbyScreenState,
} from "./lobby.ts";
import type {
  LobbyCommand,
  LobbyModelOptions,
  LobbyModelPorts,
  LobbyRole,
  SessionMatchMode,
} from "./lobby_model.ts";

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

/** One fact the room-code signaling channel reports back -- structurally
 * `@gc/app`'s `room_signaling_port.ts`'s `RoomSignalingEvent` (and a fake's,
 * in a spec), never imported. `kind` names which `lobby_model.ts` command
 * this becomes (`roomCommandFor` below is the exhaustive mapping); the
 * other fields are populated according to which. */
export interface RoomSignalingEvent {
  readonly kind:
    | "created"
    | "joined"
    | "guest_joined"
    | "guest_left"
    | "signal"
    | "failed"
    | "dropped"
    | "host_left";
  readonly code?: string;
  readonly guest_id?: string;
  readonly signal?: string;
  readonly reason?: string;
  /** Present only on a "signal" a GUEST receives from the host: the
   * invitation slot (`guest_N`) that offer answers (#601, `lobby_model.ts`'s
   * own header). A guest adopts this as its own identity before creating a
   * coordinator, instead of guessing. */
  readonly slot?: string;
}

/** A live room-code connection -- structurally `@gc/app`'s
 * `RoomSignalingHandle`, never imported. `send`'s shape mirrors
 * `lobby_model.ts`'s own `room_send` effect (`to` omitted means "the only
 * possible recipient", i.e. a guest addressing its host; `slot` is present
 * only on a HOST's outgoing offer, #601). */
export interface RoomSignalingHandle {
  poll(): readonly RoomSignalingEvent[];
  send(effect: { readonly to?: string; readonly signal: string; readonly slot?: string }): void;
  close(): void;
}

/** Opens the room-code signaling channel for either role -- injected, the
 * same pattern `starFactory` establishes for the star transport. Omitted
 * entirely (e.g. in a spec that never drives the room-code path), the
 * `room_open_host`/`room_open_guest` effects below simply do nothing: the
 * lobby stays on "connecting" until the player backs out, which is a safe,
 * inert default rather than a crash. */
export interface RoomSignalingFactory {
  openHost(): RoomSignalingHandle;
  openGuest(code: string): RoomSignalingHandle;
}

export interface OnlineLobbyOptions<TStar, TEvent extends LobbyCommand> {
  readonly starFactory: (role: LobbyRole, peerId: string) => TStar | undefined;
  readonly newLink: (star: TStar) => LobbyLinkInstance<TStar, TEvent>;
  readonly clipboard?: LobbyClipboard;
  readonly roomSignaling?: RoomSignalingFactory;
  readonly modelPorts: LobbyModelPorts;
  readonly modelOptions?: LobbyModelOptions;
  /**
   * Decided on the multiplayer front door and applied here as the lobby's
   * opening command, so a player who already chose Host or Join does not
   * choose it again: the lobby runs `lobby_model.ts`'s `room_pick` path for
   * this role immediately (#597). Dispatched rather than baked into
   * `newLobbyModel` so `room_pick`'s own effects (opening the room-code
   * channel, in particular) run through `run()` exactly as they do when
   * "HOST/JOIN WITH A ROOM CODE" is clicked -- the model needs no new
   * option, and the lobby's own role screen (including the manual
   * copy/paste fallback) stays reachable when nothing was preset, or once a
   * preset room-code attempt is cancelled or fails.
   *
   * This preempts `lobby_model.ts`'s OLD preset-manual-role path
   * (dispatching a bare `{kind:"role",...}` here) on purpose: that path
   * locked in the manual-signaling role before the lobby ever rendered, so
   * `lobby.ts`'s room-code buttons -- which only render while no role is
   * chosen -- were unreachable from the front door. See multiplayer.ts's
   * `MultiplayerAction` doc for the full mechanism.
   */
  readonly roomIntent?: LobbyRole;
  /**
   * Host-side only; a guest is told the mode by the host. Applied the
   * moment this peer first becomes the host of a real coordinator, rather
   * than up front: `lobby_model.ts`'s `setMode` refuses a "mode" command
   * before `chooseRole` has run (no coordinator yet), which is exactly the
   * state `room_pick`'s host path leaves the model in until the room-code
   * Worker confirms the room. That confirmation (`room_created`) is the
   * common case, but not the only one that resolves this peer to host: a
   * room-hosting attempt that fails or is cancelled falls back to the
   * manual role screen, and a manual host pick from there must apply this
   * mode too, or it silently reverts to `DEFAULT_MODE`. See
   * `applyPendingModeIfHostJustResolved()`'s own doc below.
   */
  readonly mode?: SessionMatchMode;
}

export type OnlineLobbyAction = { readonly go: string; readonly [key: string]: unknown };

const TICK_SECONDS = 1 / 60;

/** Maps one `RoomSignalingEvent` to the `lobby_model.ts` command it becomes
 * -- exhaustive over `RoomSignalingEvent.kind`, mirroring `lobby_link.ts`'s
 * own `LobbyLinkEvent` -> `LobbyCommand` structural compatibility, except
 * here the shapes genuinely differ (`guestId` vs `guest_id`, ...) so an
 * explicit mapping earns its keep. */
function roomCommandFor(event: RoomSignalingEvent): LobbyCommand {
  switch (event.kind) {
    case "created":
      return { kind: "room_created", code: event.code ?? "" };
    case "joined":
      return { kind: "room_joined" };
    case "guest_joined":
      return { kind: "room_guest_joined", guest_id: event.guest_id ?? "" };
    case "guest_left":
      return { kind: "room_guest_left", guest_id: event.guest_id ?? "" };
    case "signal":
      return {
        kind: "room_peer_signal",
        signal: event.signal ?? "",
        ...(event.guest_id !== undefined ? { guest_id: event.guest_id } : {}),
        ...(event.slot !== undefined ? { slot: event.slot } : {}),
      };
    case "failed":
      return { kind: "room_failed", reason: event.reason ?? "unknown" };
    case "dropped":
      return { kind: "room_dropped" };
    case "host_left":
      // Reuses the existing `room_failed` machinery rather than a new
      // command -- a host departure IS a room-code connection ending, with
      // its own distinct player-facing copy (`lobby_model.ts`'s
      // `ROOM_FAILURE_TEXT["host_left"]`), the same as every other reason
      // that pipeline already handles.
      return { kind: "room_failed", reason: "host_left" };
  }
}

export class OnlineLobby<TStar, TEvent extends LobbyCommand> {
  state: LobbyScreenState;
  link: LobbyLinkInstance<TStar, TEvent> | undefined;
  transition = 0;
  private accumulator = 0;
  private roomLink: RoomSignalingHandle | undefined;
  private readonly onAction: ((action: OnlineLobbyAction) => void) | undefined;
  private readonly clipboard: LobbyClipboard;
  private readonly starFactory: (role: LobbyRole, peerId: string) => TStar | undefined;
  private readonly newLink: (star: TStar) => LobbyLinkInstance<TStar, TEvent>;
  private readonly roomSignaling: RoomSignalingFactory | undefined;
  /**
   * The front door's chosen match size, held until this peer first becomes
   * the host of a real coordinator -- see `OnlineLobbyOptions.mode`'s own
   * doc and `applyPendingModeIfHostJustResolved()` below, the only place
   * this is read.
   */
  private pendingMode: SessionMatchMode | undefined;

  constructor(
    viewport: { readonly w: number; readonly h: number },
    onAction: ((action: OnlineLobbyAction) => void) | undefined,
    options: OnlineLobbyOptions<TStar, TEvent>,
  ) {
    this.state = lobby.newState(viewport, options.modelPorts, {
      ...(options.modelOptions !== undefined ? { options: options.modelOptions } : {}),
    });
    this.onAction = onAction;
    this.clipboard = options.clipboard ?? { read: () => undefined, write: () => undefined };
    this.starFactory = options.starFactory;
    this.newLink = options.newLink;
    this.roomSignaling = options.roomSignaling;
    this.pendingMode = options.roomIntent === "host" ? options.mode : undefined;
    // Last, and only once every field above is set: `dispatch` runs effects,
    // and `run()` reads `starFactory`/`newLink`/`roomSignaling`.
    if (options.roomIntent !== undefined) {
      this.dispatch({ kind: "room_pick", role: options.roomIntent });
      if (options.roomIntent === "guest") {
        // `dispatchCommand` (`lobby.ts`) leaves `state.focus` exactly where
        // it was for a command dispatched through the "lobby" event kind --
        // only a focus-navigation event (a click, a controller move) ever
        // changes it. The click that would normally have selected "JOIN
        // WITH A ROOM CODE" never happened here, so the composer widget
        // `room_pick` just revealed is claimed explicitly instead, exactly
        // as the acceptance criteria for #597 ask: focused immediately, no
        // extra input to reach it.
        this.state = { ...this.state, focus: ROOM_CODE_ENTRY_WIDGET };
      }
    }
  }

  dispatch(command: LobbyCommand): void {
    const hadHostRole = this.state.model.role === "host";
    const [state, action] = lobby.update(this.state, { kind: "lobby", command });
    this.state = state;
    this.run(state.effects);
    if (action && this.onAction) {
      this.onAction(action);
    }
    this.applyPendingModeIfHostJustResolved(hadHostRole);
  }

  /**
   * The front door's chosen mode reaches the manifest the host proposes the
   * moment this peer FIRST becomes the host of a real coordinator -- not
   * only via `room_created` (the room-code path this option exists for),
   * but also via a manual "role" pick made after a preset room-hosting
   * attempt fails or is cancelled (round-2 council review, blocking
   * finding 1: `pendingMode` used to be read only in `dispatch()`'s
   * `room_created` branch, so that second path silently reverted to
   * `DEFAULT_MODE`). Both routes run `lobby_model.ts`'s `chooseRole`,
   * which is what actually creates the coordinator `setMode` requires --
   * so "role just became host" is the one condition that has to be
   * checked, regardless of which command produced it. `dispatch()` covers
   * `room_created`; `event()` covers a manual "role" click, which goes
   * through `lobby.ts`'s general click path, never `dispatch()`.
   */
  private applyPendingModeIfHostJustResolved(hadHostRole: boolean): void {
    if (hadHostRole || this.state.model.role !== "host" || this.pendingMode === undefined) {
      return;
    }
    const mode = this.pendingMode;
    this.pendingMode = undefined;
    this.dispatch({ kind: "mode", mode });
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
      } else if (effect.kind === "room_open_host") {
        // Close whatever was there first -- `roomPick`/`chooseRole`'s own
        // guards (round-2 council review, blocking finding 2) mean a
        // SECOND `room_open_*` should never reach here while the first is
        // still live, but this closes the gap unconditionally rather than
        // depending on that: an unclosed socket left behind by a replaced
        // `roomLink` is exactly the orphaned-connection defect blocking
        // finding 3 describes for `room_signaling_port.ts`'s own retries.
        this.closeRoomLink();
        this.roomLink = this.roomSignaling?.openHost();
      } else if (effect.kind === "room_open_guest") {
        this.closeRoomLink();
        this.roomLink = this.roomSignaling?.openGuest(effect.code);
      } else if (effect.kind === "room_send") {
        this.roomLink?.send({
          ...(effect.to !== undefined ? { to: effect.to } : {}),
          signal: effect.signal,
          ...(effect.slot !== undefined ? { slot: effect.slot } : {}),
        });
      } else if (effect.kind === "room_close") {
        this.closeRoomLink();
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
    if (this.roomLink) {
      for (const event of this.roomLink.poll()) {
        this.dispatch(roomCommandFor(event));
      }
    }
    this.accumulator += dt;
    while (this.accumulator >= TICK_SECONDS) {
      this.accumulator -= TICK_SECONDS;
      this.dispatch({ kind: "tick" });
    }
  }

  event(evt: LobbyScreenEvent): void {
    const hadHostRole = this.state.model.role === "host";
    const [state, action] = lobby.update(this.state, evt);
    this.state = state;
    this.run(state.effects);
    if (action && this.onAction) {
      this.onAction(action);
    }
    this.applyPendingModeIfHostJustResolved(hadHostRole);
  }

  draw(backend: GraphicsBackend): void {
    draw.layout(backend, lobby.layout(this.state), this.state.viewport, this.transition);
  }

  /** Closes and clears `roomLink`, if one is open -- shared by `run()`'s
   * `room_open_host`/`room_open_guest`/`room_close` handling and
   * `teardown()`, so there is exactly one place that can leave a room-code
   * socket open without this instance's own knowledge of it. */
  private closeRoomLink(): void {
    if (this.roomLink) {
      this.roomLink.close();
      this.roomLink = undefined;
    }
  }

  teardown(): void {
    if (this.link) {
      this.link.apply({ kind: "shutdown" });
      this.link = undefined;
    }
    this.closeRoomLink();
  }
}
