//! Short, human-friendly, single-use room codes.
//!
//! Crockford's base32 alphabet: digits and uppercase letters, excluding
//! `I`, `L`, `O` and `U` so a spoken or handwritten code never collides
//! with a similar-looking digit or letter. 32 symbols, 6 of them: ~1.07e9
//! codes, plenty for a friends-scale room that is single-use and expires.

/** The 32-symbol alphabet room codes are drawn from. */
export const ROOM_CODE_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** Fixed length of a generated room code. */
export const ROOM_CODE_LENGTH = 6;

/**
 * Turn raw random bytes into a room code. `randomBytes` must supply at
 * least `ROOM_CODE_LENGTH` bytes -- the caller is expected to have drawn
 * them from a cryptographically secure source (`crypto.getRandomValues`),
 * never `Math.random()`.
 *
 * 256 is an exact multiple of the 32-symbol alphabet, so `byte % 32`
 * introduces no modulo bias.
 */
export function generateRoomCode(randomBytes: Uint8Array): string {
  if (randomBytes.length < ROOM_CODE_LENGTH) {
    throw new Error(
      `generateRoomCode needs at least ${ROOM_CODE_LENGTH} random bytes, got ${randomBytes.length}`,
    );
  }
  let code = "";
  for (let i = 0; i < ROOM_CODE_LENGTH; i += 1) {
    const byte = randomBytes[i];
    if (byte === undefined) {
      throw new Error("generateRoomCode: random byte index out of range");
    }
    code += ROOM_CODE_ALPHABET[byte % ROOM_CODE_ALPHABET.length];
  }
  return code;
}

/** Whether `candidate` has the shape of a code `generateRoomCode` could produce. */
export function isValidRoomCode(candidate: string): boolean {
  if (candidate.length !== ROOM_CODE_LENGTH) {
    return false;
  }
  for (const ch of candidate) {
    if (!ROOM_CODE_ALPHABET.includes(ch)) {
      return false;
    }
  }
  return true;
}
