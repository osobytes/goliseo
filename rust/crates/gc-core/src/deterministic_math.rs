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

/// Angle below which [`cos_sin`]'s truncated series is evaluated directly, in
/// radians.
///
/// The binding term is the COSINE series' first omitted one, `x^10 / 10!`,
/// not the sine's — and it has to be small enough to survive the doubling
/// loop, which roughly doubles the absolute error per halving. At `0.25` it
/// is `2.6e-13`, and five doublings turn that into the `2e-12` an earlier
/// draft of this function actually measured against `f64::cos`. At `0.0625`
/// it is `2.5e-19`, so nothing but ordinary rounding survives.
const COS_SIN_REDUCED_LIMIT: f64 = 0.0625;

/// Ceiling on [`cos_sin`]'s halving loop, paired with
/// [`COS_SIN_MAX_ANGLE`]: ten halvings reduce `64` radians to the limit
/// above, so the loop can never exit without having reduced the angle.
const COS_SIN_MAX_HALVINGS: u32 = 10;

/// Largest `|angle|` [`cos_sin`] accepts. Rotation steps are at most half a
/// turn, so this is two orders of magnitude of headroom rather than a limit
/// anyone should meet.
const COS_SIN_MAX_ANGLE: f64 = 64.0;

/// Cosine and sine of `angle` radians, computed identically on every target.
///
/// `f64::cos` and `f64::sin` are **not** correctly rounded, and Rust links a
/// different libm for `wasm32-unknown-unknown` than a native build uses, so
/// the two disagree in the low bits. On the simulation's path that is a
/// desync between a native peer and a browser peer: measured, a locomotion
/// draft that rotated with `sin`/`cos` reproduced its own re-recorded OMP-1
/// boundary hashes natively and diverged inside the compiled wasm module
/// eleven ticks in.
///
/// This uses only `+`, `-`, `*`, `/` and `sqrt`, every one of which IEEE 754
/// specifies as correctly rounded, in a fixed evaluation order — so it is bit
/// identical everywhere by construction rather than by luck.
///
/// Method: halve the angle until it is under [`COS_SIN_REDUCED_LIMIT`]
/// (exact in binary floating point — it is a power-of-two scale), evaluate
/// truncated Maclaurin series in Horner form, then apply the double-angle
/// identities once per halving and renormalize to shed the accumulated drift.
/// Accuracy is around `1e-13` relative, which is orders of magnitude tighter
/// than anything a feel parameter needs and is not the point; determinism is.
///
/// Do not replace this with `f64::sin_cos` — that is exactly the
/// non-bit-pinned behaviour this function exists to avoid.
///
/// # Panics
///
/// Panics if `angle` is not finite or exceeds [`COS_SIN_MAX_ANGLE`] in
/// magnitude. This is a programmer error, not a recoverable condition:
/// callers own keeping the angle a real rotation step.
#[must_use]
pub fn cos_sin(angle: f64) -> (f64, f64) {
    assert!(
        angle.is_finite() && angle.abs() <= COS_SIN_MAX_ANGLE,
        "cos_sin angle must be finite and within {COS_SIN_MAX_ANGLE} radians"
    );

    let sign = if angle < 0.0 { -1.0 } else { 1.0 };
    let mut reduced = angle.abs();
    let mut halvings: u32 = 0;
    while reduced > COS_SIN_REDUCED_LIMIT && halvings < COS_SIN_MAX_HALVINGS {
        reduced *= 0.5;
        halvings += 1;
    }

    let x = reduced;
    let x2 = x * x;
    // sin x = x - x^3/6 + x^5/120 - x^7/5040 + x^9/362880
    let mut s =
        x * (1.0 - (x2 / 6.0) * (1.0 - (x2 / 20.0) * (1.0 - (x2 / 42.0) * (1.0 - x2 / 72.0))));
    // cos x = 1 - x^2/2 + x^4/24 - x^6/720 + x^8/40320
    let mut c = 1.0 - (x2 / 2.0) * (1.0 - (x2 / 12.0) * (1.0 - (x2 / 30.0) * (1.0 - x2 / 56.0)));

    for _ in 0..halvings {
        let doubled_c = c * c - s * s;
        let doubled_s = 2.0 * c * s;
        c = doubled_c;
        s = doubled_s;
    }

    // The doubling identities drift off the unit circle by a few ulps; put the
    // pair back on it so callers can treat `(c, s)` as an exact rotation.
    let magnitude = (c * c + s * s).sqrt();
    if magnitude > 0.0 {
        c /= magnitude;
        s /= magnitude;
    }
    (c, s * sign)
}

#[cfg(test)]
mod cos_sin_tests {
    use super::cos_sin;

    /// The series is accurate enough that the only reason to prefer the libm
    /// call would be speed, which is not a trade this codebase makes.
    #[test]
    fn tracks_the_reference_trig_across_the_whole_rotation_domain() {
        let mut angle = -8.0;
        while angle <= 8.0 {
            let (c, s) = cos_sin(angle);
            assert!(
                (c - angle.cos()).abs() < 1e-14 && (s - angle.sin()).abs() < 1e-14,
                "cos_sin({angle}) = ({c}, {s}), reference ({}, {})",
                angle.cos(),
                angle.sin()
            );
            angle += 0.013;
        }
    }

    /// Callers rotate unit vectors with the returned pair, so it has to BE a
    /// rotation -- a pair a few ulps off the unit circle would shrink a
    /// direction a little on every tick.
    #[test]
    fn always_returns_a_point_on_the_unit_circle() {
        for step in 0..2000 {
            let angle = -6.5 + f64::from(step) * 0.0065;
            let (c, s) = cos_sin(angle);
            assert!(
                (c * c + s * s - 1.0).abs() < 1e-15,
                "cos_sin({angle}) is off the unit circle by {}",
                c * c + s * s - 1.0
            );
        }
    }

    #[test]
    fn is_exact_at_the_angles_that_matter_structurally() {
        let (c, s) = cos_sin(0.0);
        assert!((c - 1.0).abs() < 1e-15 && s.abs() < 1e-15);
        let (c, s) = cos_sin(std::f64::consts::PI);
        assert!((c + 1.0).abs() < 1e-15 && s.abs() < 1e-15);
        // Odd symmetry in sine, even in cosine: a rotation and its inverse.
        let (cp, sp) = cos_sin(0.7);
        let (cn, sn) = cos_sin(-0.7);
        assert!((cp - cn).abs() < 1e-15 && (sp + sn).abs() < 1e-15);
    }
}
