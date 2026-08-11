// Fails loud on a broken invariant that should be impossible if the
// surrounding code is correct (AGENTS.md §7). Never use this for expected,
// recoverable failures — see @gc/core's `Result` for those.

/** Throws with `message` when `condition` is false. Narrows on the caller's side. */
export function invariant(condition: boolean, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}
