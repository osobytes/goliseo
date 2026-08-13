//! The #490 save-rate investigation's shadow classifier, committed and
//! pinned against the frozen `outfield_ai_baseline` 60-seed fixture, the
//! same way `outfield_ai_baseline_reproduces_the_frozen_fixture_exactly`
//! pins its own stats.
//!
//! Every observation is produced by the REAL `attempt_save` logic through
//! [`gc_sim::r#match::shadow_observations_begin`]/`shadow_observations_take`
//! — see that module's `SaveShadowObservation` doc comment. Nothing here
//! re-implements `attempt_save`'s gating (the `toward` quirk, `SAVE_ZONE`,
//! the keeper eligibility flags): a second implementation would be exactly
//! the risk `ball_prediction.rs`'s own module doc warns against, applied to
//! a diagnostic instead of a query.
//!
//! ## What "disagree" and "deferred" mean
//!
//! At every candidate save evaluation, `old_on_target` is what the deleted
//! `s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz` formula would have
//! decided; the real, predictor-backed decision is `new_resolved` (did
//! `position_at_time` return `Some`?) and, when resolved, `new_on_target`.
//! Four buckets fall out:
//!
//! - `agree_true`: both say on target — the ordinary case.
//! - `disagree` (old true, new false or unresolved): the old formula would
//!   have committed the keeper where the real physics does not. Splits into
//!   `disagree_deferred` (`new_resolved` false — the query just hadn't
//!   resolved yet at this tick) and `disagree_height` (`new_resolved` true
//!   but `new_on_target` false — a genuine, resolved height disagreement).
//! - `new_only` (old false, new true): proven impossible for the
//!   grounded/landed subcase — see below — and asserted to be exactly zero
//!   on this fixture, not just observed to be.
//! - `agree_false`: both say not on target.
//!
//! ## Why `new_only` is proven zero for its subcase, not just measured zero
//!
//! For a grounded or already-landed ball (`ball_z <= 0`, `ball_vz <= 0`,
//! true for every candidate this fixture's shots reach), the deleted
//! formula `old_z = ball_z + ball_vz*tz - 0.5*GRAVITY*tz*tz` is a strictly
//! decreasing, unbounded-below function of `tz` for `tz > 0`: it never
//! models the ground bounce, so once the real ball has landed the deleted
//! formula keeps falling through the floor while the real trajectory
//! settles or bounces back up. The on-target check
//! (`z_cross < CROSSBAR && z_cross <= KEEPER_AIR_GRAB`) is upper-bound-only.
//! So for any `tz`, `old_z <= z_real` (the deleted formula is never HIGHER
//! than the truth for a ball that has landed), which means whenever the
//! real height clears both upper bounds, the deleted formula's strictly
//! lower value clears them too: `new_on_target => old_on_target`. The
//! contrapositive is `!old_on_target => !new_on_target` — `new_only` cannot
//! happen, FOR THIS SUBCASE. This is a structural argument about the shape
//! of the two formulas for a landed ball, not a sampling result and not a
//! universal guarantee: the pre-bounce, still-airborne case is a different
//! shape (both integrate the same pure gravity and track each other up to
//! the `+0.5 * GRAVITY * dt * t` discretization bias between a continuous
//! formula and the live discrete step — see `docs/design/fun_metrics.md`'s
//! drift log) and is argued informally there, not proven here. The frozen
//! fixture's 9,376-observation `new_only == 0` below is empirical
//! confirmation covering both cases as this fixture happens to exercise
//! them, not a substitute for extending the proof to the airborne case.
//!
//! ## Byte-identical reconciliation (blocking item 3, PR #501)
//!
//! `matches_with_any_disagree_or_deferred` buckets each of the 60 official
//! seeds by whether ANY candidate tick in that match's-worth of ticks
//! touched `disagree_height` or `disagree_deferred`. A one-tick RNG-stream
//! shift (which is what a `disagree_deferred` episode resolving one tick
//! later than the old formula would have IS) cascades into a different
//! state hash for the rest of a deterministic match without changing
//! anything a viewer would call different — so this count, not the
//! `disagree_height`-only count, is the right one to compare against
//! "matches where old-code and new-code diverge at all".

use gc_sim::headless::{self, HeadlessBot, HeadlessOpts};
use gc_sim::r#match::{self as sim_match, SaveShadowObservation};
use gc_sim::outfield_ai_baseline as baseline;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    candidates: i64,
    agree_true: i64,
    agree_false: i64,
    disagree_deferred: i64,
    disagree_height: i64,
    new_only: i64,
}

impl Tally {
    fn add(&mut self, obs: &SaveShadowObservation) {
        self.candidates += 1;
        match (obs.old_on_target, obs.new_resolved, obs.new_on_target) {
            (true, true, true) => self.agree_true += 1,
            (true, false, _) => self.disagree_deferred += 1,
            (true, true, false) => self.disagree_height += 1,
            (false, true, true) => self.new_only += 1,
            (false, _, _) => self.agree_false += 1,
        }
    }

    fn disagree_total(&self) -> i64 {
        self.disagree_deferred + self.disagree_height
    }
}

/// Runs the declared 60-seed fixture one match at a time (so each match's
/// observations can be attributed to its own seed), recording shadow
/// observations for every one.
fn classify(seeds: &[i64]) -> (Tally, Vec<bool>, Vec<bool>) {
    let mut total = Tally::default();
    let mut had_disagree = Vec::with_capacity(seeds.len());
    let mut had_deferred = Vec::with_capacity(seeds.len());
    for &seed in seeds {
        sim_match::shadow_observations_begin();
        let _ = headless::run_match(&HeadlessOpts {
            seed: seed as f64,
            duration: Some(baseline::DURATION_SECONDS),
            max_goals: Some(baseline::MAX_GOALS),
            field: Some(baseline::FIELD),
            bot: Some(HeadlessBot::None),
            tuning_blob: Some(""),
            ..Default::default()
        });
        let observations = sim_match::shadow_observations_take();
        let mut per_match = Tally::default();
        for obs in &observations {
            per_match.add(obs);
            total.add(obs);
        }
        had_disagree.push(per_match.disagree_height > 0);
        had_deferred.push(per_match.disagree_deferred > 0);
    }
    (total, had_disagree, had_deferred)
}

/// Pinned against the frozen `outfield_ai_baseline` 60-seed fixture
/// (`20001..20060`). A moved count here is a finding about `attempt_save`'s
/// candidate rate or the predictor's resolution behavior on this fixture —
/// confirm intent and re-pin deliberately, the same discipline
/// `outfield_ai_baseline_reproduces_the_frozen_fixture_exactly` already
/// applies to the stats these candidates feed.
#[test]
fn shadow_classifier_reproduces_the_frozen_60_seed_counts() {
    let seeds = baseline::seeds();
    let (total, had_disagree, had_deferred) = classify(&seeds);

    // Re-pinned by #488's carry-composition fix, in the SAME commit as
    // `gc_data::outfield_ai_baseline`'s re-freeze. These counts are taken
    // over that fixture's 60 seeds, so the two describe one build or neither
    // describes anything -- this file's own doc asks for "the same discipline
    // `outfield_ai_baseline_reproduces_the_frozen_fixture_exactly` already
    // applies", and splitting them would leave one narrating a build the
    // other had left.
    //
    // candidates 9376 -> 10507: more save candidates arise because carriers
    // shield rather than surrendering the ball, so play reaches the keeper
    // more often. The agree/disagree split shifts with it; `new_only` stays
    // structurally 0, which is the assertion that would have been a finding.
    assert_eq!(total.candidates, 10507);
    assert_eq!(total.agree_true, 2593);
    assert_eq!(total.agree_false, 7677);
    assert_eq!(total.disagree_deferred, 210);
    assert_eq!(total.disagree_height, 27);
    assert_eq!(
        total.new_only, 0,
        "structurally impossible per this file's module doc; a nonzero \
         count here means the deleted formula's monotonicity argument no \
         longer holds for some fixture shot -- a real finding, not a \
         fixture update"
    );
    assert_eq!(
        total.candidates,
        total.agree_true + total.agree_false + total.disagree_total() + total.new_only
    );

    let matches_with_disagree = had_disagree.iter().filter(|&&b| b).count();
    let matches_with_deferred = had_deferred.iter().filter(|&&b| b).count();
    let matches_with_either = had_disagree
        .iter()
        .zip(had_deferred.iter())
        .filter(|&(&d, &f)| d || f)
        .count();

    // Pinned per-match attribution (blocking item 3, PR #501): the
    // `disagree_height` bucket alone touches 6/60 matches (10%) -- far
    // short of the 17/60 (28%) matches whose paired save_rate/goals_total
    // pair actually differs from the old-code control arm (see
    // `docs/design/fun_metrics.md`'s drift log). Folding in
    // `disagree_deferred` -- a same-shot, one-tick-later resolution that
    // still moves the RNG stream -- reaches 30/60 (50%), which is on the
    // right side of 17/60 to explain it (every byte-divergent match should
    // be "touched"; not every "touched" match need end up with a
    // different-looking aggregate stat, since two internally-diverged
    // matches can land on the same final save/goal counts by coincidence).
    // That is the reconciliation: deferred episodes, not disagreement
    // episodes, are the dominant driver of the byte-identical split.
    // Re-pinned by #488 alongside the counts above, and the reconciliation
    // still holds in the same direction: `disagree_height` alone touches
    // 10/60 matches (17%), and folding in `disagree_deferred` reaches 29/60
    // (48%) -- still on the right side of the 17/60 byte-divergent split that
    // this paragraph exists to explain, so deferred episodes remain the
    // dominant driver rather than disagreement episodes. The shift from
    // 6/25/30 to 10/22/29 moves work between the two buckets without
    // disturbing that conclusion.
    assert_eq!(matches_with_disagree, 10);
    assert_eq!(matches_with_deferred, 22);
    assert_eq!(matches_with_either, 29);
}
