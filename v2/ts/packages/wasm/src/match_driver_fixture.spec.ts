// Exercises `crates/gc-wasm/src/match_driver_fixture_bridge.rs`'s
// wasm-bindgen surface against the real compiled artifact, under node.
//
// Two things this file specifically proves, both orchestrator-flagged:
//
// 1. `matchDriverFixtureFreezeJson`/`matchDriverFixtureManifestJson`
//    together are a real `freezeJson`/`manifestJson` pair `MatchDriverBridge`'s
//    constructor accepts -- before this wave, nothing in `@gc/wasm` could
//    produce one at all (see `match_driver_fixture_bridge.rs`'s module doc).
// 2. A structurally-plausible-but-invalid `assignments` array, hand-built
//    the way a caller without this fixture would have to guess at one,
//    throws a normal, catchable error instead of trapping the wasm
//    instance ("RuntimeError: unreachable") the way it did before
//    `MatchDriverBridge::new` validated its input.
//
// Requires `pnpm --filter @gc/wasm build` to have run first.

import { describe, expect, it } from "vitest";

import { bytesFromByteString, byteStringFromBytes } from "./binary_string.ts";
import { loadSimHost } from "./index.ts";

function newSession(host: ReturnType<typeof loadSimHost>) {
  return new host.Session("nebula", "orion", 7, 20, 3);
}

// The bridge's own signature narrows `modeWire` to the three canonical
// match modes (matching `@gc/online`'s own `MatchMode` type) -- correct for
// a real caller, but these specs deliberately probe the runtime rejection
// of a mode outside that set, which needs an escape hatch from the static
// type.
const UNRECOGNIZED_MODE = "3v3" as unknown as "1v1";

describe("matchDriverFixture bridge: pure/JSON pieces", () => {
  it("constantsJson reports the fixture's own host peer id", () => {
    const host = loadSimHost();
    const constants = JSON.parse(host.matchDriverFixtureConstantsJson()) as { host_peer_id: string };
    expect(constants.host_peer_id).toBe("host");
  });

  it("guestPeerId matches the Lua naming", () => {
    const host = loadSimHost();
    expect(host.matchDriverFixtureGuestPeerId(1)).toBe("guest_1");
    expect(host.matchDriverFixtureGuestPeerId(3)).toBe("guest_3");
  });

  it("peerIds seats the host first, then guests in order", () => {
    const host = loadSimHost();
    expect(host.matchDriverFixturePeerIds("1v1")).toEqual(["host", "guest_1"]);
    const fourVFour = host.matchDriverFixturePeerIds("4v4");
    expect(fourVFour[0]).toBe("host");
    expect(fourVFour.length).toBe(8);
  });

  it("peerIds throws on an unrecognized match mode", () => {
    const host = loadSimHost();
    expect(() => host.matchDriverFixturePeerIds(UNRECOGNIZED_MODE)).toThrow();
  });

  it("freezeJson reports a contiguous owned block per human", () => {
    const host = loadSimHost();
    const freeze = JSON.parse(host.matchDriverFixtureFreezeJson("2v2")) as {
      owned: Record<string, string[]>;
    };
    expect(Object.keys(freeze.owned).length).toBe(4);
    for (const slots of Object.values(freeze.owned)) {
      expect(slots.length).toBe(2);
    }
  });

  it("freezeJson/manifestJson throw on an unrecognized match mode", () => {
    const host = loadSimHost();
    expect(() => host.matchDriverFixtureFreezeJson(UNRECOGNIZED_MODE)).toThrow();
    expect(() => host.matchDriverFixtureManifestJson(UNRECOGNIZED_MODE)).toThrow();
  });

  it("initialSnapshot returns a freeable opaque handle", () => {
    const host = loadSimHost();
    const snapshot = host.matchDriverFixtureInitialSnapshot();
    expect(typeof snapshot.free).toBe("function");
    snapshot.free();
  });
});

describe("matchDriverFixture bridge: closes the freezeJson/manifestJson gap", () => {
  it("a freeze/manifest pair built entirely from this bridge constructs a real MatchDriverBridge", () => {
    const host = loadSimHost();
    const session = newSession(host);
    try {
      const freezeJson = host.matchDriverFixtureFreezeJson("1v1");
      const manifestJson = host.matchDriverFixtureManifestJson("1v1");

      const bridge = new host.MatchDriverBridge(session, "host", "host", freezeJson, manifestJson, undefined);
      expect(JSON.parse(bridge.statusJson())).toBe("active");
    } finally {
      session.free();
    }
  });

  it("matchDriverFixtureSession's own freezeJson/manifestJson are the same pair", () => {
    const host = loadSimHost();
    const fixtureSession = host.matchDriverFixtureSession("1v1");
    try {
      const session = newSession(host);
      try {
        const bridge = new host.MatchDriverBridge(
          session,
          "host",
          fixtureSession.hostPeerId,
          fixtureSession.freezeJson(),
          fixtureSession.manifestJson(),
          undefined,
        );
        expect(JSON.parse(bridge.statusJson())).toBe("active");
      } finally {
        session.free();
      }
    } finally {
      fixtureSession.free();
    }
  });
});

describe("matchDriverFixture bridge: session and its in-process star", () => {
  it("session links every seated guest to the host, connected both ways", () => {
    const host = loadSimHost();
    const fixtureSession = host.matchDriverFixtureSession("2v2");
    try {
      const guestIds = fixtureSession.guestPeerIds();
      expect(guestIds.length).toBe(3);
      const hostTransport = fixtureSession.hostTransport();
      try {
        for (const guestId of guestIds) {
          expect(hostTransport.peerState(guestId)).toBe("connected");
          const guestTransport = fixtureSession.guestTransport(guestId);
          expect(guestTransport).toBeDefined();
          expect(guestTransport?.peerState("host")).toBe("connected");
          guestTransport?.free();
        }
      } finally {
        hostTransport.free();
      }
      expect(fixtureSession.guestTransport("nobody")).toBeUndefined();
    } finally {
      fixtureSession.free();
    }
  });

  it("send then pump then poll delivers a real byte payload across the link, via the binary-string convention", () => {
    const host = loadSimHost();
    const fixtureSession = host.matchDriverFixtureSession("1v1");
    try {
      const guestId = fixtureSession.guestPeerIds()[0] as string;
      const hostTransport = fixtureSession.hostTransport();
      const guestTransport = fixtureSession.guestTransport(guestId);
      expect(guestTransport).toBeDefined();
      try {
        // A payload with a byte outside the printable-ASCII range, so this
        // is a genuine test of byte-exact delivery, not just text that
        // happens to survive by accident.
        const originalBytes = new Uint8Array([0, 1, 2, 0xff, 0x80, 0x41]);
        const binaryString = byteStringFromBytes(originalBytes);
        const wireBytes = bytesFromByteString(binaryString);
        expect(wireBytes).toEqual(originalBytes);

        hostTransport.send(guestId, "input", "input", 0, 0, wireBytes);
        hostTransport.pump();

        const batch = JSON.parse(guestTransport?.pollBatchJson() ?? "[]") as Array<{
          peer_id: string;
          message: { payload_bytes: number[] };
        }>;
        expect(batch.length).toBe(1);
        expect(batch[0]?.peer_id).toBe("host");
        expect(new Uint8Array(batch[0]?.message.payload_bytes ?? [])).toEqual(originalBytes);
      } finally {
        guestTransport?.free();
        hostTransport.free();
      }
    } finally {
      fixtureSession.free();
    }
  });
});

describe("matchDriverFixture bridge: the constructor-panic regression", () => {
  it("a missing field is a clean, catchable error (baseline, already true before this wave)", () => {
    const host = loadSimHost();
    const session = newSession(host);
    try {
      expect(() => new host.MatchDriverBridge(session, "host", "host", "{}", "{}", undefined)).toThrow();
    } finally {
      session.free();
    }
  });

  it("a structurally plausible but invalid assignments array is rejected, not a wasm trap", () => {
    const host = loadSimHost();
    const session = newSession(host);
    try {
      const freeze = JSON.parse(host.matchDriverFixtureFreezeJson("1v1")) as {
        assignments: unknown[];
        [key: string]: unknown;
      };
      const manifestJson = host.matchDriverFixtureManifestJson("1v1");

      // Every entry looks like a plausible producer record -- the exact
      // "hand-filled every Freeze field and guessed at the assignments
      // shape" scenario the orchestrator described -- but `producer_kind`
      // is not one of the two canonical wire values ("peer"/"bot").
      const bogusAssignments = (freeze.assignments as Array<Record<string, unknown>>).map((entry, index) => ({
        ...entry,
        producer_kind: index === 0 ? "human" : entry.producer_kind,
      }));
      const malformedFreezeJson = JSON.stringify({ ...freeze, assignments: bogusAssignments });

      let caught: unknown;
      try {
        new host.MatchDriverBridge(session, "host", "host", malformedFreezeJson, manifestJson, undefined);
      } catch (error) {
        caught = error;
      }
      // The critical assertion: an ordinary, catchable JS error was thrown
      // -- proven by reaching this line at all. Before the fix, this
      // specific input trapped the wasm instance with "RuntimeError:
      // unreachable", which `vitest`/node surfaces very differently (an
      // uncaught, uncatchable module-level failure) and poisons every
      // later call into the same instance.
      expect(caught).toBeDefined();

      // And the instance is still usable afterward -- the defect report's
      // other half ("poisons the instance"). A well-formed construction
      // right after the rejected one must still succeed.
      const bridge = new host.MatchDriverBridge(
        session,
        "host",
        "host",
        host.matchDriverFixtureFreezeJson("1v1"),
        manifestJson,
        undefined,
      );
      expect(JSON.parse(bridge.statusJson())).toBe("active");
    } finally {
      session.free();
    }
  });

  it("an empty peer id is rejected, not the underlying assert! panic", () => {
    const host = loadSimHost();
    const session = newSession(host);
    try {
      const freezeJson = host.matchDriverFixtureFreezeJson("1v1");
      const manifestJson = host.matchDriverFixtureManifestJson("1v1");
      expect(() => new host.MatchDriverBridge(session, "host", "", freezeJson, manifestJson, undefined)).toThrow();
    } finally {
      session.free();
    }
  });
});
