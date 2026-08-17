#!/usr/bin/env node
// Surfaces every `ExpectedShift::Unstated` call site outside
// `gc_sim::knob_contract`'s own `noise_floor` path (#499).
//
// WHAT BREAKS WITHOUT THIS. #487/#493 shipped the load-bearing rule -- every
// feature ships a test asserting that moving its knob moves its metric IN THE
// DOCUMENTED DIRECTION -- enforced by `knob_contract::assert_moves` via
// `KnobMoveOpts::expect: ExpectedShift`. `ExpectedShift::Unstated` is a
// structurally-permitted escape hatch: `sign_agrees` is hardcoded `true` for
// it, so a backwards-wired knob certifies as WIRED as long as nobody claims a
// direction. It is deliberately VISIBLE rather than silent -- `ExpectedShift`
// has no `Default` impl, so every call site must spell the variant out, and
// the contract's own report prints "(no direction claimed)" -- and it is
// legitimately needed by `noise_floor`'s internal call, which measures a
// metric's dispersion and has no directional claim to make.
//
// Four gameplay reworks (#488 locomotion, #489 committed actions, #490
// goalkeeper, #491 passing) are about to register a dozen-plus knobs each,
// and under time pressure `Unstated` is the path of least resistance: it is
// visible to a careful reviewer reading the diff, but nothing SURFACES it for
// scrutiny the way a wrong `Increases`/`Decreases` claim gets caught by
// `assert_moves` itself. This is that surfacing: every `expect:
// ExpectedShift::Unstated` field in a `KnobMoveOpts` literal, anywhere under
// `rust/`, except the one inside `noise_floor`'s own body.
//
// WHY "expect: ExpectedShift::Unstated" AND NOT THE BARE ENUM PATH.
// `knob_contract.rs` itself matches on `ExpectedShift::Unstated` twice more,
// inside `knob_moves_metric` -- `ExpectedShift::Unstated => true` (the
// hardcoded `sign_agrees`) and the report string's `" (no direction
// claimed)"` arm. Those are not a feature declining to state a direction;
// they ARE the definition of what declining does. Only a struct-literal field
// assignment (`expect: ExpectedShift::Unstated`) is a CALL SITE -- a place
// where a knob-moves-metric test is being written and a direction is being
// chosen not to be claimed. Anchoring on `expect:` is what tells the two
// apart; a bare-enum grep could not.
//
// WHY "outside noise_floor" AND NOT "outside knob_contract.rs". A future
// helper added to knob_contract.rs itself -- a second pilot function, a
// convenience wrapper -- that constructs `KnobMoveOpts { expect:
// ExpectedShift::Unstated, .. }` for a reason OTHER than "this measures
// dispersion, not direction" deserves the same scrutiny as one added in a
// feature test. Exempting the whole file would hide that. So the exemption is
// scoped to `noise_floor`'s function body by brace-counting from its `fn`
// line, not to the file that contains it.
//
// FRAGILITY, STATED HONESTLY. This reads Rust source with a regular
// expression and finds `noise_floor`'s extent by counting braces, the same
// trade every other regex-based checker in this repo makes explicitly (see
// check_presentation_parity.mjs's own "FRAGILITY, STATED HONESTLY"). It does
// not understand Rust scoping. Two categories of miss, and they are NOT
// equally handled:
//
// - CALL-SITE MATCHING (`CALL_SITE_RE` against raw text). A `//` line
//   containing the literal text `expect: ExpectedShift::Unstated` reads as a
//   call site -- a false positive, costing a reviewer a moment, never a
//   silent miss. But the reverse direction is real and SILENT, not loud: an
//   import alias (`use ExpectedShift as E;` then `expect: E::Unstated`), a
//   `const`/`static` holding the variant and referenced by name, or a helper
//   function that returns `ExpectedShift::Unstated` for the caller to plug
//   in -- none of these contain the literal text this regex matches, so each
//   evades the audit entirely and reports `sites=0` for that occurrence, the
//   same way a metric with no registered extraction function would. Every
//   one of these requires a deliberate rewrite to reach, not an incidental
//   code shape a reviewer would write without thinking about this gate; that
//   is why they are not fixed here, but they are not "the conservative
//   direction" either, and this file used to imply they were.
// - EXEMPTION BOUNDARY (`functionBodyLineRange`'s brace counting). This ONE
//   boundary is load-bearing for the whole audit -- widen it and a real call
//   site vanishes, not just a hypothetical one -- so it gets two independent
//   defenses, not one. First, `stripCommentsAndStrings` blanks `//` line
//   comments and `"..."` string-literal content before any brace is counted,
//   so the shape review first found (a stray `{`/`}` sitting inside a line
//   comment) cannot perturb the count. It does NOT strip raw strings
//   (`r"..."`, `r#"..."#`), byte strings, char literals, or `/* block */`
//   comments -- second defense: if depth has not returned to 0 by the next
//   line that looks like ANY `fn` declaration, AT ANY INDENTATION, that is a
//   hard failure, never "the body must just be long". This has no ceiling on
//   purpose: an earlier version bounded it to "same or lesser indentation
//   than the exempt function's own declaration", reasoning that a NESTED
//   `fn` (deeper indentation) was presumably a legitimate inner item and
//   should not trip it -- review then found a compensating pair of stray
//   char literals (`'{'` at the exempt function's own indentation, `'}'`
//   nested four spaces deeper inside a sibling method in the next `impl`
//   block) that landed brace depth on that DEEPER function's real closing
//   brace, so the ceiling let the widened exemption through in total
//   silence: `sites=0`, no error. The ceiling is gone. The residual cost is
//   real, not hypothetical: a `fn` legitimately nested inside the exempt
//   function's own body now also trips this and forces a hard fail, same as
//   an unhandled construct would. That is accepted on purpose -- it costs a
//   reviewer a moment to special-case, a silent pass costs the audit its
//   purpose entirely, and `noise_floor`/its allowlisted callers do not nest
//   functions today. So an unhandled construct, at any nesting depth,
//   cannot silently widen the exemption into the next function -- it just
//   fails loudly instead of being stripped away quietly. A parse that finds
//   zero `.rs` files, or that cannot locate `noise_floor`'s body at all (for
//   either reason above), is a hard error, never a quiet pass.
//
//   node scripts/check_unstated_knob_shift.mjs                 -- check this repo
//   node scripts/check_unstated_knob_shift.mjs --repo DIR      -- check a copy
//   node scripts/check_unstated_knob_shift.mjs --list-sources  -- print every .rs file it would scan
//   node scripts/check_unstated_knob_shift.mjs --self-test     -- prove it goes red
//
// `--self-test` runs entirely over an IN-MEMORY fixture tree (never the real
// one) and proves: a call site outside the allowlist is rejected and named; a
// stale allowlist entry (the field it excuses no longer uses `Unstated`) is
// rejected and named; the two real allowlist entries, present, are accepted;
// a bare `ExpectedShift::Unstated` match arm with no `expect:` prefix is
// correctly NOT treated as a call site; `noise_floor`'s own use is exempt
// even though it is never allowlisted; a SECOND, undeclared call site smuggled
// inside an already-allowlisted function is still reported unallowed (one
// allowlist entry excuses exactly one occurrence, never every occurrence
// sharing its id); and a stray brace hidden in a comment cannot widen the
// exemption into the next function. `scripts/check.sh`'s own self-test
// additionally drives `--repo` over mutated COPIES of the real tree, so the
// on-disk file walk this uses in the real gate -- which the in-memory
// fixtures above never exercise -- is proved able to go red too.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const KNOB_CONTRACT_SRC = "rust/crates/gc-sim/src/knob_contract.rs";
const EXEMPT_FUNCTION = "noise_floor";

// Directories a source walk under `rust/` must never descend into: build
// output (potentially gigabytes, and none of it authored source) and VCS/tool
// metadata.
const EXCLUDED_DIR_NAMES = new Set(["target", ".git", "node_modules"]);

// Every disclosed exception to "a knob declining to state a direction gets
// scrutiny", each with a reason a reviewer can weigh WITHOUT going and
// reading the referenced test. `id` is `<path>::<enclosing fn>`, never a line
// number -- a line number churns on every unrelated edit above it, which
// would make this allowlist demand upkeep no rule actually asks for. A
// function name is what a reviewer reads to judge whether the entry is still
// honest, the same shape `ALLOWED_CONSTANT_FIELDS` in
// `session_legacy_differential.rs` uses for the same reason.
//
// Both entries here are the SAME shape: each test asserts that
// `knob_moves_metric`/`noise_floor` panics on bad input (an unregistered
// knob, too few seeds) BEFORE `opts.expect` is ever read by the direction
// check in `knob_moves_metric`. `expect` is structurally irrelevant to what
// either test proves, so there is no direction to state -- not a direction
// the author declined to state.
export const ALLOWLIST = [
  {
    id: "rust/crates/gc-sim/tests/knob_contract.rs::knob_contract_panics_on_an_unregistered_knob_or_metric",
    reason:
      "asserts an unregistered knob id panics -- knob_contract::knob_moves_metric's " +
      "`unregistered tunable id` assert fires before opts.expect is ever read, so no " +
      "directional claim is being tested here",
  },
  {
    id: "rust/crates/gc-sim/tests/knob_contract.rs::knob_contract_refuses_a_seed_set_too_small_to_threshold_against",
    reason:
      "asserts a too-small seed set panics -- MIN_SEEDS is checked before opts.expect is " +
      "ever read, same reasoning as the sibling test above",
  },
  {
    id: "rust/crates/gc-sim/tests/knob_contract.rs::the_post_531_pass_census_reports_against_completion",
    reason:
      "#531 phase 4's PASS_* re-census: reproduces the original #491 census's own " +
      "methodology, which measures every knob against pass_completion in BOTH directions " +
      "to find whichever pairing is closest to WIRED, exactly like noise_floor's own " +
      "allowed Unstated use above -- the census's job is to report which knobs move at " +
      "all, not to assert one direction in advance. It uses knob_moves_metric (report, " +
      "never panics) rather than assert_moves, and any knob it finds newly WIRED gets a " +
      "separate, directional, committed contract elsewhere in the same file " +
      "(a_tighter_receiver_ceiling_lowers_completion_now_the_cone_reaches_every_producer, " +
      "a_lower_pass_speed_floor_raises_completion_once_dilution_drops) -- this test is " +
      "the ignored measurement pilot behind those two, not a substitute for them.",
  },
];

// Comfortably below the count when this gate was written (265 tracked `.rs`
// files under `rust/`, `git ls-files -- 'rust/**/*.rs' | grep -v /target/`).
// Guards the same hole every other floor in this repo's checkers guards: a
// file walk that silently matched nothing still finds zero call sites and
// exits 0, which is indistinguishable from a clean tree unless something
// requires the walk to have found a plausible number of files.
export const MIN_RUST_FILES = 250;

class AuditError extends Error {}
function fail(message) {
  throw new AuditError(message);
}

// ---------------------------------------------------------------------------
// Source access. `DiskRepo` walks a checkout; `MemoryRepo` serves an in-memory
// map so `--self-test` can build fixture trees without touching a tree.
// ---------------------------------------------------------------------------

class DiskRepo {
  constructor(root) {
    this.root = root;
  }

  // Every `.rs` file under `<root>/rust`, `target`/`.git`/`node_modules`
  // pruned. Deliberately a plain recursive walk rather than `git ls-files`:
  // the on-disk self-test scenario in scripts/check.sh drives this over a
  // plain directory of copied files, not a git checkout, and the real gate
  // must walk the same way both times.
  list() {
    const rustDir = join(this.root, "rust");
    const out = [];
    const walk = (dir, relDir) => {
      let entries;
      try {
        entries = readdirSync(dir, { withFileTypes: true });
      } catch {
        return;
      }
      for (const entry of entries) {
        if (entry.isDirectory()) {
          if (EXCLUDED_DIR_NAMES.has(entry.name)) continue;
          walk(join(dir, entry.name), relDir ? `${relDir}/${entry.name}` : entry.name);
        } else if (entry.isFile() && entry.name.endsWith(".rs")) {
          out.push(relDir ? `rust/${relDir}/${entry.name}` : `rust/${entry.name}`);
        }
      }
    };
    walk(rustDir, "");
    return out.sort();
  }

  read(relPath) {
    try {
      return readFileSync(join(this.root, relPath), "utf8");
    } catch {
      return fail(`cannot read ${relPath} under ${this.root}`);
    }
  }
}

class MemoryRepo {
  constructor(files) {
    this.files = files; // Map<relPath, text>
  }

  list() {
    return [...this.files.keys()]
      .filter((relPath) => relPath.startsWith("rust/") && relPath.endsWith(".rs"))
      .sort();
  }

  read(relPath) {
    const text = this.files.get(relPath);
    if (text === undefined) {
      return fail(`cannot read ${relPath} (memory repo)`);
    }
    return text;
  }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

// The exact shape a `KnobMoveOpts` field assignment takes. See the header's
// "WHY expect: ExpectedShift::Unstated AND NOT THE BARE ENUM PATH" for why
// this is anchored on the field name rather than the bare variant path.
const CALL_SITE_RE = /expect\s*:\s*ExpectedShift::Unstated\b/g;

const FN_DECL_RE = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)/;

function lineIndexOf(text, charIndex) {
  let line = 0;
  for (let i = 0; i < charIndex; i += 1) {
    if (text[i] === "\n") line += 1;
  }
  return line; // 0-indexed
}

// Blanks a `//` line comment and any `"..."` string-literal content out of
// one line, so a stray `{`/`}` inside either cannot perturb the brace count
// below. Not a full lexer: raw strings (`r"..."`, `r#"..."#`), byte strings
// (`b"..."`), char literals, and `/* block */` comments are NOT recognized --
// see the header's "FRAGILITY, STATED HONESTLY" for why a brace imbalance
// from one of those is still caught, just later, by the top-level `fn`
// boundary in `functionBodyLineRange` rather than by this function.
function stripCommentsAndStrings(line) {
  let out = "";
  let inString = false;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (inString) {
      if (ch === "\\") {
        i += 1; // an escaped character (\" or \\) never ends the string
        continue;
      }
      if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === "/" && line[i + 1] === "/") {
      break; // the rest of the line is a line comment
    }
    out += ch;
  }
  return out;
}

// The [startLine, endLine] (0-indexed, inclusive) of `fnName`'s body in
// `text`, found by counting braces -- over comment- and string-stripped text,
// see `stripCommentsAndStrings` -- from its `fn` declaration line. Not a real
// parser -- see the header's "FRAGILITY, STATED HONESTLY". Bounded: if the
// brace depth has not returned to 0 by the next line that looks like ANY
// `fn` declaration, at ANY indentation, that is treated as a parse failure,
// not as "the body must just be long" -- this is what stops an unhandled
// construct (a block comment, a raw string, a char literal) from silently
// widening the exemption into a second function, at any nesting depth, not
// just a sibling top-level one. No indentation ceiling: a legitimately
// nested `fn` declared inside `fnName`'s own body would also trip this and
// force a hard fail rather than being exempted -- an intentional trade, the
// same one the rest of this function already makes, because a hard fail
// costs a reviewer a moment and a silent pass costs the audit its purpose.
// Returns `null` if the function cannot be found, its braces never balance,
// or that boundary is crossed; callers must treat `null` as a hard failure,
// never as "found nothing to exempt".
function functionBodyLineRange(text, fnName) {
  const lines = text.split("\n");
  const declRe = new RegExp(`\\bfn\\s+${fnName}\\s*\\(`);
  let startLine = -1;
  for (let i = 0; i < lines.length; i += 1) {
    if (declRe.test(lines[i])) {
      startLine = i;
      break;
    }
  }
  if (startLine === -1) return null;

  let depth = 0;
  let opened = false;
  for (let i = startLine; i < lines.length; i += 1) {
    if (i > startLine && FN_DECL_RE.test(lines[i])) {
      // Another `fn` declaration reached before our own braces balanced --
      // the count is off (an unhandled comment/string/char-literal
      // construct, almost certainly), and continuing would exempt straight
      // into this next function's body, at whatever depth it sits. Refuse
      // rather than guess, regardless of its indentation relative to ours.
      return null;
    }
    for (const ch of stripCommentsAndStrings(lines[i])) {
      if (ch === "{") {
        depth += 1;
        opened = true;
      } else if (ch === "}") {
        depth -= 1;
      }
    }
    if (opened && depth === 0) {
      return [startLine, i];
    }
  }
  return null;
}

// The nearest enclosing `fn` name above `lineIdx`, or `<top-level>`. This is a
// convenience for building a stable, readable allowlist id -- not real Rust
// scope resolution, so a closure or a nested `fn` could confuse it. Every
// known call site today is a flat `#[test] fn ...`, which this handles
// correctly.
function enclosingFunctionName(lines, lineIdx) {
  for (let i = lineIdx; i >= 0; i -= 1) {
    const m = FN_DECL_RE.exec(lines[i]);
    if (m) return m[1];
  }
  return "<top-level>";
}

// Every `expect: ExpectedShift::Unstated` call site in `text`, minus any
// falling inside `exemptRange` (0-indexed, inclusive line range, or `null`).
function callSitesIn(relPath, text, exemptRange) {
  const lines = text.split("\n");
  const sites = [];
  for (const match of text.matchAll(CALL_SITE_RE)) {
    const line = lineIndexOf(text, match.index);
    if (exemptRange && line >= exemptRange[0] && line <= exemptRange[1]) {
      continue;
    }
    sites.push({
      relPath,
      line: line + 1,
      fn: enclosingFunctionName(lines, line),
      text: lines[line].trim(),
    });
    void match; // matchAll's captured groups are unused; the index is what matters.
  }
  return sites;
}

function siteId(site) {
  return `${site.relPath}::${site.fn}`;
}

// The audit, over any `repo` (disk or memory) and any `allowlist` (the real
// one, or a self-test fixture's own). Pure given those two inputs, so the
// self-test can drive it directly rather than shelling out to itself.
export function auditUnstatedKnobShift(repo, allowlist) {
  const files = repo.list();
  if (files.length === 0) {
    fail("found no rust/**/*.rs files -- the file walk is broken, or the tree is empty");
  }

  const sites = [];
  for (const relPath of files) {
    const text = repo.read(relPath);
    let exemptRange = null;
    if (relPath === KNOB_CONTRACT_SRC) {
      exemptRange = functionBodyLineRange(text, EXEMPT_FUNCTION);
      if (exemptRange === null) {
        fail(
          `could not locate ${EXEMPT_FUNCTION}'s function body inside ${relPath} -- the one ` +
            "exemption this audit relies on cannot be verified, so it refuses to proceed as if " +
            "nothing were exempt",
        );
      }
    }
    sites.push(...callSitesIn(relPath, text, exemptRange));
  }

  // Per-OCCURRENCE, not per-id. `id` (`<path>::<fn>`) names WHICH allowlist
  // entry can excuse a site, but one entry excuses exactly ONE occurrence,
  // never every occurrence that happens to share its id. A `Set`/`Map` keyed
  // on id alone cannot express that: once an id is "present", every site
  // sharing it reads as covered no matter how many there are -- which is
  // exactly how a second, undeclared `expect: ExpectedShift::Unstated`
  // smuggled inside an already-allowlisted function passed review silently.
  // Counting occurrences per id closes that: an id with 2 sites and 1
  // allowlist entry reports exactly 1 of them unallowed, not 0.
  const allowedCountById = new Map();
  for (const entry of allowlist) {
    allowedCountById.set(entry.id, (allowedCountById.get(entry.id) ?? 0) + 1);
  }

  const seenCountById = new Map();
  const unallowed = [];
  for (const site of sites) {
    const id = siteId(site);
    const seen = seenCountById.get(id) ?? 0;
    const allowed = allowedCountById.get(id) ?? 0;
    if (seen < allowed) {
      seenCountById.set(id, seen + 1);
    } else {
      unallowed.push(site);
    }
  }

  // Symmetric check for staleness: an id can have MORE allowlist entries than
  // actual sites too (a site was deleted, or two entries were ever written
  // for one id by mistake) -- the entries beyond what's actually found are
  // stale, not just entries whose id has zero sites at all.
  const totalFoundById = new Map();
  for (const site of sites) {
    const id = siteId(site);
    totalFoundById.set(id, (totalFoundById.get(id) ?? 0) + 1);
  }

  const usedCountById = new Map();
  const stale = [];
  for (const entry of allowlist) {
    const found = totalFoundById.get(entry.id) ?? 0;
    const used = usedCountById.get(entry.id) ?? 0;
    if (used < found) {
      usedCountById.set(entry.id, used + 1);
    } else {
      stale.push(entry);
    }
  }

  return { filesScanned: files.length, sites, unallowed, stale };
}

function renderProblems(result) {
  const lines = [];
  for (const site of result.unallowed) {
    lines.push(
      `undeclared: ${site.relPath}:${site.line} in ${site.fn}() -- \`${site.text}\` is not in ALLOWLIST`,
    );
  }
  for (const entry of result.stale) {
    lines.push(`stale allowlist entry: ${entry.id} (${entry.reason})`);
  }
  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Self-test
// ---------------------------------------------------------------------------

const FIXTURE_KNOB_CONTRACT_SRC = `
//! fixture: gc_sim::knob_contract
pub enum ExpectedShift { Increases, Decreases, Unstated }

/// The one legitimate directionless call site: measuring dispersion, not a claim.
pub fn noise_floor(metric: &'static str, seeds: &[f64]) -> NoiseFloor {
    let opts = KnobMoveOpts {
        knob: "",
        metric,
        expect: ExpectedShift::Unstated,
    };
    measure(&opts)
}

/// Implements what each variant MEANS. Neither arm below is a call site --
/// a match arm carries no "expect" field, only the real thing does.
pub fn knob_moves_metric(opts: &KnobMoveOpts) -> Outcome {
    let sign_agrees = match opts.expect {
        ExpectedShift::Increases => true,
        ExpectedShift::Decreases => false,
        ExpectedShift::Unstated => true,
    };
    let claim = match opts.expect {
        ExpectedShift::Increases => " (expected to increase)",
        ExpectedShift::Decreases => " (expected to decrease)",
        ExpectedShift::Unstated => " (no direction claimed)",
    };
    Outcome { sign_agrees, claim }
}
`;

const FIXTURE_KNOB_CONTRACT_TESTS = `
use gc_sim::knob_contract::{self, ExpectedShift, KnobMoveOpts};

#[test]
fn knob_contract_panics_on_an_unregistered_knob_or_metric() {
    let outcome = std::panic::catch_unwind(|| {
        knob_contract::knob_moves_metric(&KnobMoveOpts {
            knob: "NOT_A_KNOB",
            expect: ExpectedShift::Unstated,
        })
    });
}

#[test]
fn knob_contract_refuses_a_seed_set_too_small_to_threshold_against() {
    let outcome = std::panic::catch_unwind(|| {
        knob_contract::knob_moves_metric(&KnobMoveOpts {
            knob: "AI_SHOOT_RANGE",
            expect: ExpectedShift::Unstated,
        })
    });
}
`;

function fixtureFiles() {
  return new Map([
    [KNOB_CONTRACT_SRC, FIXTURE_KNOB_CONTRACT_SRC],
    ["rust/crates/gc-sim/tests/knob_contract.rs", FIXTURE_KNOB_CONTRACT_TESTS],
  ]);
}

const FIXTURE_ALLOWLIST = [
  {
    id: "rust/crates/gc-sim/tests/knob_contract.rs::knob_contract_panics_on_an_unregistered_knob_or_metric",
    reason: "fixture: panics before opts.expect is read",
  },
  {
    id: "rust/crates/gc-sim/tests/knob_contract.rs::knob_contract_refuses_a_seed_set_too_small_to_threshold_against",
    reason: "fixture: panics before opts.expect is read",
  },
];

function expectGreen(label, files, allowlist) {
  const repo = new MemoryRepo(files);
  try {
    const result = auditUnstatedKnobShift(repo, allowlist);
    if (result.unallowed.length > 0 || result.stale.length > 0) {
      console.error(`FAIL (expected green): ${label}`);
      console.error(`  ${renderProblems(result)}`);
      return false;
    }
    console.log(`ok  ${label}`);
    return true;
  } catch (error) {
    console.error(`FAIL (expected green, but the audit could not run): ${label}: ${error.message}`);
    return false;
  }
}

function expectRed(label, files, allowlist, messagePattern) {
  const repo = new MemoryRepo(files);
  let rendered;
  try {
    const result = auditUnstatedKnobShift(repo, allowlist);
    if (result.unallowed.length === 0 && result.stale.length === 0) {
      console.error(`FAIL (expected red): ${label}: the audit found nothing wrong`);
      return false;
    }
    rendered = renderProblems(result);
  } catch (error) {
    if (!(error instanceof AuditError)) throw error;
    rendered = error.message;
  }
  if (messagePattern.test(rendered)) {
    console.log(`ok  ${label} (rejected: ${rendered.split("\n")[0]})`);
    return true;
  }
  console.error(`FAIL: ${label}: rejected, but not for the expected reason`);
  console.error(`  got: ${rendered}`);
  return false;
}

function selfTest() {
  let ok = true;
  const files = fixtureFiles();

  ok = expectGreen("a clean fixture with its matching allowlist is accepted", files, FIXTURE_ALLOWLIST) && ok;

  // noise_floor's own use is exempt WITHOUT being allowlisted: an empty
  // allowlist should still leave it uncaught, because the exemption is
  // structural (the function body), not a disclosed gap.
  ok =
    expectGreen(
      "noise_floor's own directionless measurement needs no allowlist entry",
      new Map([[KNOB_CONTRACT_SRC, FIXTURE_KNOB_CONTRACT_SRC]]),
      [],
    ) && ok;

  // The match-arm-only occurrences inside knob_moves_metric (no `expect:`
  // prefix) must never be mistaken for call sites, even unallowlisted, even
  // outside noise_floor. Scenario 1 above already proves this implicitly (an
  // empty allowlist there would fail otherwise); this makes it explicit and
  // named.
  {
    const soleSrc = new Map([[KNOB_CONTRACT_SRC, FIXTURE_KNOB_CONTRACT_SRC]]);
    const result = auditUnstatedKnobShift(new MemoryRepo(soleSrc), []);
    if (result.sites.length !== 0) {
      console.error(
        `FAIL: the match-arm scenario found ${result.sites.length} call site(s) outside noise_floor when it should find 0 -- the CALL_SITE_RE regex is matching bare match arms, not just field assignments`,
      );
      ok = false;
    } else {
      console.log("ok  bare `ExpectedShift::Unstated` match arms (no `expect:` prefix) are never treated as call sites");
    }
  }

  // A new call site, outside the allowlist: the load-bearing case. A feature
  // test declining to state a direction must be named, not waved through.
  {
    const withNewSite = new Map(files);
    withNewSite.set(
      "rust/crates/gc-sim/tests/locomotion_knob.rs",
      `
use gc_sim::knob_contract::{self, ExpectedShift, KnobMoveOpts};

#[test]
fn locomotion_top_speed_moves_sprint_distance() {
    let outcome = knob_contract::knob_moves_metric(&KnobMoveOpts {
        knob: "LOCOMOTION_TOP_SPEED",
        expect: ExpectedShift::Unstated,
    });
}
`,
    );
    ok =
      expectRed(
        "a new feature test declining to state a direction is named, not waved through",
        withNewSite,
        FIXTURE_ALLOWLIST,
        /locomotion_knob\.rs.*locomotion_top_speed_moves_sprint_distance/s,
      ) && ok;
  }

  // A stale allowlist entry: the test that used to decline a direction now
  // states one (or was deleted, or was renamed). The entry must be rejected,
  // not left quietly excusing a call site that no longer exists.
  {
    const fixed = new Map(files);
    fixed.set(
      "rust/crates/gc-sim/tests/knob_contract.rs",
      FIXTURE_KNOB_CONTRACT_TESTS.replace(
        'knob: "AI_SHOOT_RANGE",\n            expect: ExpectedShift::Unstated,',
        'knob: "AI_SHOOT_RANGE",\n            expect: ExpectedShift::Increases,',
      ),
    );
    if (fixed.get("rust/crates/gc-sim/tests/knob_contract.rs") === FIXTURE_KNOB_CONTRACT_TESTS) {
      console.error("FAIL: the stale-allowlist fixture edit did not change anything -- the scenario no longer reproduces staleness");
      ok = false;
    } else {
      ok =
        expectRed(
          "a stale allowlist entry (its call site no longer declines a direction) is rejected",
          fixed,
          FIXTURE_ALLOWLIST,
          /stale allowlist entry.*knob_contract_refuses_a_seed_set_too_small_to_threshold_against/s,
        ) && ok;
    }
  }

  // An allowlist entry naming a function that was deleted outright is the
  // same failure mode as "no longer declines a direction" -- both must be
  // rejected as stale, not merely the edited-in-place case above.
  {
    const deleted = new Map(files);
    deleted.set(
      "rust/crates/gc-sim/tests/knob_contract.rs",
      "use gc_sim::knob_contract::{self, ExpectedShift, KnobMoveOpts};\n// the function this allowlist entry named was deleted entirely\n",
    );
    ok =
      expectRed(
        "an allowlist entry naming a deleted function is rejected as stale",
        deleted,
        FIXTURE_ALLOWLIST,
        /stale allowlist entry/,
      ) && ok;
  }

  // A tree with no .rs files at all must be a hard failure, never a clean
  // pass over nothing -- AGENTS.md §9's "prints nothing, exits 0" shape.
  ok =
    expectRed(
      "an empty tree (no .rs files found) is a hard failure, not a vacuous pass",
      new Map(),
      [],
      /found no rust\/\*\*\/\*\.rs files/,
    ) && ok;

  // noise_floor renamed or removed: the exemption this whole audit leans on
  // must fail loudly if it can no longer be located, rather than silently
  // exempting nothing (which would make every real call site in that
  // function suddenly "undeclared").
  {
    const renamed = new Map(files);
    renamed.set(KNOB_CONTRACT_SRC, FIXTURE_KNOB_CONTRACT_SRC.replace("fn noise_floor(", "fn measure_noise_floor("));
    ok =
      expectRed(
        "noise_floor renamed out from under the exemption is a hard failure, not a silent pass",
        renamed,
        FIXTURE_ALLOWLIST,
        /could not locate noise_floor's function body/,
      ) && ok;
  }

  // BLOCKING FINDING 1 (PR #511 review): a second, undeclared call site
  // smuggled inside an ALREADY-allowlisted function must still be reported.
  // One allowlist entry excuses exactly one occurrence sharing its id, never
  // every occurrence -- see the per-occurrence counting comment in
  // auditUnstatedKnobShift. Before that fix this fixture passed silently:
  // the id was present in the allowlist, so both the legitimate site and the
  // smuggled one read as covered.
  {
    const smuggledFiles = new Map(files);
    smuggledFiles.set(
      "rust/crates/gc-sim/tests/knob_contract.rs",
      FIXTURE_KNOB_CONTRACT_TESTS.replace(
        '            knob: "NOT_A_KNOB",\n            expect: ExpectedShift::Unstated,\n        })\n    });\n}',
        '            knob: "NOT_A_KNOB",\n            expect: ExpectedShift::Unstated,\n        })\n    });\n' +
          "    // smuggled: a second, undeclared knob test riding along beside the legitimate one\n" +
          "    let smuggled = knob_contract::knob_moves_metric(&KnobMoveOpts {\n" +
          '        knob: "SMUGGLED_KNOB",\n' +
          "        expect: ExpectedShift::Unstated,\n" +
          "    });\n}",
      ),
    );
    if (smuggledFiles.get("rust/crates/gc-sim/tests/knob_contract.rs") === FIXTURE_KNOB_CONTRACT_TESTS) {
      console.error(
        "FAIL: the smuggled-second-site fixture edit did not change anything -- the scenario no longer reproduces the shape",
      );
      ok = false;
    } else {
      const result = auditUnstatedKnobShift(new MemoryRepo(smuggledFiles), FIXTURE_ALLOWLIST);
      const smuggledFn = "knob_contract_panics_on_an_unregistered_knob_or_metric";
      const totalInFn = result.sites.filter((s) => s.fn === smuggledFn).length;
      const unallowedInFn = result.unallowed.filter((s) => s.fn === smuggledFn).length;
      if (result.unallowed.length === 1 && totalInFn === 2 && unallowedInFn === 1) {
        console.log(
          "ok  a second, undeclared call site smuggled inside an already-allowlisted function is reported unallowed",
        );
      } else {
        console.error(
          `FAIL: a second call site smuggled inside ${smuggledFn} should leave 2 sites in that ` +
            `function with exactly 1 unallowed (and 0 unallowed elsewhere), got ${totalInFn} sites ` +
            `in-function / ${unallowedInFn} unallowed in-function / ${result.unallowed.length} unallowed total`,
        );
        ok = false;
      }
    }
  }

  // BLOCKING FINDING 2 (PR #511 review). A single, UNCOMPENSATED stray brace
  // does not discriminate this fix from its absence: with nothing to
  // rebalance it, depth never returns to 0 for the rest of the file either
  // way, so even the ORIGINAL pre-fix code (no stripping, no boundary check
  // at all) falls off the end of the loop and returns `null` -- the same
  // "could not locate noise_floor's function body" a correct fix produces,
  // for an entirely different reason. That fixture used to live here and
  // passed against both. The shape that actually separates them is a
  // COMPENSATING PAIR: one stray `{` in a `/* block comment */` inside
  // `noise_floor` (block comments are not stripped, see FRAGILITY, STATED
  // HONESTLY), one stray `}` in a block comment inside `knob_moves_metric`'s
  // real content -- chosen so the extra +1/-1 cancel out and depth lands
  // EXACTLY on `knob_moves_metric`'s own true closing brace. Against the
  // original pre-fix code this reads as a perfectly balanced (wrongly
  // widened) exemption: no error, a smuggled call site inside
  // `knob_moves_metric` silently vanishes, `sites=0`. Against the current
  // boundary check, `knob_moves_metric`'s declaration line is reached while
  // depth is still 1 (not 0, from the first stray brace), which fires the
  // boundary immediately, before the compensating brace is ever read.
  {
    // Isolated to KNOB_CONTRACT_SRC alone (not `files`, which also carries
    // the unrelated tests file and its 2 always-legitimate call sites) --
    // otherwise a comparison against pre-fix code reads noise from those
    // unrelated sites instead of the exemption's own silent widening.
    const compensatingBracesText = FIXTURE_KNOB_CONTRACT_SRC.replace(
      "    let opts = KnobMoveOpts {",
      "    /* a stray brace hides in this block comment: { */\n    let opts = KnobMoveOpts {",
    ).replace(
      "pub fn knob_moves_metric(opts: &KnobMoveOpts) -> Outcome {",
      "pub fn knob_moves_metric(opts: &KnobMoveOpts) -> Outcome {\n" +
        '    let smuggled = KnobMoveOpts { knob: "SMUGGLED_BLOCK", metric: "m", expect: ExpectedShift::Unstated };\n' +
        "    /* a compensating stray brace hides here: } */",
    );
    const compensatingBraces = new Map([[KNOB_CONTRACT_SRC, compensatingBracesText]]);
    if (compensatingBracesText === FIXTURE_KNOB_CONTRACT_SRC) {
      console.error(
        "FAIL: the compensating-brace-pair fixture edit did not change anything -- the scenario no longer reproduces the shape",
      );
      ok = false;
    } else {
      ok =
        expectRed(
          "a compensating stray-brace pair across two block comments cannot widen the exemption into the next function",
          compensatingBraces,
          [],
          /could not locate noise_floor's function body/,
        ) && ok;
    }
  }

  // BLOCKING FINDING 1, SECOND REVIEW ROUND (PR #511). The boundary check
  // used to fire only for a next `fn` at the SAME OR LESSER indentation as
  // the exempt function's own declaration, on the theory that a deeper
  // indentation meant a legitimately nested item. Review then built a
  // compensating pair of stray CHAR LITERALS -- `'{'` in `noise_floor` at
  // indentation 0, `'}'` inside a sibling method nested four spaces deeper
  // in the NEXT `impl` block -- and depth landed exactly on that impl
  // block's real closing brace: the ceiling let it through in total
  // silence, `sites=0`, no error. Not reachable in today's FLAT
  // knob_contract.rs, but that file is about to grow -- seeded knobs in
  // #488-#491 will very plausibly land inside an `impl` block. The ceiling
  // is gone (see FRAGILITY, STATED HONESTLY); this fixture proves the
  // boundary now fires regardless of the next `fn`'s indentation, so a
  // smuggled call site nested inside that impl block is no longer hidden.
  {
    const nestedImplFixture = `
//! fixture: gc_sim::knob_contract
pub enum ExpectedShift { Increases, Decreases, Unstated }

pub fn noise_floor(metric: &'static str, seeds: &[f64]) -> NoiseFloor {
    let opts = KnobMoveOpts {
        knob: "",
        metric,
        expect: ExpectedShift::Unstated,
    };
    let stray_open = '{'; // a stray char literal brace
    measure(&opts)
}

pub struct Foo;
impl Foo {
    pub fn nested_method(&self) -> bool {
        let smuggled = KnobMoveOpts { knob: "SMUGGLED_NESTED", metric: "m", expect: ExpectedShift::Unstated };
        let stray_close = '}'; // a compensating stray char literal brace
        true
    }
}
`;
    ok =
      expectRed(
        "a compensating char-literal brace pair cannot widen the exemption into a nested impl block, regardless of indentation",
        new Map([[KNOB_CONTRACT_SRC, nestedImplFixture]]),
        [],
        /could not locate noise_floor's function body/,
      ) && ok;
  }

  if (!ok) {
    console.error("unstated knob shift audit self-test: FAILED");
    return 1;
  }
  console.log("unstated knob shift audit self-test: OK");
  return 0;
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

function main(argv) {
  const scriptDir = dirname(fileURLToPath(import.meta.url));
  let root = join(scriptDir, "..");
  let mode = "check";
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--self-test") {
      mode = "self-test";
    } else if (arg === "--list-sources") {
      mode = "list-sources";
    } else if (arg === "--repo") {
      const next = argv[i + 1];
      if (next === undefined) {
        console.error("--repo needs a directory");
        return 2;
      }
      root = next;
      i += 1;
    } else {
      console.error(`unknown argument: ${arg}`);
      return 2;
    }
  }

  try {
    if (mode === "self-test") {
      return selfTest();
    }
    const repo = new DiskRepo(root);
    if (mode === "list-sources") {
      for (const relPath of repo.list()) {
        console.log(relPath);
      }
      return 0;
    }

    const result = auditUnstatedKnobShift(repo, ALLOWLIST);
    const problems = renderProblems(result);

    if (result.unallowed.length > 0) {
      console.error("UNSTATED KNOB SHIFT AUDIT FAILED -- ExpectedShift::Unstated call site(s) outside knob_contract's own noise_floor path are not declared:");
      console.error(problems);
      console.error("");
      console.error("Either state the direction this knob is claimed to push its metric (ExpectedShift::Increases");
      console.error("or ExpectedShift::Decreases), or add a scripts/check_unstated_knob_shift.mjs ALLOWLIST entry");
      console.error("with a written reason a reviewer can weigh without reading the test.");
    }
    if (result.stale.length > 0) {
      if (result.unallowed.length > 0) console.error("");
      console.error("UNSTATED KNOB SHIFT AUDIT FAILED -- stale ALLOWLIST entr(y/ies) no longer match reality:");
      for (const entry of result.stale) {
        console.error(`  - ${entry.id} (${entry.reason})`);
      }
      console.error("");
      console.error("Drop the entry, or the allowlist rots silently and stops meaning anything.");
    }

    for (const entry of ALLOWLIST) {
      if (!result.stale.includes(entry)) {
        console.log(`ok  ${entry.id} is allowlisted: ${entry.reason}`);
      }
    }

    const terminatorDetail =
      result.unallowed.length + result.stale.length > 0
        ? `|detail=${[...result.unallowed.map((s) => siteId(s)), ...result.stale.map((e) => `stale:${e.id}`)].slice(0, 12).join(";")}`
        : "";
    console.log(
      `GC_UNSTATED_KNOB|files=${result.filesScanned}|sites=${result.sites.length}|unallowed=${result.unallowed.length}|stale=${result.stale.length}|allowlisted=${ALLOWLIST.length}${terminatorDetail}`,
    );

    if (result.unallowed.length > 0 || result.stale.length > 0) {
      return 1;
    }
    console.log(`unstated knob shift audit: OK (${result.filesScanned} rust files scanned, ${result.sites.length} known Unstated call site(s), all allowlisted)`);
    return 0;
  } catch (error) {
    if (error instanceof AuditError) {
      console.error(`UNSTATED KNOB SHIFT AUDIT COULD NOT BE CHECKED: ${error.message}`);
      console.error("This is a gate failure, not a pass: the check could not audit what it claims to audit.");
      console.log(`GC_UNSTATED_KNOB|error=${error.message}`);
      return 1;
    }
    throw error;
  }
}

process.exit(main(process.argv.slice(2)));
