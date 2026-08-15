// Proves #550's ICE configuration seam: `StarBridgeOptions.iceConfig` is
// read fresh on every `RTCPeerConnection` construction (never snapshotted
// at install time), defaults to no ICE servers when omitted (this bridge's
// behavior before the option existed), and threads `iceTransportPolicy`
// through only when the caller actually sets one (#248's force-relay
// toggle, off by default -- `browser_star_bridge.ts:560`'s own former
// hardcoded `{ iceServers: [] }` is exactly the gap this closes).
//
// No real WebRTC here (this suite runs under vitest's `node` environment,
// with no DOM/WebRTC at all -- `ensurePeerConnection`'s own `globalThis`
// read is written for exactly that absence). `RTCPeerConnection` is stubbed
// on `globalThis` well enough to observe the `RTCConfiguration` this module
// constructs it with; everything past that (real ICE negotiation, real data
// channels) is `scripts/browser_*.py`'s tier-5 territory (AGENTS.md §9), not
// this file's.

import { afterEach, describe, expect, it, vi } from "vitest";
import { newGoliseoStarTransportBridge, type StarBridgeIceConfig } from "./browser_star_bridge.ts";

interface CapturedConfig {
  readonly iceServers: readonly RTCIceServer[];
  readonly iceTransportPolicy?: RTCIceTransportPolicy;
}

/** A `RTCPeerConnection` stand-in that records the configuration it was
 * constructed with and does just enough to let `request_offer`'s
 * synchronous prelude (data channel creation) run without throwing -- the
 * async offer/gather/signal chain after that is irrelevant to what this
 * file asserts (the config was already captured by then). */
function fakePeerConnectionCtor(captured: CapturedConfig[]): typeof RTCPeerConnection {
  class FakePeerConnection {
    iceGatheringState = "complete";
    localDescription: unknown = null;
    oniceconnectionstatechange: (() => void) | null = null;
    onconnectionstatechange: (() => void) | null = null;
    ondatachannel: (() => void) | null = null;
    onicegatheringstatechange: (() => void) | null = null;

    constructor(config: RTCConfiguration) {
      captured.push({
        iceServers: config.iceServers ?? [],
        ...(config.iceTransportPolicy !== undefined
          ? { iceTransportPolicy: config.iceTransportPolicy }
          : {}),
      });
    }

    createDataChannel(label: string): RTCDataChannel {
      return { label, binaryType: "arraybuffer" } as unknown as RTCDataChannel;
    }

    createOffer(): Promise<RTCSessionDescriptionInit> {
      return Promise.resolve({ type: "offer", sdp: "fake-sdp" });
    }

    setLocalDescription(): Promise<void> {
      return Promise.resolve();
    }
  }
  return FakePeerConnection as unknown as typeof RTCPeerConnection;
}

function openAndOffer(
  bridge: ReturnType<typeof newGoliseoStarTransportBridge>,
  peerId: string,
): void {
  bridge.open_peer(peerId);
  const outcome = bridge.request_offer(peerId);
  expect(outcome).toBe("ok");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("browser star bridge: ICE configuration (#550)", () => {
  it("defaults to no ICE servers and no forced policy when iceConfig is omitted", () => {
    const captured: CapturedConfig[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(captured));
    const bridge = newGoliseoStarTransportBridge();
    bridge.initialize("host", 64, 7, 65536);
    openAndOffer(bridge, "guest_1");

    expect(captured).toHaveLength(1);
    expect(captured[0]?.iceServers).toEqual([]);
    expect(captured[0]?.iceTransportPolicy).toBeUndefined();
  });

  it("constructs RTCPeerConnection with the configured STUN servers", () => {
    const captured: CapturedConfig[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(captured));
    const config: StarBridgeIceConfig = {
      iceServers: [
        { urls: "stun:stun.cloudflare.com:3478" },
        { urls: "stun:stun.l.google.com:19302" },
      ],
    };
    const bridge = newGoliseoStarTransportBridge({ iceConfig: () => config });
    bridge.initialize("host", 64, 7, 65536);
    openAndOffer(bridge, "guest_1");

    expect(captured[0]?.iceServers).toEqual(config.iceServers);
  });

  it("threads iceTransportPolicy through only when the caller sets it", () => {
    const captured: CapturedConfig[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(captured));
    const bridge = newGoliseoStarTransportBridge({
      iceConfig: () => ({ iceServers: [], iceTransportPolicy: "relay" }),
    });
    bridge.initialize("host", 64, 7, 65536);
    openAndOffer(bridge, "guest_1");

    expect(captured[0]?.iceTransportPolicy).toBe("relay");
  });

  it("leaves iceTransportPolicy unset (direct path unaffected) when the config omits it", () => {
    const captured: CapturedConfig[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(captured));
    const bridge = newGoliseoStarTransportBridge({
      iceConfig: () => ({ iceServers: [{ urls: "stun:stun.cloudflare.com:3478" }] }),
    });
    bridge.initialize("host", 64, 7, 65536);
    openAndOffer(bridge, "guest_1");

    expect(captured[0]?.iceTransportPolicy).toBeUndefined();
  });

  it("reads iceConfig fresh per connection, not snapshotted at construction", () => {
    const captured: CapturedConfig[] = [];
    vi.stubGlobal("RTCPeerConnection", fakePeerConnectionCtor(captured));
    let current: StarBridgeIceConfig = { iceServers: [{ urls: "stun:before.example.com:3478" }] };
    const bridge = newGoliseoStarTransportBridge({ iceConfig: () => current });
    bridge.initialize("host", 64, 7, 65536);
    bridge.open_peer("guest_1");
    bridge.open_peer("guest_2");

    openAndOffer(bridge, "guest_1");
    // A TURN credential fetch landing between the two connections -- the
    // FIRST peer connection must not retroactively change, and the SECOND
    // must pick up the new value.
    current = { iceServers: [{ urls: "stun:after.example.com:3478" }] };
    const secondOutcome = bridge.request_offer("guest_2");
    expect(secondOutcome).toBe("ok");

    expect(captured).toHaveLength(2);
    expect(captured[0]?.iceServers).toEqual([{ urls: "stun:before.example.com:3478" }]);
    expect(captured[1]?.iceServers).toEqual([{ urls: "stun:after.example.com:3478" }]);
  });
});
