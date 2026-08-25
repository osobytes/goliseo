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

/// Magnitude below which [`exp`]'s truncated Maclaurin series is evaluated
/// directly, in absolute value of the (already halved) argument.
///
/// A power of two, like [`COS_SIN_REDUCED_LIMIT`] -- so repeated halving
/// reaches it exactly rather than by rounding. At `0.0625` the series'
/// first omitted term (`y^10 / 10!`) is about `2.5e-19`, and
/// [`EXP_MAX_HALVINGS`] squarings can amplify that by at most `2^18`,
/// landing comfortably under `1e-12` even at the domain's most demanding
/// corner.
const EXP_REDUCED_LIMIT: f64 = 0.0625;

/// Ceiling on [`exp`]'s halving loop, paired with [`EXP_MIN_ARGUMENT`]:
/// `16384 / 0.0625 == 262144 == 2^18`, so eighteen halvings reduce the most
/// negative accepted argument to the limit above exactly, and the loop can
/// never exit without having reduced the argument. The positive side needs
/// only ten (`64 / 0.0625 == 1024 == 2^10`), so this covers both directions.
const EXP_MAX_HALVINGS: u32 = 18;

/// Largest `x` [`exp`] accepts.
///
/// Every converted call site's argument tops out around `11` (the keeper
/// catch curve's most extreme reachable tunable combination -- see #517's
/// PR body for the derivation), so `64` is roughly six times that as
/// headroom, matching [`COS_SIN_MAX_ANGLE`]'s own number. It is nowhere near
/// where a *correct* answer would overflow (`exp(709.78)` is near
/// `f64::MAX`) -- a call site that reaches this ceiling is producing an
/// argument an order of magnitude outside what any of today's sites need,
/// and that is a finding about the call site, not a reason to raise this.
const EXP_MAX_ARGUMENT: f64 = 64.0;

/// Smallest `x` [`exp`] accepts.
///
/// A softmax's `(score - maximum) / temperature` is bounded above by zero
/// but not below, and #517's audit measured a real worst case: the outfield
/// carrier's "shoot" option can score as low as roughly `-2756` (a shot from
/// the far side of the pitch against `AI_SHOOT_RANGE` authored to its own
/// minimum, `160`) against a `~70`-point maximum from another option, over a
/// temperature that can authoredly shrink to `0.9` -- a ratio near `-3140`.
/// `16384` is roughly 5x that measured worst case: headroom bought by
/// actually tracing the tunables, not a guess, and still deep in the range
/// where the true answer is indistinguishable from zero in `f64` (nothing
/// survives past roughly `-745` regardless).
const EXP_MIN_ARGUMENT: f64 = -16384.0;

/// `e^x`, computed identically on every target.
///
/// `f64::exp` is not correctly rounded, and Rust links a different libm for
/// `wasm32-unknown-unknown` than a native build uses, so the two disagree in
/// the low bits -- the same mechanism [`cos_sin`] exists to avoid. `exp` is
/// entire, so unlike [`cos_sin`] there is no periodicity to manage: this
/// halves `x` until it is small, evaluates a truncated Maclaurin series in
/// Horner form, then squares back up using `e^(2y) = (e^y)^2` once per
/// halving.
///
/// Only `+`, `-`, `*` and `/` are used, every one of which IEEE 754
/// specifies as correctly rounded, in a fixed evaluation order -- so it is
/// bit identical everywhere by construction rather than by luck. Do not
/// reorder or "simplify" the nested Horner form; AGENTS.md §6 makes
/// operation order load-bearing.
///
/// Accuracy is pinned to better than `2e-13` relative across the whole
/// accepted domain (measured worst case `1.9e-13`, at `x` near the positive
/// ceiling where ten squarings have the most rounding to amplify) and
/// `1e-13` relative across the narrower range #517's four converted call
/// sites actually produce, by `exp_tests`' two domain sweeps. A relative
/// bound is the only one that means anything here: results range from
/// subnormally small to `~6e27` across the accepted domain, so a single
/// absolute tolerance couldn't describe both ends.
///
/// Do not replace this with `f64::exp` -- that is exactly the non-bit-pinned
/// behaviour this function exists to avoid.
///
/// # Panics
///
/// Panics if `x` is not finite or falls outside
/// `[`[`EXP_MIN_ARGUMENT`]`, `[`EXP_MAX_ARGUMENT`]`]`. This is a programmer
/// error, not a recoverable condition: callers own keeping the argument in
/// the domain their call site actually produces.
#[must_use]
pub fn exp(x: f64) -> f64 {
    assert!(
        x.is_finite() && (EXP_MIN_ARGUMENT..=EXP_MAX_ARGUMENT).contains(&x),
        "exp argument must be finite and within [{EXP_MIN_ARGUMENT}, {EXP_MAX_ARGUMENT}]"
    );

    let mut reduced = x;
    let mut halvings: u32 = 0;
    while reduced.abs() > EXP_REDUCED_LIMIT && halvings < EXP_MAX_HALVINGS {
        reduced *= 0.5;
        halvings += 1;
    }

    let y = reduced;
    // e^y = 1 + y + y^2/2! + y^3/3! + ... + y^9/9!, Horner form.
    let mut result = 1.0
        + y * (1.0
            + y / 2.0
                * (1.0
                    + y / 3.0
                        * (1.0
                            + y / 4.0
                                * (1.0
                                    + y / 5.0
                                        * (1.0
                                            + y / 6.0
                                                * (1.0
                                                    + y / 7.0
                                                        * (1.0 + y / 8.0 * (1.0 + y / 9.0))))))));

    for _ in 0..halvings {
        result *= result;
    }
    result
}

/// Magnitude above which [`ln_ratio`]'s square-root reduction keeps
/// reducing, expressed as how far the reduced ratio may sit above `1.0`.
/// Shares its value with [`COS_SIN_REDUCED_LIMIT`]/[`EXP_REDUCED_LIMIT`] on
/// purpose: it keeps [`negative_log_one_minus`]'s own input (`1 -
/// 1/reduced`) far inside that function's `[0, 0.95)` domain, with room to
/// spare, rather than merely inside it.
const LN_REDUCED_LIMIT: f64 = 0.0625;

/// Ceiling on [`ln_ratio`]'s square-root reduction loop. Each step halves
/// `ln(reduced)`, so reaching [`LN_MAX_RATIO`]`= 4096` down to the limit
/// above needs `log2(ln(4096) / ln(1.0625)) ~= 7.5`, i.e. 8 steps; 10 is
/// margin over that, matching [`COS_SIN_MAX_HALVINGS`]'s own number.
const LN_MAX_HALVINGS: u32 = 10;

/// Largest ratio [`ln_ratio`] accepts.
///
/// #517's domain audit of `ai::pass_intercept`'s `(launch_speed / v).ln()`
/// found the ratio can legitimately reach about `195` -- not the `< 20`
/// an unaudited guess would assume -- from a corner-to-corner pass on the
/// then-default 960x540 field with `PASS_SPEED_MAX` authored to `930` (a valid,
/// step-aligned value inside its own `[450, 1000]` range): the pass's
/// launch speed clamps to `930`, friction sheds it down to `v ~= 4.78` by
/// the last sampled interception fraction, and `930 / 4.78 ~= 194.6`. `4096`
/// is roughly 21x that measured worst case -- headroom, not a guess, since
/// the audit is what makes the real number known.
///
/// The pitch has since moved to 1648x927, whose diagonal is 1.72x longer, so
/// a corner-to-corner pass sheds more speed and the true worst-case ratio is
/// higher than the audited `195`. The 21x headroom is wide enough that the
/// bound is near-certainly still sound, but say plainly what this is now: an
/// argument from margin, not the measured result it was when written. If this
/// clamp ever binds, re-run #517's audit on the current field rather than
/// widening the constant.
const LN_MAX_RATIO: f64 = 4096.0;

/// `ln(x)` for `x` in `[1.0, `[`LN_MAX_RATIO`]`]`, computed identically on
/// every target by reducing `x` toward `1.0` with repeated square roots (an
/// operation IEEE 754 specifies as correctly rounded) and finishing with the
/// already-pinned [`negative_log_one_minus`].
///
/// Square-rooting `x` an integer number of times and multiplying the result
/// back up by the matching power of two is exact in the same way halving an
/// angle is for [`cos_sin`]: `ln(x) = 2^k * ln(x^(1/2^k))`. This is
/// deliberately narrower than a general-purpose `ln` -- it only accepts
/// `x >= 1.0` -- because every known caller computes `ln` of a ratio of two
/// positive quantities where the numerator is never smaller than the
/// denominator (`ai::pass_intercept`'s ball speed only ever decays), and a
/// narrower, audited domain is worth more than a general one nothing here
/// needs. See [`LN_MAX_RATIO`] for how its ceiling was chosen.
///
/// Do not replace this with `f64::ln` -- that is exactly the non-bit-pinned
/// behaviour this function exists to avoid. Do not reorder the reduction or
/// the final scale-up; AGENTS.md §6 makes operation order load-bearing.
///
/// # Panics
///
/// Panics if `x` is not finite or falls outside
/// `[1.0, `[`LN_MAX_RATIO`]`]`. This is a programmer error, not a
/// recoverable condition: callers own keeping the ratio in the domain their
/// call site actually produces.
#[must_use]
pub fn ln_ratio(x: f64) -> f64 {
    assert!(
        x.is_finite() && (1.0..=LN_MAX_RATIO).contains(&x),
        "ln_ratio argument must be finite and within [1.0, {LN_MAX_RATIO}]"
    );

    let mut reduced = x;
    let mut halvings: u32 = 0;
    while reduced > 1.0 + LN_REDUCED_LIMIT && halvings < LN_MAX_HALVINGS {
        reduced = reduced.sqrt();
        halvings += 1;
    }

    // reduced is now in [1.0, 1.0 + LN_REDUCED_LIMIT]. negative_log_one_minus
    // computes -ln(1 - ratio); feeding it ratio = 1 - 1/reduced gives
    // -ln(1/reduced) = ln(reduced), and ratio sits in [0, ~0.0588], well
    // inside that function's own [0, 0.95) domain.
    let ratio = 1.0 - 1.0 / reduced;
    let mut result = negative_log_one_minus(ratio);

    for _ in 0..halvings {
        result *= 2.0;
    }
    result
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

#[cfg(test)]
mod exp_tests {
    use super::exp;

    /// Covers `[-64, 64]` densely -- the whole positive side of the accepted
    /// domain and, on the negative side, well past every measured call-site
    /// extreme (worst case ~`-3140`, see [`EXP_MIN_ARGUMENT`]) while `f64::exp`
    /// itself still returns a comparable, non-zero reference to check
    /// against. `saturates_to_zero...` below covers the deep-underflow tail
    /// out to the actual domain floor, where libm's own answer is just `0`.
    #[test]
    fn tracks_the_reference_exp_across_the_practically_reachable_domain() {
        let mut x = -64.0;
        while x <= 64.0 {
            let got = exp(x);
            let want = x.exp();
            // `want` ranges from subnormally small to ~6e27 across this
            // domain, so only a relative bound means anything here -- unlike
            // cos_sin, where every result is bounded to [-1, 1] and absolute
            // and relative amount to the same thing. Skip the (extremely)
            // deep-underflow tail, where `want` itself is exactly zero and
            // relative error is undefined; `saturates_to_zero...` below
            // covers that region directly.
            if want != 0.0 && want.is_finite() {
                let rel_err = ((got - want) / want).abs();
                assert!(
                    rel_err < 2e-13,
                    "exp({x}) = {got}, reference {want}, rel_err {rel_err}"
                );
            }
            x += 0.037;
        }
    }

    /// The four call sites this exists for never see an argument outside
    /// roughly [-12, 6] (#517's PR body has the derivation); pin tighter
    /// accuracy across exactly that practically-relevant slice.
    #[test]
    fn is_accurate_to_1e_minus_13_relative_across_the_range_the_converted_sites_actually_use() {
        let mut x = -12.0;
        while x <= 6.0 {
            let got = exp(x);
            let want = x.exp();
            let rel_err = ((got - want) / want).abs();
            assert!(
                rel_err < 1e-13,
                "exp({x}) = {got}, reference {want}, rel_err {rel_err}"
            );
            x += 0.011;
        }
    }

    #[test]
    fn is_exact_at_the_argument_that_matters_structurally() {
        assert!((exp(0.0) - 1.0).abs() < 1e-15);
    }

    /// A softmax's ratio grows without bound as temperature shrinks toward
    /// zero; the whole point of a wide, non-panicking negative domain is
    /// that this saturates to zero instead of blowing up.
    #[test]
    fn saturates_to_zero_for_a_very_negative_argument_instead_of_underflowing_to_nan_or_inf() {
        let deep = exp(-16_384.0);
        assert!(deep.is_finite() && (0.0..1e-13).contains(&deep));
    }

    #[test]
    fn rejects_values_outside_its_programmer_error_domain() {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let too_high = std::panic::catch_unwind(|| exp(64.000_001));
        let too_low = std::panic::catch_unwind(|| exp(-16_384.000_001));
        let non_finite = std::panic::catch_unwind(|| exp(f64::NAN));
        std::panic::set_hook(prev_hook);

        assert!(too_high.is_err());
        assert!(too_low.is_err());
        assert!(non_finite.is_err());
    }
}

#[cfg(test)]
mod ln_ratio_tests {
    use super::ln_ratio;

    /// Covers the whole accepted domain, including the far end #517's
    /// domain audit found is actually reachable (~195, not the `< 20` an
    /// unaudited guess would assume) with generous headroom past it.
    #[test]
    fn tracks_the_reference_ln_across_the_whole_accepted_domain() {
        let mut x = 1.0;
        while x <= 4096.0 {
            let got = ln_ratio(x);
            let want = x.ln();
            let abs_err = (got - want).abs();
            let rel_err = if want.abs() > 0.0 {
                abs_err / want.abs()
            } else {
                abs_err
            };
            assert!(
                abs_err < 1e-12 || rel_err < 1e-13,
                "ln_ratio({x}) = {got}, reference {want}, abs_err {abs_err}, rel_err {rel_err}"
            );
            x *= 1.01;
        }
    }

    /// Pin tighter accuracy across the slice #517's audit found
    /// `ai::pass_intercept` actually reaches (up to ~195), plus headroom.
    #[test]
    fn is_accurate_to_1e_minus_13_across_the_range_the_converted_site_actually_uses() {
        let mut x = 1.0;
        while x <= 256.0 {
            let got = ln_ratio(x);
            let want = x.ln();
            assert!(
                (got - want).abs() < 1e-13,
                "ln_ratio({x}) = {got}, reference {want}"
            );
            x += 0.211;
        }
    }

    #[test]
    fn is_exact_at_the_ratio_that_matters_structurally() {
        assert_eq!(ln_ratio(1.0), 0.0);
    }

    #[test]
    fn rejects_values_outside_its_programmer_error_domain() {
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let below_one = std::panic::catch_unwind(|| ln_ratio(0.999_999));
        let too_high = std::panic::catch_unwind(|| ln_ratio(4_096.000_001));
        let non_finite = std::panic::catch_unwind(|| ln_ratio(f64::NAN));
        std::panic::set_hook(prev_hook);

        assert!(below_one.is_err());
        assert!(too_high.is_err());
        assert!(non_finite.is_err());
    }
}
