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

import { describe, expect, it, vi } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { fakeStar, fakeStarRendezvous, type StarTransportAdapter } from "@gc/transport";
import type { LobbyScreenState } from "@gc/screens";
import type { GraphicsBackend } from "@gc/ui";
import { createOnlinePorts, realMatchDriverPort, type OnlinePortsDeps } from "./online_ports.ts";
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
  /** `OnlineLobby.link` -- opaque outside this package's own use, but
   * exactly what `app.ts`'s `App.startOnlineMatch` reads off the mounted
   * lobby screen and forwards to `OnlinePorts.newOnlineMatchScreen`. */
  readonly link: unknown;
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

/** A `fakeStar()`-produced transport, typed as both the real
 * `StarTransportAdapter` contract (`closePeer`, `peerIds`, ... -- what
 * `realMatchDriverPort` and the direct-disconnect case below need) and its
 * `FakeStarTransport`-only `pump()` test seam. */
type TestStar = StarTransportAdapter & Pumpable;

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

interface HandshakeAction {
  readonly go: string;
  readonly [key: string]: unknown;
}

interface TwoPeerHandshake {
  readonly host: DispatchableLobby;
  readonly guest: DispatchableLobby;
  readonly hostActions: HandshakeAction[];
  readonly guestActions: HandshakeAction[];
  readonly stars: readonly TestStar[];
  readonly hostPorts: ReturnType<typeof newTestOnlinePorts>;
  readonly guestPorts: ReturnType<typeof newTestOnlinePorts>;
}

/**
 * Runs the full manual-signaling handshake -- role, mode, invite, offer,
 * paste, answer, paste, connect, admission, manifest proposal, slot
 * assignment, readiness -- to a real, ready, two-human 1v1 session, two
 * independent `createOnlinePorts` instances (mirroring two separate
 * browser tabs) sharing one `fakeStarRendezvous()`. Offer/answer are
 * relayed by reading each side's own `model.outgoing` blob and pasting it
 * into the other -- exactly what a human copy/pasting between two chat
 * windows does, minus the OS clipboard.
 *
 * This is the shared setup every case below continues from (lobby-only
 * assertions, the match-phase continuation, and the direct
 * `MatchDriverPort` coverage) -- written once so it is proven once. It
 * deliberately stops short of "start": callers that need a synchronized
 * start dispatch `{ kind: "start" }` on the host themselves, since not
 * every case needs to reach the match.
 */
function runTwoPeerHandshake(): TwoPeerHandshake {
  const rendezvous = fakeStarRendezvous();
  const stars: TestStar[] = [];
  // `FakeStarTransport`'s own manual-signaling test seam identifies a
  // guest endpoint by its `peer_id` option (defaulting to `"guest"`, which
  // does not match the host's own `"guest_1"` link id) -- `peerId` here is
  // exactly what `lobby_model.ts`'s `open_star` effect already carries
  // (the coordinator identity `chooseRole` assigned), so this is a real,
  // available parameter, not an invented one. The real `BrowserStarTransport`
  // (`browser_star.ts`) has no such option at all -- guest identity lives
  // entirely in the link the host opens, never in the transport's own
  // constructor -- so this is specific to driving `FakeStarTransport`
  // correctly, not a production wiring detail.
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
  const hostActions: HandshakeAction[] = [];
  const guestActions: HandshakeAction[] = [];
  const host = newDispatchableLobby(hostPorts, (a) => hostActions.push(a));
  const guest = newDispatchableLobby(guestPorts, (a) => guestActions.push(a));

  host.dispatch({ kind: "role", role: "host" });
  host.dispatch({ kind: "mode", mode: "1v1" });
  host.dispatch({ kind: "invite" });
  pump([host], stars);
  const offer = host.state.model.outgoing;
  if (offer === undefined) {
    throw new Error("the host must produce an offer blob after inviting");
  }

  guest.dispatch({ kind: "role", role: "guest" });
  guest.dispatch({ kind: "paste", text: offer });
  pump([host, guest], stars);
  const answer = guest.state.model.outgoing;
  if (answer === undefined) {
    throw new Error("the guest must produce an answer blob after accepting the offer");
  }

  host.dispatch({ kind: "paste", text: answer });
  pump([host, guest], stars, 40);

  // The host proposes the manifest and publishes ownership -- no
  // `bot_fill` needed, both slots are real, connected humans.
  host.dispatch({ kind: "lock" });
  pump([host, guest], stars, 40);

  host.dispatch({ kind: "ready", ready: true });
  guest.dispatch({ kind: "ready", ready: true });
  pump([host, guest], stars, 40);

  return { host, guest, hostActions, guestActions, stars, hostPorts, guestPorts };
}

function requireDefined<T>(value: T | undefined, message: string): T {
  if (value === undefined) {
    throw new Error(message);
  }
  return value;
}

describe("online_ports: a real two-peer manual-signaling handshake", () => {
  it("host and guest reach a real, connected, bot-free 1v1 session through independent OnlinePorts", () => {
    const { host, guest } = runTwoPeerHandshake();

    // Both peers are admitted -- a real, two-human session, negotiated
    // through the real `LobbyLink`/`fakeStar` transport between two
    // independent `@gc/wasm` `Coordinator`s.
    expect(host.state.model.coordinator?.role).toBe("host");
    expect(guest.state.model.coordinator?.role).toBe("guest");
    expect(host.state.model.coordinator?.peers.length).toBe(2);
    expect(host.state.model.coordinator?.phase).toBe("ready");
    expect(guest.state.model.coordinator?.phase).toBe("ready");
  });

  it("reaches a synchronized start once the host begins the countdown", () => {
    const { host, guest, stars } = runTwoPeerHandshake();

    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);
    expect(host.state.model.started).toBe(true);
    expect(guest.state.model.started).toBe(true);
  });
});

describe("online_ports: the match-phase continuation, through OnlinePorts.requestMatchSession/newOnlineMatchScreen", () => {
  it("both sides build a real online match screen and tick it without throwing, over the real star", () => {
    const { host, guest, hostActions, guestActions, stars, hostPorts, guestPorts } =
      runTwoPeerHandshake();

    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);
    expect(host.state.model.started).toBe(true);
    expect(guest.state.model.started).toBe(true);

    // The exact seam `app.ts`'s `App.startOnlineMatch` uses: it reads the
    // mounted lobby's `state.model.coordinator`/`.link` and the lobby's own
    // reported `{ go: "online_match", freeze }` action, then calls
    // `OnlinePorts.requestMatchSession` followed by `.newOnlineMatchScreen`.
    // Driving those two calls directly (rather than also reconstructing
    // `App`'s title -> lobby widget-click routing, already covered by
    // `lobby_flow.spec.ts`/`online_match_flow.spec.ts` against a fake
    // `OnlinePorts`) is what actually isolates THIS module's match-screen
    // construction as the thing under test.
    const hostStartAction = requireDefined(
      hostActions.find((a) => a.go === "online_match"),
      "the host must report a synchronized start action",
    );
    const guestStartAction = requireDefined(
      guestActions.find((a) => a.go === "online_match"),
      "the guest must report a synchronized start action",
    );

    const hostRequested = hostPorts.requestMatchSession({
      role: "host",
      peerId: host.state.model.peer_id,
      manifest: host.state.model.coordinator?.manifest,
      freeze: hostStartAction.freeze,
    });
    const guestRequested = guestPorts.requestMatchSession({
      role: "guest",
      peerId: guest.state.model.peer_id,
      manifest: guest.state.model.coordinator?.manifest,
      freeze: guestStartAction.freeze,
    });
    expect(hostRequested.ok).toBe(true);
    expect(guestRequested.ok).toBe(true);
    if (!hostRequested.ok || !guestRequested.ok) {
      return;
    }

    interface TestMatchScreen {
      update(dt: number): void;
      readonly match: { readonly state: { readonly players: readonly { readonly id: string }[] } };
    }

    const hostMatchScreen = hostPorts.newOnlineMatchScreen({
      request: hostRequested.value,
      coordinator: host.state.model.coordinator,
      link: host.link,
      onAction: (a: HandshakeAction) => hostActions.push(a),
    }) as unknown as TestMatchScreen;
    const guestMatchScreen = guestPorts.newOnlineMatchScreen({
      request: guestRequested.value,
      coordinator: guest.state.model.coordinator,
      link: guest.link,
      onAction: (a: HandshakeAction) => guestActions.push(a),
    }) as unknown as TestMatchScreen;

    expect(() => {
      for (let i = 0; i < 10; i += 1) {
        for (const star of stars) {
          star.pump();
        }
        hostMatchScreen.update(1 / 60);
        guestMatchScreen.update(1 / 60);
      }
    }).not.toThrow();

    // A manifest-sized (10-player: 5 per team, keeper included -- the
    // manifest's own `teams[].roster`, not the 8-entry `slots` list, which
    // excludes keepers), real roster reached the match screen through the
    // real driver, on both sides -- not an empty stub.
    expect(hostMatchScreen.match.state.players.length).toBe(10);
    expect(guestMatchScreen.match.state.players.length).toBe(10);
  });
});

// ---------------------------------------------------------------------------
// Direct `MatchDriverPort` coverage -- `realMatchDriverPort` and its
// `drainStarIntoBridge`/`drainBridgeIntoStar` glue had zero automated
// coverage before this: every existing match-phase spec elsewhere in this
// tree (`online_match_flow.spec.ts` and friends) drives a hand-rolled fake
// `MatchDriverPort`, never this production one. These cases call `.create`/
// `.advance`/`.roster`/`.frame` directly (bypassing `MatchScreen`/`OnlineMatch`
// entirely) so the assertions are about the port itself, not about
// whatever `MatchScreen` happens to do with its output.
// ---------------------------------------------------------------------------

describe("online_ports: realMatchDriverPort, directly", () => {
  it("produces a real roster, a real frame, and well-shaped checkpoints on both sides", () => {
    const { host, guest, stars } = runTwoPeerHandshake();
    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);

    const hostCoordinator = requireDefined(
      host.state.model.coordinator,
      "the host must have a coordinator by the time it starts",
    );
    const guestCoordinator = requireDefined(
      guest.state.model.coordinator,
      "the guest must have a coordinator by the time it starts",
    );
    const manifest = requireDefined(hostCoordinator.manifest, "the host must have a manifest");
    const guestManifest = requireDefined(
      guestCoordinator.manifest,
      "the guest must have a manifest",
    );
    // The freeze both sides agree on is carried on the coordinator's own
    // state once `begin_countdown` runs (`gc_netcode::coordinator::Freeze`,
    // `stateFromHandle`'s own JSON round-trip) -- read directly rather than
    // through a lobby action, since this describe block is testing the
    // match driver, not the lobby's action-reporting shape.
    const hostFreeze = requireDefined(
      (hostCoordinator as unknown as { readonly freeze?: unknown }).freeze,
      "the host coordinator must carry a freeze once countdown began",
    );
    const guestFreeze = requireDefined(
      (guestCoordinator as unknown as { readonly freeze?: unknown }).freeze,
      "the guest coordinator must carry a freeze once countdown began",
    );

    const wasm = nodeWasmHost();
    const hostStar = requireDefined(stars[0], "the host star must have been created");
    const guestStar = requireDefined(stars[1], "the guest star must have been created");

    const hostDriverPort = realMatchDriverPort(wasm, hostStar);
    const guestDriverPort = realMatchDriverPort(wasm, guestStar);
    const hostDriver = hostDriverPort.create({
      role: "host",
      peer_id: "host",
      freeze: hostFreeze,
      manifest,
      transport: undefined,
      initial_snapshot: undefined,
    });
    const guestDriver = guestDriverPort.create({
      role: "guest",
      peer_id: "guest_1",
      freeze: guestFreeze,
      manifest: guestManifest,
      transport: undefined,
      initial_snapshot: undefined,
    });

    const sample = { move_x: 0, move_y: 0, held: 0, edges: 0 };
    let hostBatch;
    let guestBatch;
    for (let i = 0; i < 5; i += 1) {
      for (const star of stars) {
        star.pump();
      }
      hostBatch = hostDriverPort.advance(hostDriver, sample);
      guestBatch = guestDriverPort.advance(guestDriver, sample);
    }

    expect(hostDriverPort.status(hostDriver)).toBe("active");
    expect(guestDriverPort.status(guestDriver)).toBe("active");

    // Roster: real, manifest-sized identity (5 players per team, keeper
    // included -- the manifest's own `teams[].roster`, not the 8-entry
    // `slots` list, which excludes keepers) -- not an empty placeholder.
    const hostRoster = hostDriverPort.roster(hostDriver) as unknown as {
      readonly ids: readonly string[];
      readonly teams: readonly string[];
    };
    expect(hostRoster.ids.length).toBe(10);
    expect(hostRoster.teams.length).toBe(10);

    // Frame: a real decoded frame (`@gc/render`'s `frameBuffer.decode`),
    // carrying live per-player positions -- not a raw undecoded wire and
    // not a stub. This is exactly the shape `MatchScreen.onlineFrame`
    // requires (`players`/`control` present); see `frame()`'s own comment
    // in `online_ports.ts` for the bug this assertion would have caught.
    const hostFrame = hostDriverPort.frame(hostDriver) as unknown as {
      readonly players: { readonly count: number };
      readonly control: { readonly controlled: number };
    };
    expect(hostFrame.players.count).toBeGreaterThan(0);
    expect(typeof hostFrame.control.controlled).toBe("number");

    // Checkpoints/live: well-shaped batch output from the real bridge.
    expect(
      Array.isArray(hostDriverPort.batchCheckpoints(requireDefined(hostBatch, "unreachable"))),
    ).toBe(true);
    expect(typeof hostDriverPort.batchLive(requireDefined(hostBatch, "unreachable"))).toBe(
      "object",
    );
    expect(
      Array.isArray(guestDriverPort.batchCheckpoints(requireDefined(guestBatch, "unreachable"))),
    ).toBe(true);

    hostDriverPort.dispose(hostDriver);
    guestDriverPort.dispose(guestDriver);
  });

  it("reports a star-level error instead of silently discarding it", () => {
    // `drainStarIntoBridge` used to only read `peer_state` events off the
    // star, silently dropping `star_error`/`peer_error`/`star_state` --
    // this proves the report-instead-of-discard fix (`reportTransportEvent`),
    // mirroring the dev harness's own `poll_event` loop
    // (`tools/browser_online_match/web/online_peer.ts`), which treats these
    // as worth surfacing too.
    const { host, guest, stars } = runTwoPeerHandshake();
    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);

    const hostCoordinator = requireDefined(
      host.state.model.coordinator,
      "the host must have a coordinator",
    );
    const manifest = requireDefined(hostCoordinator.manifest, "the host must have a manifest");
    const hostFreeze = requireDefined(
      (hostCoordinator as unknown as { readonly freeze?: unknown }).freeze,
      "the host coordinator must carry a freeze once countdown began",
    );
    const hostStar = requireDefined(stars[0], "the host star must have been created");

    const hostDriverPort = realMatchDriverPort(nodeWasmHost(), hostStar);
    const hostDriver = hostDriverPort.create({
      role: "host",
      peer_id: "host",
      freeze: hostFreeze,
      manifest,
      transport: undefined,
      initial_snapshot: undefined,
    });

    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    // A `send` to a peer this star never opened is a `star_error`
    // ("unknown_peer") queued on the star's own event list -- a real
    // transport-level fault, not a protocol message.
    hostStar.send("a_peer_this_star_never_opened", "input", {
      version: 1,
      type: "input",
      seq: 0,
      tick: 0,
      payload: "x",
    });
    hostDriverPort.advance(hostDriver, { move_x: 0, move_y: 0, held: 0, edges: 0 });

    expect(consoleError).toHaveBeenCalled();
    expect(String(consoleError.mock.calls[0]?.[0])).toContain("star_error");

    consoleError.mockRestore();
    hostDriverPort.dispose(hostDriver);
  });

  it("observes a mid-match peer disconnect via MatchDriverBridge.setPeerDisconnected", () => {
    const { host, guest, stars } = runTwoPeerHandshake();
    host.dispatch({ kind: "start" });
    pump([host, guest], stars, 250);

    const hostCoordinator = requireDefined(
      host.state.model.coordinator,
      "the host must have a coordinator",
    );
    const guestCoordinator = requireDefined(
      guest.state.model.coordinator,
      "the guest must have a coordinator",
    );
    const manifest = requireDefined(hostCoordinator.manifest, "the host must have a manifest");
    const guestManifest = requireDefined(
      guestCoordinator.manifest,
      "the guest must have a manifest",
    );
    const hostFreeze = requireDefined(
      (hostCoordinator as unknown as { readonly freeze?: unknown }).freeze,
      "the host coordinator must carry a freeze once countdown began",
    );
    const guestFreeze = requireDefined(
      (guestCoordinator as unknown as { readonly freeze?: unknown }).freeze,
      "the guest coordinator must carry a freeze once countdown began",
    );

    const wasm = nodeWasmHost();
    const hostStar = requireDefined(stars[0], "the host star must have been created");
    const guestStar = requireDefined(stars[1], "the guest star must have been created");

    const hostDriverPort = realMatchDriverPort(wasm, hostStar);
    const guestDriverPort = realMatchDriverPort(wasm, guestStar);
    const hostDriver = hostDriverPort.create({
      role: "host",
      peer_id: "host",
      freeze: hostFreeze,
      manifest,
      transport: undefined,
      initial_snapshot: undefined,
    });
    const guestDriver = guestDriverPort.create({
      role: "guest",
      peer_id: "guest_1",
      freeze: guestFreeze,
      manifest: guestManifest,
      transport: undefined,
      initial_snapshot: undefined,
    });

    const sample = { move_x: 0, move_y: 0, held: 0, edges: 0 };
    const tick = (): void => {
      for (const star of stars) {
        star.pump();
      }
      hostDriverPort.advance(hostDriver, sample);
      guestDriverPort.advance(guestDriver, sample);
    };
    for (let i = 0; i < 5; i += 1) {
      tick();
    }

    // Spy on the GUEST's own bridge -- the host is the side that closes the
    // link below, so the guest is the side that must observe the drop.
    const setPeerDisconnected = vi.spyOn(
      guestDriver.bridge as unknown as {
        setPeerDisconnected: (peerId: string, detail: string) => void;
      },
      "setPeerDisconnected",
    );

    const closed = hostStar.closePeer("guest_1", "test disconnect");
    expect(closed.ok).toBe(true);
    // One more tick so `drainStarIntoBridge` polls the guest's own star for
    // the `peer_state: disconnected` event `closePeer` queues on the linked
    // remote (`fake_star.ts`'s own cross-reference -- see this file's own
    // header on why closing one side updates the other's peer state too).
    tick();

    expect(setPeerDisconnected).toHaveBeenCalledWith("host", expect.any(String));

    hostDriverPort.dispose(hostDriver);
    guestDriverPort.dispose(guestDriver);
  });
});
