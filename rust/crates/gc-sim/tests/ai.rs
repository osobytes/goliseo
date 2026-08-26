//! Tests for `gc_sim::ai`.
//!
//! `ai::closest` and `ai::assign_marks` index directly into caller-supplied
//! slices, so their indices are 0-based Rust collection indices
//! (ARCHITECTURE.md §3 rule 3).

use gc_core::vec2::Vec2;
use gc_sim::ai;

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6,
        "expected ~{expected}, got {actual}"
    );
}

#[test]
fn ai_closest_returns_the_index_of_the_nearest_position() {
    let ps = [
        Vec2::new(10.0, 0.0),
        Vec2::new(3.0, 0.0),
        Vec2::new(20.0, 0.0),
    ];
    assert_eq!(ai::closest(Vec2::new(0.0, 0.0), &ps, None), Some(1));
}

#[test]
fn ai_closest_honours_the_exclude_index() {
    let ps = [Vec2::new(1.0, 0.0), Vec2::new(5.0, 0.0)];
    assert_eq!(ai::closest(Vec2::new(0.0, 0.0), &ps, Some(0)), Some(1));
}

#[test]
fn ai_closest_returns_nil_when_there_are_no_candidates() {
    assert_eq!(ai::closest(Vec2::new(0.0, 0.0), &[], None), None);
}

#[test]
fn ai_steer_snaps_to_the_target_when_within_range() {
    let (np, dir) = ai::steer(Vec2::new(0.0, 0.0), Vec2::new(3.0, 0.0), 10.0);
    assert_eq!(np.x, 3.0);
    assert_eq!(np.y, 0.0);
    near(dir.length(), 1.0);
}

#[test]
fn ai_steer_moves_at_most_max_dist_toward_the_target() {
    let (np, _) = ai::steer(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), 10.0);
    near(np.x, 10.0);
}

#[test]
fn ai_steer_yields_a_zero_direction_when_already_at_the_target() {
    let (_, dir) = ai::steer(Vec2::new(5.0, 5.0), Vec2::new(5.0, 5.0), 10.0);
    assert_eq!(dir.x, 0.0);
    assert_eq!(dir.y, 0.0);
}

#[test]
fn ai_pursue_returns_the_target_itself_when_it_is_not_moving() {
    let p = ai::pursue(
        Vec2::new(0.0, 0.0),
        Vec2::new(50.0, 0.0),
        Vec2::new(0.0, 0.0),
        0.01,
    );
    assert_eq!(p.x, 50.0);
    assert_eq!(p.y, 0.0);
}

#[test]
fn ai_pursue_leads_a_moving_target_ahead_of_its_position() {
    // dist 100, lead 0.01 -> horizon 1.0s; vel (10,0) -> +10 ahead
    let p = ai::pursue(
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 0.0),
        Vec2::new(10.0, 0.0),
        0.01,
    );
    near(p.x, 110.0);
}

#[test]
fn ai_interpose_returns_the_midpoint_at_frac_0_5() {
    let m = ai::interpose(Vec2::new(0.0, 0.0), Vec2::new(10.0, 20.0), 0.5);
    assert_eq!(m.x, 5.0);
    assert_eq!(m.y, 10.0);
}

#[test]
fn ai_interpose_returns_the_endpoints_at_0_and_1() {
    let a = ai::interpose(Vec2::new(2.0, 3.0), Vec2::new(9.0, 9.0), 0.0);
    let b = ai::interpose(Vec2::new(2.0, 3.0), Vec2::new(9.0, 9.0), 1.0);
    assert_eq!(a.x, 2.0);
    assert_eq!(b.x, 9.0);
}

#[test]
fn ai_separation_cancels_to_zero_for_symmetric_neighbours() {
    let off = ai::separation(
        Vec2::new(0.0, 0.0),
        &[Vec2::new(5.0, 0.0), Vec2::new(-5.0, 0.0)],
        20.0,
    );
    near(off.x, 0.0);
    near(off.y, 0.0);
}

#[test]
fn ai_separation_pushes_directly_away_from_a_single_neighbour() {
    let off = ai::separation(Vec2::new(0.0, 0.0), &[Vec2::new(5.0, 0.0)], 20.0);
    assert!(off.x < 0.0, "pushed away along -x");
    near(off.y, 0.0);
}

#[test]
fn ai_separation_ignores_neighbours_outside_the_radius() {
    let off = ai::separation(Vec2::new(0.0, 0.0), &[Vec2::new(50.0, 0.0)], 20.0);
    assert_eq!(off.x, 0.0);
    assert_eq!(off.y, 0.0);
}

fn field() -> ai::Field {
    ai::Field { w: 960.0, h: 540.0 }
}

#[test]
fn ai_support_spot_prefers_the_central_spot_over_a_wide_one_when_both_are_open() {
    let carrier = Vec2::new(300.0, 270.0);
    let central = Vec2::new(600.0, 270.0);
    let wide = Vec2::new(600.0, 60.0);
    let best = ai::support_spot(
        carrier,
        &[central, wide],
        &[Vec2::new(900.0, 900.0)],
        1.0,
        field(),
    );
    assert_eq!(best.x, 600.0);
    assert_eq!(best.y, 270.0);
}

#[test]
fn ai_support_spot_avoids_a_spot_sitting_on_an_opponent() {
    let carrier = Vec2::new(300.0, 270.0);
    let open = Vec2::new(600.0, 270.0);
    let marked = Vec2::new(620.0, 280.0);
    let best = ai::support_spot(
        carrier,
        &[open, marked],
        &[Vec2::new(620.0, 280.0)],
        1.0,
        field(),
    );
    assert_eq!(best.x, 600.0);
}

#[test]
fn ai_support_spot_prefers_a_lane_clear_spot_when_the_central_lane_is_blocked() {
    let carrier = Vec2::new(300.0, 270.0);
    let blocked = Vec2::new(600.0, 270.0); // opponent stands on this passing lane
    let clear = Vec2::new(600.0, 100.0);
    let best = ai::support_spot(
        carrier,
        &[blocked, clear],
        &[Vec2::new(450.0, 270.0)],
        1.0,
        field(),
    );
    assert_eq!(best.y, 100.0);
}

// Match-like tuning: friction 1.2/s, control radius 22 px, collect cap
// 350 px/s, block contact 18 px (PLAYER_RADIUS 12 + BALL_RADIUS 6 + the
// neutral species block-reach hook), block grace 0.08 s.
const F: f64 = 1.2;
const REACH: f64 = 22.0;
const CAP: f64 = 350.0;
const BLOCK: f64 = 18.0;
const GRACE: f64 = 0.08;

fn threat(pos: Vec2, speed: f64) -> ai::Threat {
    ai::Threat {
        pos,
        speed,
        block_contact: BLOCK,
    }
}

#[test]
fn ai_pass_intercept_flags_a_slow_pass_a_nearby_defender_can_step_onto() {
    // 200px pass at 320 px/s with a defender parked on the midpoint: the
    // ball needs ~0.39s to get there, the defender is already waiting.
    let f = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        320.0,
        F,
        &[threat(Vec2::new(100.0, 0.0), 200.0)],
        REACH,
        CAP,
        GRACE,
    );
    let (f, cut) = f.expect("the lane is cut");
    assert!(f > 0.0 && f < 1.0, "the fraction lies on the lane");
    assert_eq!(cut, ai::LaneCut::Collect, "a slow ball is stolen cleanly");
}

/// This case asserted `None` until the deflection-aware model: launched at
/// 620 px/s the ball never decays below the collection cap within 200 px
/// (620 - 1.2*200 = 380 > 350), so nothing can take CLEAN possession — and
/// the model stopped there, scoring the lane risk-free while the match's own
/// body-block rule ricocheted exactly this ball off exactly this body. The
/// model now describes the rule that actually fires: the surviving half of
/// the old claim is the `LaneCut` kind (never `Collect` — still too fast to
/// steal anywhere), and the lane is nonetheless cut.
#[test]
fn ai_pass_intercept_a_hard_driven_ball_cannot_be_stolen_but_a_parked_body_deflects_it() {
    let f = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        620.0,
        F,
        &[threat(Vec2::new(100.0, 0.0), 200.0)],
        REACH,
        CAP,
        GRACE,
    );
    let (f, cut) = f.expect("a body in the path cuts even a driven lane");
    assert!(f > 0.0 && f < 1.0, "the fraction lies on the lane");
    assert_eq!(
        cut,
        ai::LaneCut::Deflect,
        "too fast to collect anywhere on the lane: the cut is the block rule, \
         never a clean steal"
    );
}

/// The claim the old `None` case still legitimately made — a hard driven
/// ball outruns a defender — restated with the defender where it is true:
/// off the lane, where reaching any blocking position in time is a chase it
/// loses.
#[test]
fn ai_pass_intercept_a_hard_driven_ball_outruns_a_defender_off_the_lane() {
    let f = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        620.0,
        F,
        &[threat(Vec2::new(100.0, 120.0), 200.0)],
        REACH,
        CAP,
        GRACE,
    );
    assert_eq!(f, None, "the drive beats the chase to every lane point");
}

/// The block-grace mirror: the match's block rule holds fire for
/// `BLOCK_GRACE` seconds after release, so a body the ball passes within
/// that window — even one already standing in the path — deflects nothing,
/// and the model must not count it. A grace longer than the whole sampled
/// flight makes every fast lane point unblockable.
#[test]
fn ai_pass_intercept_grace_holds_the_block_rule_fire_early_in_the_flight() {
    let cut_normally = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        620.0,
        F,
        &[threat(Vec2::new(100.0, 0.0), 200.0)],
        REACH,
        CAP,
        GRACE,
    );
    assert!(cut_normally.is_some(), "(setup sanity: this lane is cut)");
    let f = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        620.0,
        F,
        &[threat(Vec2::new(100.0, 0.0), 200.0)],
        REACH,
        CAP,
        10.0,
    );
    assert_eq!(
        f, None,
        "within the grace window a fast ball passes straight through bodies"
    );
}

#[test]
fn ai_pass_intercept_is_safe_when_the_defender_cannot_reach_any_point_in_time() {
    let f = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        320.0,
        F,
        &[threat(Vec2::new(100.0, 250.0), 200.0)],
        REACH,
        CAP,
        GRACE,
    );
    assert_eq!(f, None, "a far defender never beats the ball");
}

#[test]
fn ai_pass_intercept_a_defender_chasing_from_behind_the_passer_never_catches_the_ball() {
    let f = ai::pass_intercept(
        Vec2::new(0.0, 0.0),
        Vec2::new(200.0, 0.0),
        320.0,
        F,
        &[threat(Vec2::new(-60.0, 0.0), 200.0)],
        REACH,
        CAP,
        GRACE,
    );
    assert_eq!(f, None, "the ball stays ahead of a trailing chaser");
}

#[test]
fn ai_pass_intercept_returns_nil_with_no_threats() {
    assert_eq!(
        ai::pass_intercept(
            Vec2::new(0.0, 0.0),
            Vec2::new(200.0, 0.0),
            320.0,
            F,
            &[],
            REACH,
            CAP,
            GRACE
        ),
        None
    );
}

#[test]
fn ai_assign_marks_matches_each_defender_to_its_nearest_opponent() {
    let defs = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
    let opps = [Vec2::new(5.0, 0.0), Vec2::new(105.0, 0.0)];
    let m = ai::assign_marks(&defs, &opps, None, None);
    assert_eq!(m.get(&0), Some(&0));
    assert_eq!(m.get(&1), Some(&1));
}

#[test]
fn ai_assign_marks_breaks_ties_deterministically_by_index() {
    let defs = [Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0)];
    let opps = [Vec2::new(5.0, 0.0), Vec2::new(5.0, 0.0)];
    let m = ai::assign_marks(&defs, &opps, None, None);
    assert_eq!(m.get(&0), Some(&0));
    assert_eq!(m.get(&1), Some(&1));
}

#[test]
fn ai_assign_marks_keeps_a_prior_mark_under_a_small_perturbation_but_switches_under_a_large_one() {
    let defs = [Vec2::new(0.0, 0.0)];
    let opps = [Vec2::new(10.0, 0.0), Vec2::new(8.0, 0.0)]; // o2 is nearer
    assert_eq!(
        ai::assign_marks(&defs, &opps, None, Some(5.0)).get(&0),
        Some(&1),
        "no history -> nearest (o2)"
    );
    let mut prev = indexmap::IndexMap::new();
    prev.insert(0usize, 0usize);
    assert_eq!(
        ai::assign_marks(&defs, &opps, Some(&prev), Some(5.0)).get(&0),
        Some(&0),
        "sticky bonus keeps o1"
    );
    assert_eq!(
        ai::assign_marks(&defs, &opps, Some(&prev), Some(1.0)).get(&0),
        Some(&1),
        "tiny bonus -> switches to o2"
    );
}
