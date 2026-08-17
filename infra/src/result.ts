//! The shared discriminated-union `Result` type, mirroring `@gc/core`'s
//! shape (AGENTS.md §7). `infra/` is a self-contained deploy unit with its
//! own `package.json` and is never imported by any game package, so this is
//! a deliberate, small duplication rather than a workspace dependency.

/** Either a success carrying `value`, or a failure carrying `error`. */
export type Result<T, E = string> =
  { readonly ok: true; readonly value: T } | { readonly ok: false; readonly error: E };

/** Build a successful `Result`. */
export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value };
}

/** Build a failed `Result`. */
export function err<E>(error: E): Result<never, E> {
  return { ok: false, error };
}
