// The deterministic generator scripted network impairment draws from.
//
// This is a port of `rust/crates/gc-core/src/rng.rs` (Park-Miller
// "minstd"), and it is a port on purpose rather than a convenience: the
// browser impairment in `impairment.ts` has to consume the SAME rolls in the
// SAME order as `gc_sim::network_conditions`, or browser evidence impairs
// traffic differently from the native rollback matrix and the two suites
// measure different things while both look green (#472).
//
// `Math.random` is never acceptable here. A failure that cannot be replayed
// from its seed is not a finding.
//
// WHY THE ARITHMETIC IS EXACT IN JAVASCRIPT. `state` is a bounded integer in
// `[1, MOD)`, so the largest product is `(MOD - 1) * MULT` -- about
// 3.6e13, comfortably below `Number.MAX_SAFE_INTEGER` (9.007e15). The
// multiply and the modulo are therefore computed on exact integers, the same
// values Rust computes in `u64`. The final division is genuine floating
// point in a fixed operation order, so its IEEE 754 rounding matches Rust's
// bit for bit.
//
// The two constants below are asserted against the Rust source by
// `scripts/check_network_profile_parity.mjs` (gate 0c of scripts/check.sh),
// and the whole generator is pinned end to end by the shared transcript in
// `impairment_parity.spec.ts`.

/** 2^31 - 1, the Mersenne prime modulus of the minstd generator. */
export const RNG_MOD = 2147483647;
/** The minimal-standard multiplier. */
export const RNG_MULT = 16807;

/** One roll: the advanced state, and a uniform sample in `[0, 1)`. */
export interface RngRoll {
  readonly state: number;
  readonly sample: number;
}

/**
 * Clamp any finite number into a valid, non-degenerate seed in `[1, MOD)`.
 *
 * Throws on a non-finite seed: a caller that computed `NaN` has a bug, and
 * silently seeding 1 would hide it behind a run that looks reproducible.
 */
export function rngSeed(seed: number): number {
  if (!Number.isFinite(seed)) {
    throw new Error("impairment rng seed must be finite");
  }
  const reduced = Math.floor(Math.abs(seed)) % RNG_MOD;
  return reduced === 0 ? 1 : reduced;
}

/** Advance the state and return the new state plus a uniform sample in `[0, 1)`. */
export function rngRoll(state: number): RngRoll {
  const next = (state * RNG_MULT) % RNG_MOD;
  return { state: next, sample: (next - 1) / (RNG_MOD - 1) };
}
