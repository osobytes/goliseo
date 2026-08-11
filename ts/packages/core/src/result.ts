/**
 * A discriminated union for an expected, recoverable failure, instead of
 * throwing.
 *
 * Use this for *expected, recoverable* failures the caller is meant to handle
 * (lookup miss, validation of external input). Programmer errors and broken
 * invariants should still throw — see AGENTS.md §7.
 */
export type Result<T, E = string> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: E };

/** Wrap a success value. */
export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value };
}

/** Wrap a recoverable failure. */
export function err<E>(error: E): Result<never, E> {
  return { ok: false, error };
}
