#!/usr/bin/env node
// Builds the `gc-wasm` crate to `wasm32-unknown-unknown`, runs `wasm-bindgen`
// to generate its JS/TS glue, and patches that glue so the per-frame RAW
// exports (`render_frame_build`/`render_frame_ptr`/`render_frame_len`) and
// `memory` are reachable from the SAME `WebAssembly.Instance` the ergonomic
// wasm-bindgen classes use. See `src/index.ts`'s doc for why one instance,
// not two, is required: `Session`'s registry handle
// (`v2/rust/crates/gc-wasm/src/registry.rs`) only means anything against the
// instance that created it.
//
// wasm-bindgen's `--target nodejs` output never re-exports its internal
// `wasm` binding (the instantiated module's raw exports object) — there is
// no documented way to reach it besides this: append one line assigning it
// to `exports.__wbg_raw` after the file's own instantiation code has run.
// That is the one, deliberate, documented patch this script applies;
// nothing else in wasm-bindgen's output is touched.
//
// Also renames the generated `.js`/`.d.ts` to `.cjs`/`.d.cts`: wasm-bindgen's
// `nodejs` target emits CommonJS (`exports.X = …`, `require(…)`), and this
// package is `"type": "module"`, so a plain `.js` extension here would be
// parsed as ESM and fail on the very first `require` call.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const packageDir = join(here, "..");
const rustDir = join(packageDir, "..", "..", "..", "rust");
const targetDir = process.env.CARGO_TARGET_DIR ?? join(rustDir, "target");
const outDir = join(packageDir, "dist", "pkg");

console.log("[gc-wasm] cargo build --release --target wasm32-unknown-unknown -p gc-wasm");
execFileSync(
  "cargo",
  ["build", "--release", "--target", "wasm32-unknown-unknown", "-p", "gc-wasm"],
  { cwd: rustDir, stdio: "inherit", env: process.env },
);

const wasmArtifact = join(targetDir, "wasm32-unknown-unknown", "release", "gc_wasm.wasm");
if (!existsSync(wasmArtifact)) {
  throw new Error(`[gc-wasm] expected build artifact missing: ${wasmArtifact}`);
}

mkdirSync(outDir, { recursive: true });

console.log("[gc-wasm] wasm-bindgen --target nodejs");
execFileSync(
  "wasm-bindgen",
  ["--target", "nodejs", "--out-dir", outDir, "--out-name", "gc_wasm", wasmArtifact],
  { stdio: "inherit" },
);

const jsPath = join(outDir, "gc_wasm.js");
const cjsPath = join(outDir, "gc_wasm.cjs");
const dtsPath = join(outDir, "gc_wasm.d.ts");
const dctsPath = join(outDir, "gc_wasm.d.cts");

let source = readFileSync(jsPath, "utf8");
source = source.replace(
  '/* @ts-self-types="./gc_wasm.d.ts" */',
  '/* @ts-self-types="./gc_wasm.d.cts" */',
);
source += `
// --- Patched by v2/ts/packages/wasm/scripts/build.mjs ---
// Exposes this module's own WebAssembly.Instance exports (raw ABI: memory,
// and gc-wasm's render_export.rs raw per-frame functions) alongside the
// ergonomic wasm-bindgen classes above, so both reach the SAME instance.
// See this package's src/index.ts.
exports.__wbg_raw = wasm;
`;
writeFileSync(cjsPath, source);
rmSync(jsPath);

let dts = readFileSync(dtsPath, "utf8");
dts += `
// --- Patched by v2/ts/packages/wasm/scripts/build.mjs ---
// Mirrors gc_wasm_bg.wasm.d.ts's raw ABI for exactly the functions
// src/index.ts's hot render path needs.
export const __wbg_raw: {
    readonly memory: WebAssembly.Memory;
    render_frame_build(handle: number, kickFollowSlots: number): number;
    render_frame_ptr(): number;
    render_frame_len(): number;
};
`;
writeFileSync(dctsPath, dts);
rmSync(dtsPath);

console.log(`[gc-wasm] wrote ${cjsPath}`);
