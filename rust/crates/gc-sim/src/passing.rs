//! Pass target selection: the teammate that best matches the aim direction.

use gc_core::vec2::Vec2;

/// Accept teammates within ~60 degrees of the aim direction.
const CONE_COS: f64 = 0.5;
/// Alignment breaks ties among comparable distances.
const ALIGN_WEIGHT: f64 = 2.0;
/// Px of distance that costs one point of score.
const DIST_NORM: f64 = 100.0;

/// Pick the index (0-based — this is an in-memory position, not a wire
/// format, per ARCHITECTURE.md §3 rule 3) of the best teammate to pass to.
///
/// A tap pass goes to the CLOSEST teammate along the aim direction —
/// proximity dominates, with alignment deciding between comparable distances
/// — so a quick pass reliably finds the near man you are pointing at.
/// Reaching a far teammate is what the charge is for: with `range`
/// (hold-to-charge), the teammate whose distance best matches the charged
/// range wins instead. Returns `None` if nobody is inside the aim cone.
#[must_use]
pub fn target(from: Vec2, dir: Vec2, teammates: &[Vec2], range: Option<f64>) -> Option<usize> {
    let ndir = dir.normalized();
    if ndir.x == 0.0 && ndir.y == 0.0 {
        return None;
    }

    let mut best: Option<usize> = None;
    let mut best_score: Option<f64> = None;
    for (i, &p) in teammates.iter().enumerate() {
        let to = p.sub(from);
        let d = to.length();
        if d > 1.0 {
            let cos = (to.x * ndir.x + to.y * ndir.y) / d;
            if cos >= CONE_COS {
                let score = if let Some(range) = range {
                    cos * ALIGN_WEIGHT - (d - range).abs() / 150.0
                } else {
                    cos * ALIGN_WEIGHT - d / DIST_NORM
                };
                if best_score.is_none() || score > best_score.unwrap() {
                    best_score = Some(score);
                    best = Some(i);
                }
            }
        }
    }
    best
}
