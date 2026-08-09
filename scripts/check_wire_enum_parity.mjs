#!/usr/bin/env node
// Cross-language parity gate for EVERY wire enum on the RenderFrame boundary
// (#433).
//
// WHAT BREAKS WITHOUT THIS. A `RenderFrame` block is produced by Rust
// (`v2/rust/crates/gc-render/src/frame_buffer.rs`) and consumed by TypeScript
// (`v2/ts/packages/render/src/frame_buffer.ts`). Every closed set on that
// boundary is defined TWICE -- once as a Rust enum with a `*_code` numbering,
// once as a TypeScript union with a `*FromCode` numbering -- and nothing but
// convention kept the two in step. Rust is self-consistent (`*_code` is an
// exhaustive, wildcard-free `match`, so a new variant is a compile error
// there) and TypeScript is self-consistent (`rig3d/pose_table.ts` is a
// `Record<PlayerPoseId, ...>`, so a missing entry is a compile error there).
// Neither language can see the other. Add a variant on the Rust side, ship
// it, and the first frame carrying the new code reaches `optDecode`, which
// throws `frame_buffer: unknown pose id code 33` IN A PLAYER'S BROWSER,
// MID-MATCH. Every gate stays green; the match dies.
//
// The quieter half is worse. A REORDERING that preserves membership but
// shifts codes leaves both sides internally consistent, throws nothing, and
// decodes every value to the wrong member. Three of these enums -- `team`,
// `species shape` and `event kind` -- go through `requireDecode`, so a
// shifted code does not even produce an unmapped-code error: it produces a
// DIFFERENT VALID VALUE. A silently swapped team is not a crash; it is a
// renderer that draws the wrong side.
//
// WHY AN ASSERTION AND NOT CODEGEN. Generating the TypeScript union from the
// Rust enum would make drift impossible, but it needs a build step wired into
// two call sites, a generated file in the tree that must be regenerated and
// reviewed, and it would hand authorship of a hand-documented, hand-ordered
// decoder to a template. The failure mode being guarded is SILENT DIVERGENCE,
// not mistyping: an assertion that reads both definitions catches divergence
// exactly as well, costs no build plumbing, and leaves both files authored by
// humans. See the PR for #433.
//
// WHAT IS COMPARED, per enum:
//
//   1. MEMBERSHIP. The Rust enum's variants (read from its `enum` definition)
//      and the TypeScript union's members are the same set.
//   2. NUMERIC CODES. `*_code`'s arm for a variant and `*FromCode`'s `case`
//      for the corresponding member carry the same number, in both
//      directions. This is the check nobody had.
//   3. DENSITY. Codes are exactly 1..N with no gaps and no duplicates on
//      either side -- encoding rule 1 ("enums are 1-based; 0 always means
//      absent") is what makes a sparse field's `None` encodable at all.
//   4. INTERNAL TS CONSISTENCY. `*FromCode`'s declared return type (a union,
//      inline or named, resolved through this file's imports) names exactly
//      the members its `case` arms return -- so a member that TypeScript
//      believes in but no code maps to, or vice versa, is caught too.
//   5. RUST INVERSES. Where Rust also has a `*_from_code`, it inverts
//      `*_code` exactly.
//   6. COVERAGE. Every `pub fn *_code` in the Rust codec and every
//      `function *FromCode` in the TypeScript decoder appears in the registry
//      below. A TWELFTH wire enum added later cannot be silently unguarded.
//
// HOW NAMES CROSS. Rust names variants `KeeperGetUp`; the wire names them
// `keeper_get_up`. The bridge is the PascalCase -> snake_case convention,
// which both trees follow universally today. Where a Rust enum ALSO declares
// its own name method (`PlayerPoseId::wire_name`, `Team::wire_str`,
// `MatchEventKind::wire_str`), that declaration is authoritative AND is
// checked against the convention: a deliberate exception must be recorded in
// `NAME_EXCEPTIONS` below, which is empty today. A variant whose spelling
// breaks the convention with no declared name is a hard error, not a guess.
//
// FRAGILITY, STATED HONESTLY. This reads Rust and TypeScript SOURCE with
// regular expressions. A regex over source that silently matches nothing is
// precisely the "prints nothing, exits 0" failure AGENTS.md §9 exists to
// prevent, so every parse here is fail-loud: a missing function, a body with
// zero arms, a wildcard `_ =>` in a `*_code` match, an enum whose variant
// list disagrees with its own `*_code` arms, an unresolvable type name, a
// non-unit variant -- each is a hard error with a message naming the file and
// the symbol. There is no code path that reports success without having
// compared a nonzero number of members for all of the registered enums, and
// the gate additionally requires the printed enum count to match the
// registry.
//
//   node scripts/check_wire_enum_parity.mjs                 -- check this repo
//   node scripts/check_wire_enum_parity.mjs --repo DIR      -- check a copy
//   node scripts/check_wire_enum_parity.mjs --list-sources  -- print the files read
//   node scripts/check_wire_enum_parity.mjs --self-test     -- prove it goes red
//
// `--self-test` mutates IN-MEMORY copies of the real sources (never the tree)
// and requires each mutation to be rejected with a specific message:
// a variant present on one side only, a two-code swap that preserves
// membership, a wildcard match arm, a decoder that lost a `case`, and a
// parse that matched nothing. `scripts/check_v2.sh --self-test` runs the same
// demonstration a second, independent way: against mutated file COPIES under
// `mktemp -d`, through `--repo`.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, posix } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// The registry. One entry per enum crossing the frame-buffer wire.
//
// `label` is the string `optDecode`/`requireDecode` already put in its error
// messages ("unknown pose id code 33"), so a gate failure and a runtime throw
// name the same thing. Coverage check 6 below is what keeps this list honest.
// ---------------------------------------------------------------------------

const RUST_FRAME_BUFFER = "v2/rust/crates/gc-render/src/frame_buffer.rs";
const TS_FRAME_BUFFER = "v2/ts/packages/render/src/frame_buffer.ts";

const WIRE_ENUMS = [
  { label: "team", rustFn: "team_code", tsFn: "teamFromCode" },
  { label: "species shape", rustFn: "species_shape_code", tsFn: "speciesShapeFromCode" },
  { label: "charge kind", rustFn: "charge_kind_code", tsFn: "chargeKindFromCode" },
  { label: "pose source", rustFn: "pose_source_code", tsFn: "poseSourceFromCode" },
  { label: "aerial style", rustFn: "aerial_style_code", tsFn: "aerialStyleFromCode" },
  { label: "aerial outcome", rustFn: "aerial_outcome_code", tsFn: "aerialOutcomeFromCode" },
  { label: "save style", rustFn: "save_style_code", tsFn: "saveStyleFromCode" },
  { label: "keeper state", rustFn: "keeper_state_code", tsFn: "keeperStateFromCode" },
  { label: "shot type", rustFn: "shot_type_code", tsFn: "shotTypeFromCode" },
  { label: "pose id", rustFn: "pose_id_code", tsFn: "poseIdFromCode" },
  { label: "event kind", rustFn: "event_kind_code", tsFn: "eventKindFromCode" },
];

// Rust variant spellings whose wire name deliberately departs from the
// PascalCase -> snake_case convention. Keyed `EnumName::Variant`. Empty
// today, and it should stay that way: an entry here is a note that two
// languages spell one wire member differently on purpose.
const NAME_EXCEPTIONS = new Map();

// A Rust crate name as it appears in a `use` path -> its source directory.
const CRATE_DIRS = new Map([
  ["gc_core", "v2/rust/crates/gc-core/src"],
  ["gc_data", "v2/rust/crates/gc-data/src"],
  ["gc_netcode", "v2/rust/crates/gc-netcode/src"],
  ["gc_render", "v2/rust/crates/gc-render/src"],
  ["gc_sim", "v2/rust/crates/gc-sim/src"],
]);

// The crate the codec itself lives in, for `use crate::...` paths.
const RUST_CODEC_CRATE_DIR = "v2/rust/crates/gc-render/src";

// Name methods a Rust enum may declare for its own wire spelling.
const NAME_METHODS = new Set(["wire_name", "wire_str", "as_str"]);

// ---------------------------------------------------------------------------
// Failure reporting. Every problem is collected with the file and symbol it
// came from; nothing is warned about and continued past.
// ---------------------------------------------------------------------------

class ParityError extends Error {}

function fail(message) {
  throw new ParityError(message);
}

// ---------------------------------------------------------------------------
// Source access. `DiskRepo` reads a checkout; `MemoryRepo` serves an
// in-memory map so `--self-test` can mutate sources without touching a tree.
// Both record which paths were read, which is what `--list-sources` prints.
// ---------------------------------------------------------------------------

class DiskRepo {
  constructor(root) {
    this.root = root;
    this.accessed = new Set();
  }

  read(relPath) {
    const full = join(this.root, relPath);
    if (!existsSync(full)) {
      fail(`source not found: ${relPath} (under ${this.root})`);
    }
    this.accessed.add(relPath);
    return readFileSync(full, "utf8");
  }

  exists(relPath) {
    return existsSync(join(this.root, relPath));
  }
}

class MemoryRepo {
  constructor(files) {
    this.files = new Map(files);
    this.accessed = new Set();
  }

  read(relPath) {
    const text = this.files.get(relPath);
    if (text === undefined) {
      fail(`source not found: ${relPath} (in-memory fixture)`);
    }
    this.accessed.add(relPath);
    return text;
  }

  exists(relPath) {
    return this.files.has(relPath);
  }
}

// ---------------------------------------------------------------------------
// Shared text helpers
// ---------------------------------------------------------------------------

// Returns the text between the brace at `openIndex` and its match. Fails
// loudly rather than returning a truncated body, because a truncated body is
// exactly how a regex check silently stops seeing arms.
function bracedBody(text, openIndex, what) {
  if (text[openIndex] !== "{") {
    fail(`${what}: expected '{' at offset ${openIndex}`);
  }
  let depth = 0;
  for (let i = openIndex; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        return text.slice(openIndex + 1, i);
      }
    }
  }
  fail(`${what}: unbalanced braces from offset ${openIndex}`);
  return "";
}

function pascalToSnake(name) {
  if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) {
    fail(`variant '${name}' is not PascalCase; its wire spelling cannot be derived`);
  }
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase();
}

function sortedList(values) {
  return [...values].sort().join(", ");
}

// ---------------------------------------------------------------------------
// Rust parsing
// ---------------------------------------------------------------------------

// Locates `fn <name>(...)` and returns its parameter list, return type and
// body. `null` is never returned: an absent function is a hard error at the
// call site, because "the function moved or was renamed" must not read as
// "nothing to check".
function rustFn(text, name, file) {
  const pattern = new RegExp(`\\bfn\\s+${name}\\s*(?:<[^>]*>)?\\s*\\(([^)]*)\\)\\s*->\\s*([^{]+?)\\s*\\{`);
  const match = pattern.exec(text);
  if (match === null) {
    fail(`${file}: no fn ${name}(...) -> ... found`);
  }
  const openIndex = match.index + match[0].length - 1;
  return {
    params: match[1],
    returns: match[2].trim(),
    body: bracedBody(text, openIndex, `${file}: fn ${name}`),
  };
}

// The enum a `*_code` function numbers, taken from its single parameter.
function rustParamType(params, fnName, file) {
  const match = /:\s*([A-Za-z_][\w]*)\s*$/.exec(params.trim());
  if (match === null) {
    fail(`${file}: cannot read the enum type from ${fnName}(${params})`);
  }
  return match[1];
}

// `EnumName::Variant => 7,` / `Variant => 7.0,` arms of a `*_code` match.
// A wildcard arm is refused: `*_code`'s exhaustiveness with no `_ =>` is the
// property that makes the Rust side compiler-checked, and this gate reads the
// arms as the authoritative variant enumeration precisely because of it.
function rustCodeArms(body, fnName, file) {
  if (/(?:^|\n)\s*_\s*=>/.test(body)) {
    fail(
      `${file}: ${fnName} has a wildcard '_ =>' arm. A wire numbering must be exhaustive with no ` +
        `wildcard, or a new variant compiles into a wrong code instead of a compile error.`,
    );
  }
  const arms = new Map();
  const pattern = /(?:^|\n)\s*(?:(?:Self|[A-Z]\w*)::)?([A-Z]\w*)\s*=>\s*(\d+)(?:\.0*)?\s*,/g;
  let match = pattern.exec(body);
  while (match !== null) {
    const [, variant, code] = match;
    if (arms.has(variant)) {
      fail(`${file}: ${fnName} maps ${variant} twice`);
    }
    arms.set(variant, Number(code));
    match = pattern.exec(body);
  }
  if (arms.size === 0) {
    fail(`${file}: ${fnName} yielded ZERO match arms -- the parse matched nothing, which is a gate failure, not a pass`);
  }
  return arms;
}

// `7 => Some(Variant),` arms of a `*_from_code` match, where one exists.
function rustFromCodeArms(body, fnName, file) {
  const arms = new Map();
  const pattern = /(?:^|\n)\s*(\d+)\s*=>\s*Some\((?:(?:Self|[A-Z]\w*)::)?([A-Z]\w*)\)\s*,/g;
  let match = pattern.exec(body);
  while (match !== null) {
    const [, code, variant] = match;
    arms.set(Number(code), variant);
    match = pattern.exec(body);
  }
  if (arms.size === 0) {
    fail(`${file}: ${fnName} yielded ZERO match arms -- the parse matched nothing`);
  }
  return arms;
}

// Resolves `Type` to the file its `pub enum` is declared in, following the
// codec's own `use` statements. Only the forms this codec actually uses are
// understood; anything else is a hard error rather than a guess.
function resolveRustEnumFile(codecText, typeName) {
  const pattern = /use\s+((?:crate|[a-z_][\w]*)(?:::[a-z_][\w]*)*)::\{?([^;]*?)\}?\s*;/g;
  let match = pattern.exec(codecText);
  while (match !== null) {
    const [, path, names] = match;
    const imported = names
      .split(",")
      .map((part) => part.trim())
      .filter((part) => part !== "");
    if (imported.includes(typeName)) {
      const segments = path.split("::");
      const head = segments[0];
      const rest = segments.slice(1);
      let dir;
      if (head === "crate") {
        dir = RUST_CODEC_CRATE_DIR;
      } else {
        dir = CRATE_DIRS.get(head);
        if (dir === undefined) {
          fail(`cannot resolve Rust crate '${head}' for type ${typeName}; add it to CRATE_DIRS`);
        }
      }
      if (rest.length === 0) {
        fail(`cannot resolve ${typeName}: 'use ${path}' names no module`);
      }
      return posix.join(dir, `${rest.join("/")}.rs`);
    }
    match = pattern.exec(codecText);
  }
  fail(`${RUST_FRAME_BUFFER}: no 'use' statement imports ${typeName}; cannot locate its enum definition`);
  return "";
}

// The unit variants of `pub enum <name> { ... }`, in declaration order.
function rustEnumVariants(text, enumName, file) {
  const pattern = new RegExp(`\\benum\\s+${enumName}\\s*\\{`);
  const match = pattern.exec(text);
  if (match === null) {
    fail(`${file}: no 'enum ${enumName} {' found`);
  }
  const body = bracedBody(text, match.index + match[0].length - 1, `${file}: enum ${enumName}`);
  const variants = [];
  for (const rawLine of body.split("\n")) {
    const line = rawLine.trim();
    if (line === "" || line.startsWith("//") || line.startsWith("#[")) {
      continue;
    }
    const variant = /^([A-Z]\w*)\s*(,|$)/.exec(line);
    if (variant !== null) {
      variants.push(variant[1]);
      continue;
    }
    if (/^[A-Z]\w*\s*[({=]/.test(line)) {
      fail(`${file}: enum ${enumName} has a non-unit variant (${line}); a wire enum must be unit-only`);
    }
    fail(`${file}: enum ${enumName} has an unparseable line: ${line}`);
  }
  if (variants.length === 0) {
    fail(`${file}: enum ${enumName} yielded ZERO variants -- the parse matched nothing`);
  }
  return variants;
}

// A `Self::Variant => "wire_name",` method on the enum, if it declares one.
function rustDeclaredNames(text, enumName, file) {
  const implPattern = new RegExp(`\\bimpl\\s+${enumName}\\s*\\{`, "g");
  let implMatch = implPattern.exec(text);
  let found = null;
  while (implMatch !== null) {
    const body = bracedBody(text, implMatch.index + implMatch[0].length - 1, `${file}: impl ${enumName}`);
    const fnPattern = /(?:pub\s+)?(?:const\s+)?fn\s+(\w+)\s*\(\s*&?self\s*\)\s*->\s*&'static\s+str\s*\{/g;
    let fnMatch = fnPattern.exec(body);
    while (fnMatch !== null) {
      const fnName = fnMatch[1];
      if (NAME_METHODS.has(fnName)) {
        if (found !== null) {
          fail(`${file}: ${enumName} declares more than one wire-name method (${found.method}, ${fnName})`);
        }
        const fnBody = bracedBody(body, fnMatch.index + fnMatch[0].length - 1, `${file}: ${enumName}::${fnName}`);
        const names = new Map();
        const armPattern = /(?:^|\n)\s*(?:(?:Self|[A-Z]\w*)::)?([A-Z]\w*)\s*=>\s*"([^"]+)"\s*,/g;
        let arm = armPattern.exec(fnBody);
        while (arm !== null) {
          names.set(arm[1], arm[2]);
          arm = armPattern.exec(fnBody);
        }
        if (names.size === 0) {
          fail(`${file}: ${enumName}::${fnName} yielded ZERO name arms -- the parse matched nothing`);
        }
        found = { method: fnName, names };
      }
      fnMatch = fnPattern.exec(body);
    }
    implMatch = implPattern.exec(text);
  }
  return found;
}

// ---------------------------------------------------------------------------
// TypeScript parsing
// ---------------------------------------------------------------------------

function tsFn(text, name, file) {
  const pattern = new RegExp(`\\bfunction\\s+${name}\\s*\\(([^)]*)\\)\\s*:\\s*([^{]+?)\\s*\\{`);
  const match = pattern.exec(text);
  if (match === null) {
    fail(`${file}: no function ${name}(...): ... found`);
  }
  return {
    returns: match[2].trim(),
    body: bracedBody(text, match.index + match[0].length - 1, `${file}: function ${name}`),
  };
}

// `case 7:\n  return "keeper_stretch";` arms of a `*FromCode` switch.
function tsCaseArms(body, fnName, file) {
  const arms = new Map();
  const pattern = /case\s+(\d+)\s*:\s*return\s+"([^"]+)"\s*;/g;
  let match = pattern.exec(body);
  while (match !== null) {
    const code = Number(match[1]);
    if (arms.has(code)) {
      fail(`${file}: ${fnName} maps code ${code} twice`);
    }
    arms.set(code, match[2]);
    match = pattern.exec(body);
  }
  if (arms.size === 0) {
    fail(`${file}: ${fnName} yielded ZERO case arms -- the parse matched nothing, which is a gate failure, not a pass`);
  }
  if (!/default\s*:\s*return\s+undefined\s*;/.test(body)) {
    fail(
      `${file}: ${fnName} has no 'default: return undefined;'. An unmapped code must reach ` +
        `optDecode's throw, not fall out of the switch as an implicit undefined.`,
    );
  }
  return arms;
}

// Resolves a `*FromCode` return type to the set of wire members it allows.
// Handles an inline literal union and a named alias, resolving the alias
// through this file's own `export type` declarations and then through its
// `import type ... from "./x.ts"` statements.
function tsReturnMembers(returns, tsFile, tsText, repo, fnName) {
  const parts = returns
    .split("|")
    .map((part) => part.trim())
    .filter((part) => part !== "");
  if (!parts.includes("undefined")) {
    fail(`${tsFile}: ${fnName}'s return type (${returns}) cannot be undefined; an unmapped code must decode to undefined`);
  }
  const named = parts.filter((part) => part !== "undefined");
  if (named.length === 0) {
    fail(`${tsFile}: ${fnName} returns only undefined`);
  }
  if (named.every((part) => /^"[^"]+"$/.test(part))) {
    return named.map((part) => part.slice(1, -1));
  }
  if (named.length !== 1) {
    fail(`${tsFile}: ${fnName}'s return type (${returns}) mixes a named type with literals; not supported`);
  }
  return tsAliasMembers(named[0], tsFile, tsText, repo);
}

function tsAliasMembers(typeName, tsFile, tsText, repo) {
  const local = tsTypeAlias(tsText, typeName);
  if (local !== null) {
    return local;
  }
  const importPattern = /import\s+type\s*\{([^}]*)\}\s*from\s*"([^"]+)"\s*;/g;
  let match = importPattern.exec(tsText);
  while (match !== null) {
    const names = match[1].split(",").map((part) => part.trim());
    if (names.includes(typeName)) {
      const spec = match[2];
      if (!spec.startsWith("./")) {
        fail(`${tsFile}: ${typeName} is imported from '${spec}', which this gate cannot resolve`);
      }
      const target = posix.join(posix.dirname(tsFile), spec.slice(2));
      const targetText = repo.read(target);
      const members = tsTypeAlias(targetText, typeName);
      if (members === null) {
        fail(`${target}: no 'export type ${typeName} = ...' union found`);
      }
      return members;
    }
    match = importPattern.exec(tsText);
  }
  fail(`${tsFile}: cannot resolve type ${typeName} -- it is neither declared nor imported here`);
  return [];
}

function tsTypeAlias(text, typeName) {
  const pattern = new RegExp(`export\\s+type\\s+${typeName}\\s*=([^;]*);`);
  const match = pattern.exec(text);
  if (match === null) {
    return null;
  }
  const members = [...match[1].matchAll(/"([^"]+)"/g)].map((literal) => literal[1]);
  if (members.length === 0) {
    fail(`export type ${typeName} is not a union of string literals`);
  }
  return members;
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

function denseCodes(codes, what, problems) {
  const sorted = [...codes].sort((a, b) => a - b);
  const expected = sorted.map((_, index) => index + 1);
  if (sorted.join(",") !== expected.join(",")) {
    problems.push(
      `${what}: codes are ${sorted.join(",")}, expected a dense 1..${sorted.length} numbering ` +
        `(encoding rule 1: enums are 1-based and 0 always means absent)`,
    );
    return false;
  }
  return true;
}

// Runs the whole comparison. Returns the problems found; throws ParityError
// on anything that stopped it from comparing at all (a parse that matched
// nothing, an unresolvable symbol, a wildcard arm).
function checkWireEnumParity(repo) {
  const problems = [];
  const rustText = repo.read(RUST_FRAME_BUFFER);
  const tsText = repo.read(TS_FRAME_BUFFER);
  const covered = [];

  // Coverage first (check 6): the registry cannot quietly stop covering the
  // boundary it claims to cover.
  const rustCodeFns = [...rustText.matchAll(/pub fn ([a-z_]+_code)\s*\(/g)].map((match) => match[1]);
  const rustDeclared = rustCodeFns.filter((name) => !name.endsWith("_from_code"));
  const tsDecodeFns = [...tsText.matchAll(/function ([A-Za-z]+FromCode)\s*\(/g)].map((match) => match[1]);
  if (rustDeclared.length === 0) {
    fail(`${RUST_FRAME_BUFFER}: found ZERO 'pub fn *_code' encoders -- the parse matched nothing`);
  }
  if (tsDecodeFns.length === 0) {
    fail(`${TS_FRAME_BUFFER}: found ZERO 'function *FromCode' decoders -- the parse matched nothing`);
  }
  const registeredRust = new Set(WIRE_ENUMS.map((entry) => entry.rustFn));
  const registeredTs = new Set(WIRE_ENUMS.map((entry) => entry.tsFn));
  for (const name of rustDeclared) {
    if (!registeredRust.has(name)) {
      problems.push(
        `${RUST_FRAME_BUFFER}: ${name} numbers a wire enum that this gate does not cover. ` +
          `Add it to WIRE_ENUMS in ${posix.basename(fileURLToPath(import.meta.url))}.`,
      );
    }
  }
  for (const name of tsDecodeFns) {
    if (!registeredTs.has(name)) {
      problems.push(
        `${TS_FRAME_BUFFER}: ${name} decodes a wire enum that this gate does not cover. ` +
          `Add it to WIRE_ENUMS in ${posix.basename(fileURLToPath(import.meta.url))}.`,
      );
    }
  }

  for (const entry of WIRE_ENUMS) {
    const { label, rustFn: rustFnName, tsFn: tsFnName } = entry;

    // --- Rust side -------------------------------------------------------
    const encoder = rustFn(rustText, rustFnName, RUST_FRAME_BUFFER);
    const enumName = rustParamType(encoder.params, rustFnName, RUST_FRAME_BUFFER);
    const arms = rustCodeArms(encoder.body, rustFnName, RUST_FRAME_BUFFER);
    const enumFile = resolveRustEnumFile(rustText, enumName);
    const enumText = repo.read(enumFile);
    const variants = rustEnumVariants(enumText, enumName, enumFile);

    const missingArms = variants.filter((variant) => !arms.has(variant));
    const strayArms = [...arms.keys()].filter((variant) => !variants.includes(variant));
    if (missingArms.length > 0) {
      problems.push(`${label}: ${enumName} declares ${sortedList(missingArms)}, which ${rustFnName} does not number`);
    }
    if (strayArms.length > 0) {
      problems.push(`${label}: ${rustFnName} numbers ${sortedList(strayArms)}, which ${enumName} does not declare`);
    }
    denseCodes([...arms.values()], `${label} (Rust ${rustFnName})`, problems);

    const declaredNames = rustDeclaredNames(enumText, enumName, enumFile);
    const rustByCode = new Map();
    for (const [variant, code] of arms) {
      let name;
      const exception = NAME_EXCEPTIONS.get(`${enumName}::${variant}`);
      if (exception !== undefined) {
        name = exception;
      } else if (declaredNames !== null && declaredNames.names.has(variant)) {
        name = declaredNames.names.get(variant);
        const derived = pascalToSnake(variant);
        if (name !== derived) {
          problems.push(
            `${label}: ${enumName}::${variant}.${declaredNames.method}() is "${name}" but the ` +
              `PascalCase->snake_case convention derives "${derived}". If that is deliberate, record it in ` +
              `NAME_EXCEPTIONS.`,
          );
        }
      } else {
        name = pascalToSnake(variant);
      }
      if (rustByCode.has(code)) {
        problems.push(`${label}: Rust code ${code} is used by two variants`);
      }
      rustByCode.set(code, name);
    }

    // Check 5: where Rust also decodes, its inverse must agree exactly.
    const fromCodeName = `${rustFnName.replace(/_code$/, "")}_from_code`;
    if (new RegExp(`\\bfn\\s+${fromCodeName}\\s*\\(`).test(rustText)) {
      const decoder = rustFn(rustText, fromCodeName, RUST_FRAME_BUFFER);
      const inverse = rustFromCodeArms(decoder.body, fromCodeName, RUST_FRAME_BUFFER);
      for (const [variant, code] of arms) {
        if (inverse.get(code) !== variant) {
          problems.push(
            `${label}: ${rustFnName} maps ${variant} to ${code}, but ${fromCodeName} maps ${code} to ` +
              `${inverse.get(code) ?? "nothing"}`,
          );
        }
      }
      for (const [code, variant] of inverse) {
        if (arms.get(variant) !== code) {
          problems.push(`${label}: ${fromCodeName} maps ${code} to ${variant}, which ${rustFnName} does not number ${code}`);
        }
      }
    }

    // --- TypeScript side -------------------------------------------------
    const decoder = tsFn(tsText, tsFnName, TS_FRAME_BUFFER);
    const cases = tsCaseArms(decoder.body, tsFnName, TS_FRAME_BUFFER);
    denseCodes([...cases.keys()], `${label} (TypeScript ${tsFnName})`, problems);

    // Check 4: the declared union and the switch agree with each other.
    const union = tsReturnMembers(decoder.returns, TS_FRAME_BUFFER, tsText, repo, tsFnName);
    const caseNames = new Set(cases.values());
    for (const member of union) {
      if (!caseNames.has(member)) {
        problems.push(`${label}: the TypeScript union names "${member}", which ${tsFnName} maps no code to`);
      }
    }
    for (const member of caseNames) {
      if (!union.includes(member)) {
        problems.push(`${label}: ${tsFnName} returns "${member}", which the TypeScript union does not name`);
      }
    }

    // --- Checks 1 and 2: the two languages against each other -------------
    const rustNames = new Set(rustByCode.values());
    for (const name of rustNames) {
      if (!caseNames.has(name)) {
        problems.push(
          `${label}: Rust has "${name}" (code ${[...rustByCode].find(([, value]) => value === name)?.[0]}) and ` +
            `TypeScript does not. A frame carrying it throws in the browser, mid-match.`,
        );
      }
    }
    for (const name of caseNames) {
      if (!rustNames.has(name)) {
        problems.push(`${label}: TypeScript has "${name}" and Rust does not produce it`);
      }
    }
    for (const [code, name] of rustByCode) {
      const tsName = cases.get(code);
      if (tsName !== undefined && tsName !== name) {
        problems.push(
          `${label}: code ${code} is "${name}" in Rust and "${tsName}" in TypeScript. Membership is ` +
            `unchanged, so nothing throws -- every value of this enum decodes to the wrong member.`,
        );
      }
    }

    covered.push({ label, enumName, count: variants.length });
  }

  return { problems, covered };
}

// ---------------------------------------------------------------------------
// Self-test: the demonstration that this gate can go red (AGENTS.md §9).
//
// Each scenario mutates an IN-MEMORY copy of the real sources and requires
// the check to reject it with a message naming the defect. A scenario that
// goes red for the wrong reason is indistinguishable from one that works,
// right up until the day the guard it names actually breaks -- so the
// expected message is pinned, not just the nonzero outcome.
// ---------------------------------------------------------------------------

function baselineFiles(root) {
  const repo = new DiskRepo(root);
  checkWireEnumParity(repo);
  const files = [];
  for (const relPath of repo.accessed) {
    files.push([relPath, readFileSync(join(root, relPath), "utf8")]);
  }
  return files;
}

function mutate(files, relPath, from, to) {
  return files.map(([path, text]) => {
    if (path !== relPath) {
      return [path, text];
    }
    if (!text.includes(from)) {
      fail(`self-test: fixture text not found in ${relPath}: ${JSON.stringify(from)}`);
    }
    return [path, text.replace(from, to)];
  });
}

function expectRejected(label, files, needle) {
  let problems = [];
  try {
    problems = checkWireEnumParity(new MemoryRepo(files)).problems;
  } catch (error) {
    if (error instanceof ParityError) {
      if (!error.message.includes(needle)) {
        console.log(`SELF-TEST FAIL: ${label} was rejected, but not for ${JSON.stringify(needle)}: ${error.message}`);
        return false;
      }
      console.log(`ok  ${label} -> hard error: ${error.message.split("\n")[0]}`);
      return true;
    }
    throw error;
  }
  if (problems.length === 0) {
    console.log(`SELF-TEST FAIL: ${label} was ACCEPTED -- the gate would not catch it`);
    return false;
  }
  const matched = problems.filter((problem) => problem.includes(needle));
  if (matched.length === 0) {
    console.log(`SELF-TEST FAIL: ${label} was rejected, but not for ${JSON.stringify(needle)}:`);
    for (const problem of problems) {
      console.log(`      ${problem}`);
    }
    return false;
  }
  console.log(`ok  ${label} -> ${matched[0]}`);
  return true;
}

function selfTest(root) {
  let ok = true;
  const files = baselineFiles(root);

  // The pristine tree must PASS, or every rejection below proves nothing.
  const baseline = checkWireEnumParity(new MemoryRepo(files));
  if (baseline.problems.length === 0) {
    console.log(`ok  the real sources pass (${baseline.covered.length} enums)`);
  } else {
    console.log("SELF-TEST FAIL: the real sources do not pass, so no rejection below is meaningful:");
    for (const problem of baseline.problems) {
      console.log(`      ${problem}`);
    }
    ok = false;
  }

  // (a) A variant present on one side only -- #433's headline case. Rust
  // grows a 33rd pose; TypeScript never hears about it.
  ok =
    expectRejected(
      "a Rust variant TypeScript has never heard of",
      mutate(
        mutate(
          mutate(files, "v2/rust/crates/gc-render/src/player_pose.rs", "    Locomotion,\n}", "    Locomotion,\n    /// A newly added pose.\n    Sprint,\n}"),
          "v2/rust/crates/gc-render/src/player_pose.rs",
          'Self::Locomotion => "locomotion",',
          'Self::Locomotion => "locomotion",\n            Self::Sprint => "sprint",',
        ),
        RUST_FRAME_BUFFER,
        "        Locomotion => 32,\n    }) as f64",
        "        Locomotion => 32,\n        Sprint => 33,\n    }) as f64",
      ),
      'Rust has "sprint" (code 33) and TypeScript does not',
    ) && ok;

  // (b) Identical membership, two codes swapped. Nothing throws; every
  // value of the enum decodes to the wrong member.
  ok =
    expectRejected(
      "two codes swapped with membership preserved",
      mutate(
        files,
        TS_FRAME_BUFFER,
        '    case 1:\n      return "home";\n    case 2:\n      return "away";',
        '    case 1:\n      return "away";\n    case 2:\n      return "home";',
      ),
      'code 1 is "home" in Rust and "away" in TypeScript',
    ) && ok;

  // (c) A TypeScript member with no Rust producer -- the mirror of (a).
  ok =
    expectRejected(
      "a TypeScript member Rust does not produce",
      mutate(
        mutate(files, TS_FRAME_BUFFER, '  | "locomotion";', '  | "locomotion"\n  | "sprint";'),
        TS_FRAME_BUFFER,
        '    case 32:\n      return "locomotion";',
        '    case 32:\n      return "locomotion";\n    case 33:\n      return "sprint";',
      ),
      'TypeScript has "sprint" and Rust does not produce it',
    ) && ok;

  // (d) A decoder that lost a case: the union still names the member, so
  // TypeScript's own `Record<PlayerPoseId, ...>` guarantee stays green.
  ok =
    expectRejected(
      "a decoder case deleted while the union keeps the member",
      mutate(files, TS_FRAME_BUFFER, '    case 31:\n      return "fatigue";\n', ""),
      'the TypeScript union names "fatigue"',
    ) && ok;

  // (e) A wildcard arm on the Rust side -- the property that makes `*_code`
  // an authoritative enumeration, removed.
  ok =
    expectRejected(
      "a wildcard '_ =>' arm in a Rust wire numbering",
      mutate(files, RUST_FRAME_BUFFER, "        KeeperShotType::Chip => 2.0,", "        _ => 2.0,"),
      "wildcard",
    ) && ok;

  // (f) The parse matching nothing -- the "prints nothing, exits 0" shape
  // AGENTS.md §9 names. A renamed decoder must be a hard error.
  ok =
    expectRejected(
      "a renamed TypeScript decoder (the parse matches nothing)",
      mutate(files, TS_FRAME_BUFFER, "function poseIdFromCode(", "function poseIdFromWireCode("),
      "no function poseIdFromCode",
    ) && ok;

  // (g) A twelfth wire enum nobody registered.
  ok =
    expectRejected(
      "an unregistered wire enum in the decoder",
      mutate(
        files,
        TS_FRAME_BUFFER,
        "function teamFromCode(",
        'function weatherFromCode(code: number): "clear" | "rain" | undefined {\n  switch (code) {\n    case 1:\n      return "clear";\n    case 2:\n      return "rain";\n    default:\n      return undefined;\n  }\n}\n\nfunction teamFromCode(',
      ),
      "weatherFromCode decodes a wire enum that this gate does not cover",
    ) && ok;

  if (!ok) {
    console.log("wire enum parity self-test: FAILED");
    return 1;
  }
  console.log("wire enum parity self-test: OK");
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
    const { problems, covered } = checkWireEnumParity(repo);
    if (mode === "list-sources") {
      for (const relPath of [...repo.accessed].sort()) {
        console.log(relPath);
      }
      return 0;
    }
    if (problems.length > 0) {
      console.error("WIRE ENUM PARITY FAILED -- the Rust producer and the TypeScript reader disagree:");
      for (const problem of problems) {
        console.error(`  - ${problem}`);
      }
      console.error("");
      console.error(`Fix ${RUST_FRAME_BUFFER} and ${TS_FRAME_BUFFER} until they agree. Do not silence this gate:`);
      console.error("a disagreement here throws in a player's browser, mid-match, or decodes to the wrong value.");
      return 1;
    }
    for (const entry of covered) {
      console.log(`ok  ${entry.label} (${entry.enumName}): ${entry.count} variants agree, codes included`);
    }
    console.log(`wire enum parity: OK (${covered.length} enums)`);
    return 0;
  } catch (error) {
    if (error instanceof ParityError) {
      console.error(`WIRE ENUM PARITY COULD NOT BE CHECKED: ${error.message}`);
      console.error("This is a gate failure, not a pass: the check could not compare what it claims to compare.");
      return 1;
    }
    throw error;
  }
}

process.exit(main(process.argv.slice(2)));
