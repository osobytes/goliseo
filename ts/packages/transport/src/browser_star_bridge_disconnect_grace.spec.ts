// #612 part 3: `browser_star_bridge.ts`'s `onconnectionstatechange` used to
// terminalize a peer on the very first transient `connectionState ===
// "disconnected"` -- indistinguishable from an ICE blip (a dropped packet
// burst, a NAT rebinding) that recovers on its own within a second or two.
// The issue's own diagnosis: exactly the kind of hiccup the rAF-stall
// window at the start boundary (#612 parts 1/2) already made routine.
// `DISCONNECT_GRACE_MS` gives "disconnected" a window to self-heal before
// this bridge reports the peer lost; "failed"/"closed" stay immediate,
// since those never recover.
//
// Same fakery level as `browser_star_bridge.spec.ts` (a stubbed
// `RTCPeerConnection`, no real WebRTC, vitest's `node` environment), plus a
// fake `RTCDataChannel` good enough to reach `peer.state === "connected"`
// (`attachChannel`'s own `onopen` handling) -- `onconnectionstatechange`'s
// "disconnected" branch is gated on exactly that state.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { newGoliseoStarTransportBridge } from "./browser_star_bridge.ts";

const DISCONNECT_GRACE_MS = 3000;

class FakeDataChannel {
  binaryType = "";
  bufferedAmountLowThreshold = 0;
  bufferedAmount = 0;
  readyState = "open";
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onbufferedamountlow: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  readonly label: string;

  constructor(label: string) {
    this.label = label;
  }

  send(): void {}
  close(): void {}
}

class FakePeerConnection {
  iceGatheringState = "complete";
  connectionState = "new";
  localDescription: unknown = null;
  oniceconnectionstatechange: (() => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  ondatachannel: (() => void) | null = null;
  onicegatheringstatechange: (() => void) | null = null;
  readonly channels: FakeDataChannel[] = [];

  createDataChannel(label: string): RTCDataChannel {
    const channel = new FakeDataChannel(label);
    this.channels.push(channel);
    return channel as unknown as RTCDataChannel;
  }

  createOffer(): Promise<RTCSessionDescriptionInit> {
    return Promise.resolve({ type: "offer", sdp: "fake-sdp" });
  }

  setLocalDescription(): Promise<void> {
    return Promise.resolve();
  }

  close(): void {}

  /** Test helper: fire a `connectionState` transition through the real
   * `onconnectionstatechange` handler the bridge installed. */
  transitionTo(state: string): void {
    this.connectionState = state;
    this.onconnectionstatechange?.();
  }
}

/** Mirrors `browser_star_bridge.spec.ts`'s own `fakePeerConnectionCtor`
 * shape (a factory closing over an array the caller reads back), extended
 * to record the constructed INSTANCE (not just its config) so a case can
 * drive `transitionTo` on it afterward. */
function fakePeerConnectionCtor(pcs: FakePeerConnection[]): typeof RTCPeerConnection {
  class Ctor extends FakePeerConnection {
    constructor() {
      super();
      pcs.push(this);
    }
  }
  return Ctor as unknown as typeof RTCPeerConnection;
}

/** Opens a host-side peer link and connects both its data channels --
 * `attachChannel`'s `onopen` handling sets `peer.state = "connected"` only
 * once BOTH the "control" and "input" channels report open, matching a
 * real completed handshake. Returns the fake `RTCPeerConnection` so a case
 * can drive `transitionTo`. */
function openConnectedPeer(
  bridge: ReturnType<typeof newGoliseoStarTransportBridge>,
  pcs: FakePeerConnection[],
  peerId: string,
): FakePeerConnection {
  bridge.open_peer(peerId);
  const outcome = bridge.request_offer(peerId);
  expect(outcome).toBe("ok");
  const pc = pcs[pcs.length - 1] as FakePeerConnection;
  for (const channel of pc.channels) {
    channel.onopen?.();
  }
  return pc;
}

/** `peer.state` for one peer, read off `diagnostics()`'s `peer|...` line
 * (`browser_star_bridge.ts`'s `diagnostics()`: `["peer", id, slot, state,
 * ...]`) -- the same public surface a real caller has, never internal
 * module state. */
function peerState(
  bridge: ReturnType<typeof newGoliseoStarTransportBridge>,
  peerId: string,
): string | undefined {
  for (const line of bridge.diagnostics().split("\n")) {
    const fields = line.split("|");
    if (fields[0] === "peer" && fields[1] === peerId) {
      return fields[3];
    }
  }
  return undefined;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("browser star bridge: disconnect grace (#612 part 3)", () => {
  it("does not report loss on the instant connectionState reads 'disconnected'", () => {
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");
    expect(peerState(bridge, "guest_1")).toBe("connected");

    pc.transitionTo("disconnected");

    expect(peerState(bridge, "guest_1")).toBe("connected");
  });

  it("reports loss once 'disconnected' persists for the full grace period", () => {
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");

    pc.transitionTo("disconnected");
    vi.advanceTimersByTime(DISCONNECT_GRACE_MS - 1);
    expect(peerState(bridge, "guest_1")).toBe("connected");

    vi.advanceTimersByTime(1);
    expect(peerState(bridge, "guest_1")).toBe("disconnected");
  });

  it("never reports loss if the connection recovers before the grace elapses", () => {
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");

    pc.transitionTo("disconnected");
    vi.advanceTimersByTime(DISCONNECT_GRACE_MS / 2);
    pc.transitionTo("connected");
    // Advance well past the original grace window -- the cancelled timer
    // must never fire late and report a loss that already healed.
    vi.advanceTimersByTime(DISCONNECT_GRACE_MS * 2);

    expect(peerState(bridge, "guest_1")).toBe("connected");
  });

  it("a repeated 'disconnected' before the grace elapses does not restart or shorten the window", () => {
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");

    pc.transitionTo("disconnected");
    vi.advanceTimersByTime(DISCONNECT_GRACE_MS - 500);
    pc.transitionTo("disconnected"); // a second, redundant "still disconnected" callback
    // If this had restarted the timer, only `DISCONNECT_GRACE_MS - 500` of
    // the ORIGINAL grace would have elapsed by now, not the full window.
    vi.advanceTimersByTime(500);

    expect(peerState(bridge, "guest_1")).toBe("disconnected");
  });

  it("reports 'failed' immediately, with no grace", () => {
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");

    pc.transitionTo("failed");

    expect(peerState(bridge, "guest_1")).toBe("disconnected");
  });

  it("reports 'closed' immediately, with no grace", () => {
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");

    pc.transitionTo("closed");

    expect(peerState(bridge, "guest_1")).toBe("disconnected");
  });

  it("a pending grace timer is actually cancelled (not merely rendered harmless) when the peer is closed outright", () => {
    // As originally written, this case only asserted `peer.state` stayed
    // "closed" across the grace window -- which the timer callback's OWN
    // `peer.state === "connected"` re-check would ALSO produce even with
    // NO cancellation at all (closed is never "connected"), so it passed
    // whether or not `closePeer` actually cleared the timer. Spying on
    // `setTimeout`/`clearTimeout` distinguishes "the cancel call happened,
    // synchronously, as an effect of `closePeer`" from "no report ever
    // surfaced, for an unrelated reason" -- the bug this case used to miss.
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const setTimeoutSpy = vi.spyOn(globalThis, "setTimeout");
    const clearTimeoutSpy = vi.spyOn(globalThis, "clearTimeout");
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");

    setTimeoutSpy.mockClear(); // ignore any timers armed while connecting
    pc.transitionTo("disconnected");
    expect(setTimeoutSpy).toHaveBeenCalledTimes(1);
    const graceHandle: unknown = setTimeoutSpy.mock.results[0]?.value;

    const outcome = bridge.close_peer("guest_1", "left the lobby");
    expect(outcome).toBe("ok");

    // The cancel is a direct, synchronous effect of `close_peer` -- proven
    // BEFORE any timer is ever advanced, so a version that merely relies on
    // the callback's own state re-check (and never calls `clearTimeout` at
    // all) fails this assertion outright.
    expect(clearTimeoutSpy).toHaveBeenCalledWith(graceHandle);
    expect(peerState(bridge, "guest_1")).toBe("closed");

    // Advancing past the grace window afterward must not throw or change
    // anything further -- the cancelled timer never fires at all.
    expect(() => vi.advanceTimersByTime(DISCONNECT_GRACE_MS * 2)).not.toThrow();
    expect(peerState(bridge, "guest_1")).toBe("closed");
  });

  it("a wire sent and received while a peer's disconnect grace is pending is still delivered", () => {
    // The bridge's own state machine still considers the peer "connected"
    // throughout the grace window (only the DELAYED report, if the grace
    // elapses, flips that) -- `enqueue`'s `peer.state !== "connected"`
    // guard and `receive`'s lack of any state check at all mean traffic
    // was never blocked during a pending grace. Pinning that here, not
    // just arguing it from the source.
    const pcs: FakePeerConnection[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(pcs));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    const pc = openConnectedPeer(bridge, pcs, "guest_1");
    const controlChannel = pc.channels.find((channel) => channel.label === "control");
    if (!controlChannel) {
      throw new Error("expected a control channel to have been attached");
    }

    pc.transitionTo("disconnected");
    vi.advanceTimersByTime(DISCONNECT_GRACE_MS / 2); // still within the grace window
    expect(peerState(bridge, "guest_1")).toBe("connected");

    // Send: addressed to the peer, over the control channel.
    const outboundWire = ["1", "event", "1", "", "outbound"].join("|");
    const sendOutcome = bridge.send(["guest_1", "control", outboundWire].join("|"));
    expect(sendOutcome).toBe("ok");

    // Receive: a real inbound message arriving over the same channel while
    // the grace is still pending.
    const inboundWire = ["1", "event", "1", "", "inbound"].join("|");
    controlChannel.onmessage?.({ data: inboundWire } as MessageEvent);
    expect(bridge.poll()).toBe(["guest_1", "control", inboundWire].join("|"));

    // The grace itself is untouched by any of this -- it still elapses and
    // reports loss on schedule if the connection never recovers.
    vi.advanceTimersByTime(DISCONNECT_GRACE_MS / 2);
    expect(peerState(bridge, "guest_1")).toBe("disconnected");
  });
});
