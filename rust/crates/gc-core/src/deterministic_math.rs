//! Cross-runtime-stable replacements for math functions whose bits are not
//! pinned by any spec (ECMAScript explicitly leaves `log` etc.
//! implementation-approximated). `negative_log_one_minus` computes
//! `-log(1 - ratio)` via an ordered atanh power series instead, so every wasm
//! runtime produces the identical bits.

/// Minimum number of series terms evaluated before the fixed-point
/// termination check is allowed to fire. Keeps early coincidental
/// `next_sum == sum` equalities (e.g. `ratio == 0`) from cutting the series
/// short before it has actually converged for larger ratios.
const MIN_ATANH_TERMS: u32 = 30;

/// Compute `-log(1 - ratio)` with an ordered atanh series.
///
/// The termination test stops the loop
/// only once adding the next term no longer changes the running sum in
/// binary64 (`next_sum == sum`), not via any closed-form log or a fixed
/// iteration count. Do not replace this with `f64::ln` or similar — that is
/// exactly the non-bit-pinned behaviour this function exists to avoid.
///
/// # Panics
///
/// Panics if `ratio` is not in `[0, 0.95)`. This is a programmer error, not a
/// recoverable condition: callers own keeping the ratio in the domain the
/// series converges well inside.
#[must_use]
pub fn negative_log_one_minus(ratio: f64) -> f64 {
    assert!(
        (0.0..0.95).contains(&ratio),
        "negative_log_one_minus ratio must be in [0, 0.95)"
    );

    let z = ratio / (2.0 - ratio);
    let z_squared = z * z;
    let mut power = z;
    let mut sum = 0.0_f64;
    let mut term_index: u32 = 0;
    loop {
        let contribution = power / (2.0 * f64::from(term_index) + 1.0);
        let next_sum = sum + contribution;
        term_index += 1;
        if term_index >= MIN_ATANH_TERMS && next_sum == sum {
            return 2.0 * sum;
        }
        sum = next_sum;
        power *= z_squared;
    }
}
