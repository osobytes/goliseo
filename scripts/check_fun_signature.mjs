// Fun-signature visibility and its one safe gate (owner request, 2026-08-28).
//
// Two modes, one parser, zero dependencies:
//
//   node scripts/check_fun_signature.mjs check \
//        --baseline <head file> --baseline-base <base file> \
//        --driftlog <head file> --driftlog-base <base file>
//
//     THE GATE. Exits 1 when the frozen fun signature moved
//     (outfield_ai_baseline's `signature`) but docs/design/fun_metrics.md
//     did not change with it. This mechanizes the drift-log ritual the doc
//     already demands ("a sim change that moves the fun signature owes ...
//     an entry here before the baseline is refreshed") -- it does NOT gate
//     the fun value itself: #633's dip (0.325 -> 0.209) was a correct trade
//     for pass-reception correctness, and a value gate would have blocked
//     it. Documentation is the invariant; direction is a judgement.
//
//   node scripts/check_fun_signature.mjs report \
//        --baseline <head file> --baseline-base <base file> \
//        --bands <tunables.rs>
//
//     THE REPORT. Prints a markdown table of the fun score and every
//     banded metric the frozen baseline records -- base vs head, delta,
//     and band residency -- for CI to post on the pull request. The
//     numbers come from the frozen RECORD, not a fresh simulation: the
//     same CI run's `outfield_ai_baseline_reproduces_the_frozen_fixture_
//     exactly` test proves record == simulation on the head, and the base
//     commit's own CI proved it for the base, so this is measurement,
//     not trust.
//
// Both modes parse the machine-written Rust literals (the recorder emits
// the RECORD; `gc_data::tunables::METRICS` is formatted by rustfmt). This
// reads the single authored source rather than keeping a mirrored table --
// if the format ever changes, the parse fails LOUDLY (exit 2), never
// silently wrong.

import { readFileSync } from "node:fs";

function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

function die(msg) {
  console.error(`check_fun_signature: ${msg}`);
  process.exit(2);
}

function parseBaseline(path) {
  const src = readFileSync(path, "utf8");
  const version = src.match(/baseline_version: (\d+),/);
  const signature = src.match(/signature: "([0-9a-f]{16})",/);
  if (!version || !signature) die(`${path}: no baseline_version/signature -- format drifted?`);
  const stats = {};
  const re = /(\w+): OutfieldAiBaselineStat \{\s*n: \d+,\s*mean: ([0-9eE+.-]+),/g;
  for (const m of src.matchAll(re)) stats[m[1]] = Number(m[2]);
  if (!("fun" in stats)) die(`${path}: no 'fun' stat parsed -- format drifted?`);
  return { version: Number(version[1]), signature: signature[1], stats };
}

function parseBands(path) {
  const whole = readFileSync(path, "utf8");
  // Scope to the METRICS array: TunableDef entries also carry `id:` lines,
  // and an unscoped scan pairs a knob's id with the next metric's band.
  const start = whole.indexOf("pub static METRICS");
  if (start < 0) die(`${path}: no METRICS block -- format drifted?`);
  const src = whole.slice(start);
  const bands = {};
  const re = /id: "(\w+)",[\s\S]{0,4000}?band: \[([^\]]+)\],/g;
  for (const m of src.matchAll(re)) {
    const nums = m[2].split(",").map((s) => {
      const t = s.trim();
      return t === "f64::INFINITY" ? Infinity : Number(t);
    });
    if (nums.length === 4 && nums.every((n) => !Number.isNaN(n))) bands[m[1]] = nums;
  }
  if (Object.keys(bands).length < 8) die(`${path}: parsed only ${Object.keys(bands).length} bands -- format drifted?`);
  return bands;
}

function residency(v, band) {
  const [zl, gl, gh, zh] = band;
  if (v >= gl && v <= gh) return "in band";
  if (v < gl) return `below (des ${((v - zl) / (gl - zl)).toFixed(2)})`;
  return zh === Infinity ? "in band" : `ABOVE (des ${((zh - v) / (zh - gh)).toFixed(2)})`;
}

const mode = process.argv[2];
if (mode === "check") {
  const head = readFileSync(arg("baseline"), "utf8");
  const base = readFileSync(arg("baseline-base"), "utf8");
  const sig = (s) => (s.match(/signature: "([0-9a-f]{16})",/) || [])[1];
  if (!sig(head) || !sig(base)) die("no signature parsed -- format drifted?");
  if (sig(head) === sig(base)) {
    console.log(`fun signature drift-log: OK (signature unchanged, ${sig(head)})`);
    process.exit(0);
  }
  const log = readFileSync(arg("driftlog"), "utf8");
  const logBase = readFileSync(arg("driftlog-base"), "utf8");
  if (log === logBase) {
    console.error(
      `fun signature drift-log: the frozen fun signature moved ` +
        `(${sig(base)} -> ${sig(head)}) but docs/design/fun_metrics.md did not change. ` +
        `A moved baseline owes a drift-log entry BEFORE it is refreshed -- see that ` +
        `doc's own ritual and gc_data::outfield_ai_baseline's module doc.`,
    );
    process.exit(1);
  }
  console.log(`fun signature drift-log: OK (signature moved with a drift-log change)`);
  process.exit(0);
} else if (mode === "report") {
  const head = parseBaseline(arg("baseline"));
  const base = parseBaseline(arg("baseline-base"));
  const bands = parseBands(arg("bands"));
  const rows = ["fun", ...Object.keys(bands).filter((k) => k in head.stats)];
  console.log("<!-- fun-signature-report -->");
  console.log("## Fun signature");
  if (head.signature === base.signature) {
    console.log(`\nUnchanged by this PR (baseline v${head.version}, \`${head.signature}\`).`);
    process.exit(0);
  }
  console.log(
    `\nBaseline **v${base.version} → v${head.version}** ` +
      `(\`${base.signature}\` → \`${head.signature}\`); 60-seed frozen battery, ` +
      `verified record == simulation by this run's own baseline test.\n`,
  );
  console.log("| metric | base | head | Δ | band |");
  console.log("|---|---|---|---|---|");
  for (const k of rows) {
    if (!(k in head.stats) || !(k in base.stats)) continue;
    const a = base.stats[k];
    const b = head.stats[k];
    const d = b - a;
    const arrow = Math.abs(d) < 1e-9 ? "·" : `${d > 0 ? "+" : ""}${d.toFixed(3)}`;
    const band = k === "fun" ? "—" : residency(b, bands[k]);
    const mark = band.startsWith("ABOVE") || band.startsWith("below") ? " ⚠️" : "";
    console.log(`| ${k} | ${a.toFixed(3)} | ${b.toFixed(3)} | ${arrow} | ${band}${mark} |`);
  }
  console.log("\nDetails belong in `docs/design/fun_metrics.md`'s drift log (gated).");
  process.exit(0);
}
die(`unknown mode '${mode}' (want 'check' or 'report')`);
