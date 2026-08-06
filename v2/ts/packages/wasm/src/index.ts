// Typed loader for the `gc-wasm` build artifact.
//
// `gc-wasm` binds the simulation two different ways (see
// `v2/rust/crates/gc-wasm/src/lib.rs`'s doc):
//
// - Session lifecycle, the determinism check, and lobby wire helpers go
//   through `wasm-bindgen` — ergonomic, generated JS classes/functions,
//   reached here via `dist/pkg/gc_wasm.cjs` (built by `scripts/build.mjs`).
// - The per-frame render path is RAW: plain `extern "C"` exports with no
//   wasm-bindgen glue, so a rendered frame's flat `f64` block crosses once,
//   copy-free, as a `Float64Array` view over wasm linear memory.
//
// Both live in the same compiled module and therefore the same
// `WebAssembly.Instance` — `Session`'s registry handle
// (`crates/gc-wasm/src/registry.rs`) is only meaningful against the
// instance that created it. `scripts/build.mjs` patches wasm-bindgen's
// generated output to expose that one shared instance's raw exports
// (`__wbg_raw`) alongside its ergonomic classes, and `loadSimHost` below is
// what stitches the two into one typed surface.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  ControlMessageHeader,
  DeterminismEvidence,
  GcWasmModule,
  SimSessionConstructor,
} from "./types.ts";

export type {
  ControlMessageHeader,
  DeterminismEvidence,
  RawExports,
  SimSession,
  SimSessionConstructor,
} from "./types.ts";

const here = dirname(fileURLToPath(import.meta.url));
const artifactPath = join(here, "..", "dist", "pkg", "gc_wasm.cjs");

/** The loaded `gc-wasm` module: session lifecycle, the determinism check,
 * lobby wire helpers, and the raw per-frame render path in one typed
 * surface. */
export interface SimHost {
  /** Constructs a live match session. See `SimSession`'s doc. */
  readonly Session: SimSessionConstructor;
  /** This build's protocol vocabulary id, for lobby version negotiation. */
  protocolVocabularyId(): string;
  /** Decodes a control message's routing header (kind/session/peer/
   * sequence), without exposing its kind-specific body. */
  decodeControlMessageHeader(wire: string): ControlMessageHeader;
  /** Runs the frozen OMP-1 determinism campaign inside this wasm module and
   * returns its evidence. This is the acceptance surface for "did compiling
   * to wasm change simulation floating-point behavior" — compare
   * `final_hash`/`sequence_digest` against the pinned native-build
   * digests. */
  runDeterminismEvidence(): DeterminismEvidence;
  /** This module's linear memory, for reading the `Float64Array` views
   * `buildRenderFrame` hands back. */
  readonly memory: WebAssembly.Memory;
  /**
   * Builds the current render frame for the session named by `handle`
   * (`SimSession.handle`) and returns a zero-copy `Float64Array` view over
   * it — `gc_render::frame_buffer::encode`'s layout
   * (`v2/rust/crates/gc-render/src/frame_buffer.rs`), decoded field-by-field
   * on the JS side per that module's documented header/field-order
   * contract. Returns `null` if `handle` names no live session.
   *
   * The returned view is only valid until the next `buildRenderFrame`
   * call (the underlying buffer is reused and can move if it grows) —
   * copy out anything that needs to outlive that call.
   */
  buildRenderFrame(handle: number): Float64Array | null;
}

let cached: SimHost | undefined;

/**
 * Loads the compiled `gc-wasm` artifact (see `scripts/build.mjs`) once per
 * process and wraps it as a {@link SimHost}.
 *
 * @throws If the artifact has not been built yet — run
 * `pnpm --filter @gc/wasm build` first.
 */
export function loadSimHost(): SimHost {
  if (cached) {
    return cached;
  }

  const require = createRequire(import.meta.url);
  let native: GcWasmModule;
  try {
    native = require(artifactPath) as GcWasmModule;
  } catch (cause) {
    throw new Error(
      `@gc/wasm: compiled artifact missing at ${artifactPath}. ` +
        'Run "pnpm --filter @gc/wasm build" first (see scripts/build.mjs).',
      { cause },
    );
  }

  const raw = native.__wbg_raw;
  const host: SimHost = {
    Session: native.Session,
    protocolVocabularyId: native.protocolVocabularyId,
    decodeControlMessageHeader: native.decodeControlMessageHeader,
    runDeterminismEvidence: native.runDeterminismEvidence,
    memory: raw.memory,
    buildRenderFrame(handle: number): Float64Array | null {
      const ok = raw.render_frame_build(handle);
      if (ok === 0) {
        return null;
      }
      const ptr = raw.render_frame_ptr();
      const len = raw.render_frame_len();
      // `ptr` is already a byte offset (Vec<f64>::as_ptr() cast to u32);
      // `len` is an element count. A fresh view every call, never cached:
      // `raw.memory.buffer` is replaced whenever wasm linear memory grows,
      // and a stale view would point at a detached ArrayBuffer.
      return new Float64Array(raw.memory.buffer, ptr, len);
    },
  };
  cached = host;
  return host;
}
