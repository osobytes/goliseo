// A narrow, TS-local desync-package builder.
//
// This file exists for exactly one caller: `net_diagnostics.spec.ts`'s
// "stops poisoned free text reaching a desync package" case. That case is a
// privacy claim, not a wire-format claim: the package embeds runtime events
// verbatim, so redaction has to have happened on the way in rather than on
// the way out. The behaviour worth proving is that `net_diagnostics.ts`'s
// own redaction (`recordEvent` -> `diagnostics_schema.ts`'s
// `redactFreeText`) is what keeps a desync package clean, not some second
// sanitising pass here. This file has none -- it embeds `exportArtifact`'s
// output as-is, on purpose.
//
// Per ARCHITECTURE.md §1, the wire-format half of a desync package
// (protocol-shaped identity, the schema-checked round trip, a shared
// cross-language digest) is Rust-owned and already lives at
// `crates/gc-netcode/src/desync_package.rs`, complete and tested against a
// committed identity vector. This is not that module, and does not
// duplicate it.
//
// ## Deliberately left out, and why
//
//   * `input_protocol.decode`-derived `from_input_tick` / `through_input_tick`
//     / the `"fixture_boundary_zero"` classification. The full
//     (`crates/gc-netcode`) implementation decodes every wire to discover
//     which ticks it actually covers. `input_protocol` is Rust-owned with no
//     TS (or `@gc/wasm`-bridge) decode surface this package may depend on --
//     `net_diagnostics` is TS *by design* specifically so it never has to
//     reach across the determinism line (ARCHITECTURE.md §1). `build` below
//     never claims `"fixture_boundary_zero"`, the one classification only
//     decoding can actually prove; it reports `"tape_reference"` when a tape
//     is given and otherwise the weakest honest claim, `"retained_window"`.
//     A weak claim that is true beats a strong claim this file cannot check.
//   * `desync_package.SHAPE`, `schema.validate` against it, `digest`,
//     `rows` (wire -> row decode), and `summary`. No caller of this file
//     needs schema-checked round-tripping, a content digest over the whole
//     package, decoding wires back into rows, or an issue-comment
//     summary -- only that poisoned text is absent from a built package and
//     from its encoded form. `encode` below is therefore a plain
//     deterministic `JSON.stringify`, not `diagnostics_schema`'s canonical
//     length-prefixed encoding.
//   * `first_difference` (`MatchSnapshotDifference`). Nothing in the one
//     case this file serves supplies one.
//   * Per-wire length bounds and an `an input wire must be a string`
//     shape check performed before schema validation -- dropped along with
//     the schema validation itself.
//
// If a second caller ever needs more of this, extend it then. This file's
// job is the one behaviour above, not full parity with `crates/gc-netcode`'s
// desync-package implementation.

import { type Result, ok, err } from "@gc/core";
import * as schema from "./diagnostics_schema.ts";
import { exportArtifact, type NetDiagnostics } from "./net_diagnostics.ts";

export type DesyncReproducibleFrom = "tape_reference" | "retained_window";

export interface DesyncTapeReference {
  readonly tape_id: string;
  readonly tape_digest: string;
  readonly tape_version: number;
}

export interface DesyncPackageOptions {
  readonly recorder: NetDiagnostics;
  readonly peer_id: string;
  readonly remote_peer_id: string;
  /** Last boundary both peers hashed identically. */
  readonly agreed_boundary_tick: number;
  readonly agreed_boundary_hash: string;
  /** First boundary they disagreed on. */
  readonly divergence_tick: number;
  readonly local_hash: string;
  readonly remote_hash: string;
  /** Canonical input packet wires, sender order preserved, opaque here. */
  readonly input_wires: readonly string[];
  readonly tape?: DesyncTapeReference;
}

export const VERSION = 1;

// 192 wires at the protocol's 1 KiB envelope bound is a 192 KiB ceiling --
// same rationale and same value as `crates/gc-netcode`'s `MAX_WIRES`.
export const MAX_WIRES = 192;

// Only the parts of `exportArtifact`'s dynamic result this file reads back.
// `net_diagnostics.ts` does not export a typed shape for its own artifact
// (see that module's `exportArtifact` returning `Record<string, unknown>`),
// so this mirrors `net_diagnostics.spec.ts`'s own `TestArtifact` pattern:
// a narrow local view, not a claim about the whole shape.
interface DiagnosticsArtifactView {
  readonly session: Readonly<Record<string, unknown>>;
  readonly canonical: {
    readonly checkpoints: readonly Readonly<Record<string, unknown>>[];
    readonly control: readonly Readonly<Record<string, unknown>>[];
  };
  readonly runtime: {
    readonly events: readonly Readonly<Record<string, unknown>>[];
  };
}

function isInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && Math.floor(value) === value;
}

// Assemble a package. Returns `err` on anything malformed, rather than
// throwing -- a capture is produced at the worst moment of a session and
// must never be the thing that throws.
export function build(options: DesyncPackageOptions): Result<Record<string, unknown>, string> {
  if (!isInteger(options.agreed_boundary_tick) || !isInteger(options.divergence_tick)) {
    return err("a desync package needs finite integer boundary ticks");
  }
  if (options.divergence_tick <= options.agreed_boundary_tick) {
    return err("the divergence must be later than the boundary the peers agreed on");
  }

  const diagnostics = exportArtifact(options.recorder);
  if (!diagnostics.ok) {
    return err("a desync package needs an opted-in diagnostic export");
  }
  const artifact = diagnostics.value as unknown as DiagnosticsArtifactView;

  const wires: string[] = [];
  let truncated = false;
  for (const wire of options.input_wires) {
    if (wires.length >= MAX_WIRES) {
      truncated = true;
      break;
    }
    wires.push(wire);
  }

  const reproducibleFrom: DesyncReproducibleFrom =
    options.tape !== undefined ? "tape_reference" : "retained_window";

  const pkg: Record<string, unknown> = {
    package_version: VERSION,
    digest_algorithm: schema.DIGEST,
    session: artifact.session,
    reproduction: {
      reproducible_from: reproducibleFrom,
      local_peer_id: options.peer_id,
      remote_peer_id: options.remote_peer_id,
      ...(options.tape !== undefined ? { tape: options.tape } : {}),
    },
    divergence: {
      agreed_boundary_tick: options.agreed_boundary_tick,
      agreed_boundary_hash: options.agreed_boundary_hash,
      divergence_tick: options.divergence_tick,
      local_hash: options.local_hash,
      remote_hash: options.remote_hash,
    },
    inputs: {
      wire_count: wires.length,
      wire_digest: schema.tupleDigest("desync_input_wires", wires),
      retention: truncated ? schema.TRUNCATED : schema.COMPLETE,
      wires,
    },
    checkpoints: artifact.canonical.checkpoints,
    control: artifact.canonical.control,
    // The field the one behaviour this file proves is actually about:
    // embedded verbatim from the recorder's own export, which already
    // redacted anything sensitive at `recordEvent` time
    // (`diagnostics_schema.ts`'s `redactFreeText`). No sanitising happens
    // here, deliberately -- see this file's header.
    runtime_events: artifact.runtime.events,
  };
  return ok(pkg);
}

// A plain deterministic JSON encoding -- not `diagnostics_schema`'s
// canonical length-prefixed form. See this file's header for why that form
// is not ported here.
export function encode(pkg: Record<string, unknown>): Result<string, string> {
  return ok(JSON.stringify(pkg));
}
