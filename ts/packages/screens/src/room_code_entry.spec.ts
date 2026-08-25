// The pure room-code composer, exercised directly (#610 round-2 review,
// "also" item: a dedicated spec for the newly extracted module) --
// `lobby.spec.ts` already covers it indirectly through both call sites
// (the guest's own composer, the host's inline "switch to guest" entry),
// but neither of those proves the editing rules themselves in isolation,
// independent of a screen's own focus/widget plumbing.

import { describe, expect, it } from "vitest";
import {
  newRoomCodeEntry,
  roomCodeCursor,
  roomCodeCycle,
  roomCodeDisplay,
  roomCodeKey,
  roomCodeText,
  ROOM_CODE_ALPHABET,
  ROOM_CODE_LENGTH,
  type RoomCodeEntry,
} from "./room_code_entry.ts";

function typed(chars: string): RoomCodeEntry {
  let entry = newRoomCodeEntry();
  for (const ch of chars) {
    entry = roomCodeKey(entry, ch);
  }
  return entry;
}

describe("room_code_entry", () => {
  it("starts empty, at the first slot", () => {
    const entry = newRoomCodeEntry();
    expect(entry.chars.length).toBe(ROOM_CODE_LENGTH);
    expect(entry.chars.every((ch) => ch === "")).toBe(true);
    expect(entry.cursor).toBe(0);
  });

  it("types a character, uppercased, and advances the cursor", () => {
    const entry = roomCodeKey(newRoomCodeEntry(), "a");
    expect(entry.chars[0]).toBe("A");
    expect(entry.cursor).toBe(1);
  });

  it("does not mutate its input entry", () => {
    const entry = newRoomCodeEntry();
    const next = roomCodeKey(entry, "A");
    expect(entry.chars[0]).toBe("");
    expect(entry.cursor).toBe(0);
    expect(next.chars[0]).toBe("A");
  });

  it("ignores a character outside the closed alphabet", () => {
    // I, L, O, U are deliberately excluded (`ROOM_CODE_ALPHABET`'s own
    // doc, mirroring `infra/src/room_code.ts`).
    for (const excluded of ["I", "L", "O", "U", "!", " "]) {
      const entry = roomCodeKey(newRoomCodeEntry(), excluded);
      expect(entry.chars[0], `expected "${excluded}" to be rejected`).toBe("");
      expect(entry.cursor).toBe(0);
    }
  });

  it("ignores a multi-character key name (Enter, Shift, ...)", () => {
    const entry = roomCodeKey(newRoomCodeEntry(), "Enter");
    expect(entry.chars[0]).toBe("");
    expect(entry.cursor).toBe(0);
  });

  it("stops ADVANCING the cursor past the last slot, but a keystroke there still edits it", () => {
    const entry = typed("ABCDEF");
    expect(entry.chars.join("")).toBe("ABCDEF");
    expect(entry.cursor).toBe(ROOM_CODE_LENGTH - 1);
    // The cursor is clamped at the last slot, not past it -- typing there
    // is a normal, in-place edit (overwrite the last character), not a
    // no-op the way typing past a truly full field might read.
    const overwritten = roomCodeKey(entry, "G");
    expect(overwritten.chars.join("")).toBe("ABCDEG");
    expect(overwritten.cursor).toBe(ROOM_CODE_LENGTH - 1);
  });

  it("Backspace clears the slot the cursor lands on, stepping back first when that slot is already empty", () => {
    let entry = typed("AB");
    expect(entry.cursor).toBe(2);
    // The cursor sits past the typed characters, on an empty slot -- steps
    // back and clears the one before it ("B"). The cursor now sits on
    // THAT slot, which the clear itself just emptied.
    entry = roomCodeKey(entry, "Backspace");
    expect(entry.chars.join("")).toBe("A");
    expect(entry.cursor).toBe(1);
    // The cursor is on an empty slot again (the one just cleared) -- steps
    // back once more and clears "A".
    entry = roomCodeKey(entry, "Backspace");
    expect(entry.chars.join("")).toBe("");
    expect(entry.cursor).toBe(0);
  });

  it("Backspace at the very first, empty slot does nothing", () => {
    const entry = newRoomCodeEntry();
    expect(roomCodeKey(entry, "Backspace")).toEqual(entry);
  });

  it("moves the cursor by delta, clamped to the composer's bounds", () => {
    const entry = newRoomCodeEntry();
    expect(roomCodeCursor(entry, 3).cursor).toBe(3);
    expect(roomCodeCursor(entry, -5).cursor).toBe(0);
    expect(roomCodeCursor(entry, 99).cursor).toBe(ROOM_CODE_LENGTH - 1);
    // Pure: chars are untouched by a cursor move.
    const moved = roomCodeCursor(typed("A"), 1);
    expect(moved.chars[0]).toBe("A");
  });

  it("cycles the character under the cursor through the alphabet, wrapping both ways", () => {
    const first = ROOM_CODE_ALPHABET[0] as string;
    const last = ROOM_CODE_ALPHABET[ROOM_CODE_ALPHABET.length - 1] as string;
    // From an empty slot (treated as "one before the first" internally),
    // cycling up lands on the first character. Cycling down does NOT
    // land symmetrically on the last one -- it treats empty as index -1
    // and steps one further, landing one short of the end. This is the
    // pre-existing behavior carried over unchanged from `lobby_model.ts`'s
    // own `roomCycle` (#610's extraction moved it, it did not change it);
    // pinned here as a characterization, not endorsed as ideal.
    expect(roomCodeCycle(newRoomCodeEntry(), 1).chars[0]).toBe(first);
    expect(roomCodeCycle(newRoomCodeEntry(), -1).chars[0]).toBe(
      ROOM_CODE_ALPHABET[ROOM_CODE_ALPHABET.length - 2],
    );
    // From the last character, one more step up wraps back to the first --
    // symmetric wrapping DOES hold once a real character occupies the
    // slot.
    const atLast = { chars: [last, "", "", "", "", ""], cursor: 0 };
    expect(roomCodeCycle(atLast, 1).chars[0]).toBe(first);
    expect(roomCodeCycle({ chars: [first, "", "", "", "", ""], cursor: 0 }, -1).chars[0]).toBe(
      last,
    );
    // Cycling never moves the cursor.
    expect(roomCodeCycle(atLast, 1).cursor).toBe(0);
  });

  it("reports the code only once every slot is filled", () => {
    expect(roomCodeText(newRoomCodeEntry())).toBeUndefined();
    expect(roomCodeText(typed("ABC"))).toBeUndefined();
    expect(roomCodeText(typed("A3F9K2"))).toBe("A3F9K2");
  });

  it("displays the cursor bracketed, empty slots as underscores", () => {
    const entry = typed("A3");
    expect(roomCodeDisplay(entry)).toBe("A 3 [_] _ _ _");
    expect(roomCodeDisplay(newRoomCodeEntry())).toBe("[_] _ _ _ _ _");
    expect(roomCodeDisplay(typed("A3F9K2"))).toBe("A 3 F 9 K [2]");
  });
});
