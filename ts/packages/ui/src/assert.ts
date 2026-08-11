// The TS analogue of Lua's `assert(cond, msg)` (AGENTS.md §7): a broken
// invariant that should be impossible if the surrounding code is correct.
// Never use this for expected, recoverable failures — see @gc/core's
// `Result` for those.

/** Throws with `message` when `condition` is false. Narrows on the caller's side. */
export function invariant(condition: boolean, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}
