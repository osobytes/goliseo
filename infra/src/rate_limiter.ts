//! A pure fixed-window rate limiter. No clock reads, no storage -- the
//! caller supplies `nowMs` and persists the returned state itself (room
//! join attempts persist it in the room's SQLite row; see
//! `room_durable_object.ts`).

import { type Result, err, ok } from "./result.ts";

/** A fixed window's counter, as a plain, serializable value. */
export interface FixedWindowState {
  /** Epoch milliseconds the current window started. */
  readonly windowStartMs: number;
  /** Requests admitted so far in the current window. */
  readonly count: number;
}

/** A fixed window's admission policy. */
export interface FixedWindowLimit {
  /** Maximum admitted requests per window. */
  readonly limit: number;
  /** Window length in milliseconds. */
  readonly windowMs: number;
}

/**
 * Attempt to admit one request. Returns the next `FixedWindowState` to
 * persist on success, or `"rate_limited"` when the window's `limit` is
 * already spent. Starting a fresh window (state is `null`, or the previous
 * window has elapsed) always admits.
 */
export function tryConsume(
  state: FixedWindowState | null,
  nowMs: number,
  policy: FixedWindowLimit,
): Result<FixedWindowState> {
  if (state === null || nowMs - state.windowStartMs >= policy.windowMs) {
    return ok({ windowStartMs: nowMs, count: 1 });
  }
  if (state.count >= policy.limit) {
    return err("rate_limited");
  }
  return ok({ ...state, count: state.count + 1 });
}
