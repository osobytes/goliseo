//! Pure steering/selection helpers used by the match AI.
//!
//! [`closest`] and [`assign_marks`] index directly into the `positions`/
//! `defenders`/`opponents` slices the caller passes for this call, so those
//! indices are ordinary 0-based Rust collection indices, not the 1-based
//! player identity `sim::r#match` uses elsewhere (ARCHITECTURE.md §3 rule 3).

use gc_core::deterministic_math;
use gc_core::vec2::Vec2;
use indexmap::IndexMap;

/// Bump when this module's steering/selection behaviour changes without
/// moving one of the public constants below — `sim::outfield_ai_policy`
/// hashes it, so this is where a deliberate policy change to the file-local
/// intercept sampling constants is recorded.
///
/// 2: [`pass_intercept`] learned deflection risk (a body in blocking
/// position cuts a lane even where the ball is too fast to collect —
/// mirroring `sim::r#match`'s block rule), a behaviour change to the AI's
/// lane model with no public constant of its own to move.
pub const VERSION: i64 = 2;

/// Playable pitch dimensions, as used by [`support_spot`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// Pitch width.
    pub w: f64,
    /// Pitch height.
    pub h: f64,
}

/// The index into `positions` of the closest position to `point`, or `None`
/// if there are no candidates. `exclude` skips one index (e.g. self).
#[must_use]
pub fn closest(point: Vec2, positions: &[Vec2], exclude: Option<usize>) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_dist: Option<f64> = None;
    for (i, p) in positions.iter().enumerate() {
        if Some(i) != exclude {
            let d = point.dist(*p);
            if best_dist.is_none_or(|bd| d < bd) {
                best_dist = Some(d);
                best = Some(i);
            }
        }
    }
    best
}

/// Move `pos` toward `target`, covering at most `max_dist`. Returns the new
/// position and the unit direction travelled (zero direction if already
/// there).
#[must_use]
pub fn steer(pos: Vec2, target: Vec2, max_dist: f64) -> (Vec2, Vec2) {
    let to = target.sub(pos);
    let d = to.length();
    if d == 0.0 {
        return (Vec2::new(pos.x, pos.y), Vec2::new(0.0, 0.0));
    }
    let dir = to.normalized();
    if d <= max_dist {
        return (Vec2::new(target.x, target.y), dir);
    }
    (pos.add(dir.scale(max_dist)), dir)
}

/// Predict where a moving target will be and aim there (Reynolds "pursuit"):
/// the lead horizon grows with distance, so far targets are led more.
/// Returns the point to chase; callers still pipe it through [`steer`] for
/// the speed clamp. `lead` is the prediction coefficient (seconds per unit
/// distance).
#[must_use]
pub fn pursue(pos: Vec2, target_pos: Vec2, target_vel: Vec2, lead: f64) -> Vec2 {
    let horizon = lead * pos.dist(target_pos);
    target_pos.add(target_vel.scale(horizon))
}

/// Point `frac` of the way from `a` to `b`. The marking/cover primitive:
/// stand goal-side of a man (interpose between opponent and goal) or behind
/// the presser. `frac`: 0 = `a`, 1 = `b`.
#[must_use]
pub fn interpose(a: Vec2, b: Vec2, frac: f64) -> Vec2 {
    a.add(b.sub(a).scale(frac))
}

/// Summed repulsion from neighbours within `radius`, with linear falloff, so
/// players don't collapse onto the same spot. Returns an offset to add to a
/// steering target (zero if nothing is close). Coincident neighbours are
/// skipped.
#[must_use]
pub fn separation(pos: Vec2, others: &[Vec2], radius: f64) -> Vec2 {
    let mut off = Vec2::new(0.0, 0.0);
    for o in others {
        let away = pos.sub(*o);
        let d = away.length();
        if d > 0.0 && d < radius {
            off = off.add(away.normalized().scale((radius - d) / radius));
        }
    }
    off
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + deterministic_math::exp(-x))
}

/// Shortest distance from point `p` to segment `a`-`b`.
fn point_seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b.sub(a);
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 == 0.0 {
        return p.dist(a);
    }
    let tt = ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2;
    let tt = tt.clamp(0.0, 1.0);
    p.dist(a.add(ab.scale(tt)))
}

/// Off-ball support scoring: sigmoid steepness over normalized attacking
/// depth. Public because it IS gameplay-AI policy — `sim::outfield_ai_policy`
/// hashes it into the frozen policy id, so a change here is visible to the
/// evidence that cites it (#59).
pub const IMPORTANCE_K: f64 = 4.0;
/// Off-ball support scoring: gaussian width toward vertical centre (fraction
/// of field height).
pub const CENTER_SIGMA: f64 = 0.28;
/// Off-ball support scoring: an opponent within this of the pass line blocks
/// the lane.
pub const LANE_WIDTH: f64 = 26.0;
/// Off-ball support scoring: score multiplier when the passing lane is
/// blocked.
pub const LANE_BLOCK: f64 = 0.25;

/// Pick the best off-ball support point: the candidate that is most open (far
/// from opponents), in a valuable area (upfield toward the attacking goal x
/// central), and reachable by a clear straight pass from the carrier.
/// Deterministic; ties resolve to the lowest candidate index. `attack_dir` is
/// +1 (attack +x) or -1. Returns the carrier's own position if there are no
/// candidates.
#[must_use]
pub fn support_spot(
    carrier_pos: Vec2,
    candidates: &[Vec2],
    opponents: &[Vec2],
    attack_dir: f64,
    field: Field,
) -> Vec2 {
    let mut best: Option<Vec2> = None;
    let mut best_score: Option<f64> = None;
    for c in candidates {
        let mut open = field.w;
        for o in opponents {
            open = open.min(c.dist(*o));
        }
        let depth = if attack_dir >= 0.0 {
            c.x / field.w
        } else {
            1.0 - c.x / field.w
        };
        let imp_x = sigmoid(IMPORTANCE_K * (depth - 0.5));
        let cy = (c.y - field.h / 2.0) / (field.h * CENTER_SIGMA);
        let imp_y = deterministic_math::exp(-cy * cy);
        let mut lane = 1.0;
        for o in opponents {
            if point_seg_dist(*o, carrier_pos, *c) < LANE_WIDTH {
                lane = LANE_BLOCK;
                break;
            }
        }
        let score = open * imp_x * imp_y * lane;
        if best_score.is_none_or(|bs| score > bs) {
            best_score = Some(score);
            best = Some(*c);
        }
    }
    best.unwrap_or(carrier_pos)
}

/// If any of `points` lies within `width` of the segment `from`->`to`
/// (excluding the very ends), return the lane-fraction (0..1) of the closest
/// such blocker, else `None`. Used to decide whether a pass lane is blocked
/// and where to lob over.
#[must_use]
pub fn lane_blocker(from: Vec2, to: Vec2, points: &[Vec2], width: f64) -> Option<f64> {
    let ab = to.sub(from);
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 < 1.0 {
        return None;
    }
    let mut best_f: Option<f64> = None;
    let mut best_d: Option<f64> = None;
    for p in points {
        let f = ((p.x - from.x) * ab.x + (p.y - from.y) * ab.y) / len2;
        if f > 0.1 && f < 0.95 {
            let d = p.dist(from.add(ab.scale(f)));
            if d < width && best_d.is_none_or(|bd| d < bd) {
                best_d = Some(d);
                best_f = Some(f);
            }
        }
    }
    best_f
}

/// Seconds before a threat is at full chase.
const INTERCEPT_REACT: f64 = 0.1;
/// Sample window start (lane fraction).
const INTERCEPT_F0: f64 = 0.1;
/// Sample window end: past this the receiver meets the ball.
const INTERCEPT_F1: f64 = 0.7;
/// Sample step.
const INTERCEPT_STEP: f64 = 0.05;

/// A moving defender or presser threatening to intercept a pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threat {
    /// The threat's current position.
    pub pos: Vec2,
    /// The threat's chase speed, px/s.
    pub speed: f64,
    /// Contact radius within which this body deflects a fast, low ball —
    /// the block rule's own `radius + BALL_RADIUS + species::block_reach`,
    /// per threat because radius and verb are per player. Distinct from
    /// [`pass_intercept`]'s `reach` parameter, which is the CLEAN-collection
    /// radius a slow ball is stolen inside.
    pub block_contact: f64,
}

/// How [`pass_intercept`] says a lane is cut.
///
/// Both variants mean "do not trust this lane": every current caller treats
/// them identically (see [`pass_intercept`]'s doc for why), and the kind is
/// reported because the branch already knows it — it costs nothing and lets
/// a test pin *which* rule fires where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneCut {
    /// The ball is slow enough (below the collection cap, or dead) for a
    /// threat to take clean possession.
    Collect,
    /// The ball is too fast to collect, but a body reaches blocking
    /// position in time and ricochets it loose (`sim::r#match`'s block
    /// rule).
    Deflect,
}

/// Interception model for a driven ground pass. Friction sheds a fraction of
/// the ball's speed per second (`dv/dt = -friction * v`), which makes the
/// decay linear in distance: after covering `d` the ball moves at
/// `launch - friction * d`, and it took
/// `ln(launch / (launch - friction * d)) / friction` seconds to get there. An
/// opponent cuts the pass out if it can reach some point of the flight before
/// the ball does, in one of two ways, mirroring the two rules the match
/// actually resolves:
///
/// * **Collection** ([`LaneCut::Collect`]): where the ball has slowed below
///   the collection cap, a threat within collection `reach` takes clean
///   possession.
/// * **Deflection** ([`LaneCut::Deflect`]): at or above the cap the ball
///   rolls past clean feet — but `sim::r#match`'s body-block rule ricochets
///   a fast, low ball off any outfield body it runs into, once `block_grace`
///   seconds have elapsed since release. A lane point a threat can occupy
///   (within its own `block_contact`) before the ball arrives is therefore a
///   risk at ANY speed. The remaining block-rule conditions hold
///   structurally rather than needing terms here: this models a driven
///   GROUND pass (`ball_z` = 0, always under block height — its one
///   production caller, `pass_risk`, never sees a loft), the threat arrives
///   facing an incoming ball (the rule's toward-the-body test), threats are
///   opposing outfielders so the keeper and designated-receiver exclusions
///   never apply, and the block-grace window is the explicit `block_grace`
///   term. The same `INTERCEPT_REACT` read-and-turn delay is charged as for
///   collection: the model asks "can this defender be there in time", the
///   same question either way.
///
/// Both cuts are reported as "lane is cut", undifferentiated in severity: a
/// deflection scatters the ball loose rather than conceding possession
/// outright, but every caller consumes this as a boolean lane-safety verdict
/// plus a lob-over point, and a numeric discount for the milder outcome
/// would be an unregistered, unmeasured weight — decoration by another name.
/// The [`LaneCut`] kind is returned so the distinction stays observable
/// (and testable) without pretending to price it.
///
/// Earliest point of a friction-decayed ground pass that some threat reaches
/// before the ball. Returns its lane fraction (0..1) — a ready-made
/// lob-over point — with how it is cut, or `None` when the pass outruns
/// every threat. Closed-form and sampled on a fixed grid: deterministic.
///
/// `launch_speed` is px/s at release, `friction` is the fraction of ball
/// speed shed per second, `reach` is the radius within which a threat
/// collects the ball, `max_collect_speed` is the speed at/above which a
/// ball can't be collected (only deflected), and `block_grace` is the
/// seconds after release during which the block rule holds fire.
///
/// The ln-ratio domain audit (#517, `gc_core::deterministic_math`'s
/// `LN_MAX_RATIO`) still holds with the deflection term: the new branch
/// computes the same `launch_speed / v` ratio at `v >= max_collect_speed`,
/// which is strictly SMALLER than the collection branch's worst case at the
/// same launch speed (`v` there decays toward 1), so it adds no new domain
/// edge.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn pass_intercept(
    from: Vec2,
    to: Vec2,
    launch_speed: f64,
    friction: f64,
    threats: &[Threat],
    reach: f64,
    max_collect_speed: f64,
    block_grace: f64,
) -> Option<(f64, LaneCut)> {
    let total = from.dist(to);
    if total < 1.0 || threats.is_empty() {
        return None;
    }
    let dir = to.sub(from).normalized();
    let steps = ((INTERCEPT_F1 - INTERCEPT_F0) / INTERCEPT_STEP + 0.5).floor() as i64;
    for i in 0..=steps {
        let f = INTERCEPT_F0 + (i as f64) * INTERCEPT_STEP;
        let d = f * total;
        let v = launch_speed - friction * d;
        if v <= 1.0 {
            // The ball dies on the lane: anyone can walk onto it.
            return Some((f, LaneCut::Collect));
        }
        let t_ball = deterministic_math::ln_ratio(launch_speed / v) / friction;
        let point = from.add(dir.scale(d));
        if v < max_collect_speed {
            for th in threats {
                let t_threat = INTERCEPT_REACT + (point.dist(th.pos) - reach).max(0.0) / th.speed;
                if t_threat <= t_ball {
                    return Some((f, LaneCut::Collect));
                }
            }
        } else if t_ball >= block_grace {
            // Too fast to collect, but not to block: the match deflects a
            // fast, low ball off any body it runs into once the release
            // grace has elapsed.
            for th in threats {
                let t_threat =
                    INTERCEPT_REACT + (point.dist(th.pos) - th.block_contact).max(0.0) / th.speed;
                if t_threat <= t_ball {
                    return Some((f, LaneCut::Deflect));
                }
            }
        }
    }
    None
}

/// Assign defenders to opponents (man-marking) with a stable greedy
/// matching. Pairs are ranked by distance with (defender, opponent) index
/// tiebreaks, making the sort a total order -> fully deterministic. A
/// `stick_bonus` discount on the previous tick's pair adds hysteresis so two
/// defenders don't swap the same mark every frame. Returns a
/// `defender_index -> opponent_index` map (partial if the counts differ).
#[must_use]
pub fn assign_marks(
    defenders: &[Vec2],
    opponents: &[Vec2],
    prev_map: Option<&IndexMap<usize, usize>>,
    stick_bonus: Option<f64>,
) -> IndexMap<usize, usize> {
    let stick_bonus = stick_bonus.unwrap_or(0.0);
    struct Pair {
        d: usize,
        o: usize,
        cost: f64,
    }
    let mut list: Vec<Pair> = Vec::with_capacity(defenders.len() * opponents.len());
    for (di, dp) in defenders.iter().enumerate() {
        for (oi, op) in opponents.iter().enumerate() {
            let mut cost = dp.dist(*op);
            if let Some(map) = prev_map
                && map.get(&di) == Some(&oi)
            {
                cost -= stick_bonus;
            }
            list.push(Pair { d: di, o: oi, cost });
        }
    }
    list.sort_by(|a, b| {
        if a.cost != b.cost {
            a.cost
                .partial_cmp(&b.cost)
                .expect("mark costs must be comparable")
        } else if a.d != b.d {
            a.d.cmp(&b.d)
        } else {
            a.o.cmp(&b.o)
        }
    });
    let mut result: IndexMap<usize, usize> = IndexMap::new();
    let mut d_taken = vec![false; defenders.len()];
    let mut o_taken = vec![false; opponents.len()];
    for pr in &list {
        if !d_taken[pr.d] && !o_taken[pr.o] {
            result.insert(pr.d, pr.o);
            d_taken[pr.d] = true;
            o_taken[pr.o] = true;
        }
    }
    result
}
