//! Tests for `Vec2` arithmetic, length, normalization and distance.
//!
//! Length/distance/normalization checks use an epsilon comparison
//! (`assert_near`) since they route through `sqrt`; equality checks
//! (`assert_eq!`) stay exact — the zero-vector case in particular is
//! asserting that `normalized` returns a true zero rather than a NaN, so
//! weakening it to an epsilon check would defeat the test.

use gc_core::vec2::Vec2;

/// Tolerance for approximate (`sqrt`-derived) comparisons.
const EPSILON: f64 = 1e-9;

fn assert_near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < EPSILON,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn vec2_adds_component_wise() {
    let v = Vec2::new(1.0, 2.0).add(Vec2::new(3.0, 4.0));
    assert_eq!(v.x, 4.0);
    assert_eq!(v.y, 6.0);
}

#[test]
fn vec2_subtracts_component_wise() {
    let v = Vec2::new(5.0, 5.0).sub(Vec2::new(2.0, 1.0));
    assert_eq!(v.x, 3.0);
    assert_eq!(v.y, 4.0);
}

#[test]
fn vec2_scales() {
    let v = Vec2::new(2.0, -3.0).scale(2.0);
    assert_eq!(v.x, 4.0);
    assert_eq!(v.y, -6.0);
}

#[test]
fn vec2_computes_length() {
    assert_near(Vec2::new(3.0, 4.0).length(), 5.0);
}

#[test]
fn vec2_normalizes_to_unit_length() {
    assert_near(Vec2::new(0.0, 10.0).normalized().length(), 1.0);
    assert_eq!(Vec2::new(0.0, 10.0).normalized().y, 1.0);
}

#[test]
fn vec2_normalizes_the_zero_vector_to_zero_no_nan() {
    let n = Vec2::new(0.0, 0.0).normalized();
    assert_eq!(n.x, 0.0);
    assert_eq!(n.y, 0.0);
}

#[test]
fn vec2_measures_distance() {
    assert_near(Vec2::new(0.0, 0.0).dist(Vec2::new(3.0, 4.0)), 5.0);
}
