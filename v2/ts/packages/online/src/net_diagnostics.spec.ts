// Ported from spec/game/online_net_diagnostics_spec.lua.
//
// The Lua spec's "diagnostics schema" describe block is pure and ports
// one-to-one. Everything else drives `net_diagnostics_fixture.lua`'s
// `fixture.harness`, which -- on this side of the port -- needs a real
// `game.online.match_driver` (Rust-owned, `crates/gc-netcode`).
//
// Two different adaptations follow, kept apart deliberately:
//
//   * Where the *only* reason the Lua case reaches for `fixture.harness` is
//     to obtain a valid, opted-in recorder (manifest/freeze data plus a
//     handful of direct `record_*` calls), this port constructs that
//     recorder directly with `newTestRecorder` instead. The assertion under
//     test is unchanged; only how the fixture is obtained is different.
//   * Where the case's actual claim is about the *driver's own* behaviour
//     under a live rollback session (a real correction, a real hash
//     mismatch, a real ownership violation, a real transport loss) it is
//     ported as `it.skip`, because faking that behaviour in TypeScript
//     would mean re-implementing rollback scheduling here -- exactly what
//     v2/README.md §2.1 says must never happen on this side of the
//     determinism line.
//
// # Status as of the `gc-wasm` `MatchDriverBridge`/`Coordinator` landing
//
// `@gc/online`'s `package.json` now declares `@gc/wasm` as a dependency
// (fixed -- `import("@gc/wasm")` resolves here now), and a wasm bridge over
// `game.online.match_driver` exists (`@gc/wasm`'s `MatchDriverBridge`,
// `RollbackEventsTimeline`). But `net_diagnostics_fixture.ts`'s
// `NetDiagnosticsFixtureEnv` needs five separate ports to build
// `fixture.harness`'s equivalent (`matchDriver`, `matchDriverFixture`,
// `protocol`, `inputProtocol`, `inputFrame`), and the bridge fills at most
// one of them, incompletely:
//
//   * There is still no wasm bridge at all, in `gc-wasm`'s current source,
//     for `game.online.match_driver_fixture` (session/initial-snapshot
//     construction -- confirmed empirically: `MatchDriverBridge`'s
//     constructor needs a `freezeJson`/`manifestJson` pair, nothing in
//     `@gc/wasm` can produce one, and hand-rolling one from the wire shape
//     alone reaches a Rust panic deep in `DriverRules`/`live_slot`, not a
//     clean validation error -- see `match_presentation.spec.ts`'s header
//     for the exact probe), `game.online.input_protocol` (the forged
//     bundles the ownership-violation/authority-conflict cases need), or
//     `sim.input_frame` (sample construction/encoding). `MatchDriverBridge`
//     itself only exposes the aggregate `advance()` step plus read-only
//     diagnostics/JSON dumps -- not the `create`/`advance`/`diagnostics`
//     shape `MatchDriverPort` here expects, and building samples/wires by
//     hand in TypeScript to work around the gap would mean re-implementing
//     `sim.input_frame` encoding on this side of the determinism line,
//     which v2/README.md §2.1 forbids outright ("input encoding is Rust").
//
// So every live-driver case below is still `it.skip`. Re-port once bridges
// for `match_driver_fixture`, `input_protocol`, and `input_frame` exist
// alongside `MatchDriverBridge`, closing all five `NetDiagnosticsFixtureEnv`
// ports for real.

import { describe, expect, it } from "vitest";
import { newMessage, fakeStar, type TransportChannelDiagnostics, type TransportPeerMessage, type TransportStarDiagnostics } from "@gc/transport";
import * as schema from "./diagnostics_schema.ts";
import {
  newNetDiagnostics,
  recordAnchor,
  recordCheckpoint,
  recordControl,
  recordEvent,
  recordMismatch,
  recordPacket,
  recordRuntimeSample,
  recordSignal,
  recordStep,
  recordTeardown,
  recordTransport,
  optInExport,
  exportArtifact,
  summary,
  EXPORT,
  type CoordinatorFreeze,
  type MatchDriverBatch,
  type MatchDriverDiagnostics,
  type NetDiagnostics,
  type NetDiagnosticsLimits,
  type NetDiagnosticsOptions,
  type ProtocolControlMessage,
  type ProtocolDecoder,
  type SessionManifest,
} from "./net_diagnostics.ts";
import { DiagnosticTransport } from "./diagnostic_transport.ts";

// ---------------------------------------------------------------------------
// Test fixtures (direct construction -- see the module doc comment)
// ---------------------------------------------------------------------------

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

function testDriverDiagnostics(overrides: Partial<MatchDriverDiagnostics> = {}): MatchDriverDiagnostics {
  return {
    status: "active",
    present_input_tick: 0,
    confirmed_input_tick: -1,
    confirmed_output_tick: -1,
    retained_floor_tick: 0,
    rollback_count: 0,
    correction_count: 0,
    predicted_slot_samples: 0,
    max_rollback_depth: 0,
    hash_mismatches: 0,
    checkpoint_count: 0,
    owned: [],
    authored: [],
    ...overrides,
  };
}

function testDriverBatch(overrides: Partial<MatchDriverBatch> = {}): MatchDriverBatch {
  return {
    input_tick: 0,
    applied_rows: 0,
    reconciliations: 0,
    sent_packets: 0,
    outputs: [],
    checkpoints: [],
    control: [],
    ...overrides,
  };
}

function channelDiagnostics(): TransportChannelDiagnostics {
  return {
    state: "connected",
    outbound_depth: 0,
    inbound_depth: 0,
    buffered_amount: 0,
    sent: 1,
    received: 1,
    dropped_outbound: 0,
    dropped_inbound: 0,
  };
}

function starDiagnostics(overrides: Partial<TransportStarDiagnostics> = {}): TransportStarDiagnostics {
  return {
    role: "host",
    state: "connected",
    capacity: 7,
    peer_count: 1,
    queue_limit: 64,
    buffered_amount_limit: 65536,
    event_depth: 0,
    sent: 1,
    received: 1,
    dropped_outbound: 0,
    dropped_inbound: 0,
    malformed: 0,
    unsupported_version: 0,
    overflow: 0,
    backpressure: 0,
    last_error: null,
    peers: [
      {
        peer_id: "guest_1",
        slot: 1,
        state: "connected",
        ice_state: "connected",
        control: channelDiagnostics(),
        input: channelDiagnostics(),
        sequence_gaps: 0,
        backpressure: 0,
        malformed: 0,
        last_error: null,
      },
    ],
    ...overrides,
  };
}

// The exported artifact's shape is dynamically constructed (see
// `net_diagnostics.ts`'s `exportArtifact`), so this narrow view names only
// the fields these tests actually read back.
interface TestArtifact {
  readonly schema_version: number;
  readonly session: Record<string, unknown>;
  readonly collection: {
    readonly rejected_values: number;
    readonly retention: Record<string, string>;
    readonly dropped: Record<string, number>;
  };
  readonly canonical: {
    readonly simulation: { readonly status: string; readonly checkpoint_count: number };
    readonly checkpoints: readonly { readonly tick: number; readonly hash: string }[];
    readonly mismatches: readonly { readonly tick: number }[];
    readonly control: readonly { readonly ordinal: number; readonly kind: string }[];
    readonly delivery: { readonly packets: readonly unknown[] };
  };
  readonly runtime: {
    readonly star: { readonly last_error: string | null } | undefined;
    readonly peers: readonly { readonly last_error: string | null }[];
    readonly events: readonly { readonly detail?: string; readonly code?: string }[];
    readonly signals: readonly { readonly content: string; readonly byte_length: number; readonly direction: string }[];
    readonly teardown: { readonly requested: boolean; readonly complete: boolean; readonly orphaned_peers: readonly string[] } | undefined;
  };
  readonly anchors: readonly { readonly mapping_error_ms: number }[];
}

function exportOf(recorder: NetDiagnostics): TestArtifact {
  const result = exportArtifact(recorder);
  if (!result.ok) {
    throw new Error(result.error);
  }
  return result.value as unknown as TestArtifact;
}

// Every string anywhere in an artifact, so a redaction assertion can be
// about the whole thing rather than about the handful of fields a test
// remembered.
function collectStrings(value: unknown, out: string[]): void {
  if (typeof value === "string") {
    out.push(value);
  } else if (Array.isArray(value)) {
    for (const child of value) {
      collectStrings(child, out);
    }
  } else if (typeof value === "object" && value !== null) {
    for (const [key, child] of Object.entries(value)) {
      collectStrings(key, out);
      collectStrings(child, out);
    }
  }
}

function allText(artifact: unknown): string {
  const parts: string[] = [];
  collectStrings(artifact, parts);
  parts.sort();
  return parts.join("\n");
}

function assertAbsent(artifact: unknown, poison: string): void {
  const text = allText(artifact);
  expect(text.includes(poison), `${poison} survived into the export`).toBe(false);
  for (const token of poison.match(/[\w.\-:@]+/g) ?? []) {
    if (token.length >= 6 && /[.:@]/.test(token)) {
      expect(text.includes(token), `fragment ${token} survived into the export`).toBe(false);
    }
  }
}

describe("diagnostics schema", () => {
  it("refuses a canonical field that borrows wall-clock vocabulary", () => {
    expect(() =>
      schema.record("bad", "canonical", [
        { name: "confirmed_tick", kind: "integer" },
        { name: "rtt_ms", kind: "number" },
      ])
    ).toThrow();
  });

  it("refuses a runtime field that borrows simulation vocabulary", () => {
    expect(() =>
      schema.record("bad", "runtime", [
        { name: "monotonic_ms", kind: "number" },
        { name: "confirmed_tick", kind: "integer" },
      ])
    ).toThrow();
  });

  it("requires an anchor to bind both clocks and declare its error", () => {
    expect(() =>
      schema.record("bad_anchor", "anchor", [
        { name: "input_tick", kind: "integer" },
        { name: "monotonic_ms", kind: "number" },
      ])
    ).toThrow();

    const good = schema.record("good_anchor", "anchor", [
      { name: "input_tick", kind: "integer" },
      { name: "monotonic_ms", kind: "number" },
      { name: "mapping_error_ms", kind: "number" },
    ]);
    expect(good.kind).toBe("record");
  });

  // Regression: the completeness check used to be gated on being the
  // outermost call, so it never ran for the shape that actually ships --
  // `EXPORT` is a domainless container and its `anchors` field is nested
  // one level down. These build the anchor exactly the way `EXPORT` does.
  it("enforces anchor completeness on a nested anchor, as EXPORT nests it", () => {
    expect(EXPORT.domain, "EXPORT is no longer a domainless container").toBeUndefined();
    let anchorPaths = 0;
    for (const domain of Object.values(schema.domains(EXPORT))) {
      if (domain === "anchor") {
        anchorPaths += 1;
      }
    }
    expect(anchorPaths > 0, "EXPORT declares no anchor field to enforce").toBe(true);

    function builds(fields: schema.DiagnosticsField[]): boolean {
      try {
        schema.record("nested_probe", undefined, [
          { name: "session", kind: "record", domain: "identity", fields: [{ name: "peer_id", kind: "id" }] },
          { name: "anchors", kind: "array", domain: "anchor", element: { kind: "record", fields } },
        ]);
        return true;
      } catch {
        return false;
      }
    }

    expect(
      builds([
        { name: "input_tick", kind: "integer" },
        { name: "monotonic_ms", kind: "number" },
      ]),
      "a nested anchor without a mapping error was accepted"
    ).toBe(false);
    expect(
      builds([
        { name: "monotonic_ms", kind: "number" },
        { name: "mapping_error_ms", kind: "number" },
      ]),
      "a nested anchor naming no simulation tick was accepted"
    ).toBe(false);
    expect(
      builds([
        { name: "input_tick", kind: "integer" },
        { name: "mapping_error_ms", kind: "number" },
      ]),
      "a nested anchor whose only wall-clock word is its own error term is still a binding"
    ).toBe(true);
    expect(
      builds([
        { name: "input_tick", kind: "integer" },
        { name: "monotonic_ms", kind: "number" },
        { name: "mapping_error_ms", kind: "number" },
      ]),
      "a well-formed nested anchor was rejected"
    ).toBe(true);
  });

  // The scope must be per-anchor-subtree. A sibling section naming a tick
  // or a millisecond must not be able to satisfy the anchor's own
  // requirements.
  it("does not let sibling sections satisfy an anchor's completeness", () => {
    expect(() =>
      schema.record("sibling_probe", undefined, [
        { name: "canonical", kind: "record", domain: "canonical", fields: [{ name: "confirmed_tick", kind: "integer" }] },
        { name: "runtime", kind: "record", domain: "runtime", fields: [{ name: "mapping_error_ms", kind: "number" }] },
        {
          name: "anchors",
          kind: "array",
          domain: "anchor",
          element: { kind: "record", fields: [{ name: "monotonic_ms", kind: "number" }] },
        },
      ])
    ).toThrow();
  });

  it("keeps the two vocabularies disjoint across the whole export shape", () => {
    const domains = schema.domains(EXPORT);
    let canonical = 0;
    let runtime = 0;
    let anchor = 0;
    for (const [path, domain] of Object.entries(domains)) {
      const match = /([^.[]+)\]?$/.exec(path);
      const name = match?.[1] ?? path;
      if (domain === "canonical" || domain === "identity") {
        canonical += 1;
        expect(
          schema.isWallClockName(name),
          `${path} is deterministic evidence but reads as a wall-clock measurement`
        ).toBe(false);
      } else if (domain === "runtime") {
        runtime += 1;
        expect(
          schema.isSimulationName(name),
          `${path} is a runtime observation but reads as simulation truth`
        ).toBe(false);
      } else if (domain === "anchor") {
        anchor += 1;
      }
    }
    expect(canonical > 0 && runtime > 0 && anchor > 0).toBe(true);
  });

  it("rejects direct identifiers, paths, and raw addresses as ids", () => {
    const shape = schema.record("id_probe", "identity", [{ name: "peer_id", kind: "id" }]);
    for (const bad of [
      "player@example.com",
      "home/oscar/save.json",
      "c:/users/oscar",
      "../secrets",
      "192.168.1.14",
      "[fe80::1]",
    ]) {
      expect(schema.validate(shape, { peer_id: bad }).ok, `${bad} was accepted as a pseudonymous id`).toBe(false);
    }
    expect(schema.validate(shape, { peer_id: "guest_1" }).ok).toBe(true);
  });

  it("rejects non-finite and out-of-range values", () => {
    const shape = schema.record("numbers", "canonical", [
      { name: "count", kind: "integer", min: 0 },
      { name: "ratio", kind: "number" },
    ]);
    expect(schema.validate(shape, { count: 1, ratio: 0 / 0 }).ok).toBe(false);
    expect(schema.validate(shape, { count: 1, ratio: Infinity }).ok).toBe(false);
    expect(schema.validate(shape, { count: -1, ratio: 0 }).ok).toBe(false);
    expect(schema.validate(shape, { count: 1.5, ratio: 0 }).ok).toBe(false);
    expect(schema.validate(shape, { count: 1, ratio: 0.5 }).ok).toBe(true);
  });

  it("encodes maps in sorted key order, not table order", () => {
    const shape = schema.record("m", "canonical", [{ name: "live", kind: "map", element: { kind: "id" } }]);
    const first = schema.encode(shape, { live: { host: "home_1", guest_1: "home_2" } });
    const second = schema.encode(shape, { live: { guest_1: "home_2", host: "home_1" } });
    expect(first.ok && second.ok && first.value).toBe(second.ok && second.value);
  });
});

describe("net diagnostics collection", () => {
  // Ported cases 1-6 in the Lua original ("summarises a clean 2v2 run...",
  // "folds star and per-channel transport counters...", "records the
  // sample, send, arrival, and apply lifecycle of a packet", "counts
  // rollback, resimulation, and event reconciliation under impairment",
  // "keeps canonical evidence byte-stable while runtime observation
  // moves", "reproduces the canonical projection across two identical
  // runs") all assert on values a *live* `match_driver` produces across a
  // real multi-tick run -- rollback counts, checkpoint hashes, packet
  // lifecycle timing. See this file's header comment: re-port these once a
  // `NetDiagnosticsFixtureEnv` backed by the real `gc-netcode` exists.
  describe.skip("live-driver runs (blocked: no wasm bridge exists for match_driver_fixture/input_protocol/input_frame -- see the file header comment)", () => {
    it.skip("summarises a clean 2v2 run with both clocks kept apart", () => {});
    it.skip("folds star and per-channel transport counters into the runtime section", () => {});
    it.skip("records the sample, send, arrival, and apply lifecycle of a packet", () => {});
    it.skip("counts rollback, resimulation, and event reconciliation under impairment", () => {});
    it.skip("keeps canonical evidence byte-stable while runtime observation moves", () => {});
    it.skip("reproduces the canonical projection across two identical runs", () => {});
  });

  it("holds the export behind an explicit opt-in", () => {
    const recorder = newTestRecorder({ export_opt_in: false });
    const failed = exportArtifact(recorder);
    expect(failed.ok).toBe(false);
    expect(!failed.ok && failed.error.includes("opted into")).toBe(true);
    // The summary is local-only and never gated: it does not leave the machine.
    expect(summary(recorder).length > 0).toBe(true);
    optInExport(recorder);
    expect(exportArtifact(recorder).ok).toBe(true);
  });
});

describe("net diagnostics privacy", () => {
  it("keeps signalling blobs out of the export entirely", () => {
    const recorder = newTestRecorder();
    // A blob shaped like the real thing: SDP with ICE credentials and a
    // candidate line carrying a host address.
    const secret = [
      "v=0",
      "a=ice-ufrag:F7gI",
      "a=ice-pwd:x9Yb3kQwPl2sVn8tRc4mHd",
      "a=candidate:1 1 udp 2130706431 192.168.1.14 54321 typ host",
      "a=fingerprint:sha-256 AB:CD:EF",
    ].join("\r\n");
    recordSignal(recorder, "guest_1", "inbound", secret);

    const artifact = exportOf(recorder);
    expect(artifact.runtime.signals.length).toBe(1);
    const signal = artifact.runtime.signals[0];
    expect(signal?.content).toBe(schema.REDACTED);
    expect(signal?.byte_length).toBe(secret.length);
    expect(signal?.direction).toBe("inbound");

    const text = allText(artifact);
    for (const forbidden of [
      "ice-ufrag",
      "ice-pwd",
      "x9Yb3kQwPl2sVn8tRc4mHd",
      "candidate",
      "192.168.1.14",
      "fingerprint",
      "v=0",
    ]) {
      expect(text.includes(forbidden), `${forbidden} survived into the diagnostic export`).toBe(false);
    }
  });

  const POISON = [
    "ICE failed for candidate 192.168.1.14:54321 typ host",
    "connection reset by [fe80::1a2b:3c4d:5e6f:7890]",
    "bridge error: a=ice-pwd:x9Yb3kQwPl2sVn8tRc4mHd",
    "failed to apply sdp offer",
    "signalling failed for turn:turn.example.com:3478",
    "peer someone@example.com is unreachable",
  ];

  it("redacts sensitive free text arriving as a transport error", () => {
    const recorder = newTestRecorder();
    for (const poison of POISON) {
      expect(
        recordTransport(
          recorder,
          starDiagnostics({
            last_error: poison,
            peers: [
              {
                peer_id: "guest_1",
                slot: 1,
                state: "connected",
                ice_state: "connected",
                control: channelDiagnostics(),
                input: channelDiagnostics(),
                sequence_gaps: 0,
                backpressure: 0,
                malformed: 0,
                last_error: poison,
              },
            ],
          })
        ).ok
      ).toBe(true);
      const artifact = exportOf(recorder);
      expect(artifact.runtime.star?.last_error).toBe(schema.REDACTED);
      expect(artifact.runtime.peers[0]?.last_error).toBe(schema.REDACTED);
      assertAbsent(artifact, poison);
    }
  });

  it("redacts sensitive free text arriving as a runtime event detail", () => {
    const recorder = newTestRecorder();
    for (const [index, poison] of POISON.entries()) {
      expect(
        recordEvent(recorder, {
          kind: "peer_error",
          monotonic_ms: index * 10,
          peer_id: "guest_1",
          code: "signal_error",
          detail: poison,
        }).ok
      ).toBe(true);
    }
    const artifact = exportOf(recorder);
    const redacted = artifact.runtime.events.filter((event) => event.detail === schema.REDACTED).length;
    expect(redacted, "a poisoned event detail survived unredacted").toBe(POISON.length);
    for (const poison of POISON) {
      assertAbsent(artifact, poison);
    }
  });

  it("keeps benign transport error text readable", () => {
    const recorder = newTestRecorder();
    expect(
      recordEvent(recorder, {
        kind: "star_error",
        monotonic_ms: 5,
        code: "overflow",
        detail: "outbound queue reached its limit of 64 messages",
      }).ok
    ).toBe(true);
    const artifact = exportOf(recorder);
    const last = artifact.runtime.events[artifact.runtime.events.length - 1];
    expect(last?.detail).toBe("outbound queue reached its limit of 64 messages");
    expect(last?.code).toBe("overflow");
  });

  // Not expressible from @gc/online: `desync_package.build` belongs to
  // `game.online.desync_package`, which is Rust-owned (`crates/gc-netcode`)
  // and out of this package's scope entirely -- no TS port of it exists to
  // call. Re-port once a `desync_package` equivalent exists on the Rust
  // side and this package is handed an injected port for it, mirroring
  // `match_presentation.ts`.
  it.skip("stops poisoned free text reaching a desync package", () => {});

  it("refuses to store a direct identifier as a peer id", () => {
    const recorder = newTestRecorder();
    recordRuntimeSample(recorder, { peer_id: "someone@example.com", monotonic_ms: 10, rtt_ms: 20 });
    const result = exportArtifact(recorder);
    expect(result.ok, "an email address reached a validated export").toBe(false);
    expect(!result.ok && result.error.includes("pseudonymous")).toBe(true);
  });

  it("carries no participant or playtest payload fields at all", () => {
    const paths = schema.domains(EXPORT);
    for (const path of Object.keys(paths)) {
      const lowered = path.toLowerCase();
      for (const forbidden of ["participant", "consent", "research", "clipboard", "address", "sdp", "candidate", "email"]) {
        expect(lowered.includes(forbidden), `${path} names a field this schema must never carry`).toBe(false);
      }
    }
  });
});

describe("net diagnostics bounds", () => {
  it("marks truncation explicitly instead of silently shrinking", () => {
    const limits: NetDiagnosticsLimits = {
      checkpoints: 2,
      packets: 4,
      control: 2,
      events: 2,
      signals: 1,
      mismatches: 2,
      anchors: 2,
      latency_peers: 2,
    };
    const recorder = newTestRecorder({ limits });
    // The published count and the retained count are different claims (see
    // `net_diagnostics.ts`'s `recordStep`); fold one step declaring how
    // many checkpoints the driver published before pushing the checkpoints
    // themselves, so the "published >= retained" assertion below is
    // actually exercised rather than vacuously true at zero.
    recordStep(recorder, testDriverDiagnostics({ checkpoint_count: 6 }), testDriverBatch());
    for (let index = 0; index < 6; index += 1) {
      recordCheckpoint(recorder, { tick: index, hash: (16 + index).toString(16).padStart(16, "0"), live: {} });
    }
    for (let index = 0; index < 10; index += 1) {
      recordPacket(recorder, {
        sender_id: "guest_1",
        disposition: "sent",
        sequence: index,
        payload_bytes: 10,
        sample_step: index,
        send_transport_tick: index,
        authority_input_tick: index,
      });
    }
    for (let index = 0; index < 6; index += 1) {
      recordAnchor(recorder, { input_tick: index, monotonic_ms: index, mapping_error_ms: 1 });
    }
    const artifact = exportOf(recorder);

    expect(artifact.collection.retention.packets).toBe(schema.TRUNCATED);
    expect((artifact.collection.dropped.packets ?? 0) > 0).toBe(true);
    expect(artifact.canonical.delivery.packets.length <= limits.packets).toBe(true);
    expect(artifact.canonical.checkpoints.length <= limits.checkpoints).toBe(true);
    expect(artifact.anchors.length <= limits.anchors).toBe(true);
    // The count of published checkpoints is not the count retained, and the
    // artifact says both rather than quietly conflating them.
    expect(artifact.canonical.simulation.checkpoint_count >= artifact.canonical.checkpoints.length).toBe(true);

    const lines = summary(recorder);
    expect(lines.some((line) => line.includes("truncated")), "a truncated ring did not show up in the human summary").toBe(true);
  });

  it("keeps the oldest mismatches and the newest packets", () => {
    const recorder = newTestRecorder({
      limits: {
        checkpoints: 8,
        packets: 8,
        control: 8,
        events: 8,
        signals: 4,
        mismatches: 2,
        anchors: 4,
        latency_peers: 4,
      },
    });
    for (let index = 1; index <= 5; index += 1) {
      recordMismatch(recorder, {
        tick: index * 10,
        peer_id: "guest_1",
        local_hash: index.toString(16).padStart(16, "0"),
        remote_hash: (index + 100).toString(16).padStart(16, "0"),
      });
    }
    const artifact = exportOf(recorder);
    expect(artifact.canonical.mismatches.length).toBe(2);
    // The first divergence is the causal one, so it is the one kept.
    expect(artifact.canonical.mismatches[0]?.tick).toBe(10);
    expect(artifact.canonical.mismatches[1]?.tick).toBe(20);
    expect(artifact.collection.retention.mismatches).toBe(schema.TRUNCATED);
  });

  it("counts rejected values instead of storing them", () => {
    const recorder = newTestRecorder();
    expect(recordAnchor(recorder, { input_tick: 0, monotonic_ms: 0 / 0, mapping_error_ms: 1 }).ok).toBe(false);
    expect(recordAnchor(recorder, { input_tick: 0, monotonic_ms: 10, mapping_error_ms: -1 }).ok).toBe(false);
    expect(recordRuntimeSample(recorder, { peer_id: "guest_1", monotonic_ms: 10, rtt_ms: Infinity }).ok).toBe(false);
    const artifact = exportOf(recorder);
    expect(artifact.collection.rejected_values).toBe(3);
  });
});

describe("net diagnostics failure fixtures", () => {
  // Ported cases: an ownership violation, an authority conflict,
  // over-window input, hash divergence, and a guest disconnect / host loss
  // all require a real `match_driver` to actually detect the fault (the
  // Lua originals forge input bundles and hand them to a live driver's
  // transport, or call `match_driver.observe_checkpoint` directly). See
  // this file's header comment.
  describe.skip("live-driver fault detection (blocked: no wasm bridge exists for match_driver_fixture/input_protocol's forged bundles -- see the file header comment)", () => {
    it.skip("types an ownership violation", () => {});
    it.skip("types an authority conflict", () => {});
    it.skip("types over-window input and names the tick it was attributed to", () => {});
    it.skip("types hash divergence and keeps the first divergent boundary", () => {});
    it.skip("types a guest disconnect and a host loss", () => {});
  });

  it("reports orphaned links and residual queues at teardown", () => {
    const recorder = newTestRecorder();
    const star = fakeStar({ role: "host" });
    expect(star.initialize().ok).toBe(true);
    const tap = new DiagnosticTransport({
      transport: star,
      recorder,
      peerId: "host_1",
      firstInputTick: 0,
      inputDelayTicks: 0,
    });
    expect(tap.shutdown().ok).toBe(true);
    const artifact = exportOf(recorder);
    const teardown = artifact.runtime.teardown;
    expect(teardown, "shutdown recorded no teardown").toBeDefined();
    expect(teardown?.requested).toBe(true);
    expect(teardown?.orphaned_peers.length).toBe(0);
    expect(teardown?.complete, "a clean shutdown was not reported complete").toBe(true);
  });

  it("reports an incomplete teardown as incomplete", () => {
    const recorder = newTestRecorder();
    recordTeardown(recorder, {
      requested: true,
      closed_peers: 1,
      orphaned_peers: ["guest_1"],
      residual_outbound: 3,
      residual_inbound: 0,
    });
    const artifact = exportOf(recorder);
    const teardown = artifact.runtime.teardown;
    expect(teardown, "teardown was not recorded").toBeDefined();
    expect(teardown?.complete).toBe(false);
    expect(teardown?.orphaned_peers[0]).toBe("guest_1");
  });

  it("records accepted control traffic in order", () => {
    // A fake, injected decoder standing in for `game.online.protocol`
    // (Rust-owned) -- see the module doc comment. `recordControl` itself
    // does not care what the decoder's grammar is, only that it recovers
    // `kind`/`sequence`/`message_id`, so a simple pipe-delimited fake wire
    // exercises exactly the same code path a real one would.
    const decode: ProtocolDecoder = (payload) => {
      const [kind, sequence, messageId] = payload.split("|");
      if (kind === undefined || sequence === undefined || messageId === undefined) {
        return null;
      }
      const message: ProtocolControlMessage = { kind: kind as ProtocolControlMessage["kind"], sequence: Number(sequence), message_id: messageId };
      return message;
    };
    const recorder = newTestRecorder({ decodeControlMessage: decode });
    for (let index = 1; index <= 3; index += 1) {
      const envelope = newMessage({ type: "event", seq: index, payload: `hash_report|${index}|msg_${index}` });
      if (!envelope.ok) {
        throw new Error(envelope.error.message);
      }
      const addressed: TransportPeerMessage = {
        peer_id: "guest_1",
        channel: "control",
        message: envelope.value,
        arrival_seq: index,
      };
      expect(recordControl(recorder, addressed).ok).toBe(true);
    }
    const artifact = exportOf(recorder);
    expect(artifact.canonical.control.length).toBe(3);
    for (let index = 1; index < 3; index += 1) {
      const current = artifact.canonical.control[index];
      const previous = artifact.canonical.control[index - 1];
      expect(current !== undefined && previous !== undefined && current.ordinal > previous.ordinal, "control ordinals are not monotonic").toBe(true);
      expect(current?.kind).toBe("hash_report");
    }
  });
});
