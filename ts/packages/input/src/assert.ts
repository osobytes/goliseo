// Throws on a broken invariant (AGENTS.md §7): a condition that should be
// impossible if the surrounding code is correct.
//
// This is intentionally a standalone copy of @gc/ui's identical helper
// rather than a shared dependency — a 4-line utility does not justify a
// cross-package dependency edit (see `controller.ts`'s header for the same
// reasoning re: @gc/ui).

/** Throws with `message` when `condition` is false. Narrows on the caller's side. */
export function invariant(condition: boolean, message: string): asserts condition {
  if (!condition) {
    throw new Error(message);
  }
}
