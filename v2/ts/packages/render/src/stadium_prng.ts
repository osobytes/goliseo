// Deterministic pseudo-random source for stadium.ts and its helper modules.
//
// AGENTS.md-for-this-task forbids `Math.random()` at module scope or inside a
// constructor: `Stadium`'s whole object graph (crowd jitter, banner phase,
// brazier placement, arcade tint) has to be identical across two
// `new Stadium(options)` calls with the same options, or the determinism test
// in stadium.spec.ts (two instances -> identical instance matrices) cannot
// pass. `mulberry32` is a tiny, fast, good-enough (not cryptographic) 32-bit
// PRNG: same seed in, same infinite stream of `[0, 1)` floats out, no global
// state, no `Date.now()`. It is the standard "seeded RNG in ~6 lines"
// algorithm (public domain, by Tommy Ettinger).

/** A seeded pseudo-random generator: call repeatedly for `[0, 1)` floats. */
export type Prng = () => number;

/** Builds a `Prng` from a 32-bit integer seed. Same seed -> same stream, always. */
export function mulberry32(seed: number): Prng {
  let a = seed >>> 0;
  return function next(): number {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A float uniformly distributed in `[min, max)`, drawn from `rng`. */
export function prngRange(rng: Prng, min: number, max: number): number {
  return min + rng() * (max - min);
}

/** An integer uniformly distributed in `[min, max]` (inclusive both ends). */
export function prngInt(rng: Prng, min: number, max: number): number {
  return Math.floor(prngRange(rng, min, max + 1));
}
