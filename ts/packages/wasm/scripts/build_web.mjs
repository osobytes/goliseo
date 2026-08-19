#!/usr/bin/env node
// Builds the SAME `gc-wasm` crate build.mjs already produces
// (`wasm32-unknown-unknown`, release) into a SECOND, browser-shaped
// artifact via `wasm-bindgen --target web` -- see this package's
// task brief (v2 app-shell milestone, W3-J): `build.mjs`'s `--target
// nodejs` output is CommonJS and loads its `.wasm` bytes with a
// synchronous `require('fs').readFileSync` -- neither `require` nor
// `node:fs` exist in a browser, so that artifact cannot be the thing a
// browser entry point imports. `--target web` instead emits plain ESM
// with an async `init()` that `fetch()`-loads `gc_wasm_bg.wasm` (located
// via `new URL(..., import.meta.url)`, which Vite resolves and bundles
// as a static asset automatically -- no bundler-specific wasm plugin
// needed).
//
// This does NOT replace `build.mjs` or touch its output -- `dist/pkg`
// (node target) keeps working exactly as before; this writes a sibling
// `dist/pkg-web` (web target). `@gc/wasm`'s Node-loaded `src/index.ts`
// (`loadSimHost`) is unaffected -- see that file's own header for why
// `Session`'s registry handle is only meaningful against the instance
// that created it, which is exactly why a browser caller needs its own
// loader over ITS OWN instance rather than reaching into this one.
//
// Same raw-exports patch `build.mjs` applies, adapted to the web
// target's shape: `--target web` keeps its instantiated exports in a
// module-scope `let wasm` (not a CommonJS `exports` object), assigned
// only after the async `init()` this target requires actually resolves.
// So instead of one `exports.__wbg_raw = wasm` line (a snapshot, valid
// under `nodejs`'s synchronous `require`-time instantiation), this
// appends a GETTER FUNCTION that reads the live `wasm` binding on
// demand -- safe to import before `init()` resolves; only unsafe to
// CALL before then, same as calling any other export before `init()`.
//
// Rebuild after `build.mjs` (or on its own -- it reuses the SAME
// `wasm32-unknown-unknown` artifact `build.mjs` builds, rebuilding it
// again here only if `build.mjs` has not run yet this session).

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const packageDir = join(here, "..");
const rustDir = join(packageDir, "..", "..", "..", "rust");
const targetDir = process.env.CARGO_TARGET_DIR ?? join(rustDir, "target");
const outDir = join(packageDir, "dist", "pkg-web");

const wasmArtifact = join(targetDir, "wasm32-unknown-unknown", "release", "gc_wasm.wasm");
if (!existsSync(wasmArtifact)) {
  console.log("[gc-wasm] cargo build --release --target wasm32-unknown-unknown -p gc-wasm");
  execFileSync(
    "cargo",
    ["build", "--release", "--target", "wasm32-unknown-unknown", "-p", "gc-wasm"],
    { cwd: rustDir, stdio: "inherit", env: process.env },
  );
}
if (!existsSync(wasmArtifact)) {
  throw new Error(`[gc-wasm] expected build artifact missing: ${wasmArtifact}`);
}

mkdirSync(outDir, { recursive: true });

console.log("[gc-wasm] wasm-bindgen --target web --out-dir dist/pkg-web");
execFileSync(
  "wasm-bindgen",
  ["--target", "web", "--out-dir", outDir, "--out-name", "gc_wasm", wasmArtifact],
  { stdio: "inherit" },
);

const jsPath = join(outDir, "gc_wasm.js");
const dtsPath = join(outDir, "gc_wasm.d.ts");

let source = readFileSync(jsPath, "utf8");
source += `
// --- Patched by ts/packages/wasm/scripts/build_web.mjs ---
// Exposes this module's own WebAssembly.Instance exports (raw ABI: memory,
// and gc-wasm's render_export.rs raw per-frame functions) alongside the
// ergonomic wasm-bindgen classes above, once \`default\`/\`initSync\` has
// resolved -- see this script's header for why a getter, not a snapshot
// binding, is required for the web target.
export function __getRawExports() {
  return wasm;
}
`;
writeFileSync(jsPath, source);

let dts = readFileSync(dtsPath, "utf8");
dts += `
// --- Patched by ts/packages/wasm/scripts/build_web.mjs ---
// Mirrors gc_wasm_bg.wasm.d.ts's raw ABI -- both the SimSession per-frame
// path (\`render_frame_*\`) and the MatchDriverBridge one
// (\`driver_render_frame_*\`, added for #550's online-match wiring:
// \`@gc/app\`'s \`browser_online_wasm_host.ts\` needs these to build a live
// online frame the same way \`@gc/wasm\`'s own Node-target \`SimHost.buildMatchDriverRenderFrame\`
// already does) -- see \`@gc/wasm/src/types.ts\`'s \`RawExports\` for the
// identical shape on the Node target. \`__getRawExports()\` itself always
// returned the WHOLE instantiated module (\`return wasm\`, below) -- this is
// a type-annotation widening only, not a runtime change. Only meaningful to
// call after \`default\`/\`initSync\` has resolved -- see this script's header.
export function __getRawExports(): {
    readonly memory: WebAssembly.Memory;
    render_frame_build(handle: number, kickFollowSlots: number, dispossessedSlots: number): number;
    render_frame_ptr(): number;
    render_frame_len(): number;
    driver_render_frame_ptr(): number;
    driver_render_frame_len(): number;
};
`;
writeFileSync(dtsPath, dts);

console.log(`[gc-wasm] wrote ${jsPath}`);
