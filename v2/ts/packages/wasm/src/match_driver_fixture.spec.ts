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

describe("matchDriverFixture bridge: MatchDriverBridge's queueLimit and transportDiagnosticsJson", () => {
  it("queueLimit defaults to 64 and is configurable, reaching the wrapped transport", () => {
    // Before `queueLimit` existed, `MatchDriverBridge`'s own internal
    // transport's inbound queue was permanently fixed at 64 with nothing on
    // this bridge's public surface able to raise it -- a burst wide enough
    // to exceed the ~30-tick rollback window overflowed the queue itself
    // first, so a genuine window-exceeded condition was unreachable through
    // the public API (see `crates/gc-wasm/src/match_driver_bridge.rs`'s
    // `MatchDriverBridge::new` doc).
    const host = loadSimHost();
    const freezeJson = host.matchDriverFixtureFreezeJson("1v1");
    const manifestJson = host.matchDriverFixtureManifestJson("1v1");
    const session = newSession(host);
    try {
      const defaultBridge = new host.MatchDriverBridge(
        session,
        "host",
        "host",
        freezeJson,
        manifestJson,
        undefined,
      );
      const defaultDiagnostics = JSON.parse(defaultBridge.transportDiagnosticsJson()) as {
        queue_limit: number;
      };
      expect(defaultDiagnostics.queue_limit).toBe(64);

      const raisedBridge = new host.MatchDriverBridge(
        session,
        "host",
        "host2",
        freezeJson,
        manifestJson,
        undefined,
        200,
      );
      const raisedDiagnostics = JSON.parse(raisedBridge.transportDiagnosticsJson()) as {
        queue_limit: number;
      };
      expect(raisedDiagnostics.queue_limit).toBe(200);

      raisedBridge.initializeTransport();
      raisedBridge.openPeer("guest_x");
      raisedBridge.setPeerConnected("guest_x");
      // Well past the old fixed cap of 64, with nothing ever draining them
      // (`advance` drains; this never calls it) -- the exact scenario the
      // defect made unreachable.
      for (let seq = 0; seq < 100; seq += 1) {
        expect(() =>
          raisedBridge.enqueueInbound("guest_x", "input", "input", seq, seq, new Uint8Array()),
        ).not.toThrow();
      }

      defaultBridge.initializeTransport();
      defaultBridge.openPeer("guest_y");
      defaultBridge.setPeerConnected("guest_y");
      let overflowedAt = -1;
      for (let seq = 0; seq < 100; seq += 1) {
        try {
          defaultBridge.enqueueInbound("guest_y", "input", "input", seq, seq, new Uint8Array());
        } catch {
          overflowedAt = seq;
          break;
        }
      }
      expect(overflowedAt).toBe(64);
    } finally {
      session.free();
    }
  });

  it("diagnosticsJson omits terminal/late_input_tick when absent, rather than nulling them", () => {
    // The exact JSON-boundary defect this closes: `terminal`,
    // `late_input_tick`, and `control_slot` used to always be present keys
    // (`null` when absent), which `JSON.parse` turns into `null`, not
    // `undefined` -- `@gc/online`'s `MatchDriverDiagnostics` declares them
    // TypeScript-optional (`field?: T`), and its `recordStep` checks
    // `!== undefined`, which a JSON `null` satisfies, crashing on
    // `terminal.status` the moment a step has no active terminal (every
    // step until one is reached).
    const host = loadSimHost();
    const freezeJson = host.matchDriverFixtureFreezeJson("1v1");
    const manifestJson = host.matchDriverFixtureManifestJson("1v1");
    const session = newSession(host);
    try {
      const bridge = new host.MatchDriverBridge(session, "host", "host", freezeJson, manifestJson, undefined);
      bridge.initializeTransport();
      bridge.openPeer("guest_1");
      bridge.setPeerConnected("guest_1");
      bridge.advance(host.inputFrameNeutralSample());

      const diagnostics = JSON.parse(bridge.diagnosticsJson()) as Record<string, unknown>;
      expect(Object.prototype.hasOwnProperty.call(diagnostics, "terminal")).toBe(false);
      expect(Object.prototype.hasOwnProperty.call(diagnostics, "late_input_tick")).toBe(false);
      // control_slot, by contrast, genuinely is present for a seated host.
      expect(Object.prototype.hasOwnProperty.call(diagnostics, "control_slot")).toBe(true);
      expect(typeof diagnostics.control_slot).toBe("string");
    } finally {
      session.free();
    }
  });
});

describe("matchDriverFixture bridge: MatchDriverBridge's observeCheckpoint", () => {
  it("agrees with its own published checkpoint hash, and forces a hash_mismatch on repeated disagreement", () => {
    // Before this method existed there was no way to force a hash
    // divergence deterministically from `@gc/wasm` at all -- confirmed
    // absent from `MatchDriverBridge`'s surface and `types.ts`.
    const host = loadSimHost();
    const freezeJson = host.matchDriverFixtureFreezeJson("1v1");
    const manifestJson = host.matchDriverFixtureManifestJson("1v1");
    const session = newSession(host);
    try {
      const bridge = new host.MatchDriverBridge(session, "host", "host", freezeJson, manifestJson, undefined);
      bridge.initializeTransport();
      bridge.openPeer("guest_1");
      bridge.setPeerConnected("guest_1");

      let checkpoint: { tick: number; hash: string } | undefined;
      for (let step = 0; step < 40 && checkpoint === undefined; step += 1) {
        const batch = JSON.parse(bridge.advance(host.inputFrameNeutralSample())) as {
          checkpoints: Array<{ tick: number; hash: string }>;
        };
        checkpoint = batch.checkpoints[0];
      }
      expect(checkpoint).toBeDefined();
      const { tick, hash } = checkpoint as { tick: number; hash: string };

      // Agreement: the driver's own hash, reported back to it.
      expect(bridge.observeCheckpoint(tick, hash)).toBe(true);
      expect(JSON.parse(bridge.diagnosticsJson()).hash_mismatches).toBe(0);

      // A tick this driver never published a checkpoint at all is treated
      // as agreement too (nothing to compare against yet).
      expect(bridge.observeCheckpoint(999_999, "deadbeef")).toBe(true);

      // Repeated disagreement on a real, published tick eventually
      // terminates the driver -- `gc_netcode::match_driver::MAX_HASH_MISMATCHES`
      // is 3.
      expect(bridge.observeCheckpoint(tick, "0000000000000000")).toBe(false);
      expect(bridge.observeCheckpoint(tick, "0000000000000000")).toBe(false);
      expect(bridge.observeCheckpoint(tick, "0000000000000000")).toBe(false);

      expect(JSON.parse(bridge.statusJson())).toBe("hash_mismatch");
      const terminal = JSON.parse(bridge.terminalJson()) as { status: string; failure?: string };
      expect(terminal.status).toBe("hash_mismatch");
      expect(terminal.failure).toBe("desync");
    } finally {
      session.free();
    }
  });
});

describe("matchDriverFixture bridge: MatchDriverBridge reaching full time (wasm-target regression)", () => {
  // This is the one case in this file that MUST run against the real
  // compiled wasm32 artifact to mean anything -- `crates/gc-wasm/src/
  // match_driver_bridge.rs`'s own native `mod tests` (`cargo test
  // --workspace`) cannot catch this defect at all: off wasm32,
  // `gc_netcode::match_driver::default_clock` calls
  // `std::time::SystemTime::now()`, which works fine natively and only
  // traps ("RuntimeError: unreachable", uncatchable the way a normal
  // `Result`/thrown `Error` is) on `wasm32-unknown-unknown`. That gap --
  // green native tests, a crash the instant a real online match reaches
  // full time in a browser -- is exactly how this defect survived; see
  // `crates/gc-wasm/src/match_driver_bridge.rs`'s `driver_settle_clock` for
  // the fix. This file already only runs against `dist/pkg/gc_wasm.cjs`
  // (the real compiled artifact, per this file's own header), so it is the
  // right home for the one case that specifically needs that to be true.
  it("does not trap the wasm instance when a match runs all the way to full time", () => {
    const host = loadSimHost();
    // `humans: 1` seats only the host as a live human ("1v1" mode's other
    // slot runs entirely bot-controlled) -- deliberately avoiding the need
    // for a second, real guest driver to confirm input. A lone host with an
    // opened-but-silent guest peer stalls on `confirmation_stalled` well
    // before full time (confirmed empirically while building this test:
    // status flips at driver step 30, `ROLLBACK_WINDOW_TICKS`), which would
    // never exercise the crash site this test exists to cover.
    const freezeJson = host.matchDriverFixtureFreezeJson("1v1", undefined, 1);
    const manifestJson = host.matchDriverFixtureManifestJson("1v1");
    const session = newSession(host);
    try {
      // A one-second match (60 ticks at the fixed 60 Hz driver rate) so
      // this test reaches full time -- and therefore the crash site,
      // `reach_full_time`'s call into the injected settle clock -- in a
      // handful of `advance()` calls rather than the fixture's own
      // multi-minute default duration.
      const shortSnapshot = host.matchDriverFixtureInitialSnapshot(1, false, 7);
      const bridge = new host.MatchDriverBridge(
        session,
        "host",
        "host",
        freezeJson,
        manifestJson,
        undefined,
        undefined,
        shortSnapshot,
      );
      bridge.initializeTransport();

      let status: string = "active";
      for (let step = 0; step < 200 && status === "active"; step += 1) {
        // The exact crash site: `advance` -> `MatchDriver::advance` ->
        // `reach_full_time` -> the injected settle clock. Before this
        // fix, every call once the match reached full time trapped the
        // wasm instance instead of returning or throwing a catchable
        // error.
        expect(() => bridge.advance(host.inputFrameNeutralSample())).not.toThrow();
        status = JSON.parse(bridge.statusJson()) as string;
      }

      expect(status, "a one-second match must reach a terminal status within 200 driver steps").not.toBe(
        "active",
      );
      // A wasm trap poisons the whole instance -- every subsequent call
      // into it fails too, not just the one that hit `unreachable`. Calling
      // back into the bridge after full time proves this was a real,
      // fully-recovered path, not merely a call that happened not to throw.
      expect(() => bridge.diagnosticsJson()).not.toThrow();
      expect(() => bridge.transportDiagnosticsJson()).not.toThrow();
    } finally {
      session.free();
    }
  });
});
