// WebRTC and WebSocket transports.
//
// This module is the package's facade: it re-exports the transport
// contract and every implementation, and provides the same factory
// functions `game/transport.lua` exposed to the rest of the Lua codebase
// (`transport.fake()`, `transport.browser_star()`, ...). `game/transport.lua`
// has no dedicated file of its own in this port — it *is* this facade.

export * from "./contract.ts";

export { FakeTransport, type FakeTransportOptions } from "./fake.ts";
export { BrowserTransport, type BrowserTransportOptions, type EvalFn } from "./browser.ts";
export {
  FakeStarTransport,
  type FakeStarTransportOptions,
  type FakeStarRendezvous,
  newRendezvous as newFakeStarRendezvous,
} from "./fake_star.ts";
export {
  FakeRelayTransport,
  type FakeRelayTransportOptions,
  type FakeRelayRoom,
  type FakeRelayRoomCounters,
  type FakeRelayWireCounters,
  newRoom as newFakeRelayRoom,
} from "./fake_relay.ts";
export {
  BrowserStarTransport,
  type BrowserStarTransportOptions,
  type StarEvalFn,
} from "./browser_star.ts";

import { FakeTransport, type FakeTransportOptions } from "./fake.ts";
import { BrowserTransport, type BrowserTransportOptions } from "./browser.ts";
import {
  FakeStarTransport,
  type FakeStarTransportOptions,
  type FakeStarRendezvous,
  newRendezvous,
} from "./fake_star.ts";
import {
  FakeRelayTransport,
  type FakeRelayTransportOptions,
  type FakeRelayRoom,
  newRoom,
} from "./fake_relay.ts";
import { BrowserStarTransport, type BrowserStarTransportOptions } from "./browser_star.ts";

export function fake(options?: FakeTransportOptions): FakeTransport {
  return new FakeTransport(options);
}

export function browser(options?: BrowserTransportOptions): BrowserTransport {
  return new BrowserTransport(options);
}

// Host-star adapters: one host endpoint with up to seven independently
// addressed guest links. The fake star is pure and in-process; the browser
// star drives real peer connections behind the JavaScript bridge.
export function fakeStar(options?: FakeStarTransportOptions): FakeStarTransport {
  return new FakeStarTransport(options);
}

// One rendezvous per logical star. Pass the same one to a host and its
// guests to let them complete a manual handshake in process; endpoints that
// were not handed it cannot see each other's signals.
export function fakeStarRendezvous(): FakeStarRendezvous {
  return newRendezvous();
}

export function browserStar(options?: BrowserStarTransportOptions): BrowserStarTransport {
  return new BrowserStarTransport(options);
}

// Relay adapter: every member holds one link to a room that forwards opaque
// payloads and parses nothing. No member is privileged, so a `broadcast`
// costs one upload rather than one per recipient. In-process only; it
// exists to measure the OMP-4 relay topology in the fault harness.
export function fakeRelay(options: FakeRelayTransportOptions): FakeRelayTransport {
  return new FakeRelayTransport(options);
}

// One room per logical relay session. Pass the same one to every member; an
// endpoint handed a different room cannot see them at all.
export function fakeRelayRoom(): FakeRelayRoom {
  return newRoom();
}
