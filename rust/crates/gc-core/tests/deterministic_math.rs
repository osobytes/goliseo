//! Port of `spec/core/deterministic_math_spec.lua`.
//!
//! The Lua spec pins its regression case through
//! `sim.match_snapshot.number_bytes`, which lives in `sim/` (→ `gc-sim`, per
//! `v2/README.md`'s layer table). `gc-core` must not depend on `gc-sim` — the
//! dependency only ever points the other way — so this test reproduces just
//! that one canonical-byte-encoding helper locally. It is a direct,
//! byte-for-byte transcription of `sim/match_snapshot.lua`'s
//! `number_bytes` (same `frexp`/mantissa-split/rounding steps, same output
//! format), used here only to keep the pinned-bytes assertion alive without
//! reaching across the crate boundary.

use gc_core::deterministic_math::negative_log_one_minus;

/// `math.frexp` equivalent: split `x` into `(mantissa, exponent)` such that
/// `x == mantissa * 2^exponent` and `0.5 <= |mantissa| < 1` (or both zero).
fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let sign = bits & 0x8000_0000_0000_0000;
    let biased_exponent = ((bits >> 52) & 0x7ff) as i32;
    let mantissa_bits = bits & 0x000f_ffff_ffff_ffff;

    if biased_exponent == 0 {
        // Subnormal: normalize by hand.
        let mut m = mantissa_bits;
        let mut shift = 0;
        while m & 0x0010_0000_0000_0000 == 0 {
            m <<= 1;
            shift += 1;
        }
        m &= 0x000f_ffff_ffff_ffff;
        let mantissa = f64::from_bits(sign | (1022_u64 << 52) | m);
        return (mantissa, -1021 - shift);
    }

    let mantissa = f64::from_bits(sign | (1022_u64 << 52) | mantissa_bits);
    (mantissa, biased_exponent - 1022)
}

/// Transcribed from `sim.match_snapshot.number_bytes` in
/// `sim/match_snapshot.lua`.
fn number_bytes(number: f64) -> String {
    assert!(number.is_finite(), "canonical numbers must be finite");
    if number == 0.0 {
        return if 1.0 / number == f64::NEG_INFINITY {
            "Z".to_string()
        } else {
            "z".to_string()
        };
    }
    let sign = if number < 0.0 { "m" } else { "p" };
    let (mantissa, exponent) = frexp(number.abs());
    let high = (mantissa * 67_108_864.0).floor();
    let low = ((mantissa * 67_108_864.0 - high) * 134_217_728.0 + 0.5).floor();
    let high = high as i64;
    let low = low as i64;
    format!("{sign}:{exponent}:{high}:{low}")
}

#[test]
fn core_deterministic_math_pins_the_cross_runtime_negative_log_regression_to_exact_binary64_bytes()
{
    let ratio = 0.618_234_602_637_309;
    let result = negative_log_one_minus(ratio);
    assert_eq!(number_bytes(result), "p:0:64622413:83045015");
}

#[test]
fn core_deterministic_math_is_finite_and_monotonic_from_zero_through_the_callers_upper_range() {
    let zero = negative_log_one_minus(0.0);
    let middle = negative_log_one_minus(0.9);
    let near_limit = negative_log_one_minus(0.949_999_999_999);
    assert_eq!(zero, 0.0);
    assert!(zero < middle && middle < near_limit);
    assert!(near_limit == near_limit && near_limit < f64::INFINITY);
}

#[test]
fn core_deterministic_math_rejects_values_outside_its_programmer_error_domain() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let below_zero = std::panic::catch_unwind(|| negative_log_one_minus(-0.000_001));
    let at_boundary = std::panic::catch_unwind(|| negative_log_one_minus(0.95));
    let at_one = std::panic::catch_unwind(|| negative_log_one_minus(1.0));
    std::panic::set_hook(prev_hook);

    assert!(below_zero.is_err());
    assert!(at_boundary.is_err());
    assert!(at_one.is_err());
}
