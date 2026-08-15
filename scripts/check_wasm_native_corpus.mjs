#!/usr/bin/env node
// #517's seeded native-vs-wasm differential: runs `gc_sim::wasm_native_corpus`'s
// corpus through BOTH targets and diffs them tick for tick.
//
// WHAT BREAKS WITHOUT THIS. `gc-sim` calls `sin`/`cos`/`exp`/`ln` on thirteen
// per-tick or per-decision paths. Rust links a different libm for
// `wasm32-unknown-unknown` than for native, and none of those functions is
// required to be correctly rounded, so the compiled wasm module can compute
// DIFFERENT simulation state than a native build of the identical source.
// Every determinism gate in this repository (`cargo test`, the OMP-1
// campaign, `session_ai_driven`) runs the comparison natively or pins the
// wasm side against a single frozen scenario, so a divergence that a
// DIFFERENT trajectory would expose is invisible to all of them. #517's own
// scoping comment: this is trajectory-dependent -- WHICH transcendental
// calls fire, with which arguments, moves with the exact match, so a single
// recorded scenario can reproduce a pin for months while the sites it never
// happens to exercise stay silently unconverted.
//
// WHAT THIS COMPARES, and why it is LIVE rather than pinned. For every
// scenario in `gc_sim::wasm_native_corpus::CORPUS` (read from the compiled
// wasm module's own `corpusScenarios()` export -- one source of truth, never
// a second scenario list hardcoded here), this script runs the scenario
// through a FRESH native `cargo test` invocation and through the freshly
// built wasm module, and compares their `tick_hashes` element for element.
// Unlike `gate_determinism`'s OMP-1 comparison (which pins two digests
// captured once and compared forever), nothing here is pinned: both sides
// are computed fresh on every run, against whatever `gc-sim` currently is.
// See `rust/crates/gc-sim/tests/wasm_native_corpus.rs`'s module doc for the
// fuller argument.
//
// WHAT A DIVERGENCE MEANS, AND THE ALLOWLIST. With 13 sites unconverted, this
// corpus DOES currently find real divergences (see KNOWN_DIVERGENCES below) --
// that is the finding #517 exists to make actionable, not a bug in this
// script. A gate that stayed permanently red would block every unrelated PR
// until all 13 sites are converted, which is separate, sequenced work (#517's
// own body: "not by you, and not yet"). So a divergence that matches a
// KNOWN_DIVERGENCES entry exactly (same scenario, same first-divergent tick)
// is reported LOUDLY but does not fail the gate. Anything else does:
//   - a NEW divergence (a scenario not in the allowlist, or at a different
//     tick than recorded) fails hard -- this is the regression the whole
//     mechanism exists to catch, and a silently-growing allowlist would defeat
//     that as surely as no allowlist at all;
//   - a STALE entry (an allowlisted scenario that no longer diverges) also
//     fails hard -- converting a site is supposed to be noticed and
//     celebrated, not silently absorbed by an allowlist nobody revisits.
// This mirrors `check_unstated_knob_shift.mjs`'s ALLOWLIST exactly
// (unallowed / stale, both fail loud) for the same reason: `ExpectedShift::
// Unstated` and "libm disagrees here" are both "yes, we know, here is why, and
// here is what would make us look again."

import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const projectRoot = join(here, "..");
const rustDir = join(projectRoot, "rust");
const require = createRequire(import.meta.url);

// ---------------------------------------------------------------------------
// Known, tracked divergences (#517). Each entry names the scenario id (from
// `gc_sim::wasm_native_corpus::CORPUS`) and the exact tick native and wasm
// first disagree at, MEASURED on the commit this list was last updated --
// see that commit's message for the measurement. A scenario reaching this
// list is evidence of the defect #517 describes, not a defect in the gate.
//
// Update this list ONLY by re-measuring (`node scripts/check_wasm_native_corpus.mjs`
// prints the exact tick on a mismatch) -- never by guessing a tick to make a
// red gate pass.
// ---------------------------------------------------------------------------
// Empty as of #517's mechanical sweep (site 1's match.rs dribble-touch
// cos/sin and site 4's aerial.rs contact-angle cos/sin, plus the other 7
// mechanically-replaceable sites, converted to gc_core::deterministic_math
// / precomputed constants). This corpus's 8 scenarios now agree tick for
// tick, native vs wasm, with nothing allowlisted. Two entries were tracked
// here and removed on that PR: "corpus/short-b" (tick 21, attributed to site
// 1) and "corpus/short-c" (tick 532, attributed to site 4) -- both went
// STALE (stopped reproducing) the moment their attributed site converted,
// which is this allowlist's own bidirectional check working as designed. The
// four `exp`/`ln` sites (#517's remaining scope) could still reintroduce an
// entry here; if one appears, attribute it the same way before adding it.
export const KNOWN_DIVERGENCES = {};

/**
 * Parse `dump_corpus_tick_hashes`'s stdout
 * (`GC_CORPUS_TICK|<id>|<tick>|<hash>` lines) into `Map<id, string[]>`.
 * Pure, so it is unit-testable without running cargo -- see `selfTest`.
 */
export function parseNativeDump(stdout) {
  const byScenario = new Map();
  for (const line of stdout.split("\n")) {
    if (!line.startsWith("GC_CORPUS_TICK|")) continue;
    const parts = line.split("|");
    if (parts.length !== 4) {
      throw new Error(`malformed GC_CORPUS_TICK line: ${line}`);
    }
    const [, id, tickStr, hash] = parts;
    const tick = Number(tickStr);
    if (!Number.isInteger(tick) || tick < 0) {
      throw new Error(`malformed tick in GC_CORPUS_TICK line: ${line}`);
    }
    if (!byScenario.has(id)) byScenario.set(id, []);
    byScenario.get(id)[tick] = hash;
  }
  return byScenario;
}

/**
 * The first index at which `a` and `b` disagree, or -1 if one is a prefix of
 * the other or they are equal -- the same rule
 * `gc_sim::wasm_native_corpus::first_divergence` implements natively, kept in
 * lockstep by `every_targeted_site_is_reached_by_the_corpus`'s sibling test
 * asserting on real data, not by a shared implementation (there is no
 * practical way to share code between Rust and this script for one ten-line
 * function without a much larger cross-language build step).
 */
export function firstDivergence(a, b) {
  const len = Math.min(a.length, b.length);
  for (let i = 0; i < len; i++) {
    if (a[i] !== b[i]) return i;
  }
  return -1;
}

/**
 * Compare one scenario's native and wasm tick-hash vectors against
 * `allowlist`. Pure -- no cargo, no wasm module -- so `selfTest` can drive it
 * directly. Returns one of:
 *   `{ status: "agree" }`
 *   `{ status: "known", tick }`      -- diverges, matches the allowlist
 *   `{ status: "new", tick }`        -- diverges, NOT allowlisted (or at a
 *                                       different tick than recorded)
 *   `{ status: "stale" }`            -- allowlisted, but no longer diverges
 */
export function compareScenario(id, nativeHashes, wasmHashes, allowlist) {
  const divergedAt = firstDivergence(nativeHashes, wasmHashes);
  const allowed = allowlist[id];
  if (divergedAt === -1) {
    if (allowed) return { status: "stale" };
    return { status: "agree" };
  }
  if (allowed && allowed.tick === divergedAt) {
    return { status: "known", tick: divergedAt };
  }
  return { status: "new", tick: divergedAt };
}

function runNativeDump() {
  const stdout = execFileSync(
    "cargo",
    [
      "test",
      "-p",
      "gc-sim",
      "--test",
      "wasm_native_corpus",
      "--",
      "--ignored",
      "--nocapture",
      "dump_corpus_tick_hashes",
    ],
    { cwd: rustDir, encoding: "utf8" },
  );
  return parseNativeDump(stdout);
}

function loadWasmModule(pkgDir) {
  const cjsPath = join(pkgDir, "dist", "pkg", "gc_wasm.cjs");
  return require(cjsPath);
}

function runDifferential(pkgDir, allowlist) {
  const wasm = loadWasmModule(pkgDir);
  const scenarios = wasm.corpusScenarios();
  if (scenarios.length === 0) {
    throw new Error(
      "corpusScenarios() returned zero scenarios -- the corpus is empty",
    );
  }
  const native = runNativeDump();

  const results = [];
  for (const scenario of scenarios) {
    const nativeHashes = native.get(scenario.id);
    if (!nativeHashes) {
      results.push({ id: scenario.id, status: "missing-native" });
      continue;
    }
    const run = wasm.runCorpusScenario(
      scenario.id,
      scenario.match_seed,
      scenario.bot_seed,
      scenario.ticks,
      scenario.combat_enabled,
    );
    if (run.tick_hashes.length !== scenario.ticks + 1) {
      results.push({
        id: scenario.id,
        status: "malformed",
        detail: `wasm returned ${run.tick_hashes.length} tick hashes, want ${scenario.ticks + 1}`,
      });
      continue;
    }
    const comparison = compareScenario(
      scenario.id,
      nativeHashes,
      run.tick_hashes,
      allowlist,
    );
    results.push({ id: scenario.id, ...comparison, ticks: scenario.ticks });
  }
  for (const [id] of native) {
    if (!scenarios.some((s) => s.id === id)) {
      results.push({ id, status: "missing-wasm" });
    }
  }
  return results;
}

function report(results) {
  let unallowedCount = 0;
  let staleCount = 0;
  let knownCount = 0;
  let agreeCount = 0;

  for (const r of results) {
    switch (r.status) {
      case "agree":
        agreeCount++;
        console.log(
          `  ok    ${r.id} (${r.ticks} ticks): native and wasm agree tick for tick`,
        );
        break;
      case "known":
        knownCount++;
        console.log(
          `  KNOWN ${r.id}: diverges at tick ${r.tick} (tracked, #517) -- ${KNOWN_DIVERGENCES[r.id].note}`,
        );
        break;
      case "new":
        unallowedCount++;
        console.error(
          `  FAIL  ${r.id}: diverges at tick ${r.tick}, ` +
            (KNOWN_DIVERGENCES[r.id]
              ? `but the allowlist records tick ${KNOWN_DIVERGENCES[r.id].tick} -- the divergence moved, re-measure before trusting the allowlist entry`
              : `and this scenario is not in KNOWN_DIVERGENCES -- a NEW native/wasm disagreement`),
        );
        break;
      case "stale":
        staleCount++;
        console.error(
          `  FAIL  ${r.id}: KNOWN_DIVERGENCES records a divergence at tick ${KNOWN_DIVERGENCES[r.id].tick}, ` +
            "but native and wasm now agree tick for tick -- remove this allowlist entry (a site was converted, or the trajectory moved past it)",
        );
        break;
      case "missing-native":
        unallowedCount++;
        console.error(
          `  FAIL  ${r.id}: wasm's corpusScenarios() names this scenario, but the native dump did not produce it`,
        );
        break;
      case "missing-wasm":
        unallowedCount++;
        console.error(
          `  FAIL  ${r.id}: the native dump produced this scenario, but wasm's corpusScenarios() does not name it`,
        );
        break;
      case "malformed":
        unallowedCount++;
        console.error(`  FAIL  ${r.id}: ${r.detail}`);
        break;
      default:
        unallowedCount++;
        console.error(
          `  FAIL  ${r.id}: unrecognized comparison status ${r.status}`,
        );
    }
  }

  console.log("");
  if (unallowedCount > 0 || staleCount > 0) {
    console.error(
      `wasm native corpus differential: FAILED (${agreeCount} agree, ${knownCount} known, ${unallowedCount} unallowed, ${staleCount} stale)`,
    );
    return 1;
  }
  console.log(
    `wasm native corpus differential: OK (${agreeCount} agree, ${knownCount} known and tracked (#517), 0 unallowed, 0 stale)`,
  );
  return 0;
}

// ---------------------------------------------------------------------------
// Self-test: proves the comparison/allowlist LOGIC can go red, entirely in
// memory -- no cargo, no wasm module. Per AGENTS.md §9's "a harness self-test
// is not a harness run", this does not substitute for actually running the
// gate; it proves `compareScenario` classifies each shape of disagreement
// correctly, which is the part a fabricated fixture cannot otherwise reach
// (an "unallowed" or "stale" result depends on the CURRENT allowlist, so
// testing it against real, moving corpus data would make the self-test
// itself flake every time a site is converted).
// ---------------------------------------------------------------------------
function selfTest() {
  let failures = 0;
  const check = (label, cond) => {
    if (cond) {
      console.log(`ok  ${label}`);
    } else {
      console.error(`SELF-TEST FAIL: ${label}`);
      failures++;
    }
  };

  const allow = { "corpus/x": { tick: 3, note: "fixture" } };

  check(
    "two identical hash vectors agree",
    compareScenario("corpus/x", ["a", "b", "c"], ["a", "b", "c"], {}).status ===
      "agree",
  );
  check(
    "an unallowlisted divergence is NEW",
    compareScenario("corpus/x", ["a", "b", "c"], ["a", "b", "Z"], {}).status ===
      "new",
  );
  {
    const r = compareScenario(
      "corpus/x",
      ["a", "b", "c", "d"],
      ["a", "b", "c", "Z"],
      allow,
    );
    check(
      "a divergence at the allowlisted tick is KNOWN",
      r.status === "known" && r.tick === 3,
    );
  }
  {
    const r = compareScenario(
      "corpus/x",
      ["a", "Z", "c", "d"],
      ["a", "Y", "c", "d"],
      allow,
    );
    check(
      "a divergence at a DIFFERENT tick than the allowlist records is NEW, not silently accepted",
      r.status === "new" && r.tick === 1,
    );
  }
  check(
    "an allowlisted scenario that no longer diverges is STALE",
    compareScenario("corpus/x", ["a", "b", "c"], ["a", "b", "c"], allow)
      .status === "stale",
  );
  check(
    "firstDivergence treats a shorter agreeing prefix as agreeing",
    firstDivergence(["a", "b"], ["a"]) === -1,
  );
  check(
    "firstDivergence finds the first differing index",
    firstDivergence(["a", "b", "c"], ["a", "Z", "c"]) === 1,
  );

  const dump =
    "GC_CORPUS_TICK|corpus/x|0|aaa\nGC_CORPUS_TICK|corpus/x|1|bbb\nnoise\nGC_CORPUS_DONE|corpus/x|1|bbb|deadbeef\n";
  const parsed = parseNativeDump(dump);
  check(
    "parseNativeDump reads tick hashes and ignores other terminator lines",
    JSON.stringify(parsed.get("corpus/x")) === JSON.stringify(["aaa", "bbb"]),
  );

  let threw = false;
  try {
    parseNativeDump("GC_CORPUS_TICK|corpus/x|not-a-number|aaa\n");
  } catch {
    threw = true;
  }
  check("parseNativeDump rejects a malformed tick", threw);

  return failures === 0 ? 0 : 1;
}

function main(argv) {
  if (argv.includes("--self-test")) {
    return selfTest();
  }
  const pkgDir = join(projectRoot, "ts", "packages", "wasm");
  const results = runDifferential(pkgDir, KNOWN_DIVERGENCES);
  return report(results);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exit(main(process.argv.slice(2)));
}
