//! Port of `spec/core/rng_spec.lua`.

use gc_core::rng;

#[test]
fn core_rng_is_reproducible_same_seed_same_sequence() {
    let mut a = rng::seed(123.0);
    let mut b = rng::seed(123.0);
    for _ in 0..10 {
        let (next_a, xa) = rng::roll(a);
        let (next_b, xb) = rng::roll(b);
        a = next_a;
        b = next_b;
        assert_eq!(xa, xb);
    }
}

#[test]
fn core_rng_samples_stay_in_0_1_and_vary() {
    let mut s = rng::seed(7.0);
    let mut seen: Vec<f64> = Vec::new();
    for _ in 0..100 {
        let (next_s, x) = rng::roll(s);
        s = next_s;
        assert!((0.0..1.0).contains(&x), "sample in range");
        if !seen.contains(&x) {
            seen.push(x);
        }
    }
    assert!(seen.len() > 90, "samples do not repeat degenerately");
}

#[test]
fn core_rng_is_roughly_uniform() {
    let mut s = rng::seed(99.0);
    let mut sum = 0.0_f64;
    for _ in 0..2000 {
        let (next_s, x) = rng::roll(s);
        s = next_s;
        sum += x;
    }
    let mean = sum / 2000.0;
    assert!(mean > 0.45 && mean < 0.55, "mean near 0.5, got {mean}");
}

#[test]
fn core_rng_normalizes_degenerate_seeds() {
    assert!(rng::seed(0.0) >= 1);
    assert!(rng::seed(-5.0) >= 1);
    assert!(rng::seed(2_147_483_647.0 * 3.0 + 0.7) >= 1);
}
