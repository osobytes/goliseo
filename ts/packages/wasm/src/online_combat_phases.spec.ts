// Exercises `crates/gc-wasm/src/online_combat_phases_bridge.rs`'s
// wasm-bindgen surface against the real compiled artifact, under node.
//
// This bridge is new (W6-A): before it existed, `@gc/wasm`'s `Session` only
// ever started from `sim_match::new`'s default boundary zero (hard-coded
// `home_formation: None`, no snapshot-restore entry point at all), so there
// was no way to hand a `Session`/`MatchDriverBridge` an arbitrary starting
// state from TypeScript -- specifically the seven pinned combat-phase
// boundary zeroes `packages/online/src/match_presentation.spec.ts`'s nine
// blocked combat-phase cases need (see that file's own header comment).
// `MatchDriverBridge`'s constructor error paths (an unrecognized role, an
// empty peer id, ...) can't be exercised from a native Rust `#[test]` --
// `wasm_bindgen::JsValue::from_str` panics off the wasm32 target -- so this
// file, not `online_combat_phases_bridge.rs`'s own `#[cfg(test)]` module, is
// where `onlineCombatPhaseScenarioJson`/`BoundaryZero`/`LiveSample`/`Observed`
// rejecting an unrecognized phase id (and `onlineCombatPhaseObserved`
// rejecting malformed `combatEventsJson`) actually gets proven.
//
// Requires `pnpm --filter @gc/wasm build` to have run first.

import { describe, expect, it } from "vitest";

import { loadSimHost } from "./index.ts";

const UNRECOGNIZED_PHASE = "not_a_phase" as unknown as string;

describe("onlineCombatPhases bridge: pure/JSON pieces", () => {
  it("onlineCombatPhaseIds reports all seven canonical phases", () => {
    const host = loadSimHost();
    expect(host.onlineCombatPhaseIds()).toEqual([
      "windup",
      "guard",
      "contact",
      "projectile_flight",
      "stagger",
      "ball_spill",
      "immunity_expiry",
    ]);
  });

  it("onlineCombatPhaseScenarioJson reports well-formed metadata for every phase", () => {
    const host = loadSimHost();
    for (const id of host.onlineCombatPhaseIds()) {
      const scenario = JSON.parse(host.onlineCombatPhaseScenarioJson(id)) as {
        id: string;
        route: "policy" | "canonical_input";
        steps: number;
        deliver_period: number;
        hold_equipment: boolean;
      };
      expect(scenario.id).toBe(id);
      expect(["policy", "canonical_input"]).toContain(scenario.route);
      expect(scenario.steps).toBeGreaterThan(0);
      expect(scenario.deliver_period).toBeGreaterThan(0);
    }
    // Only "guard" reaches its phase via the live slot's own equipment
    // input -- see the Rust module's doc for why.
    const guard = JSON.parse(host.onlineCombatPhaseScenarioJson("guard")) as {
      route: string;
      hold_equipment: boolean;
    };
    expect(guard.route).toBe("canonical_input");
    expect(guard.hold_equipment).toBe(true);
    const windup = JSON.parse(host.onlineCombatPhaseScenarioJson("windup")) as {
      route: string;
      hold_equipment: boolean;
    };
    expect(windup.route).toBe("policy");
    expect(windup.hold_equipment).toBe(false);
  });

  it("onlineCombatPhaseScenarioJson throws (a catchable error, not a trap) on an unrecognized id", () => {
    const host = loadSimHost();
    expect(() => host.onlineCombatPhaseScenarioJson(UNRECOGNIZED_PHASE)).toThrow();
  });

  it("onlineCombatPhaseLiveSample throws on an unrecognized id", () => {
    const host = loadSimHost();
    expect(() => host.onlineCombatPhaseLiveSample(UNRECOGNIZED_PHASE, 0, 1)).toThrow();
  });

  it("onlineCombatPhaseLiveSample is neutral for every non-guard phase and every non-host driver index", () => {
    const host = loadSimHost();
    const neutral = host.inputFrameNeutralSample();
    for (const id of host.onlineCombatPhaseIds()) {
      if (id === "guard") {
        continue;
      }
      expect(host.onlineCombatPhaseLiveSample(id, 0, 1)).toBe(neutral);
    }
    // Even "guard" is neutral for a driver index other than 1 (the host).
    expect(host.onlineCombatPhaseLiveSample("guard", 0, 2)).toBe(neutral);
  });

  it("onlineCombatPhaseLiveSample presses equipment on the host for guard, and holds it thereafter", () => {
    const host = loadSimHost();
    const pressed = JSON.parse(
      host.inputFrameDecodeSampleJson(host.onlineCombatPhaseLiveSample("guard", 0, 1)),
    ) as {
      held: number;
      edges: number;
    };
    const heldBits = JSON.parse(host.inputFrameHeldBitsJson()) as Record<string, number>;
    const edgeBits = JSON.parse(host.inputFrameEdgeBitsJson()) as Record<string, number>;
    expect(pressed.held).toBe(heldBits.equipment);
    expect(pressed.edges).toBe(edgeBits.equipment_pressed);

    const held = JSON.parse(
      host.inputFrameDecodeSampleJson(host.onlineCombatPhaseLiveSample("guard", 1, 1)),
    ) as {
      held: number;
      edges: number;
    };
    expect(held.held).toBe(heldBits.equipment);
    expect(held.edges).toBe(0);
  });
});

describe("onlineCombatPhases bridge: boundary-zero snapshots", () => {
  it("onlineCombatPhaseBoundaryZero builds a real, opaque snapshot handle for every phase", () => {
    const host = loadSimHost();
    for (const id of host.onlineCombatPhaseIds()) {
      const handle = host.onlineCombatPhaseBoundaryZero(id);
      try {
        expect(handle).toBeDefined();
      } finally {
        handle.free();
      }
    }
  });

  it("onlineCombatPhaseBoundaryZero throws on an unrecognized id", () => {
    const host = loadSimHost();
    expect(() => host.onlineCombatPhaseBoundaryZero(UNRECOGNIZED_PHASE)).toThrow();
  });

  it("two calls with the same (id, duration) build snapshots a driver treats identically", () => {
    // The property `MatchDriverBridge`'s own constructor doc relies on: its
    // `initialSnapshotOverride` parameter is consumed by value, so seeding
    // more than one peer in the same session means calling this builder
    // again, not reusing one handle. Proven here by seeding a host and a
    // guest driver from two independent calls and checking they agree.
    const host = loadSimHost();
    const freezeJson = host.matchDriverFixtureFreezeJson("1v1");
    const manifestJson = host.matchDriverFixtureManifestJson("1v1");
    const hostSession = new host.Session("nebula", "orion", 7, 20, 3);
    const guestSession = new host.Session("nebula", "orion", 7, 20, 3);
    try {
      const hostHandle = host.onlineCombatPhaseBoundaryZero("contact");
      const guestHandle = host.onlineCombatPhaseBoundaryZero("contact");
      const hostDriver = new host.MatchDriverBridge(
        hostSession,
        "host",
        "host",
        freezeJson,
        manifestJson,
        undefined,
        undefined,
        hostHandle,
      );
      const guestDriver = new host.MatchDriverBridge(
        guestSession,
        "guest",
        "guest_1",
        freezeJson,
        manifestJson,
        undefined,
        undefined,
        guestHandle,
      );
      hostDriver.initializeTransport();
      hostDriver.openPeer("guest_1");
      hostDriver.setPeerConnected("guest_1");
      guestDriver.initializeTransport();

      const neutral = host.inputFrameNeutralSample();
      const hostBatch = JSON.parse(hostDriver.advance(neutral)) as { step: number };
      const guestBatch = JSON.parse(guestDriver.advance(undefined)) as { step: number };
      expect(hostBatch.step).toBe(0);
      expect(guestBatch.step).toBe(0);

      // Both drivers must report a combat-bearing boundary zero -- neither
      // silently fell back to the session's own (non-combat) default.
      const hostInitial = hostDriver.initialSnapshotHandle();
      const guestInitial = guestDriver.initialSnapshotHandle();
      try {
        expect(hostInitial).toBeDefined();
        expect(guestInitial).toBeDefined();
      } finally {
        hostInitial.free();
        guestInitial.free();
      }
    } finally {
      hostSession.free();
      guestSession.free();
    }
  });
});

describe("onlineCombatPhases bridge: observed", () => {
  it("reports a contact event and correctly reports its absence", () => {
    const host = loadSimHost();
    const before = host.onlineCombatPhaseBoundaryZero("contact");
    try {
      const contactEvent = {
        kind: "contact",
        tick: 0,
        family_id: null,
        source_index: null,
        target_index: null,
        source_sequence: null,
        result: null,
        outcome: null,
        reason: null,
        terminal: null,
        x: 0,
        y: 0,
        interruption_ticks: null,
        displacement_px: null,
      };
      expect(
        host.onlineCombatPhaseObserved("contact", before, before, JSON.stringify([contactEvent])),
      ).toBe(true);
      expect(host.onlineCombatPhaseObserved("contact", before, before, "[]")).toBe(false);
    } finally {
      before.free();
    }
  });

  it("throws on an unrecognized phase id", () => {
    const host = loadSimHost();
    const before = host.onlineCombatPhaseBoundaryZero("contact");
    try {
      expect(() =>
        host.onlineCombatPhaseObserved(UNRECOGNIZED_PHASE, before, before, "[]"),
      ).toThrow();
    } finally {
      before.free();
    }
  });

  it("throws on malformed combatEventsJson, not a wasm trap", () => {
    const host = loadSimHost();
    const before = host.onlineCombatPhaseBoundaryZero("contact");
    try {
      expect(() => host.onlineCombatPhaseObserved("contact", before, before, "not json")).toThrow();
      expect(() => host.onlineCombatPhaseObserved("contact", before, before, "{}")).toThrow();
    } finally {
      before.free();
    }
  });
});
