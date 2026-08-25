// A pure, reusable six-character room-code composer: the character grid a
// player edits before submitting a room code. Shared, within `lobby.ts`,
// between the guest's own post-role composer (`lobby_model.ts`'s
// `room_entry`, `ROOM_CODE_ENTRY_WIDGET`) and the host's inline "have a
// code? switch to guest" entry on the auto-hosted handshake screen
// (`JOIN_ENTRY_WIDGET`, #610 round-2 review, blocking finding 1c) -- both
// edit the exact same six-slot, closed-alphabet grid, so the editing rules
// live here once rather than twice. (An earlier revision of #610 also
// threaded this through a since-deleted `multiplayer.ts` front door; that
// screen folded into the hosting screen itself, but the extraction still
// earns its keep for the two composers that remain.)
//
// Every function here is pure: given an entry and an edit, return the next
// entry. Submission, focus, and what a completed code connects to are each
// screen's own concern -- this module never emits an effect and never
// knows what, if anything, exists behind a code once it is complete.

/** Mirrors `infra/src/room_code.ts`'s own alphabet and length exactly --
 * see `@gc/online`'s `room_signaling.ts` for the identical, independently
 * disclosed duplication on the wire-protocol side. `infra/` must stay
 * outside the game's dependency graph (AGENTS.md §8/§11), so the same
 * constant is duplicated a second time here; if the Worker's alphabet or
 * length ever changes, every one of these needs a matching update. */
export const ROOM_CODE_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export const ROOM_CODE_LENGTH = 6;

/** A room code composer's in-progress state: `ROOM_CODE_LENGTH` character
 * slots (empty string until typed) plus the cursor position being edited.
 * Keyboard input types a character and advances the cursor; a controller
 * cycles the character at the cursor (up/down) and moves it (left/right) --
 * each screen's own `update()` is where both are wired to this module's
 * functions. */
export interface RoomCodeEntry {
  readonly chars: readonly string[];
  readonly cursor: number;
}

/** A fresh, empty composer, cursor at the first slot. */
export function newRoomCodeEntry(): RoomCodeEntry {
  return { chars: new Array(ROOM_CODE_LENGTH).fill(""), cursor: 0 };
}

/** A single keystroke: `"Backspace"` clears the char under the cursor (or
 * steps back first if it is already empty), any other single alphabet
 * character overwrites it and advances the cursor. Anything else --
 * multi-character key names ("Enter", "Shift", ...), a character outside
 * the closed alphabet, typing past the last slot -- is ignored, returning
 * the entry unchanged. */
export function roomCodeKey(entry: RoomCodeEntry, key: string): RoomCodeEntry {
  if (key === "Backspace") {
    if (entry.chars[entry.cursor] === "" && entry.cursor === 0) {
      return entry;
    }
    const chars = [...entry.chars];
    const cursor = chars[entry.cursor] !== "" ? entry.cursor : entry.cursor - 1;
    chars[cursor] = "";
    return { chars, cursor };
  }
  if (key.length !== 1) {
    return entry;
  }
  const upper = key.toUpperCase();
  if (!ROOM_CODE_ALPHABET.includes(upper) || entry.cursor >= ROOM_CODE_LENGTH) {
    return entry;
  }
  const chars = [...entry.chars];
  chars[entry.cursor] = upper;
  return { chars, cursor: Math.min(ROOM_CODE_LENGTH - 1, entry.cursor + 1) };
}

/** Moves the cursor by `delta` slots, clamped to the composer's bounds. */
export function roomCodeCursor(entry: RoomCodeEntry, delta: number): RoomCodeEntry {
  return {
    chars: entry.chars,
    cursor: Math.max(0, Math.min(ROOM_CODE_LENGTH - 1, entry.cursor + delta)),
  };
}

/** Cycles the character under the cursor through the alphabet by `delta`
 * steps (wrapping both ways) -- a controller's up/down, with no keyboard. */
export function roomCodeCycle(entry: RoomCodeEntry, delta: number): RoomCodeEntry {
  const current = entry.chars[entry.cursor] ?? "";
  const n = ROOM_CODE_ALPHABET.length;
  const index = current === "" ? -1 : ROOM_CODE_ALPHABET.indexOf(current);
  const nextIndex = (((index + delta) % n) + n) % n;
  const chars = [...entry.chars];
  chars[entry.cursor] = ROOM_CODE_ALPHABET[nextIndex] as string;
  return { chars, cursor: entry.cursor };
}

/** The composer's code, once every slot is filled -- `undefined` while any
 * slot is still empty, so a caller cannot submit a partial code by
 * accident. */
export function roomCodeText(entry: RoomCodeEntry): string | undefined {
  if (entry.chars.some((ch) => ch === "")) {
    return undefined;
  }
  return entry.chars.join("");
}

/** The composer rendered as one line, the cursor bracketed -- the exact
 * display `lobby.ts`'s own composer widget used before this extraction,
 * shared so both screens read identically. */
export function roomCodeDisplay(entry: RoomCodeEntry): string {
  return entry.chars
    .map((ch, index) => (index === entry.cursor ? `[${ch || "_"}]` : ch || "_"))
    .join(" ");
}
