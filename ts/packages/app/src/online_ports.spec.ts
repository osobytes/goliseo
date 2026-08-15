// Proves #550's production `OnlinePorts` wiring (`online_ports.ts`) for
// real, not against a hand-written fake: a real `@gc/wasm` `Coordinator`
// (via `loadSimHost()`, this package's own `@gc/wasm` devDependency -- the
// SAME split `browser_sim_host.ts`/`sim_host.ts` already establish between
// the Node- and browser-target loaders, see `online_wasm_host.ts`'s
// header), a real `@gc/screens` `OnlineLobby`, and a real `@gc/transport`
// star transport (`fakeStar` -- in-process, no DOM/WebRTC, but the SAME
// `StarTransportAdapter` contract `BrowserStarTransport` implements; only
// the peer-connection mechanics differ, not the framing/effect/event
// protocol this file's wiring drives).
//
// Three things this suite proves and one it does not:
//
// - `App.showLobby()` reaches the real online lobby screen without
//   throwing, through THIS module's `createOnlinePorts` -- the exact defect
//   this issue's title names ("the shipped build's online lobby is dead
//   code"), now closed. Mirrors `lobby_flow.spec.ts`'s "online lobby app
//   routing" case, but through the real wiring instead of a hand-written
//   `OnlinePorts` fake.
// - A real host, alone, reaches a locked, ready, bot-filled session through
//   the real coordinator -- proving `online_ports.ts`'s coordinator/manifest/
//   assignment/readiness wiring is genuinely driving `@gc/wasm`'s
//   `Coordinator`, not a local reimplementation of it.
// - A real host and a real, independently-constructed guest complete the
//   FULL manual-signaling flow end to end -- invite, offer, paste, answer,
//   paste, connect, admission, manifest proposal, slot assignment,
//   readiness -- entirely through this module's production wiring on both
//   sides, over the real `LobbyLink` framing and `FakeStarTransport`'s own
//   manual signaling (not `.link()`'s bypass test seam). This is the
//   closest a headless suite can come to the acceptance criterion "the
//   host/guest manual-signaling flow works between two browser contexts."
// - It does NOT prove real two-peer WebRTC NAT traversal (`fakeStar` is
//   in-process) -- that is `scripts/browser_*.py`'s tier-5 territory
//   (AGENTS.md §9), reported as a residual risk in this issue's PR rather
//   than claimed here.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar, fakeStarRendezvous, type StarTransportAdapter } from "@gc/transport";
import type { LobbyScreenState } from "@gc/screens";
import type { GraphicsBackend } from "@gc/ui";
import { createOnlinePorts, type OnlinePortsDeps } from "./online_ports.ts";
import type { OnlineWasmHost } from "./online_wasm_host.ts";
import { App, type OnlineLobbyScreen } from "./app.ts";
import { hit, menuLayout, viewport } from "./ui_bridge.ts";
import { APP_CONTENT, fakeKeyboard, noopRenderPort } from "./test_support/fixtures.ts";

// Mirrors `lobby.spec.ts`'s (`@gc/screens`) own `fakeGraphicsBackend` -- a
// headless `GraphicsBackend` stand-in, since no real implementation exists
// outside a real `<canvas>` (`canvas_graphics_backend.ts`).
function fakeGraphicsBackend(): GraphicsBackend {
  const noop = () => {};
  return {
    getDimensions: () => ({ width: 1280, height: 720 }),
    setColor: noop,
    setLineWidth: noop,
    rectangle: noop,
    circle: noop,
    ellipse: noop,
    polygon: noop,
    line: noop,
    print: noop,
    printf: noop,
    push: noop,
    pop: noop,
    translate: noop,
    scale: noop,
    setFont: noop,
  };
}

// Wraps `@gc/wasm`'s Node-target `loadSimHost()` as an `OnlineWasmHost` --
// this spec's counterpart of `browser_online_wasm_host.ts`'s browser-target
// wrapper, against the exact same interface (`online_wasm_host.ts`). Method
// names match 1:1 (both wasm-bindgen targets bind the identical
// `#[wasm_bindgen(js_name = ...)]`-annotated Rust source), so this is a
// structural cast, not a reimplementation.
function nodeWasmHost(): OnlineWasmHost {
  const sim = loadSimHost();
  return sim as unknown as OnlineWasmHost;
}

function clickWidget(app: App, id: string): void {
  const layout = menuLayout(app.stack.current());
  if (!layout) {
    throw new Error(`no menu layout on the current screen (looking for widget ${id})`);
  }
  const widget = hit.find(layout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  const [x, y] = viewport.toActual(
    app.transform,
    widget.rect.x + widget.rect.w / 2,
    widget.rect.y + widget.rect.h / 2,
  );
  app.event({ kind: "click", x, y, button: 1 });
}

function newTestOnlinePorts(starFactory: OnlinePortsDeps["starFactory"]) {
  return createOnlinePorts({
    wasm: nodeWasmHost(),
    starFactory,
    renderer: noopRenderPort,
    keyboard: fakeKeyboard(),
    content: APP_CONTENT.matchContract,
  });
}

describe("online_ports: production App wiring", () => {
  it("reaches the real online lobby from the title, through App, without throwing", () => {
    const rendezvous = fakeStarRendezvous();
    const onlinePorts = newTestOnlinePorts((role) => {
      const star = fakeStar({ role, rendezvous });
      const initialized = star.initialize();
      return initialized.ok ? star : undefined;
    });
    const app = new App(APP_CONTENT, { online: onlinePorts });
    expect(app.currentRoute()).toBe("title");

    clickWidget(app, "online_lobby");

    expect(app.currentRoute()).toBe("lobby");
    // A real `LobbyScreenState`, not a stub -- the model actually
    // constructed, at the "role" phase (neither host nor guest chosen yet).
    const lobby = app.stack.current() as OnlineLobbyScreen;
    expect(lobby.state.model.coordinator).toBeUndefined();

    app.event({ kind: "key", key: "escape" });
    expect(app.currentRoute()).toBe("title");
  });

  it("updates without throwing, and draws through the 2D GraphicsBackend browser_main.ts routes it to", () => {
    // `OnlineLobby` is not a `Menu`-wrapped screen, so `App.draw()`
    // (`Screen.draw?.()`, no-arg -- the same call every OTHER menu screen
    // never actually goes through in production either, since
    // `browser_main.ts`'s frame loop draws menu layouts directly) is not
    // the real contract here: `OnlineLobby.draw(backend)` takes an
    // explicit `GraphicsBackend`, which `browser_main.ts`'s "lobby" route
    // branch supplies (`menuBackend`). This proves the same thing that
    // branch relies on: the screen updates cleanly and draws through a
    // backend without throwing.
    const rendezvous = fakeStarRendezvous();
    const onlinePorts = newTestOnlinePorts((role) => {
      const star = fakeStar({ role, rendezvous });
      return star.initialize().ok ? star : undefined;
    });
    const app = new App(APP_CONTENT, { online: onlinePorts });
    clickWidget(app, "online_lobby");
    expect(() => {
      app.update(1 / 60);
    }).not.toThrow();
    const screen = app.stack.current() as unknown as { draw(backend: GraphicsBackend): void };
    expect(() => {
      screen.draw(fakeGraphicsBackend());
    }).not.toThrow();
  });
});

describe("online_ports: a real coordinator drives a solo host to ready", () => {
  it("locks a bot-filled 1v1 session and reaches readiness through the real @gc/wasm Coordinator", () => {
    const rendezvous = fakeStarRendezvous();
    const onlinePorts = newTestOnlinePorts((role: "host" | "guest") => {
      const star: StarTransportAdapter = fakeStar({ role, rendezvous });
      const initialized = star.initialize();
      return initialized.ok ? star : undefined;
    });

    const actions: { readonly go: string }[] = [];
    const lobbyScreen = onlinePorts.newLobbyScreen((action) => actions.push(action), {
      template: onlinePorts.matchManifestTemplate,
    }) as unknown as OnlineLobbyScreen & {
      dispatch(command: { readonly kind: string; readonly [key: string]: unknown }): void;
      state: LobbyScreenState;
    };

    lobbyScreen.dispatch({ kind: "role", role: "host" });
    lobbyScreen.dispatch({ kind: "mode", mode: "1v1" });
    lobbyScreen.dispatch({ kind: "bot_fill" });
    lobbyScreen.dispatch({ kind: "lock" });

    // Every step above round-tripped through the REAL `@gc/wasm`
    // `Coordinator` (`create`/`propose_manifest`/`assign_slots`) -- a fake
    // coordinator would never actually validate a manifest or an
    // assignment, so reaching "assigned" (or further) here is evidence the
    // real reducer accepted this module's wiring, not just that this
    // module's own code ran.
    const coordinator = lobbyScreen.state.model.coordinator;
    expect(coordinator).toBeDefined();
    expect(["assigned", "ready"]).toContain(coordinator?.phase);
    expect(coordinator?.role).toBe("host");

    lobbyScreen.dispatch({ kind: "ready", ready: true });
    const readyCoordinator = lobbyScreen.state.model.coordinator;
    expect(readyCoordinator?.phase).toBe("ready");
  });
});

// ---------------------------------------------------------------------------
// The two-peer manual-signaling handshake -- host and guest, two independent
// `createOnlinePorts` instances (mirroring two separate browser tabs), one
// shared `fakeStarRendezvous()` so their stars can see each other. Drives
// the offer/answer exchange by reading each side's own `model.outgoing`
// blob and pasting it into the other -- exactly what a human copy/pasting
// between two chat windows does, minus the OS clipboard. This is the
// strongest evidence this suite can offer for the acceptance criterion "the
// host/guest manual-signaling flow works between two browser contexts"
// short of a real two-browser run (`scripts/browser_*.py`'s tier-5
// territory).
// ---------------------------------------------------------------------------

interface DispatchableLobby {
  dispatch(command: { readonly kind: string; readonly [key: string]: unknown }): void;
  update(dt: number): void;
  readonly state: LobbyScreenState;
}

function newDispatchableLobby(
  onlinePorts: ReturnType<typeof newTestOnlinePorts>,
  onAction: (action: { readonly go: string }) => void,
): DispatchableLobby {
  return onlinePorts.newLobbyScreen(onAction, {
    template: onlinePorts.matchManifestTemplate,
  }) as unknown as DispatchableLobby;
}

/** `FakeStarTransport.pump()`'s own shape -- a test seam absent from
 * `StarTransportAdapter` (the real `BrowserStarTransport` delivers over an
 * actual data channel, driven by the browser's own event loop; nothing
 * analogous exists to call). "Delivers everything currently buffered on
 * this endpoint and on every endpoint linked to it" (that method's own
 * doc), so pumping either side of a link is enough to move traffic both
 * ways. */
interface Pumpable {
  pump(): void;
}

/** Pumps every star's own buffered delivery (`FakeStarTransport.pump()`)
 * and `update(dt)`s every lobby, `cycles` times -- the "test seam" delivery
 * step every `FakeStarTransport`-driven flow needs (`fake_star.ts`'s own
 * doc: the browser adapter has no equivalent because it drains its data
 * channels asynchronously; this is the in-process stand-in). */
function pump(
  lobbies: readonly DispatchableLobby[],
  stars: readonly Pumpable[],
  cycles = 10,
): void {
  for (let i = 0; i < cycles; i += 1) {
    for (const star of stars) {
      star.pump();
    }
    for (const lobby of lobbies) {
      lobby.update(1 / 60);
    }
  }
}

describe("online_ports: a real two-peer manual-signaling handshake", () => {
  it("host and guest reach a real, connected, bot-free 1v1 session through independent OnlinePorts", () => {
    const rendezvous = fakeStarRendezvous();
    const stars: Pumpable[] = [];
    // `FakeStarTransport`'s own manual-signaling test seam identifies a
    // guest endpoint by its `peer_id` option (defaulting to `"guest"`,
    // which does not match the host's own `"guest_1"` link id) -- `peerId`
    // here is exactly what `lobby_model.ts`'s `open_star` effect already
    // carries (the coordinator identity `chooseRole` assigned), so this is
    // a real, available parameter, not an invented one. The real
    // `BrowserStarTransport` (`browser_star.ts`) has no such option at all
    // -- guest identity lives entirely in the link the host opens, never in
    // the transport's own constructor -- so this is specific to driving
    // `FakeStarTransport` correctly, not a production wiring detail.
    const starFactory: OnlinePortsDeps["starFactory"] = (role, peerId) => {
      const star = fakeStar({ role, rendezvous, peer_id: peerId });
      if (!star.initialize().ok) {
        return undefined;
      }
      stars.push(star);
      return star;
    };
    const hostPorts = newTestOnlinePorts(starFactory);
    const guestPorts = newTestOnlinePorts(starFactory);
    const hostActions: { readonly go: string }[] = [];
    const guestActions: { readonly go: string }[] = [];
    const host = newDispatchableLobby(hostPorts, (a) => hostActions.push(a));
    const guest = newDispatchableLobby(guestPorts, (a) => guestActions.push(a));

    host.dispatch({ kind: "role", role: "host" });
    host.dispatch({ kind: "mode", mode: "1v1" });
    host.dispatch({ kind: "invite" });
    pump([host], stars);
    const offer = host.state.model.outgoing;
    expect(offer, "the host must produce an offer blob after inviting").toBeDefined();

    guest.dispatch({ kind: "role", role: "guest" });
    guest.dispatch({ kind: "paste", text: offer });
    pump([host, guest], stars);
    const answer = guest.state.model.outgoing;
    expect(answer, "the guest must produce an answer blob after accepting the offer").toBeDefined();

    host.dispatch({ kind: "paste", text: answer });
    pump([host, guest], stars, 40);

    // Both peers are admitted -- a real, two-human session, negotiated
    // through the real `LobbyLink`/`fakeStar` transport between two
    // independent `@gc/wasm` `Coordinator`s.
    expect(host.state.model.coordinator?.role).toBe("host");
    expect(guest.state.model.coordinator?.role).toBe("guest");
    expect(host.state.model.coordinator?.peers.length).toBe(2);

    // The host proposes the manifest and publishes ownership -- no
    // `bot_fill` needed, both slots are real, connected humans.
    host.dispatch({ kind: "lock" });
    pump([host, guest], stars, 40);

    const hostCoordinator = host.state.model.coordinator;
    const guestCoordinator = guest.state.model.coordinator;
    expect(["assigned", "ready"]).toContain(hostCoordinator?.phase);
    expect(["assigned", "ready"]).toContain(guestCoordinator?.phase);

    host.dispatch({ kind: "ready", ready: true });
    guest.dispatch({ kind: "ready", ready: true });
    pump([host, guest], stars, 40);
    expect(host.state.model.coordinator?.phase).toBe("ready");
    expect(guest.state.model.coordinator?.phase).toBe("ready");

    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);
    expect(host.state.model.started).toBe(true);
    expect(guest.state.model.started).toBe(true);
  });
});
