// Pins `input_sample.ts`'s quantization and bit-packing against
// `./fixtures/input_sample_vector.ts` (a verbatim embedded copy of
// `../fixtures/input_sample_vector.txt` -- see that file's header for why
// it is embedded rather than read from disk), generated straight from
// `gc_sim::input_frame` (rust/crates/gc-sim/tests/input_sample_vector_generator.rs;
// see that file for the generation command). Per ARCHITECTURE.md §1.2, a module
// duplicated into a second language "must be pinned by a shared vector
// file" -- a round trip against this file's own code would prove nothing
// about whether it agrees with Rust, only that it agrees with itself.
//
// Every case supplies `held_actions`/`edge_actions` by canonical NAME, not
// by precomputed bitmask, so this test exercises the same bit constants
// `input_sample.ts` ships (`HELD_BITS`/`EDGE_BITS`) rather than re-deriving
// its own copy of them here. If those constants ever drifted from
// `gc_sim::input_frame::HeldAction::bit()`/`EdgeAction::bit()` -- the exact
// mistake this vector exists to catch -- the packed `held`/`edges` this
// test computes would stop matching the vector's `held`/`edges` fields,
// even though `raw_move_x`/`raw_move_y` -> `move_x`/`move_y` still agreed.

import { describe, expect, it } from "vitest";
import { buildInputSample, type EdgeActionName, type HeldActionName } from "./input_sample.ts";
import { INPUT_SAMPLE_VECTOR } from "./fixtures/input_sample_vector.ts";

interface VectorCase {
  readonly name: string;
  readonly rawMoveX: number;
  readonly rawMoveY: number;
  readonly heldActions: readonly HeldActionName[];
  readonly edgeActions: readonly EdgeActionName[];
  readonly moveX: number;
  readonly moveY: number;
  readonly held: number;
  readonly edges: number;
}

function splitNamed(value: string): readonly string[] {
  return value === "" ? [] : value.split(",");
}

function parseVector(text: string): readonly VectorCase[] {
  const cases: VectorCase[] = [];
  let fields: Map<string, string> | null = null;

  function flush(): void {
    if (fields === null) {
      return;
    }
    const activeFields = fields;
    const get = (key: string): string => {
      const value = activeFields.get(key);
      if (value === undefined) {
        throw new Error(`vector case is missing field ${key}`);
      }
      return value;
    };
    cases.push({
      name: get("case"),
      rawMoveX: Number(get("raw_move_x")),
      rawMoveY: Number(get("raw_move_y")),
      heldActions: splitNamed(get("held_actions")) as HeldActionName[],
      edgeActions: splitNamed(get("edge_actions")) as EdgeActionName[],
      moveX: Number(get("move_x")),
      moveY: Number(get("move_y")),
      held: Number(get("held")),
      edges: Number(get("edges")),
    });
    fields = null;
  }

  for (const rawLine of text.split("\n")) {
    // Strip only a trailing carriage return (Windows checkout), never
    // trailing whitespace in general -- a legitimately empty
    // `held_actions`/`edge_actions` value is nothing at all after the key
    // (see the fallback below), so there is no trailing content here to
    // accidentally eat either way.
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    if (line === "") {
      flush();
      continue;
    }
    if (line.startsWith("#")) {
      continue;
    }
    const tabIndex = line.indexOf("\t");
    // A field with an empty value (`held_actions`/`edge_actions` when a
    // case names no actions) may have lost its trailing tab in transit
    // (e.g. a template literal, or an editor stripping trailing
    // whitespace) without losing any information -- there is nothing
    // after it either way. Treat "no tab" as "key with an empty value"
    // rather than a malformed line.
    const key = tabIndex === -1 ? line : line.slice(0, tabIndex);
    const value = tabIndex === -1 ? "" : line.slice(tabIndex + 1);
    if (key === "case") {
      flush();
      fields = new Map();
    }
    if (fields === null) {
      throw new Error(`vector field ${key} appears before a case line`);
    }
    fields.set(key, value);
  }
  flush();
  return cases;
}

const cases = parseVector(INPUT_SAMPLE_VECTOR);

describe("input sample vector (pinned against gc_sim::input_frame)", () => {
  it("the fixture actually has cases to check", () => {
    expect(cases.length).toBeGreaterThan(20);
  });

  for (const testCase of cases) {
    it(`reproduces ${testCase.name}`, () => {
      const sample = buildInputSample({
        rawMoveX: testCase.rawMoveX,
        rawMoveY: testCase.rawMoveY,
        held: testCase.heldActions,
        edges: testCase.edgeActions,
      });
      expect(sample.move_x).toBe(testCase.moveX);
      expect(sample.move_y).toBe(testCase.moveY);
      expect(sample.held).toBe(testCase.held);
      expect(sample.edges).toBe(testCase.edges);
    });
  }
});
