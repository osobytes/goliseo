#!/usr/bin/env node
// Cross-language parity gate for the character-presentation content mapping
// (#447).
//
// WHAT BREAKS WITHOUT THIS. `gc-data` authors which theme each character
// presentation belongs to, and which equipment presentation each fixed
// loadout carries. The renderer needs both, in ITS own vocabulary -- and
// ARCHITECTURE.md forbids a TypeScript package reading a Rust crate's source, so
// `ts/packages/render/src/rig3d/presentation_content.ts` restates the two
// mappings by hand. That is the same shape #433 found in the wire enums: each
// side is compiler-checked against itself, neither can see the other, and
// drift is therefore invisible to every other gate.
//
// The failure it produces is quiet rather than loud. Rename a presentation in
// `gc-data` and the renderer throws "no theme for presentation id" the first
// time that player is drawn -- mid-match, in a browser. Change which
// equipment a loadout carries and nothing throws at all: the player simply
// renders the wrong item forever, which is a defect nothing in either
// language can see, because both sides remain internally consistent.
//
// A third table is checked for a different reason. `EQUIPMENT_RIG3D` maps
// `gc-data`'s equipment presentation ids onto `rig3d/equipment.ts`'s builder
// ids, which are a THIRD vocabulary that exists only on the TypeScript side.
// Rust cannot be asked whether `medieval_heater_shield` should build
// `heater_shield` -- only that every equipment presentation it authors has an
// entry, and that the entry names a builder `rig3d/equipment.ts` actually
// has and a socket `rig3d/body.ts` can hang it from. Both of those failures
// otherwise surface inside `player_renderer_3d.ts`'s `build()` try/catch,
// which disables rigged players for the whole process and logs a warning.
//
// The wire constant `ROSTER_STRING_FIELD_COUNT` is compared too. It is
// duplicated in exactly the way `LAYOUT_VERSION` and `RENDER_FRAME_VERSION`
// are, and unlike those two it is NOT stamped into the numeric block, so a
// disagreement is not caught by a header assertion -- only by the blob's own
// part count, at decode time, in a browser.
//
// WHY AN ASSERTION AND NOT CODEGEN: the same argument #433 settled. A shared
// content schema generating both sides is a later milestone (ARCHITECTURE.md
// rule 6.7 says so explicitly); an assertion that reads both sources costs no
// build plumbing and catches the same divergence.
//
// FRAGILITY, STATED HONESTLY. This reads Rust and TypeScript SOURCE with
// regular expressions. A regex over source that silently matches nothing is
// precisely the "prints nothing, exits 0" failure AGENTS.md §9 exists to
// prevent, so every parse here is fail-loud: a table that yields zero
// entries, a file that does not exist, a constant that cannot be found -- each
// is a hard error naming the file and the symbol. There is no code path that
// reports success without having compared a nonzero number of mappings, and
// the gate additionally requires the printed mapping count to clear a floor.
//
//   node scripts/check_presentation_parity.mjs                 -- check this repo
//   node scripts/check_presentation_parity.mjs --repo DIR      -- check a copy
//   node scripts/check_presentation_parity.mjs --list-sources  -- print the files read
//   node scripts/check_presentation_parity.mjs --self-test     -- prove it goes red
//
// `--self-test` mutates IN-MEMORY copies of the real sources (never the tree)
// and requires each mutation to be rejected with a specific message: a
// presentation whose theme was changed on one side, a presentation dropped
// from the TypeScript table, a loadout pointed at the wrong equipment, an
// equipment id with no rig3d builder, a drifted
// `ROSTER_STRING_FIELD_COUNT`, and a parse that matched nothing.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

const RUST_PRESENTATIONS = "rust/crates/gc-data/src/character_presentations.rs";
const RUST_LOADOUTS = "rust/crates/gc-data/src/loadouts.rs";
const RUST_EQUIPMENT = "rust/crates/gc-data/src/equipment_presentations.rs";
const RUST_FRAME_BUFFER = "rust/crates/gc-render/src/frame_buffer.rs";
const TS_CONTENT = "ts/packages/render/src/rig3d/presentation_content.ts";
const TS_EQUIPMENT = "ts/packages/render/src/rig3d/equipment.ts";
const TS_BODY = "ts/packages/render/src/rig3d/body.ts";
const TS_FRAME_BUFFER = "ts/packages/render/src/frame_buffer.ts";

// `PrototypeThemeId` variant -> the theme key `rig3d/themes.ts` uses. This is
// the ONE naming bridge between the two languages here, and it is declared
// rather than derived: `MedievalFantasy -> medieval` is not a mechanical
// transformation of the spelling, so guessing it would be worse than writing
// it down. A Rust variant missing from this map is a hard error, not a skip.
const THEME_KEY_BY_RUST_VARIANT = new Map([
  ["MedievalFantasy", "medieval"],
  ["GalacticScifi", "scifi"],
  ["Toybox", "toybox"],
]);

// Floors, so a parse that silently matched fewer rows than exist cannot pass
// as a clean run. Content may only grow; if it ever legitimately shrinks,
// lowering these is a deliberate edit with a reason.
const MIN_PRESENTATIONS = 6;
const MIN_LOADOUTS = 6;
const MIN_EQUIPMENT = 6;

class ParityError extends Error {}

function fail(message) {
  throw new ParityError(message);
}

// ---------------------------------------------------------------------------
// Source access. `DiskRepo` reads a checkout; `MemoryRepo` serves an
// in-memory map so `--self-test` can mutate sources without touching a tree.
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
// Rust parsing
// ---------------------------------------------------------------------------

// Splits a `pub static ALL: &[T] = &[ ... ];` body into one string per struct
// literal. Brace counting, not a regex over the whole thing: the entries
// contain nested braces (`StatBlock { .. }`) in other tables of this shape,
// and a lazy match would stop at the first `}`.
function rustStaticAllEntries(source, relPath) {
  const start = source.indexOf("pub static ALL:");
  if (start < 0) {
    fail(`${relPath}: no 'pub static ALL:' table found`);
  }
  // `= &[`, NOT the first `&[`: the declaration reads
  // `pub static ALL: &[CharacterPresentationData] = &[ ... ];`, so the first
  // `&[` is the TYPE and matching on it would parse the type's brackets as
  // the table's body and yield zero entries.
  const assign = source.indexOf("= &[", start);
  if (assign < 0) {
    fail(`${relPath}: 'pub static ALL:' has no '= &[' body`);
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
    fail(`${relPath}: 'pub static ALL:' body has unbalanced brackets`);
  }
  const bodyText = source.slice(open + 1, end);

  // Now split the body on top-level struct literals by brace depth.
  const entries = [];
  let braceDepth = 0;
  let entryStart = -1;
  for (let i = 0; i < bodyText.length; i += 1) {
    const ch = bodyText[i];
    if (ch === "{") {
      if (braceDepth === 0) {
        entryStart = i + 1;
      }
      braceDepth += 1;
    } else if (ch === "}") {
      braceDepth -= 1;
      if (braceDepth === 0) {
        entries.push(bodyText.slice(entryStart, i));
      } else if (braceDepth < 0) {
        fail(`${relPath}: 'pub static ALL:' body has unbalanced braces`);
      }
    }
  }
  if (entries.length === 0) {
    fail(`${relPath}: 'pub static ALL:' parsed to zero entries -- the parse matched nothing, which is a gate failure, not a pass`);
  }
  return entries;
}

// Reads one `field: "value",` out of a struct-literal body.
function rustStringField(entry, field, relPath) {
  const match = new RegExp(`(?:^|[\\s,])${field}\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)"`).exec(entry);
  if (match === null) {
    fail(`${relPath}: a '${field}' string field is missing from an ALL entry`);
  }
  return match[1].replace(/\\"/g, '"');
}

// Reads one `field: Enum::Variant,` out of a struct-literal body.
function rustEnumField(entry, field, relPath) {
  const match = new RegExp(`(?:^|[\\s,])${field}\\s*:\\s*[A-Za-z_][A-Za-z0-9_]*::([A-Za-z_][A-Za-z0-9_]*)`).exec(entry);
  if (match === null) {
    fail(`${relPath}: a '${field}' enum field is missing from an ALL entry`);
  }
  return match[1];
}

// Reads `pub const NAME: <ty> = <int>;`.
function rustUsizeConst(source, name, relPath) {
  const match = new RegExp(`pub const ${name}\\s*:\\s*[A-Za-z0-9_]+\\s*=\\s*(\\d+)\\s*;`).exec(source);
  if (match === null) {
    fail(`${relPath}: no 'pub const ${name}' found`);
  }
  return Number(match[1]);
}

// ---------------------------------------------------------------------------
// TypeScript parsing
// ---------------------------------------------------------------------------

// Extracts the object-literal body of a `const NAME<: type> = { ... };`
// declaration by brace counting.
//
// The anchor is found first, then the first `= {` AFTER it, rather than one
// regex spanning the type annotation: `equipment.ts`'s `BUILDERS` is typed
// `Readonly<Record<string, (c: SlotIndex) => ...>>`, and an annotation
// containing `=>` defeats any `[^=]*` bridge between the name and the `=`.
// `=\s*\{` cannot match `=>` (a `>` is not whitespace and not `{`), so this
// lands on the assignment.
function declaredObjectBody(source, declaration, name, relPath) {
  // The trailing guard matters: a plain `indexOf` for
  // `export const PRESENTATION_THEME` also matches
  // `export const PRESENTATION_THEMES`, so a RENAMED table would be parsed as
  // if nothing had happened. That is the silent-match failure this whole file
  // is written to avoid.
  const anchorPattern = new RegExp(`${declaration.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?![A-Za-z0-9_])`);
  const anchorMatch = anchorPattern.exec(source);
  if (anchorMatch === null) {
    fail(`${relPath}: no '${declaration}' found`);
  }
  const anchor = anchorMatch.index;
  const assign = /=\s*\{/.exec(source.slice(anchor));
  if (assign === null) {
    fail(`${relPath}: '${declaration}' is not followed by an '= {' object literal`);
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

function tsObjectBody(source, name, relPath) {
  return declaredObjectBody(source, `export const ${name}`, name, relPath);
}

// `key: "value",` pairs at any depth of the given body -- used for the flat
// string-valued tables.
function tsStringMap(source, name, relPath) {
  const body = tsObjectBody(source, name, relPath);
  const map = new Map();
  const pattern = /([A-Za-z_][A-Za-z0-9_]*)\s*:\s*"((?:[^"\\]|\\.)*)"/g;
  let match = pattern.exec(body);
  while (match !== null) {
    map.set(match[1], match[2]);
    match = pattern.exec(body);
  }
  if (map.size === 0) {
    fail(`${relPath}: '${name}' parsed to zero entries -- the parse matched nothing, which is a gate failure, not a pass`);
  }
  return map;
}

// `key: { id: "x", slot: "y" },` pairs.
function tsEquipmentMap(source, name, relPath) {
  const body = tsObjectBody(source, name, relPath);
  const map = new Map();
  const pattern = /([A-Za-z_][A-Za-z0-9_]*)\s*:\s*\{\s*id\s*:\s*"([^"]*)"\s*,\s*slot\s*:\s*"([^"]*)"\s*\}/g;
  let match = pattern.exec(body);
  while (match !== null) {
    map.set(match[1], { id: match[2], slot: match[3] });
    match = pattern.exec(body);
  }
  if (map.size === 0) {
    fail(`${relPath}: '${name}' parsed to zero '{ id, slot }' entries -- the parse matched nothing, which is a gate failure, not a pass`);
  }
  return map;
}

// The keys of `const BUILDERS: ... = { id: fn, ... }` in rig3d/equipment.ts.
function tsBuilderIds(source, relPath) {
  const body = declaredObjectBody(source, "const BUILDERS", "BUILDERS", relPath);
  const ids = new Set();
  const pattern = /([A-Za-z_][A-Za-z0-9_]*)\s*:\s*[A-Za-z_][A-Za-z0-9_]*\s*,/g;
  let match = pattern.exec(body);
  while (match !== null) {
    ids.add(match[1]);
    match = pattern.exec(body);
  }
  if (ids.size === 0) {
    fail(`${relPath}: 'BUILDERS' parsed to zero entries -- the parse matched nothing, which is a gate failure, not a pass`);
  }
  return ids;
}

// Reads `export const NAME = <int>;`.
function tsNumberConst(source, name, relPath) {
  const match = new RegExp(`export const ${name}\\s*(?::[^=]*)?=\\s*(\\d+)\\s*;`).exec(source);
  if (match === null) {
    fail(`${relPath}: no 'export const ${name} = <number>' found`);
  }
  return Number(match[1]);
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

function checkPresentationParity(repo) {
  const problems = [];
  let compared = 0;

  // --- 1. presentation_id -> theme -------------------------------------
  const rustPresentations = new Map();
  for (const entry of rustStaticAllEntries(repo.read(RUST_PRESENTATIONS), RUST_PRESENTATIONS)) {
    const id = rustStringField(entry, "id", RUST_PRESENTATIONS);
    const variant = rustEnumField(entry, "theme_id", RUST_PRESENTATIONS);
    const themeKey = THEME_KEY_BY_RUST_VARIANT.get(variant);
    if (themeKey === undefined) {
      fail(
        `${RUST_PRESENTATIONS}: PrototypeThemeId::${variant} has no entry in this checker's ` +
          `THEME_KEY_BY_RUST_VARIANT -- a new theme needs its rig3d key declared here, not guessed`,
      );
    }
    rustPresentations.set(id, themeKey);
  }
  if (rustPresentations.size < MIN_PRESENTATIONS) {
    fail(`${RUST_PRESENTATIONS}: parsed only ${rustPresentations.size} presentations (want >= ${MIN_PRESENTATIONS})`);
  }

  const tsContent = repo.read(TS_CONTENT);
  const tsPresentations = tsStringMap(tsContent, "PRESENTATION_THEME", TS_CONTENT);

  for (const [id, themeKey] of rustPresentations) {
    const tsTheme = tsPresentations.get(id);
    if (tsTheme === undefined) {
      problems.push(`presentation '${id}' is authored in ${RUST_PRESENTATIONS} but missing from PRESENTATION_THEME in ${TS_CONTENT}`);
    } else if (tsTheme !== themeKey) {
      problems.push(`presentation '${id}': gc-data says theme '${themeKey}', PRESENTATION_THEME says '${tsTheme}'`);
    }
    compared += 1;
  }
  for (const id of tsPresentations.keys()) {
    if (!rustPresentations.has(id)) {
      problems.push(`presentation '${id}' is in PRESENTATION_THEME (${TS_CONTENT}) but no such presentation is authored in ${RUST_PRESENTATIONS}`);
    }
  }

  // --- 2. loadout_id -> equipment_presentation_id -----------------------
  const rustLoadouts = new Map();
  for (const entry of rustStaticAllEntries(repo.read(RUST_LOADOUTS), RUST_LOADOUTS)) {
    rustLoadouts.set(
      rustStringField(entry, "id", RUST_LOADOUTS),
      rustStringField(entry, "equipment_presentation_id", RUST_LOADOUTS),
    );
  }
  if (rustLoadouts.size < MIN_LOADOUTS) {
    fail(`${RUST_LOADOUTS}: parsed only ${rustLoadouts.size} loadouts (want >= ${MIN_LOADOUTS})`);
  }

  const tsLoadouts = tsStringMap(tsContent, "LOADOUT_EQUIPMENT", TS_CONTENT);
  for (const [id, equipmentId] of rustLoadouts) {
    const tsEquipmentId = tsLoadouts.get(id);
    if (tsEquipmentId === undefined) {
      problems.push(`loadout '${id}' is authored in ${RUST_LOADOUTS} but missing from LOADOUT_EQUIPMENT in ${TS_CONTENT}`);
    } else if (tsEquipmentId !== equipmentId) {
      problems.push(`loadout '${id}': gc-data carries '${equipmentId}', LOADOUT_EQUIPMENT says '${tsEquipmentId}'`);
    }
    compared += 1;
  }
  for (const id of tsLoadouts.keys()) {
    if (!rustLoadouts.has(id)) {
      problems.push(`loadout '${id}' is in LOADOUT_EQUIPMENT (${TS_CONTENT}) but no such loadout is authored in ${RUST_LOADOUTS}`);
    }
  }

  // --- 3. equipment_presentation_id -> rig3d builder + socket -----------
  const rustEquipment = new Set();
  for (const entry of rustStaticAllEntries(repo.read(RUST_EQUIPMENT), RUST_EQUIPMENT)) {
    rustEquipment.add(rustStringField(entry, "id", RUST_EQUIPMENT));
  }
  if (rustEquipment.size < MIN_EQUIPMENT) {
    fail(`${RUST_EQUIPMENT}: parsed only ${rustEquipment.size} equipment presentations (want >= ${MIN_EQUIPMENT})`);
  }

  const tsEquipment = tsEquipmentMap(tsContent, "EQUIPMENT_RIG3D", TS_CONTENT);
  const builderIds = tsBuilderIds(repo.read(TS_EQUIPMENT), TS_EQUIPMENT);
  const socketIds = new Set(tsStringMap(repo.read(TS_BODY), "SOCKETS", TS_BODY).keys());
  const validSlots = new Set(["right", "left", "hip"]);

  for (const id of rustEquipment) {
    const item = tsEquipment.get(id);
    if (item === undefined) {
      problems.push(`equipment presentation '${id}' is authored in ${RUST_EQUIPMENT} but missing from EQUIPMENT_RIG3D in ${TS_CONTENT}`);
      compared += 1;
      continue;
    }
    if (!builderIds.has(item.id)) {
      problems.push(`equipment presentation '${id}' maps to rig3d item '${item.id}', which ${TS_EQUIPMENT}'s BUILDERS does not have`);
    }
    if (!socketIds.has(item.id)) {
      problems.push(`equipment presentation '${id}' maps to rig3d item '${item.id}', which ${TS_BODY}'s SOCKETS cannot hang`);
    }
    if (!validSlots.has(item.slot)) {
      problems.push(`equipment presentation '${id}' names loadout slot '${item.slot}', which is not one of right/left/hip`);
    }
    compared += 1;
  }
  for (const id of tsEquipment.keys()) {
    if (!rustEquipment.has(id)) {
      problems.push(`equipment presentation '${id}' is in EQUIPMENT_RIG3D (${TS_CONTENT}) but no such presentation is authored in ${RUST_EQUIPMENT}`);
    }
  }

  // --- 4. the duplicated wire constant ----------------------------------
  const rustParts = rustUsizeConst(repo.read(RUST_FRAME_BUFFER), "ROSTER_STRING_FIELD_COUNT", RUST_FRAME_BUFFER);
  const tsParts = tsNumberConst(repo.read(TS_FRAME_BUFFER), "ROSTER_STRING_FIELD_COUNT", TS_FRAME_BUFFER);
  if (rustParts !== tsParts) {
    problems.push(
      `ROSTER_STRING_FIELD_COUNT is ${rustParts} in ${RUST_FRAME_BUFFER} but ${tsParts} in ${TS_FRAME_BUFFER} -- ` +
        `the roster string blob would be mis-parsed at decode time, in a browser`,
    );
  }
  compared += 1;

  return { problems, compared, counts: { presentations: rustPresentations.size, loadouts: rustLoadouts.size, equipment: rustEquipment.size } };
}

// ---------------------------------------------------------------------------
// Self-test: prove the checker goes red on every drift shape it claims to
// catch, over IN-MEMORY mutations of the real sources.
// ---------------------------------------------------------------------------

const ALL_SOURCES = [RUST_PRESENTATIONS, RUST_LOADOUTS, RUST_EQUIPMENT, RUST_FRAME_BUFFER, TS_CONTENT, TS_EQUIPMENT, TS_BODY, TS_FRAME_BUFFER];

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
    fail(`self-test: ${relPath} does not contain the text this scenario mutates: ${JSON.stringify(from)}`);
  }
  copy.set(relPath, text.replace(from, to));
  return copy;
}

function expectRed(name, files, pattern) {
  let problems;
  try {
    ({ problems } = checkPresentationParity(new MemoryRepo(files)));
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
    console.error(`SELF-TEST FAIL: ${name} was NOT rejected. Problems found: ${problems.length === 0 ? "(none)" : problems.join("; ")}`);
    return false;
  }
  console.log(`ok  ${name} (rejected: ${hit})`);
  return true;
}

function selfTest(root) {
  const files = loadAll(root);
  let ok = true;

  // The unmutated tree must be clean, or every scenario below proves nothing.
  const { problems, compared } = checkPresentationParity(new MemoryRepo(files));
  if (problems.length > 0) {
    console.error("SELF-TEST FAIL: the real sources do not agree, so the mutations below prove nothing:");
    for (const problem of problems) {
      console.error(`  - ${problem}`);
    }
    return 1;
  }
  console.log(`ok  the real sources agree (${compared} mappings compared)`);

  ok =
    expectRed(
      "a presentation's theme changed on the Rust side only",
      mutated(files, RUST_PRESENTATIONS, 'id: "scifi_axi",\n        name: "AX-7 \\"Axi\\"",\n        theme_id: PrototypeThemeId::GalacticScifi,', 'id: "scifi_axi",\n        name: "AX-7 \\"Axi\\"",\n        theme_id: PrototypeThemeId::Toybox,'),
      /presentation 'scifi_axi': gc-data says theme 'toybox'/,
    ) && ok;

  ok =
    expectRed(
      "a presentation dropped from the TypeScript table",
      mutated(files, TS_CONTENT, "  toy_tock: \"toybox\",", ""),
      /presentation 'toy_tock' is authored in .* but missing from PRESENTATION_THEME/,
    ) && ok;

  ok =
    expectRed(
      "a loadout pointed at the wrong equipment on the TypeScript side",
      mutated(files, TS_CONTENT, "  loadout_vector_blade: \"scifi_energy_blade\",", "  loadout_vector_blade: \"scifi_pulse_blaster\","),
      /loadout 'loadout_vector_blade': gc-data carries 'scifi_energy_blade'/,
    ) && ok;

  ok =
    expectRed(
      "an equipment presentation mapped to a rig3d item that has no builder",
      mutated(files, TS_CONTENT, 'medieval_heater_shield: { id: "heater_shield", slot: "left" }', 'medieval_heater_shield: { id: "kite_shield", slot: "left" }'),
      /maps to rig3d item 'kite_shield', which .* does not have/,
    ) && ok;

  ok =
    expectRed(
      "a new equipment presentation authored in Rust with no TypeScript entry",
      mutated(files, RUST_EQUIPMENT, 'id: "toy_foam_sword",', 'id: "toy_pool_noodle",'),
      /equipment presentation 'toy_pool_noodle' is authored in .* but missing from EQUIPMENT_RIG3D/,
    ) && ok;

  ok =
    expectRed(
      "ROSTER_STRING_FIELD_COUNT drifted between the two languages",
      mutated(files, TS_FRAME_BUFFER, "export const ROSTER_STRING_FIELD_COUNT = 4;", "export const ROSTER_STRING_FIELD_COUNT = 5;"),
      /ROSTER_STRING_FIELD_COUNT is 4 in .* but 5 in/,
    ) && ok;

  // A parse that matches nothing must be a hard error, not a clean run --
  // AGENTS.md §9's "prints nothing, exits 0" shape.
  ok =
    expectRed(
      "a TypeScript table renamed out from under the parser",
      mutated(files, TS_CONTENT, "export const PRESENTATION_THEME", "export const PRESENTATION_THEMES"),
      /no 'export const PRESENTATION_THEME' found/,
    ) && ok;

  ok =
    expectRed(
      "a Rust content table emptied",
      mutated(files, RUST_LOADOUTS, "pub static ALL:", "static UNUSED_ALL:"),
      /no 'pub static ALL:' table found/,
    ) && ok;

  if (!ok) {
    console.error("presentation parity self-test: FAILED");
    return 1;
  }
  console.log("presentation parity self-test: OK");
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
    const { problems, compared, counts } = checkPresentationParity(repo);
    if (mode === "list-sources") {
      for (const relPath of [...repo.accessed].sort()) {
        console.log(relPath);
      }
      return 0;
    }
    if (problems.length > 0) {
      console.error("PRESENTATION PARITY FAILED -- gc-data's authored content and the renderer's mapping disagree:");
      for (const problem of problems) {
        console.error(`  - ${problem}`);
      }
      console.error("");
      console.error(`Fix ${TS_CONTENT} until it mirrors gc-data. Do not silence this gate: a disagreement here`);
      console.error("either throws in a player's browser mid-match, or draws the wrong item forever and throws nothing.");
      return 1;
    }
    console.log(`ok  ${counts.presentations} character presentations map to a rig3d theme`);
    console.log(`ok  ${counts.loadouts} fixed loadouts map to an equipment presentation`);
    console.log(`ok  ${counts.equipment} equipment presentations map to a rig3d builder and socket`);
    console.log("ok  ROSTER_STRING_FIELD_COUNT agrees across Rust and TypeScript");
    console.log(`presentation parity: OK (${compared} mappings)`);
    return 0;
  } catch (error) {
    if (error instanceof ParityError) {
      console.error(`PRESENTATION PARITY COULD NOT BE CHECKED: ${error.message}`);
      console.error("This is a gate failure, not a pass: the check could not compare what it claims to compare.");
      return 1;
    }
    throw error;
  }
}

process.exit(main(process.argv.slice(2)));
