//! Pass receiver selection and the distance-to-speed launch curve.
//!
//! Pure — no I/O, no RNG, no clock. Every function here is a function of
//! positions and registered knobs, so a preview call and the authoritative
//! release-tick call on the same state return the same answer, and a
//! resimulated release re-resolves identically.
//!
//! ## Soft cone, not a gate
//!
//! What this replaced was a hard 60-degree acceptance cone: a teammate one
//! degree outside it was invisible, so a near-miss on the aim stick produced
//! *no pass at all* and the player was back to guessing. The score is now a
//! blend — distance plus a weighted angular term — with no acceptance test on
//! the angle at all:
//!
//! ```text
//! score(t) = distance_term(t) + PASS_ANGULAR_WEIGHT * angular_term(t)
//! ```
//!
//! Aim biases the choice toward whoever is roughly in the aimed direction and
//! reasonably close; selection still always resolves to someone eligible. The
//! only exclusions left are the two *distance* bounds (`PASS_ELIGIBLE_MIN`,
//! `PASS_ELIGIBLE_MAX`), which are about what a pass **is** — a handoff to
//! someone at your elbow is not a pass, and neither is a punt to the far
//! corner — rather than about how well you aimed.
//!
//! ## The angular term is a CHORD, and that is a determinism requirement
//!
//! The issue that specified this work writes the angular term in radians,
//! with `PASS_ANGULAR_WEIGHT` in "distance per radian". Radians would need
//! `acos` on a dot product, and `acos` is exactly the class of function
//! ARCHITECTURE.md §1 and `gc_sim::locomotion` name as
//! implementation-approximated: Rust links a different libm for
//! `wasm32-unknown-unknown` than a native build uses, so a native peer and a
//! browser peer would disagree in the low bits of every pass score, and a
//! score comparison is a *discrete* choice — a low-bit disagreement does not
//! blur the answer, it selects a different receiver and desyncs the match.
//!
//! So the angular term is the **chord** between the two unit vectors,
//! `|aim - dir|`, which needs one subtraction and one `sqrt` and nothing
//! else. `gc_core::vec2` and `gc_sim::locomotion` already take this trade for
//! the same reason; what is new here is only that the knob's unit follows it.
//! Chord is monotone in the angle over `0..=pi`, so it ranks two candidates'
//! *angles* exactly as radians would, and it tracks the angle closely where
//! aim actually matters:
//!
//! | angle off aim | chord | radians | chord / radians |
//! | --- | --- | --- | --- |
//! | 15 deg | 0.261 | 0.262 | 99.7% |
//! | 30 deg | 0.518 | 0.524 | 98.9% |
//! | 60 deg | 1.000 | 1.047 | 95.5% |
//! | 90 deg | 1.414 | 1.571 | 90.0% |
//! | 180 deg | 2.000 | 3.142 | 63.7% |
//!
//! ### Monotonicity is NOT equivalence, and the difference is real
//!
//! It would be wrong to read the table as "chord is radians with rounding".
//! The score is a *sum* — `distance + weight × angular` — and chord is a
//! **concave reparametrisation** of the angle, so a monotone substitution
//! inside a sum does not preserve the argmin. Two candidates trading a
//! distance gap against an angle gap can rank differently under the two
//! forms, near the crossover.
//!
//! Worked, from `the_angular_weight_decides_between_near_off_axis_and_far_on_axis`
//! in `tests/passing.rs`: a near candidate 200 px away at 90 degrees off,
//! against a far one 300 px away dead on the line. The 100 px distance gap is
//! bought off at `weight = 100 / 1.414 ≈ 70.7` px/chord, and at
//! `100 / 1.571 ≈ 63.7` px/rad. Between those two the two formulations pick
//! **different receivers**.
//!
//! That is bounded and it is tunable, which is why it is a trade rather than
//! a defect: the gap between the two crossovers is `chord/radians` at the
//! angle in play, at worst the 63.7% at the back of the table, and
//! `PASS_ANGULAR_WEIGHT` is a registered knob whose whole job is to move that
//! crossover. It is a feel parameter being calibrated in chord units instead
//! of radian units, not an approximation drifting away from a correct answer.
//! Nothing downstream depends on the crossover sitting at a particular
//! *number*; the determinism requirement above does depend on the term being
//! computable without `acos`.
//!
//! The squash is also at the *back*, where the term is already dominant and a
//! teammate is already losing. `PASS_ANGULAR_WEIGHT`'s unit is therefore
//! authored as `px/chord`, not `px/rad` — a range quoted in radian units
//! would not transfer. Its declared range began as a metric prior converted
//! at a mistaken pitch scale and was then widened by play-testing; see
//! `gc_data::tunables`'s Passing note for why the conversion is not worth
//! reconstructing and the widened range is the real authority.
//!
//! ## The charge range is a distance TARGET, not a distance
//!
//! `distance_term` is the raw distance for a tap, and `|d - range|` for a
//! charged pass. That is a deliberate departure from the issue's plain
//! `dist(passer, teammate)`: hold-to-charge already shipped, and it means
//! "pick out the man at *this* range", so a charged release must prefer the
//! teammate nearest the charged range rather than the teammate nearest full
//! stop. Both forms are the same soft blend against the same angular term.

use crate::tuning::Tuning;
use gc_core::vec2::Vec2;

/// Knob id: how much a teammate's angle off the aim costs, in px per unit
/// chord. See the module doc for why the unit is a chord.
pub const ANGULAR_WEIGHT_KNOB: &str = "PASS_ANGULAR_WEIGHT";
/// Knob id: teammates closer than this are not pass candidates.
pub const ELIGIBLE_MIN_KNOB: &str = "PASS_ELIGIBLE_MIN";
/// Knob id: teammates further than this are not pass candidates.
pub const ELIGIBLE_MAX_KNOB: &str = "PASS_ELIGIBLE_MAX";
/// Knob id: pace the ball still carries when it reaches the receiver.
pub const ARRIVE_PACE_KNOB: &str = "PASS_ARRIVE_PACE";
/// Knob id: floor under the launch speed of a ground pass, px/s.
pub const SPEED_MIN_KNOB: &str = "PASS_SPEED_MIN";
/// Knob id: ceiling over the launch speed of a ground pass, px/s.
pub const SPEED_MAX_KNOB: &str = "PASS_SPEED_MAX";

/// The three registered scalars receiver selection reads.
///
/// Resolved once per call rather than per candidate: a registry lookup is a
/// string hash, and the scoring loop runs over every teammate at every
/// preview frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionKnobs {
    /// Cost of one unit of chord off the aim direction, in px.
    pub angular_weight: f64,
    /// Teammates nearer than this are excluded (a handoff is not a pass).
    pub eligible_min: f64,
    /// Teammates further than this are excluded.
    pub eligible_max: f64,
}

impl SelectionKnobs {
    /// Read the live values out of `tune`.
    #[must_use]
    pub fn of(tune: &Tuning) -> Self {
        SelectionKnobs {
            angular_weight: tune.value(ANGULAR_WEIGHT_KNOB),
            eligible_min: tune.value(ELIGIBLE_MIN_KNOB),
            eligible_max: tune.value(ELIGIBLE_MAX_KNOB),
        }
    }
}

/// Whether a teammate `distance` px away is a pass candidate at all.
#[must_use]
pub fn eligible(distance: f64, knobs: &SelectionKnobs) -> bool {
    distance >= knobs.eligible_min && distance <= knobs.eligible_max
}

/// The angular term: the chord between two directions, in `0..=2`.
///
/// `aim` is expected normalized; `offset` is the raw passer-to-teammate
/// vector and is normalized here. A zero `aim` means the passer has no aim
/// direction at all, and the honest answer is that no teammate is more or
/// less aimed-at than any other — so the term is zero for everybody and the
/// choice falls through to distance. That is deliberately not `None`: a soft
/// cone that returns nobody is the gate this module exists to delete.
#[must_use]
pub fn angular_term(aim: Vec2, offset: Vec2) -> f64 {
    if (aim.x == 0.0 && aim.y == 0.0) || (offset.x == 0.0 && offset.y == 0.0) {
        return 0.0;
    }
    aim.sub(offset.normalized()).length()
}

/// One teammate's soft-cone score. Lower is better.
///
/// `range` is the charged pass range when the player held the button, and
/// `None` for a tap — see the module doc on why a charged pass scores against
/// `|d - range|`.
#[must_use]
pub fn score(from: Vec2, aim: Vec2, target: Vec2, range: Option<f64>, angular_weight: f64) -> f64 {
    let offset = target.sub(from);
    let d = offset.length();
    let distance_term = match range {
        Some(range) => (d - range).abs(),
        None => d,
    };
    distance_term + angular_weight * angular_term(aim, offset)
}

/// Pick the index (0-based — this is an in-memory position, not a wire
/// format, per ARCHITECTURE.md §3 rule 3) of the best teammate to pass to.
///
/// Soft-scored, with no angular acceptance test: the only way to return
/// `None` is for every teammate to fall outside `[eligible_min,
/// eligible_max]`. Recomputed from scratch on every call and never cached —
/// the world moves, so the answer must too.
///
/// **Ties break on the lower index**, and that is load-bearing rather than
/// incidental: `>` would let a later teammate displace an equal-scoring
/// earlier one, and two peers whose candidate lists were assembled in the
/// same order would still agree — but a resimulation that reordered nothing
/// would agree only by luck. The strict `<` below makes the first candidate
/// win every exact tie, on every peer, at every tick.
#[must_use]
pub fn select_receiver(
    from: Vec2,
    aim: Vec2,
    teammates: &[Vec2],
    range: Option<f64>,
    knobs: &SelectionKnobs,
) -> Option<usize> {
    let aim = aim.normalized();
    let mut best: Option<usize> = None;
    let mut best_score = f64::INFINITY;
    for (i, &p) in teammates.iter().enumerate() {
        if !eligible(p.dist(from), knobs) {
            continue;
        }
        let s = score(from, aim, p, range, knobs.angular_weight);
        if s < best_score {
            best_score = s;
            best = Some(i);
        }
    }
    best
}

/// Launch speed for a ground pass covering `d` px, from the registered
/// distance-to-speed curve.
///
/// Three authored breakpoints rather than a table: friction sheds `FRICTION`
/// of the ball's speed per second, so a pass launched at `v` covers roughly
/// `v / FRICTION` before dying, and the pace that must survive the trip is
/// `PASS_ARRIVE_PACE`. `PASS_SPEED_MIN` and `PASS_SPEED_MAX` are the two ends
/// the linear middle is clamped between, so a short pass is never a tap the
/// defender walks onto and a cross-pitch ball is never a bullet.
///
/// The slope is `FRICTION` itself and is deliberately **not** a knob: it is
/// the ball physics' own constant, and a fourth knob able to disagree with it
/// would let the curve promise an arrival pace the ball never delivers.
///
/// The two ends overlap in their declared ranges, so a sweep can author
/// `PASS_SPEED_MIN > PASS_SPEED_MAX`. `f64::clamp` **panics** on an inverted
/// pair, and the release profile is `panic = "abort"` — that would take a
/// player's match down over a knob combination a sweep is entitled to try.
/// The ceiling is therefore raised to the floor rather than trusted.
#[must_use]
pub fn speed_for(d: f64, tune: &Tuning) -> f64 {
    let lo = tune.value(SPEED_MIN_KNOB);
    let hi = tune.value(SPEED_MAX_KNOB).max(lo);
    (tune.value(ARRIVE_PACE_KNOB) + crate::ball_flight::FRICTION * d).clamp(lo, hi)
}
