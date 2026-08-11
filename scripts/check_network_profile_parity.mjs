#!/usr/bin/env node
// Cross-language parity gate for SCRIPTED NETWORK IMPAIRMENT (#472).
//
// WHAT BREAKS WITHOUT THIS. The native rollback matrix drives every scenario
// through `gc_sim::network_conditions` under the four profiles authored in
// `rust/crates/gc-data/src/network_profiles.rs`. Browser evidence now drives
// the same scenarios through `ts/packages/transport/src/impairment.ts`. If the
// two impair traffic differently, BOTH SUITES STAY GREEN and they measure
// different things: a browser soak that ran a "stress" link at half the
// authored loss rate reports a clean hour and proves nothing, and a native
// matrix that catches a desync the browser run cannot reproduce looks like a
// browser-only defect. Neither failure announces itself.
//
// ARCHITECTURE.md §4 rule 6 forbids a TypeScript package restating a `gc-data`
// table and allows a carve-out only when the duplicate lands together with a
// cross-language assertion wired into the gate. This is that assertion, and it
// is the third such carve-out after #433 (wire enums) and #447 (presentation
// content).
//
// WHAT IS COMPARED:
//
//   1. PROFILE MEMBERSHIP. The Rust `NetworkProfileName` variants, the Rust
//      `ALL` table's rows, the TypeScript `NetworkProfileName` union, the
//      `NETWORK_PROFILES` keys and the `NETWORK_PROFILE_NAMES` order are all
//      the same set, in the same order.
//   2. PROFILE VALUES. Every one of the seven tuning fields agrees, per
//      profile, exactly. This is the check nobody had: a drifted
//      `independent_loss_rate` throws nothing anywhere.
//   3. THE GENERATOR'S CONSTANTS. `MOD` and `MULT` from
//      `rust/crates/gc-core/src/rng.rs` against `RNG_MOD`/`RNG_MULT` in
//      `impairment_rng.ts`. A different multiplier is a different sequence of
//      impairment decisions from the same seed.
//   4. THE SHARED TRANSCRIPT. `rust/crates/gc-sim/tests/browser_impairment_parity.rs`
//      and `ts/packages/transport/src/impairment_parity.spec.ts` each assert a
//      transcript literal of the same scripted scenarios. Those two literals
//      must be byte-identical -- so a drift is caught even when only one
//      language's tests are run, which is exactly what happens on a
//      TypeScript-only or Rust-only change.
//   5. THE SCENARIO TABLES behind that transcript agree row for row, so the
//      two sides cannot assert the same literal while running different work.
//   6. THE TRANSCRIPT STILL CONTAINS IMPAIRMENT. Two equal literals prove
//      nothing if both sides quietly became a pass-through: the transcript
//      must still record an independent loss, a burst loss, a duplicate and a
//      reordering. This is the "prints nothing, exits 0" guard AGENTS.md §9
//      demands, applied to the evidence rather than to the exit code.
//
// HOW NAMES CROSS. Rust names variants `Omp0Parity`; the profile key is
// `omp0_parity`. The bridge is the PascalCase -> snake_case convention both
// trees follow. A variant whose spelling breaks the convention is a hard
// error, not a guess.
//
// FRAGILITY, STATED HONESTLY. This reads Rust and TypeScript SOURCE with
// regular expressions, so every parse is fail-loud: a missing table, a body
// that yields zero rows, a field that does not parse as a number, a missing
// constant -- each is a hard error naming the file and the symbol. There is no
// path that reports success without having compared a nonzero number of
// profiles, and the gate additionally requires the printed comparison count to
// clear a floor.
//
// Usage:
//   node scripts/check_network_profile_parity.mjs
//   node scripts/check_network_profile_parity.mjs --self-test    -- prove it goes red
//   node scripts/check_network_profile_parity.mjs --list-sources
//   node scripts/check_network_profile_parity.mjs --repo <dir>

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

const RUST_PROFILES = "rust/crates/gc-data/src/network_profiles.rs";
const RUST_RNG = "rust/crates/gc-core/src/rng.rs";
const RUST_TRANSCRIPT = "rust/crates/gc-sim/tests/browser_impairment_parity.rs";
const TS_PROFILES = "ts/packages/transport/src/network_profiles.ts";
const TS_RNG = "ts/packages/transport/src/impairment_rng.ts";
const TS_TRANSCRIPT = "ts/packages/transport/src/impairment_parity.spec.ts";

const ALL_SOURCES = [
  RUST_PROFILES,
  RUST_RNG,
  RUST_TRANSCRIPT,
  TS_PROFILES,
  TS_RNG,
  TS_TRANSCRIPT,
];

// The tuning fields are DERIVED from `gc_data::network_profiles`'s own
// `NetworkProfile` struct at check time -- never listed here.
//
// A hardcoded list was the first version of this file and it was wrong in the
// one way that matters. Adding an eighth tuning field to the authored profiles
// is an ordinary future change; with a fixed list, the new field is added to
// the Rust struct and all four rows, left out of the TypeScript copy
// entirely, and this gate compares the seven names it knows about, finds them
// all in agreement, and prints the same "OK (N comparisons)" line it always
// does. The two tables would have genuinely diverged -- on a value that
// changes what the browser measures -- with every check green. That is the
// precise scenario this gate exists to prevent, so the field set cannot be an
// assumption the gate carries; it has to be read from the source of truth.
//
// `name` is excluded: it is the lookup key (a `NetworkProfileName` variant in
// Rust, the record key in TypeScript), not a tuning value.
const PROFILE_KEY_FIELD = "name";

// Content may only grow. A parse that silently matched fewer rows or fields
// than exist must not pass as a clean run.
const MIN_PROFILES = 4;
const MIN_PROFILE_FIELDS = 7;
const MIN_SCENARIOS = 5;

class ParityError extends Error {}

function fail(message) {
  throw new ParityError(message);
}

// ---------------------------------------------------------------------------
// Source access. `DiskRepo` reads a checkout; `MemoryRepo` serves an in-memory
// map so `--self-test` can mutate sources without touching a tree.
// ---------------------------------------------------------------------------

class DiskRepo {
  constructor(root) {
    this.root = root;
    this.accessed = new Set();
  }

  read(relPath) {
    this.accessed.add(relPath);
    try {
      return readFileSync(join(this.root, relPath), "utf8");
    } catch {
      return fail(`cannot read ${relPath} under ${this.root}`);
    }
  }
}

class MemoryRepo {
  constructor(files) {
    this.files = files;
    this.accessed = new Set();
  }

  read(relPath) {
    this.accessed.add(relPath);
    const text = this.files.get(relPath);
    if (text === undefined) {
      return fail(`cannot read ${relPath} (memory repo)`);
    }
    return text;
  }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

// `Omp0Parity` -> `omp0_parity`. Digits attach to the word they follow.
function snakeCase(pascal) {
  return pascal
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase();
}

// Rust writes `20_260_811.0` and `0.0025`; TypeScript writes `20260811` and
// `0.0025`. Both reduce to the same number, and anything that does not parse
// is an error rather than a NaN that compares unequal for a confusing reason.
function numeric(raw, what) {
  const cleaned = String(raw).replace(/_/g, "").replace(/(f32|f64|i64|u32|usize)$/, "");
  const value = Number(cleaned);
  if (!Number.isFinite(value)) {
    fail(`${what} is not a finite number: ${JSON.stringify(raw)}`);
  }
  return value;
}

// Splits a `<decl> = &[ ... ];` body into one string per struct literal, by
// bracket and brace counting. A regex over the whole thing would stop at the
// first `}` of a nested literal.
function rustSliceEntries(source, decl, relPath) {
  const start = source.indexOf(decl);
  if (start < 0) {
    fail(`${relPath}: no '${decl}' table found`);
  }
  // `= &[`, not the first `&[`: the declaration's TYPE contains one too.
  const assign = source.indexOf("= &[", start);
  if (assign < 0) {
    fail(`${relPath}: '${decl}' has no '= &[' body`);
  }
  const open = assign + 3;
  let depth = 0;
  let end = -1;
  for (let i = open; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "[") {
      depth += 1;
    } else if (ch === "]") {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end < 0) {
    fail(`${relPath}: '${decl}' body has unbalanced brackets`);
  }
  const body = source.slice(open + 1, end);

  const entries = [];
  let braceDepth = 0;
  let entryStart = -1;
  for (let i = 0; i < body.length; i += 1) {
    const ch = body[i];
    if (ch === "{") {
      if (braceDepth === 0) {
        entryStart = i + 1;
      }
      braceDepth += 1;
    } else if (ch === "}") {
      braceDepth -= 1;
      if (braceDepth === 0) {
        entries.push(body.slice(entryStart, i));
      } else if (braceDepth < 0) {
        fail(`${relPath}: '${decl}' body has unbalanced braces`);
      }
    }
  }
  if (entries.length === 0) {
    fail(
      `${relPath}: '${decl}' parsed to zero entries -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }
  return entries;
}

function rustEnumField(entry, field, relPath) {
  const match = new RegExp(
    `(?:^|[\\s,])${field}\\s*:\\s*[A-Za-z_][A-Za-z0-9_]*::([A-Za-z_][A-Za-z0-9_]*)`,
  ).exec(entry);
  if (match === null) {
    fail(`${relPath}: a '${field}' enum field is missing from a table entry`);
  }
  return match[1];
}

function rustStringField(entry, field, relPath) {
  const match = new RegExp(`(?:^|[\\s,])${field}\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)"`).exec(entry);
  if (match === null) {
    fail(`${relPath}: a '${field}' string field is missing from a table entry`);
  }
  return match[1];
}

function rustNumberField(entry, field, relPath) {
  const match = new RegExp(`(?:^|[\\s,])${field}\\s*:\\s*(-?[0-9_]+(?:\\.[0-9_]+)?)`).exec(entry);
  if (match === null) {
    fail(`${relPath}: a '${field}' numeric field is missing from a table entry`);
  }
  return numeric(match[1], `${relPath}: ${field}`);
}

// Reads `const NAME: <ty> = <number>;` -- `pub` optional.
function rustNumberConst(source, name, relPath) {
  const match = new RegExp(
    `(?:pub\\s+)?const ${name}\\s*:\\s*[A-Za-z0-9_]+\\s*=\\s*([0-9_]+)\\s*;`,
  ).exec(source);
  if (match === null) {
    fail(`${relPath}: no 'const ${name}' found`);
  }
  return numeric(match[1], `${relPath}: ${name}`);
}

function tsNumberConst(source, name, relPath) {
  const match = new RegExp(`export const ${name}\\s*(?::\\s*[A-Za-z0-9_]+)?\\s*=\\s*(\\d+)`).exec(
    source,
  );
  if (match === null) {
    fail(`${relPath}: no 'export const ${name}' found`);
  }
  return numeric(match[1], `${relPath}: ${name}`);
}

// Extracts the object-literal body of `export const NAME<: type> = { ... };`.
function tsObjectBody(source, name, relPath) {
  const anchorPattern = new RegExp(`export const ${name}(?![A-Za-z0-9_])`);
  const anchorMatch = anchorPattern.exec(source);
  if (anchorMatch === null) {
    fail(`${relPath}: no 'export const ${name}' found`);
  }
  const anchor = anchorMatch.index;
  const assign = /=\s*\{/.exec(source.slice(anchor));
  if (assign === null) {
    fail(`${relPath}: '${name}' is not followed by an '= {' object literal`);
  }
  const open = anchor + assign.index + assign[0].length - 1;
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, i);
      }
    }
  }
  return fail(`${relPath}: '${name}' object literal has unbalanced braces`);
}

// ---------------------------------------------------------------------------
// Parsing: the authored profile table
// ---------------------------------------------------------------------------

// The declared field names of a `pub struct <name> { ... }`, in declaration
// order. Doc comments, attributes and blank lines are skipped; only lines that
// declare a `pub <field>:` are taken, which is what rustfmt guarantees for a
// plain data struct.
function rustStructFields(source, structName, relPath) {
  const anchor = new RegExp(`pub struct ${structName}\\s*\\{`).exec(source);
  if (anchor === null) {
    fail(`${relPath}: no 'pub struct ${structName}' found`);
  }
  const open = anchor.index + anchor[0].length - 1;
  let depth = 0;
  let end = -1;
  for (let i = open; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end < 0) {
    fail(`${relPath}: 'pub struct ${structName}' has unbalanced braces`);
  }
  const fields = [];
  for (const line of source.slice(open + 1, end).split("\n")) {
    const match = /^\s*pub\s+([a-z_][a-z0-9_]*)\s*:/.exec(line);
    if (match !== null) {
      fields.push(match[1]);
    }
  }
  if (fields.length === 0) {
    fail(
      `${relPath}: 'pub struct ${structName}' parsed to zero fields -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }
  return fields;
}

// The field names present in one struct-literal or object-literal body, taken
// a line at a time. A key this misses becomes a MISSING field against the
// derived set, which is a hard error -- never a silent pass.
function literalFieldNames(body) {
  const names = [];
  for (const line of body.split("\n")) {
    const match = /^\s*([a-z_][a-z0-9_]*)\s*:/.exec(line);
    if (match !== null) {
      names.push(match[1]);
    }
  }
  return names;
}

function rustProfiles(repo) {
  const source = repo.read(RUST_PROFILES);

  // THE FIELD SET IS DERIVED, NEVER ASSUMED -- see PROFILE_KEY_FIELD's comment.
  const declared = rustStructFields(source, "NetworkProfile", RUST_PROFILES);
  if (!declared.includes(PROFILE_KEY_FIELD)) {
    fail(
      `${RUST_PROFILES}: 'NetworkProfile' declares no '${PROFILE_KEY_FIELD}' field, so this checker cannot tell the lookup key from a tuning value`,
    );
  }
  const fields = declared.filter((field) => field !== PROFILE_KEY_FIELD);
  if (fields.length < MIN_PROFILE_FIELDS) {
    fail(
      `${RUST_PROFILES}: 'NetworkProfile' parsed to only ${fields.length} tuning field(s), fewer than the ${MIN_PROFILE_FIELDS} that exist -- the parse has silently narrowed`,
    );
  }

  // The enum's variant list, so a variant with no row (or a row with no
  // variant) is caught rather than silently ignored.
  const enumMatch = /pub enum NetworkProfileName\s*\{([\s\S]*?)\n\}/.exec(source);
  if (enumMatch === null) {
    fail(`${RUST_PROFILES}: no 'pub enum NetworkProfileName' found`);
  }
  const variants = [];
  const variantPattern = /^\s*([A-Z][A-Za-z0-9]*)\s*,\s*$/gm;
  let variantMatch = variantPattern.exec(enumMatch[1]);
  while (variantMatch !== null) {
    variants.push(variantMatch[1]);
    variantMatch = variantPattern.exec(enumMatch[1]);
  }
  if (variants.length === 0) {
    fail(
      `${RUST_PROFILES}: 'NetworkProfileName' parsed to zero variants -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }

  const rows = new Map();
  const order = [];
  for (const entry of rustSliceEntries(source, "pub static ALL:", RUST_PROFILES)) {
    const variant = rustEnumField(entry, "name", RUST_PROFILES);
    const key = snakeCase(variant);
    if (rows.has(key)) {
      fail(`${RUST_PROFILES}: profile '${key}' is authored twice`);
    }
    // Every declared field, and nothing but declared fields. Rust would not
    // compile a row that omitted one, but this checker must not depend on
    // that: it is what proves the derived set is the set actually compared.
    const present = literalFieldNames(entry);
    for (const field of declared) {
      if (!present.includes(field)) {
        fail(`${RUST_PROFILES}: profile '${key}' has no '${field}' row field`);
      }
    }
    for (const extra of present) {
      if (!declared.includes(extra)) {
        fail(
          `${RUST_PROFILES}: profile '${key}' sets '${extra}', which 'pub struct NetworkProfile' does not declare`,
        );
      }
    }
    const values = {};
    for (const field of fields) {
      values[field] = rustNumberField(entry, field, RUST_PROFILES);
    }
    rows.set(key, values);
    order.push({ key, variant });
  }
  for (const variant of variants) {
    if (!rows.has(snakeCase(variant))) {
      fail(
        `${RUST_PROFILES}: 'NetworkProfileName::${variant}' has no row in ALL, so gc-data itself is inconsistent`,
      );
    }
  }
  if (rows.size !== variants.length) {
    fail(`${RUST_PROFILES}: ALL has ${rows.size} rows but the enum declares ${variants.length}`);
  }
  return { rows, order: order.map((entry) => entry.key), fields };
}

function tsProfiles(repo, fields) {
  const source = repo.read(TS_PROFILES);

  const unionMatch = /export type NetworkProfileName\s*=\s*([^;]+);/.exec(source);
  if (unionMatch === null) {
    fail(`${TS_PROFILES}: no 'export type NetworkProfileName' found`);
  }
  const union = [];
  const memberPattern = /"([a-z0-9_]+)"/g;
  let memberMatch = memberPattern.exec(unionMatch[1]);
  while (memberMatch !== null) {
    union.push(memberMatch[1]);
    memberMatch = memberPattern.exec(unionMatch[1]);
  }
  if (union.length === 0) {
    fail(
      `${TS_PROFILES}: 'NetworkProfileName' parsed to zero members -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }

  // The declared shape, checked alongside the rows: a field renamed here and
  // in the rows together would still typecheck, and only a cross-language read
  // can see that gc-data never heard of the new name.
  const interfaceMatch = /export interface NetworkProfile\s*\{([\s\S]*?)\n\}/.exec(source);
  if (interfaceMatch === null) {
    fail(`${TS_PROFILES}: no 'export interface NetworkProfile' found`);
  }
  const interfaceFields = [];
  for (const line of interfaceMatch[1].split("\n")) {
    const match = /^\s*(?:readonly\s+)?([a-z_][a-z0-9_]*)\s*:/.exec(line);
    if (match !== null) {
      interfaceFields.push(match[1]);
    }
  }
  if (interfaceFields.length === 0) {
    fail(
      `${TS_PROFILES}: 'interface NetworkProfile' parsed to zero fields -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }

  const body = tsObjectBody(source, "NETWORK_PROFILES", TS_PROFILES);
  const rows = new Map();
  const order = [];
  const rowPattern = /([a-z0-9_]+)\s*:\s*\{([^{}]*)\}/g;
  let rowMatch = rowPattern.exec(body);
  while (rowMatch !== null) {
    const key = rowMatch[1];
    if (rows.has(key)) {
      fail(`${TS_PROFILES}: profile '${key}' appears twice in NETWORK_PROFILES`);
    }
    // EXACTLY the fields gc-data's struct declares -- not "at least". A field
    // added to the authored profiles and left out here would otherwise be
    // invisible, and one invented here that gc-data has never heard of would
    // silently do nothing.
    const present = literalFieldNames(rowMatch[2]);
    for (const extra of present) {
      if (!fields.includes(extra)) {
        fail(
          `${TS_PROFILES}: profile '${key}' declares '${extra}', which gc-data's 'pub struct NetworkProfile' does not -- the browser is tuned by a value nothing authors`,
        );
      }
    }
    const values = {};
    for (const field of fields) {
      const fieldMatch = new RegExp(`${field}\\s*:\\s*(-?[0-9.]+)`).exec(rowMatch[2]);
      if (fieldMatch === null) {
        fail(
          `${TS_PROFILES}: profile '${key}' has no '${field}' -- gc-data's 'pub struct NetworkProfile' declares it, so the browser's copy has silently diverged from the profiles the native matrix runs`,
        );
      }
      values[field] = numeric(fieldMatch[1], `${TS_PROFILES}: ${key}.${field}`);
    }
    rows.set(key, values);
    order.push(key);
    rowMatch = rowPattern.exec(body);
  }
  if (rows.size === 0) {
    fail(
      `${TS_PROFILES}: 'NETWORK_PROFILES' parsed to zero entries -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }

  const namesMatch = /export const NETWORK_PROFILE_NAMES[^=]*=\s*\[([^\]]*)\]/.exec(source);
  if (namesMatch === null) {
    fail(`${TS_PROFILES}: no 'export const NETWORK_PROFILE_NAMES' array found`);
  }
  const names = [];
  const namePattern = /"([a-z0-9_]+)"/g;
  let nameMatch = namePattern.exec(namesMatch[1]);
  while (nameMatch !== null) {
    names.push(nameMatch[1]);
    nameMatch = namePattern.exec(namesMatch[1]);
  }
  if (names.length === 0) {
    fail(
      `${TS_PROFILES}: 'NETWORK_PROFILE_NAMES' parsed to zero entries -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }
  return { rows, order, union, names, interfaceFields };
}

// ---------------------------------------------------------------------------
// Parsing: the shared transcript and the scenarios behind it
// ---------------------------------------------------------------------------

function rustTranscript(repo) {
  const source = repo.read(RUST_TRANSCRIPT);
  const match = /const EXPECTED_TRANSCRIPT: &str = r"([\s\S]*?)";/.exec(source);
  if (match === null) {
    fail(`${RUST_TRANSCRIPT}: no 'const EXPECTED_TRANSCRIPT: &str = r"..."' literal found`);
  }
  const scenarios = [];
  for (const entry of rustSliceEntries(source, "const SCENARIOS:", RUST_TRANSCRIPT)) {
    scenarios.push({
      name: rustStringField(entry, "name", RUST_TRANSCRIPT),
      profile: snakeCase(rustEnumField(entry, "profile", RUST_TRANSCRIPT)),
      seed: rustNumberField(entry, "seed", RUST_TRANSCRIPT),
      sends: rustNumberField(entry, "sends", RUST_TRANSCRIPT),
      slots: rustNumberField(entry, "slots", RUST_TRANSCRIPT),
    });
  }
  return { text: match[1], scenarios };
}

function tsTranscript(repo) {
  const source = repo.read(TS_TRANSCRIPT);
  const match = /const EXPECTED_TRANSCRIPT = `([\s\S]*?)`;/.exec(source);
  if (match === null) {
    fail(`${TS_TRANSCRIPT}: no 'const EXPECTED_TRANSCRIPT = \`...\`' literal found`);
  }
  const listMatch = /const SCENARIOS: readonly ScenarioSpec\[\] = \[([\s\S]*?)\n\];/.exec(source);
  if (listMatch === null) {
    fail(`${TS_TRANSCRIPT}: no 'const SCENARIOS: readonly ScenarioSpec[]' table found`);
  }
  const scenarios = [];
  const rowPattern = /\{([^{}]*)\}/g;
  let rowMatch = rowPattern.exec(listMatch[1]);
  while (rowMatch !== null) {
    const row = rowMatch[1];
    const field = (name, pattern) => {
      const found = new RegExp(`${name}\\s*:\\s*${pattern}`).exec(row);
      if (found === null) {
        fail(`${TS_TRANSCRIPT}: a SCENARIOS row is missing '${name}'`);
      }
      return found[1];
    };
    scenarios.push({
      name: field("name", '"([a-z0-9_]+)"'),
      profile: field("profile", '"([a-z0-9_]+)"'),
      seed: numeric(field("seed", "(-?[0-9.]+)"), `${TS_TRANSCRIPT}: seed`),
      sends: numeric(field("sends", "(\\d+)"), `${TS_TRANSCRIPT}: sends`),
      slots: numeric(field("slots", "(\\d+)"), `${TS_TRANSCRIPT}: slots`),
    });
    rowMatch = rowPattern.exec(listMatch[1]);
  }
  if (scenarios.length === 0) {
    fail(
      `${TS_TRANSCRIPT}: 'SCENARIOS' parsed to zero rows -- the parse matched nothing, which is a gate failure, not a pass`,
    );
  }
  return { text: match[1], scenarios };
}

// The transcript is only evidence if it still records impairment. Two equal
// literals prove nothing when both sides became a pass-through together.
function impairmentEvidence(text, problems) {
  const lines = text.split("\n");
  const sends = lines.filter((line) => line.startsWith("sends|")).join("\n");
  const deliveries = lines.filter((line) => line.startsWith("deliveries|")).join("\n");
  const totals = {
    independent_lost: 0,
    burst_lost: 0,
    duplicated: 0,
    reordered: 0,
  };
  let counterLines = 0;
  for (const line of lines) {
    if (!line.startsWith("counters|")) {
      continue;
    }
    counterLines += 1;
    for (const key of Object.keys(totals)) {
      const match = new RegExp(`${key}=(\\d+)`).exec(line);
      if (match === null) {
        problems.push(`the shared transcript has a counters line with no '${key}': ${line}`);
        continue;
      }
      totals[key] += Number(match[1]);
    }
  }
  if (counterLines === 0) {
    fail("the shared transcript contains no 'counters|' line at all");
  }
  const markers = [
    ["an independent loss ('x' in a sends line)", /(?:^|[|,])x(?:,|$)/m.test(sends)],
    ["a burst loss ('b' in a sends line)", /(?:^|[|,])b(?:,|$)/m.test(sends)],
    ["a duplicate scheduled ('+' in a sends line)", sends.includes("+")],
    ["a duplicate delivered ('d' in a deliveries line)", /\d+d(?:,|$)/m.test(deliveries)],
  ];
  for (const [what, present] of markers) {
    if (!present) {
      problems.push(
        `the shared transcript no longer records ${what} -- an impairment transcript with no impairment in it is not evidence`,
      );
    }
  }
  for (const [key, total] of Object.entries(totals)) {
    if (total <= 0) {
      problems.push(
        `the shared transcript's '${key}' totals ${total} across every scenario -- the impairment mechanism has become a pass-through, or the scenarios no longer reach it`,
      );
    }
  }
  return counterLines;
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

function checkNetworkProfileParity(repo) {
  const problems = [];
  let compared = 0;

  // --- 1 & 2. the authored profiles -------------------------------------
  // `rust.fields` is read off gc-data's own struct; everything below compares
  // that set and only that set, on both sides.
  const rust = rustProfiles(repo);
  const ts = tsProfiles(repo, rust.fields);

  const interfaceFields = ts.interfaceFields.join(",");
  if (interfaceFields !== rust.fields.join(",")) {
    problems.push(
      `the browser's 'interface NetworkProfile' declares [${ts.interfaceFields.join(", ")}] but gc-data's 'pub struct NetworkProfile' declares [${rust.fields.join(", ")}] -- the two describe different networks`,
    );
  }
  compared += 1;

  for (const key of rust.rows.keys()) {
    if (!ts.rows.has(key)) {
      problems.push(
        `network profile '${key}' is authored in ${RUST_PROFILES} but missing from NETWORK_PROFILES (${TS_PROFILES}) -- browser evidence cannot run the profile the native matrix names`,
      );
      continue;
    }
    const rustRow = rust.rows.get(key);
    const tsRow = ts.rows.get(key);
    for (const field of rust.fields) {
      if (rustRow[field] !== tsRow[field]) {
        problems.push(
          `network profile '${key}': gc-data authors ${field}=${rustRow[field]} but ${TS_PROFILES} says ${tsRow[field]} -- the browser would impair a differently-shaped link and its evidence could not be compared to the native run`,
        );
      }
      compared += 1;
    }
  }
  for (const key of ts.rows.keys()) {
    if (!rust.rows.has(key)) {
      problems.push(
        `network profile '${key}' is in NETWORK_PROFILES (${TS_PROFILES}) but no such profile is authored in ${RUST_PROFILES}`,
      );
    }
  }
  if (rust.order.join(",") !== ts.order.join(",")) {
    problems.push(
      `network profile order differs: gc-data declares [${rust.order.join(", ")}] and ${TS_PROFILES} declares [${ts.order.join(", ")}]`,
    );
  }
  if (ts.union.join(",") !== ts.order.join(",")) {
    problems.push(
      `${TS_PROFILES}: the NetworkProfileName union [${ts.union.join(", ")}] does not match the NETWORK_PROFILES keys [${ts.order.join(", ")}]`,
    );
  }
  if (ts.names.join(",") !== ts.order.join(",")) {
    problems.push(
      `${TS_PROFILES}: NETWORK_PROFILE_NAMES [${ts.names.join(", ")}] does not match the NETWORK_PROFILES keys [${ts.order.join(", ")}]`,
    );
  }
  compared += 3;
  if (rust.rows.size < MIN_PROFILES) {
    fail(
      `${RUST_PROFILES}: parsed only ${rust.rows.size} profile(s), fewer than the ${MIN_PROFILES} that exist -- the parse has silently narrowed`,
    );
  }

  // --- 3. the generator's constants --------------------------------------
  const rngSource = repo.read(RUST_RNG);
  const tsRngSource = repo.read(TS_RNG);
  for (const [rustName, tsName] of [
    ["MOD", "RNG_MOD"],
    ["MULT", "RNG_MULT"],
  ]) {
    const rustValue = rustNumberConst(rngSource, rustName, RUST_RNG);
    const tsValue = tsNumberConst(tsRngSource, tsName, TS_RNG);
    if (rustValue !== tsValue) {
      problems.push(
        `the impairment generator's ${rustName} is ${rustValue} in ${RUST_RNG} but ${tsValue} in ${TS_RNG} -- the same seed would produce a different impairment sequence in each language`,
      );
    }
    compared += 1;
  }

  // --- 4, 5 & 6. the shared transcript ------------------------------------
  const rustSide = rustTranscript(repo);
  const tsSide = tsTranscript(repo);
  if (rustSide.text !== tsSide.text) {
    const rustLines = rustSide.text.split("\n");
    const tsLines = tsSide.text.split("\n");
    let firstDifference = "the literals differ in length";
    for (let i = 0; i < Math.max(rustLines.length, tsLines.length); i += 1) {
      if (rustLines[i] !== tsLines[i]) {
        firstDifference = `line ${i + 1}:\n      rust: ${rustLines[i] ?? "(absent)"}\n      ts:   ${tsLines[i] ?? "(absent)"}`;
        break;
      }
    }
    problems.push(
      `the shared impairment transcript differs between ${RUST_TRANSCRIPT} and ${TS_TRANSCRIPT} -- ${firstDifference}`,
    );
  }
  compared += 1;

  if (rustSide.scenarios.length < MIN_SCENARIOS) {
    fail(
      `${RUST_TRANSCRIPT}: parsed only ${rustSide.scenarios.length} scenario(s), fewer than the ${MIN_SCENARIOS} required -- the differential has been narrowed`,
    );
  }
  if (rustSide.scenarios.length !== tsSide.scenarios.length) {
    problems.push(
      `the differential runs ${rustSide.scenarios.length} scenario(s) in Rust and ${tsSide.scenarios.length} in TypeScript, so the two sides assert the same transcript over different work`,
    );
  }
  for (let i = 0; i < Math.min(rustSide.scenarios.length, tsSide.scenarios.length); i += 1) {
    const rustScenario = rustSide.scenarios[i];
    const tsScenario = tsSide.scenarios[i];
    for (const key of ["name", "profile", "seed", "sends", "slots"]) {
      if (rustScenario[key] !== tsScenario[key]) {
        problems.push(
          `differential scenario ${i + 1} disagrees on '${key}': Rust says ${JSON.stringify(rustScenario[key])}, TypeScript says ${JSON.stringify(tsScenario[key])}`,
        );
      }
      compared += 1;
    }
    if (!rust.rows.has(rustScenario.profile)) {
      problems.push(
        `differential scenario '${rustScenario.name}' names profile '${rustScenario.profile}', which is not authored in ${RUST_PROFILES}`,
      );
    }
  }

  const counterLines = impairmentEvidence(rustSide.text, problems);
  compared += 1;

  return {
    problems,
    compared,
    counts: {
      profiles: rust.rows.size,
      fields: rust.fields.length,
      scenarios: rustSide.scenarios.length,
      transcriptScenarios: counterLines,
    },
  };
}

// ---------------------------------------------------------------------------
// Self-test: prove the checker goes red on every drift shape it claims to
// catch, over IN-MEMORY mutations of the real sources.
// ---------------------------------------------------------------------------

function loadAll(root) {
  const files = new Map();
  const disk = new DiskRepo(root);
  for (const relPath of ALL_SOURCES) {
    files.set(relPath, disk.read(relPath));
  }
  return files;
}

function mutated(files, relPath, from, to) {
  const copy = new Map(files);
  const text = copy.get(relPath);
  if (text === undefined || !text.includes(from)) {
    fail(
      `self-test: ${relPath} does not contain the text this scenario mutates: ${JSON.stringify(from)}`,
    );
  }
  copy.set(relPath, text.replace(from, to));
  return copy;
}

// Rewrite every match, not just the first -- used where a scenario has to
// touch all four authored rows, as adding a tuning field does.
function mutatedEvery(files, relPath, pattern, replacement) {
  const copy = new Map(files);
  const text = copy.get(relPath);
  if (text === undefined || !pattern.test(text)) {
    fail(`self-test: ${relPath} does not match this scenario's pattern: ${pattern}`);
  }
  copy.set(relPath, text.replace(pattern, replacement));
  return copy;
}

// Zero every impairment counter in BOTH transcript literals at once. The two
// literals still agree; what is gone is the impairment they record.
function withNeuteredTranscripts(files) {
  const copy = new Map(files);
  for (const relPath of [RUST_TRANSCRIPT, TS_TRANSCRIPT]) {
    const text = copy.get(relPath);
    if (text === undefined) {
      fail(`self-test: ${relPath} is missing`);
    }
    copy.set(
      relPath,
      text.replace(
        /(independent_lost|burst_lost|duplicated|reordered)=\d+/g,
        (_match, key) => `${key}=0`,
      ),
    );
  }
  return copy;
}

function expectRed(name, files, pattern) {
  let problems;
  try {
    ({ problems } = checkNetworkProfileParity(new MemoryRepo(files)));
  } catch (error) {
    if (error instanceof ParityError) {
      if (pattern.test(error.message)) {
        console.log(`ok  ${name} (rejected: ${error.message})`);
        return true;
      }
      console.error(`SELF-TEST FAIL: ${name} failed, but with an unexpected message: ${error.message}`);
      return false;
    }
    throw error;
  }
  const hit = problems.find((problem) => pattern.test(problem));
  if (hit === undefined) {
    console.error(
      `SELF-TEST FAIL: ${name} was NOT rejected. Problems found: ${problems.length === 0 ? "(none)" : problems.join("; ")}`,
    );
    return false;
  }
  console.log(`ok  ${name} (rejected: ${hit})`);
  return true;
}

function selfTest(root) {
  const files = loadAll(root);
  let ok = true;

  const { problems, compared } = checkNetworkProfileParity(new MemoryRepo(files));
  if (problems.length > 0) {
    console.error("SELF-TEST FAIL: the real sources do not agree, so the mutations below prove nothing:");
    for (const problem of problems) {
      console.error(`  - ${problem}`);
    }
    return 1;
  }
  console.log(`ok  the real sources agree (${compared} comparisons)`);

  // THE SILENT ONE: a tuning value drifts on one side. Nothing throws in
  // either language; the browser simply impairs a different link forever.
  ok =
    expectRed(
      "a profile's loss rate changed in gc-data only",
      mutated(files, RUST_PROFILES, "        independent_loss_rate: 0.03,", "        independent_loss_rate: 0.3,"),
      /network profile 'stress': gc-data authors independent_loss_rate=0\.3/,
    ) && ok;

  ok =
    expectRed(
      "a profile's base delay changed in TypeScript only",
      mutated(files, TS_PROFILES, "  stress: {\n    base_delay_ticks: 6,", "  stress: {\n    base_delay_ticks: 9,"),
      /network profile 'stress': gc-data authors base_delay_ticks=6 but .* says 9/,
    ) && ok;

  // THE ONE A HARDCODED FIELD LIST COULD NOT SEE. An eighth tuning field is
  // added to gc-data's struct and to all four authored rows, with values that
  // genuinely change what a link does, and the browser's copy is not touched
  // at all. A checker carrying its own list of seven field names compares
  // those seven, finds them in agreement, and prints the same OK line.
  ok =
    expectRed(
      "a tuning field added to gc-data only, with the browser's copy untouched",
      mutatedEvery(
        mutated(
          files,
          RUST_PROFILES,
          "    /// Length of a loss burst, in ticks.\n    pub burst_length_ticks: i64,",
          "    /// Length of a loss burst, in ticks.\n    pub burst_length_ticks: i64,\n    /// Probability a packet arrives corrupted.\n    pub corruption_rate: f64,",
        ),
        RUST_PROFILES,
        /^(        burst_length_ticks: \d+,)$/gm,
        "$1\n        corruption_rate: 0.9,",
      ),
      /profile 'clean' has no 'corruption_rate' -- gc-data's 'pub struct NetworkProfile' declares it/,
    ) && ok;

  // The mirror image: a tuning value the browser invents and nothing authors.
  ok =
    expectRed(
      "a tuning field the browser declares and gc-data has never heard of",
      mutatedEvery(
        files,
        TS_PROFILES,
        /^(    burst_length_ticks: \d+,)$/gm,
        "$1\n    corruption_rate: 0.9,",
      ),
      /profile 'clean' declares 'corruption_rate', which gc-data's 'pub struct NetworkProfile' does not/,
    ) && ok;

  ok =
    expectRed(
      "a profile dropped from the TypeScript table",
      mutated(files, TS_PROFILES, "  playable: {", "  playable_disabled: {"),
      /network profile 'playable' is authored in .* but missing from NETWORK_PROFILES/,
    ) && ok;

  ok =
    expectRed(
      "the generator's multiplier drifted",
      mutated(files, TS_RNG, "export const RNG_MULT = 16807;", "export const RNG_MULT = 48271;"),
      /impairment generator's MULT is 16807 .* but 48271/,
    ) && ok;

  ok =
    expectRed(
      "the shared transcript drifted on one side",
      mutated(files, TS_TRANSCRIPT, "counters|sent=24,delivered=24", "counters|sent=24,delivered=23"),
      /shared impairment transcript differs between/,
    ) && ok;

  ok =
    expectRed(
      "a differential scenario changed seed on one side",
      mutated(files, TS_TRANSCRIPT, 'profile: "stress", seed: 20260811', 'profile: "stress", seed: 20260812'),
      /differential scenario \d+ disagrees on 'seed'/,
    ) && ok;

  // The one that matters most: both literals still agree, and both record
  // nothing. Equality alone would call that a pass.
  ok =
    expectRed(
      "both transcripts agree but no longer record any impairment",
      withNeuteredTranscripts(files),
      /has become a pass-through, or the scenarios no longer reach it/,
    ) && ok;

  // A parse that matches nothing must be a hard error, not a clean run --
  // AGENTS.md §9's "prints nothing, exits 0" shape.
  ok =
    expectRed(
      "the Rust profile table renamed out from under the parser",
      mutated(files, RUST_PROFILES, "pub static ALL:", "static UNUSED_ALL:"),
      /no 'pub static ALL:' table found/,
    ) && ok;

  ok =
    expectRed(
      "the TypeScript profile table renamed out from under the parser",
      mutated(files, TS_PROFILES, "export const NETWORK_PROFILES", "export const NETWORK_PROFILE_TABLE"),
      /no 'export const NETWORK_PROFILES' found/,
    ) && ok;

  ok =
    expectRed(
      "the shared transcript literal removed entirely",
      mutated(files, RUST_TRANSCRIPT, "const EXPECTED_TRANSCRIPT: &str = r\"", "const UNUSED_TRANSCRIPT: &str = r\""),
      /no 'const EXPECTED_TRANSCRIPT: &str = r"\.\.\."' literal found/,
    ) && ok;

  ok =
    expectRed(
      "the authored profile struct renamed out from under the parser",
      mutated(files, RUST_PROFILES, "pub struct NetworkProfile {", "pub struct NetworkTuning {"),
      /no 'pub struct NetworkProfile' found/,
    ) && ok;

  if (!ok) {
    console.error("network profile parity self-test: FAILED");
    return 1;
  }
  console.log("network profile parity self-test: OK");
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
      return selfTest(root);
    }
    const repo = new DiskRepo(root);
    const { problems, compared, counts } = checkNetworkProfileParity(repo);
    if (mode === "list-sources") {
      for (const relPath of [...repo.accessed].sort()) {
        console.log(relPath);
      }
      return 0;
    }
    if (problems.length > 0) {
      console.error(
        "NETWORK PROFILE PARITY FAILED -- the authored impairment profiles and the browser's impairment disagree:",
      );
      for (const problem of problems) {
        console.error(`  - ${problem}`);
      }
      console.error("");
      console.error(
        `Fix ${TS_PROFILES} and the impairment differential until they mirror gc-data. Do not silence this gate:`,
      );
      console.error(
        "a disagreement here makes the browser and native suites measure different networks while both stay green.",
      );
      return 1;
    }
    console.log(
      `ok  ${counts.profiles} authored network profiles agree on all ${counts.fields} tuning fields gc-data declares (read from the struct, not assumed)`,
    );
    console.log("ok  the impairment generator's constants agree across Rust and TypeScript");
    console.log(
      `ok  ${counts.scenarios} differential scenarios assert one byte-identical transcript`,
    );
    console.log("ok  that transcript still records loss, bursts, duplication and reordering");
    console.log(`network profile parity: OK (${compared} comparisons)`);
    return 0;
  } catch (error) {
    if (error instanceof ParityError) {
      console.error(`NETWORK PROFILE PARITY COULD NOT BE CHECKED: ${error.message}`);
      console.error(
        "This is a gate failure, not a pass: the check could not compare what it claims to compare.",
      );
      return 1;
    }
    throw error;
  }
}

process.exit(main(process.argv.slice(2)));
