// Spec for the narrow `desync_package.ts` port -- see that file's header for
// what this covers and, more importantly, what it deliberately does not:
// there is no `crates/gc-netcode/tests/desync_package.rs`-shaped coverage
// here (wire identity, cross-language digest, round-trip decode). This
// tests only the one behaviour this port exists for: that a package built
// from a `NetDiagnostics` recorder never carries free text that arrived
// poisoned, plus the small bookkeeping (`build`'s guards, wire truncation,
// `reproducible_from`) needed to build one at all.

import { describe, expect, it } from "vitest";

import { build, encode, MAX_WIRES, type DesyncPackageOptions } from "./desync_package.ts";
import {
  newNetDiagnostics,
  optOutExport,
  recordEvent,
  type CoordinatorFreeze,
  type NetDiagnostics,
  type NetDiagnosticsOptions,
  type ProtocolDecoder,
  type SessionManifest,
} from "./net_diagnostics.ts";
import { REDACTED } from "./diagnostics_schema.ts";

function testManifest(overrides: Partial<SessionManifest> = {}): SessionManifest {
  return {
    session_id: "session_1",
    match_mode: "2v2",
    combat_status: "accepted_proceed",
    build_id: "build_1",
    source_id: "source_1",
    content_id: "content_1",
    tuning_id: "tuning_1",
    match_config_id: "match_config_1",
    fixture_id: "fixture_1",
    arena_id: "arena_1",
    combat_rules_id: "combat_rules_1",
    gameplay_ai_policy_id: "policy_1",
    protocol_version: 1,
    input_version: 1,
    snapshot_version: 1,
    tape_version: 1,
    combat_schema_version: 1,
    seed: 1,
    tick_rate: 60,
    duration_ticks: 18000,
    max_goals: 5,
    ...overrides,
  };
}

function testFreeze(overrides: Partial<CoordinatorFreeze> = {}): CoordinatorFreeze {
  return {
    manifest_id: "0123456789abcdef",
    assignment_id: "fedcba9876543210",
    countdown_id: "countdown_1",
    first_input_tick: 0,
    ...overrides,
  };
}

function neverDecodes(): ProtocolDecoder {
  return () => null;
}

function newTestRecorder(overrides: Partial<NetDiagnosticsOptions> = {}): NetDiagnostics {
  return newNetDiagnostics({
    role: "host",
    peer_id: "host_1",
    manifest: testManifest(),
    freeze: testFreeze(),
    export_opt_in: true,
    decodeControlMessage: neverDecodes(),
    ...overrides,
  });
}

function baseOptions(overrides: Partial<DesyncPackageOptions> = {}): DesyncPackageOptions {
  return {
    recorder: newTestRecorder(),
    peer_id: "host_1",
    remote_peer_id: "guest_1",
    agreed_boundary_tick: 0,
    agreed_boundary_hash: "0123456789abcdef",
    divergence_tick: 30,
    local_hash: "fedcba9876543210",
    remote_hash: "deadbeefdeadbeef",
    input_wires: [],
    ...overrides,
  };
}

describe("desync_package", () => {
  it("refuses a divergence at or before the agreed boundary", () => {
    const atBoundary = build(baseOptions({ agreed_boundary_tick: 10, divergence_tick: 10 }));
    expect(atBoundary.ok).toBe(false);

    const beforeBoundary = build(baseOptions({ agreed_boundary_tick: 10, divergence_tick: 5 }));
    expect(beforeBoundary.ok).toBe(false);
  });

  it("refuses a package built from a recorder that never opted into export", () => {
    const recorder = newTestRecorder();
    optOutExport(recorder);
    const result = build(baseOptions({ recorder }));
    expect(result.ok).toBe(false);
  });

  it("truncates wires past MAX_WIRES and marks retention truncated", () => {
    const wires = Array.from({ length: MAX_WIRES + 5 }, (_, index) => `wire_${index}`);
    const result = build(baseOptions({ input_wires: wires }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.inputs).toMatchObject({ wire_count: MAX_WIRES, retention: "truncated" });
  });

  it("keeps retention complete and every wire when under the bound", () => {
    const wires = ["wire_0", "wire_1", "wire_2"];
    const result = build(baseOptions({ input_wires: wires }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.value.inputs).toMatchObject({ wire_count: 3, retention: "complete", wires });
  });

  it("reports the weakest honest reproducible_from, never a claim it cannot check", () => {
    const withoutTape = build(baseOptions());
    expect(withoutTape.ok).toBe(true);
    if (withoutTape.ok) {
      expect((withoutTape.value.reproduction as { reproducible_from: string }).reproducible_from).toBe(
        "retained_window"
      );
    }

    const withTape = build(
      baseOptions({ tape: { tape_id: "tape_1", tape_digest: "0011223344556677", tape_version: 1 } })
    );
    expect(withTape.ok).toBe(true);
    if (withTape.ok) {
      expect((withTape.value.reproduction as { reproducible_from: string }).reproducible_from).toBe(
        "tape_reference"
      );
    }
  });

  it("embeds runtime events verbatim from the export, already redacted", () => {
    const recorder = newTestRecorder();
    const poison = "ICE failed for candidate 192.168.1.14:54321 typ host";
    expect(
      recordEvent(recorder, {
        kind: "peer_error",
        monotonic_ms: 12,
        peer_id: "guest_1",
        detail: poison,
      }).ok
    ).toBe(true);

    const result = build(baseOptions({ recorder }));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    const events = result.value.runtime_events as readonly { readonly detail?: string }[];
    expect(events.length).toBe(1);
    expect(events[0]?.detail).toBe(REDACTED);

    const encoded = encode(result.value);
    expect(encoded.ok).toBe(true);
    if (encoded.ok) {
      expect(encoded.value.includes("192.168.1.14")).toBe(false);
      expect(encoded.value.includes(REDACTED)).toBe(true);
    }
  });
});
