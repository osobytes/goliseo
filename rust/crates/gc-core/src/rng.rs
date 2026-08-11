//! Deterministic PRNG (Park-Miller "minstd"). The sim must stay reproducible —
//! same seed, same match — so randomness is explicit state threaded through
//! `MatchState`, never a global RNG.
//!
//! Numeric-type note: `state` is a `u32` (the value is unambiguously a bounded
//! modular integer, not a continuous quantity) and the multiply happens in
//! `u64` before reducing back into `u32`. `u64` holds `MULT * (MOD - 1)`
//! (~3.6e13) exactly, so this integer arithmetic computes the exact integer
//! result with no float rounding to reason about. The final sample conversion
//! `(s - 1) / (MOD - 1)` is kept as genuine `f64` division, in a fixed
//! operation order, so its IEEE 754 rounding is reproducible bit for bit —
//! required for the determinism evidence this module is validated against
//! (see `tests/rng.rs`).

/// 2^31 - 1, the Mersenne prime modulus of the minstd generator.
const MOD: u32 = 2_147_483_647;
/// The minimal-standard multiplier.
const MULT: u64 = 16_807;

/// Clamp any finite number into a valid, non-degenerate seed in `[1, MOD)`.
#[must_use]
pub fn seed(seed: f64) -> u32 {
    let floored = seed.abs().floor();
    let reduced = floored % f64::from(MOD);
    let mut state = reduced as u32;
    if state == 0 {
        state = 1;
    }
    state
}

/// Advance the state and return the new state plus a uniform sample in `[0, 1)`.
#[must_use]
pub fn roll(state: u32) -> (u32, f64) {
    let next_state = ((u64::from(state) * MULT) % u64::from(MOD)) as u32;
    let sample = (f64::from(next_state) - 1.0) / (f64::from(MOD) - 1.0);
    (next_state, sample)
}
