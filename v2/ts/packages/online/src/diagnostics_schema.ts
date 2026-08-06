// Ported from game/online/diagnostics_schema.lua.
//
// Declarative shapes, strict validation, canonical serialization, and content
// hashing for the OMP-3 network diagnostic contracts.
//
// Serialization follows the length-prefixed discipline already used by the
// research contracts in `sim/research_schema.lua` (Rust-owned; not ported
// here): every scalar emits its byte length before its bytes, records emit
// declared field order, and maps emit sorted keys. Delimiter-joined or
// table-order hashing is ambiguous and is forbidden. This module keeps its
// own header and version rather than reusing the research serializer,
// because it adds one concept that serializer has no room for -- the field
// `domain` below -- and because a diagnostic export must never share a
// version bump, a preimage, or a vocabulary with a participant payload.
//
// ## Domains: the separation this schema exists to enforce
//
// Every field belongs to exactly one domain, and the domain decides which
// vocabulary the field name is allowed to use:
//
// | Domain      | Means                                   | Forbidden vocabulary |
// | ----------- | ---------------------------------------- | -------------------- |
// | `identity`  | Build/protocol/manifest identity.       | wall clock            |
// | `canonical` | Simulation and tick-space evidence.     | wall clock            |
// | `runtime`   | Wall-clock and transport observation.   | simulation             |
// | `anchor`    | The one binding between the two clocks. | neither, both needed   |
//
// So a reader cannot mistake an RTT sample for a tick, or a boundary hash for
// a frame time, by reading the schema alone: the names that would allow the
// confusion are rejected at shape-construction time. `anchor` is the sole
// exception and it earns it -- an anchor exists only to state "input tick X
// was observed at monotonic time Y, +/- this mapping error", so it must name
// both and must declare its own error term.
//
// Shape errors throw: a malformed *shape* is a code bug. Value errors return
// a `Result`: diagnostic values arrive from transports, drivers, and
// operators and are external input.
//
// ## Boundary note: fnv1a64
//
// `core/fnv1a64.lua` is Rust-owned per v2/README.md's core split (it is one
// of the four `sim/` dependencies moved wholesale to `crates/gc-core`), but
// this module -- which the same README assigns to `@gc/online` -- needs it
// purely to content-address its own export artifact. That digest never has
// to agree with anything Rust computes and never crosses the determinism
// line (see this file's own docs above: it hashes *diagnostics*, not
// simulation state), so reimplementing the algorithm here is the same kind
// of deliberate, safe duplication as `vec2` existing on both sides -- it is
// a small, exact, integer-only algorithm (XOR and multiply, no
// transcendentals), not a content table. See this port's final report for
// the recommendation to note this in the README's core-split table.

import { ok, err, type Result } from "@gc/core";

export function fnv1a64Hash(text: string): string {
  const mask = 0xffffffffffffffffn;
  let hash = 0xcbf29ce484222325n; // FNV-1a 64 offset basis.
  const prime = 0x100000001b3n; // FNV-1a 64 prime.
  for (let i = 0; i < text.length; i += 1) {
    // Binary-string convention: one UTF-16 code unit is one byte, values
    // 0-255 — the same convention @gc/transport uses for wire payloads. NOT
    // TextEncoder: Lua strings are raw byte arrays, and UTF-8 encoding a JS
    // string turns Lua's single byte 0xff into the two bytes c3 bf, which
    // silently changes the digest.
    hash = (hash ^ BigInt(text.charCodeAt(i) & 0xff)) & mask;
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

function byteLength(text: string): number {
  // Binary string: one code unit per byte, matching Lua's `#text`.
  return text.length;
}

/**
 * Format a number exactly as C's (and therefore Lua's) `%.17g` does.
 *
 * `toPrecision(17)` is NOT equivalent and must not be substituted: `%g` strips
 * trailing zeros while `toPrecision` pads them, so 480.75 becomes "480.75" here
 * but "480.75000000000000" there — and even an integer like 100 diverges. Since
 * this value is length-prefixed into the canonical encoding and then hashed, any
 * difference changes the content digest, and a digest computed in Rust on one
 * peer must equal one computed here on another.
 *
 * Validated against the real Lua across 31 values including 1e-05, 1e+21,
 * subnormals and DBL_MAX.
 */
export function formatG17(value: number): string {
  const precision = 17;
  if (Number.isNaN(value)) {
    return "nan";
  }
  if (!Number.isFinite(value)) {
    return value > 0 ? "inf" : "-inf";
  }
  if (value === 0) {
    return Object.is(value, -0) ? "-0" : "0";
  }
  const exponential = value.toExponential(precision - 1);
  const exponent = Number(exponential.slice(exponential.indexOf("e") + 1));
  if (exponent >= -4 && exponent < precision) {
    const fixed = value.toFixed(Math.max(0, precision - 1 - exponent));
    return fixed.includes(".") ? fixed.replace(/0+$/, "").replace(/\.$/, "") : fixed;
  }
  const split = exponential.indexOf("e");
  const rawMantissa = exponential.slice(0, split);
  const mantissa = rawMantissa.includes(".")
    ? rawMantissa.replace(/0+$/, "").replace(/\.$/, "")
    : rawMantissa;
  // C pads the exponent to at least two digits; JS does not.
  const sign = exponent < 0 ? "-" : "+";
  return `${mantissa}e${sign}${String(Math.abs(exponent)).padStart(2, "0")}`;
}

export type DiagnosticsDomain = "identity" | "canonical" | "runtime" | "anchor";

export type DiagnosticsFieldKind =
  | "boolean"
  | "integer"
  | "number"
  | "id" // bounded join key: no direct identifiers, no paths, no addresses
  | "string" // bounded opaque machine string
  | "text" // bounded human-readable text, never a join key
  | "hash" // lowercase fnv1a64 hex digest
  | "enum"
  | "array"
  | "map"
  | "record";

export interface DiagnosticsField {
  readonly name?: string;
  readonly kind: DiagnosticsFieldKind;
  readonly domain?: DiagnosticsDomain; // Inherited from the enclosing record when absent.
  readonly optional?: boolean;
  readonly values?: Readonly<Record<string, true>>; // enum members
  readonly element?: DiagnosticsField; // array/map element shape
  readonly fields?: readonly DiagnosticsField[]; // record fields, in canonical order
  readonly min?: number;
  readonly max?: number;
  readonly min_length?: number;
  readonly max_length?: number;
}

export interface DiagnosticsShape extends DiagnosticsField {
  readonly kind: "record";
  readonly name: string;
  readonly fields: readonly DiagnosticsField[];
}

function invariant(condition: boolean, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}

// Bumping this changes every canonical preimage and therefore every export
// digest. It is a coordinated breaking change; see docs/online/net_diagnostics.md.
export const SERIALIZATION_VERSION = 1;

// Named so a future SHA-256 digest can be introduced as a new value instead
// of silently reinterpreting stored digests.
export const DIGEST = "fnv1a64/v1";
export const HASH_LENGTH = 16;

export const MAX_ID_LENGTH = 128;
export const MAX_STRING_LENGTH = 512;
export const MAX_TEXT_LENGTH = 2048;
export const MAX_ARRAY_LENGTH = 4096;
export const MAX_SAFE_INTEGER = 9007199254740992;

// The marker a bounded collection carries once it has dropped anything. It is
// a field, never a silently shorter array: a truncated export that looks
// complete is worse than no export.
export const TRUNCATED = "truncated";
export const COMPLETE = "complete";

// What a redacted field reads as. It is a constant so a spec can assert the
// absence of the original *and* the presence of the marker: a field that
// simply vanished cannot be told apart from a field nobody recorded.
export const REDACTED = "[redacted]";

const HEADER = "GCND";

// Join-key grammar. Alphanumeric start, then alphanumerics plus `_`, `-`, `.`.
// `@`, `:`, `/`, and `\` are excluded from the charset itself, so an email
// address, a URL, a filesystem path, and a bare IPv6 literal all fail on a
// character rather than on a substring blacklist that could be slipped past.
const ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_\-.]*$/;
const HASH_PATTERN = /^[0-9a-f]+$/;

// Belt and braces over the charset. `..` is the one a charset alone cannot
// catch, and a dotted quad is the one shape that satisfies the charset while
// being exactly the direct identifier this schema must never carry.
const FORBIDDEN_ID_SUBSTRINGS = ["..", "@"];
const IPV4_PATTERN = /^\d+\.\d+\.\d+\.\d+$/;

// Free text arriving from a transport is not a controlled vocabulary. A
// star's `last_error` and a peer event's message are produced by
// `String(error)` on a WebRTC or DOM exception and by the browser bridge's
// own prose, and neither this schema nor the transport contract constrains a
// single byte of it. Once STUN/TURN is configured there is no reason to
// assume such a string will not contain a candidate line, a relay URL, or a
// peer address verbatim.
//
// So free text is *checked*, not merely shortened. A string matching any
// sensitive shape is replaced whole rather than scrubbed in place: partial
// scrubbing of a grammar nobody controls is the same half-measure as
// digesting an SDP instead of dropping it, and it fails the moment the text
// is shaped slightly differently than expected. A benign "outbound queue
// reached its limit" survives untouched.
//
// This deliberately over-rejects. Two or more colons is treated as
// address-shaped even though a wall-clock time like `12:34:56` also matches,
// because losing one timestamp from a diagnostic detail is cheap and leaking
// an IPv6 address is not.
const SENSITIVE_TEXT_SUBSTRINGS = [
  "ice-",
  "candidate",
  "fingerprint",
  "sdp",
  "stun:",
  "turn:",
  "turns:",
  "://",
  "@",
];

// SDP body lines are `<key>=<value>` at a line start. Matched at line starts
// only, so ordinary prose such as "data=17 rejected" is not swept up.
const SDP_LINE_KEYS = ["v", "o", "s", "c", "t", "m", "a", "b", "i", "u", "k", "r", "z"];

// Vocabulary guards. These are matched against lowercase field names.
const WALL_CLOCK_WORDS = [
  "_ms$",
  "^ms_",
  "_ms_",
  "rtt",
  "jitter",
  "wall",
  "monotonic",
  "elapsed",
  "timestamp",
  "_at$",
  "second",
  "millis",
  "realtime",
  "frame_time",
];

const SIMULATION_WORDS = [
  "tick",
  "boundary",
  "hash",
  "checkpoint",
  "confirmed",
  "rollback",
  "resimulat",
  "snapshot",
];

const DOMAINS: Readonly<Record<DiagnosticsDomain, true>> = {
  identity: true,
  canonical: true,
  runtime: true,
  anchor: true,
};

function isFinite_(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isInteger(value: unknown): value is number {
  return isFinite_(value) && value === Math.floor(value) && Math.abs(value) <= MAX_SAFE_INTEGER;
}

function matchesVocabulary(name: string, words: readonly string[]): string | null {
  const lowered = name.toLowerCase();
  for (const word of words) {
    if (new RegExp(word).test(lowered)) {
      return word;
    }
  }
  return null;
}

// Does this free text look like it carries network or identity material?
export function isSensitiveText(text: unknown): boolean {
  if (typeof text !== "string") {
    return false;
  }
  const lowered = text.toLowerCase();
  for (const needle of SENSITIVE_TEXT_SUBSTRINGS) {
    if (lowered.includes(needle)) {
      return true;
    }
  }
  // A dotted quad, anywhere in the string.
  if (/\d+\.\d+\.\d+\.\d+/.test(lowered)) {
    return true;
  }
  // Address-shaped: two or more colons covers every IPv6 form, compressed or
  // not, without needing to parse one.
  let colons = 0;
  for (const char of lowered) {
    if (char === ":") {
      colons += 1;
      if (colons >= 2) {
        return true;
      }
    }
  }
  // An SDP body line at a line start.
  const probe = "\n" + lowered.replace(/\r/g, "\n");
  for (const key of SDP_LINE_KEYS) {
    if (probe.includes("\n" + key + "=")) {
      return true;
    }
  }
  return false;
}

// Bound and redact one free-text field. Returns the redaction marker when
// the text looks sensitive, the truncated text when it is merely long, and
// the text unchanged otherwise.
export function redactFreeText(text: unknown, maxBytes: number): string | null {
  if (typeof text !== "string") {
    return null;
  }
  if (isSensitiveText(text)) {
    return REDACTED;
  }
  if (byteLength(text) <= maxBytes) {
    return text;
  }
  // Truncate to `maxBytes` bytes (not code units) before appending the
  // marker, mirroring the Lua original's `#text` byte accounting.
  const budget = Math.max(0, maxBytes - byteLength(TRUNCATED) - 1);
  // Binary string: slicing code units is slicing bytes, so this matches Lua's
  // `string.sub` byte accounting exactly.
  return text.slice(0, budget) + " " + TRUNCATED;
}

export function isWallClockName(name: string): boolean {
  return matchesVocabulary(name, WALL_CLOCK_WORDS) !== null;
}

export function isSimulationName(name: string): boolean {
  return matchesVocabulary(name, SIMULATION_WORDS) !== null;
}

interface DomainScope {
  wall: boolean;
  simulation: boolean;
  mapping_error: boolean;
}

function assertDomain(
  field: DiagnosticsField,
  domain: DiagnosticsDomain,
  path: string,
  seen: DomainScope
): void {
  const name = field.name;
  if (name === undefined) {
    return;
  }
  const wall = matchesVocabulary(name, WALL_CLOCK_WORDS);
  const simulation = matchesVocabulary(name, SIMULATION_WORDS);
  if (wall !== null) {
    seen.wall = true;
  }
  if (simulation !== null) {
    seen.simulation = true;
  }
  if (name === "mapping_error_ms") {
    seen.mapping_error = true;
  }
  if (domain === "identity" || domain === "canonical") {
    invariant(
      wall === null,
      `${path} is ${domain} and may not use wall-clock vocabulary (${String(wall)})`
    );
  } else if (domain === "runtime") {
    invariant(
      simulation === null,
      `${path} is runtime observation and may not use simulation vocabulary (${String(simulation)})`
    );
  }
}

// One scope per anchor subtree, keyed on the *domain transition* into
// `anchor` rather than on being the outermost call. See the Lua original's
// comment for the regression this fixes: the scope must be created fresh
// whenever a domainless container's descendant first declares `anchor`, not
// only at the root.
function assertFieldShape(
  field: DiagnosticsField,
  inherited: DiagnosticsDomain | undefined,
  path: string,
  seen: DomainScope | undefined
): void {
  invariant(typeof field.kind === "string", `${path} shape needs a kind`);
  const domain = field.domain ?? inherited;
  if (field.domain !== undefined) {
    invariant(DOMAINS[field.domain] === true, `${path} declares an unknown domain`);
    invariant(
      inherited === undefined || inherited === field.domain,
      `${path} may not change domain from ${String(inherited)}`
    );
  }
  const enteringAnchor = domain === "anchor" && inherited !== "anchor";
  let scope = seen;
  if (enteringAnchor || scope === undefined) {
    scope = { wall: false, simulation: false, mapping_error: false };
  }
  if (domain !== undefined) {
    assertDomain(field, domain, path, scope);
  } else {
    // Only a container may be domainless, and only so its children can each
    // declare one. A leaf with no domain would escape the vocabulary guard.
    invariant(
      field.kind === "record" || field.kind === "array" || field.kind === "map",
      `${path} has no domain and none is inherited`
    );
  }
  if (field.kind === "enum") {
    invariant(typeof field.values === "object" && field.values !== null, `${path} enum shape needs values`);
    let count = 0;
    for (const member of Object.keys(field.values)) {
      invariant(field.values[member] === true, `${path} enum member is invalid`);
      count += 1;
    }
    invariant(count > 0, `${path} enum shape needs at least one member`);
  } else if (field.kind === "array" || field.kind === "map") {
    invariant(field.element !== undefined, `${path} needs an element shape`);
    assertFieldShape(field.element, domain, `${path}[]`, scope);
  } else if (field.kind === "record") {
    invariant(field.fields !== undefined, `${path} record shape needs fields`);
    const names = new Set<string>();
    for (const child of field.fields) {
      invariant(typeof child.name === "string" && child.name !== "", `${path} field needs a name`);
      invariant(!names.has(child.name), `${path} field ${child.name} is declared twice`);
      names.add(child.name);
      assertFieldShape(child, domain, `${path}.${child.name}`, scope);
    }
  }
  // An anchor is the only place both vocabularies may meet, and it only
  // earns that by actually binding them and by naming its own error term.
  if (enteringAnchor) {
    invariant(scope.wall, `${path} anchor must name a wall-clock observation`);
    invariant(scope.simulation, `${path} anchor must name a simulation tick`);
    invariant(scope.mapping_error, `${path} anchor must declare mapping_error_ms`);
  }
}

// Build a validated record shape. A broken shape is a programmer error, so
// this throws rather than returning a diagnostic.
export function record(
  name: string,
  domain: DiagnosticsDomain | undefined,
  fields: readonly DiagnosticsField[]
): DiagnosticsShape {
  invariant(name !== "", "diagnostic shape needs a name");
  const shape: DiagnosticsShape = {
    name,
    kind: "record",
    fields,
    ...(domain !== undefined ? { domain } : {}),
  };
  assertFieldShape(shape, undefined, name, undefined);
  return shape;
}

// A record shape nested inside another: it inherits its parent's domain, so
// it is checked when the parent is built rather than standalone.
export function nested(name: string, fields: readonly DiagnosticsField[]): DiagnosticsField {
  return { name, kind: "record", fields };
}

export function enumValues(members: readonly string[]): Readonly<Record<string, true>> {
  const values: Record<string, true> = {};
  for (const member of members) {
    invariant(member !== "", "enum member must be a non-empty string");
    invariant(values[member] === undefined, `enum member ${member} is declared twice`);
    values[member] = true;
  }
  return values;
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

type ValidationResult = Result<true, string>;

function fail(error: string): ValidationResult {
  return err(error);
}

function validateString(field: DiagnosticsField, value: unknown, path: string): ValidationResult {
  if (typeof value !== "string") {
    return fail(`${path} must be a string`);
  }
  const max =
    field.max_length ??
    (field.kind === "text" ? MAX_TEXT_LENGTH : field.kind === "id" ? MAX_ID_LENGTH : MAX_STRING_LENGTH);
  const min = field.min_length ?? (field.kind === "text" ? 0 : 1);
  const length = byteLength(value);
  if (length < min) {
    return fail(`${path} must be at least ${min} bytes`);
  }
  if (length > max) {
    return fail(`${path} must be at most ${max} bytes`);
  }
  if (field.kind === "hash") {
    if (length !== HASH_LENGTH || !HASH_PATTERN.test(value)) {
      return fail(`${path} must be a ${DIGEST} hex digest`);
    }
    return ok(true);
  }
  if (field.kind === "id") {
    if (!ID_PATTERN.test(value)) {
      return fail(`${path} is not a bounded pseudonymous id`);
    }
    for (const forbidden of FORBIDDEN_ID_SUBSTRINGS) {
      if (value.includes(forbidden)) {
        return fail(`${path} may not contain ${forbidden}`);
      }
    }
    if (IPV4_PATTERN.test(value)) {
      return fail(`${path} looks like a raw address`);
    }
  }
  return ok(true);
}

function validateField(field: DiagnosticsField, value: unknown, path: string): ValidationResult {
  const kind = field.kind;
  if (kind === "boolean") {
    if (typeof value !== "boolean") {
      return fail(`${path} must be a boolean`);
    }
    return ok(true);
  }
  if (kind === "integer") {
    if (!isInteger(value)) {
      return fail(`${path} must be a finite integer`);
    }
  } else if (kind === "number") {
    if (!isFinite_(value)) {
      return fail(`${path} must be a finite number`);
    }
  }
  if (kind === "integer" || kind === "number") {
    const numeric = value as number;
    if (field.min !== undefined && numeric < field.min) {
      return fail(`${path} must be at least ${field.min}`);
    }
    if (field.max !== undefined && numeric > field.max) {
      return fail(`${path} must be at most ${field.max}`);
    }
    return ok(true);
  }
  if (kind === "enum") {
    if (typeof value !== "string") {
      return fail(`${path} must be a string`);
    }
    invariant(field.values !== undefined, `${path} enum shape needs values`);
    if (field.values[value] !== true) {
      return fail(`${path} is not a declared member`);
    }
    return ok(true);
  }
  if (kind === "id" || kind === "string" || kind === "text" || kind === "hash") {
    return validateString(field, value, path);
  }
  if (kind === "array") {
    if (!Array.isArray(value)) {
      return fail(`${path} must be an array`);
    }
    const max = field.max_length ?? MAX_ARRAY_LENGTH;
    if (value.length > max) {
      return fail(`${path} holds more than ${max} entries`);
    }
    invariant(field.element !== undefined, `${path} needs an element shape`);
    for (let index = 0; index < value.length; index += 1) {
      const result = validateField(field.element, value[index], `${path}[${index + 1}]`);
      if (!result.ok) {
        return result;
      }
    }
    return ok(true);
  }
  if (kind === "map") {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return fail(`${path} must be a map`);
    }
    invariant(field.element !== undefined, `${path} needs an element shape`);
    const entries = Object.entries(value as Record<string, unknown>);
    for (const [key, child] of entries) {
      const keyResult = validateString({ kind: "id" }, key, `${path} key`);
      if (!keyResult.ok) {
        return keyResult;
      }
      const childResult = validateField(field.element, child, `${path}.${key}`);
      if (!childResult.ok) {
        return childResult;
      }
    }
    const max = field.max_length ?? MAX_ARRAY_LENGTH;
    if (entries.length > max) {
      return fail(`${path} holds more than ${max} entries`);
    }
    return ok(true);
  }
  if (kind === "record") {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      return fail(`${path} must be a record`);
    }
    invariant(field.fields !== undefined, `${path} record shape needs fields`);
    const record = value as Record<string, unknown>;
    const declared = new Set<string>();
    for (const child of field.fields) {
      invariant(child.name !== undefined, `${path} field needs a name`);
      declared.add(child.name);
      const childValue = record[child.name];
      if (childValue === undefined) {
        if (child.optional !== true) {
          return fail(`${path}.${child.name} is required`);
        }
      } else {
        const result = validateField(child, childValue, `${path}.${child.name}`);
        if (!result.ok) {
          return result;
        }
      }
    }
    for (const key of Object.keys(record)) {
      if (!declared.has(key)) {
        return fail(`${path}.${key} is not declared by this schema`);
      }
    }
    return ok(true);
  }
  return fail(`${path} has an unsupported kind`);
}

// Validate a value against a shape.
export function validate(shape: DiagnosticsShape, value: unknown): ValidationResult {
  return validateField(shape, value, shape.name);
}

// ---------------------------------------------------------------------------
// Canonical serialization
// ---------------------------------------------------------------------------

function lp(text: string): string {
  return `${byteLength(text)}:${text}`;
}

// Integers print exactly; other numbers print with a fixed
// round-trip-precision format so the preimage cannot depend on locale or an
// implementation-specific default `toString`.
function numberBytes(value: number): string {
  if (value === Math.floor(value) && Math.abs(value) <= MAX_SAFE_INTEGER) {
    return String(value);
  }
  return formatG17(value);
}

function encodeField(field: DiagnosticsField, value: unknown, out: string[]): void {
  const kind = field.kind;
  if (kind === "boolean") {
    out.push(value === true ? "T" : "F");
    return;
  }
  if (kind === "integer" || kind === "number") {
    out.push(lp(numberBytes(value as number)));
    return;
  }
  if (kind === "array") {
    const array = value as readonly unknown[];
    out.push(lp(String(array.length)));
    invariant(field.element !== undefined, "array field needs an element shape");
    for (const item of array) {
      encodeField(field.element, item, out);
    }
    return;
  }
  if (kind === "map") {
    const map = value as Record<string, unknown>;
    const keys = Object.keys(map).sort();
    out.push(lp(String(keys.length)));
    invariant(field.element !== undefined, "map field needs an element shape");
    for (const key of keys) {
      out.push(lp(key));
      encodeField(field.element, map[key], out);
    }
    return;
  }
  if (kind === "record") {
    invariant(field.fields !== undefined, "record field needs fields");
    const record = value as Record<string, unknown>;
    for (const child of field.fields) {
      invariant(child.name !== undefined, "record field needs a name");
      const childValue = record[child.name];
      if (childValue === undefined) {
        out.push("-");
      } else {
        out.push("+");
        encodeField(child, childValue, out);
      }
    }
    return;
  }
  out.push(lp(value as string));
}

// Canonical bytes for a validated value. Byte-identical for byte-identical
// input, independent of object key order.
export function encode(shape: DiagnosticsShape, value: unknown): Result<string, string> {
  const validated = validate(shape, value);
  if (!validated.ok) {
    return err(validated.error);
  }
  const out = [HEADER, ";", String(SERIALIZATION_VERSION), ";"];
  out.push(lp(shape.name));
  encodeField(shape, value, out);
  return ok(out.join(""));
}

export function digest(shape: DiagnosticsShape, value: unknown): Result<string, string> {
  const encoded = encode(shape, value);
  if (!encoded.ok) {
    return encoded;
  }
  return ok(fnv1a64Hash(encoded.value));
}

export function tupleDigest(label: string, parts: readonly string[]): string {
  const buffer = [HEADER, ";", String(SERIALIZATION_VERSION), ";", lp(label)];
  buffer.push(lp(String(parts.length)));
  for (const part of parts) {
    buffer.push(lp(part));
  }
  return fnv1a64Hash(buffer.join(""));
}

// A sub-shape holding only the top-level sections in `allowed`. The point is
// a digest over *just* the deterministic half of an artifact: canonical
// evidence can honestly be claimed byte-stable across runs, and runtime
// observation cannot, so the two need separate preimages rather than one
// preimage and an optimistic assertion.
export function project(
  shape: DiagnosticsShape,
  allowed: Readonly<Partial<Record<DiagnosticsDomain, true>>>
): DiagnosticsShape {
  const kept: DiagnosticsField[] = [];
  const names: string[] = [];
  for (const field of shape.fields) {
    if (field.domain !== undefined && allowed[field.domain] === true) {
      kept.push(field);
      names.push(field.domain);
    }
  }
  invariant(kept.length > 0, "a projection must keep at least one section");
  names.sort();
  return record(`${shape.name}/${names.join("+")}`, undefined, kept);
}

// Walk a shape and report every declared field path in its domain. Specs use
// this to assert the separation holds across the *whole* schema rather than
// at the handful of fields a hand-written test happens to name.
export function domains(shape: DiagnosticsShape): Readonly<Record<string, DiagnosticsDomain | undefined>> {
  const result: Record<string, DiagnosticsDomain | undefined> = {};
  function walk(field: DiagnosticsField, inherited: DiagnosticsDomain | undefined, path: string): void {
    const domain = field.domain ?? inherited;
    if (field.name !== undefined) {
      result[path] = domain;
    }
    if (field.kind === "record") {
      invariant(field.fields !== undefined, `${path} record shape needs fields`);
      for (const child of field.fields) {
        walk(child, domain, `${path}.${String(child.name)}`);
      }
    } else if (field.kind === "array" || field.kind === "map") {
      invariant(field.element !== undefined, `${path} needs an element shape`);
      walk(field.element, domain, `${path}[]`);
    }
  }
  walk(shape, undefined, shape.name);
  return result;
}
