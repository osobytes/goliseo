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

    // Re-pinned by #490 (the keeper save-fatigue pool and its catch band), in
    // the SAME commit as `gc_data::outfield_ai_baseline`'s v12 -> v13
    // re-freeze.
    //
    // candidates 9746 -> 9941: below `KEEPER_CATCH_THRESHOLD` a save that used
    // to be a clean catch now resolves as a parry, and a parry leaves the ball
    // live instead of ending the phase in the keeper's gloves -- so more
    // sequences continue and more of them come back to a keeper for a second
    // judgement. `disagree_deferred` 268 -> 230 and `disagree_height` 25 -> 24
    // both fall slightly; `agree_true` 3364 -> 3490 and `agree_false`
    // 6089 -> 6197 absorb the candidate-count rise. `new_only` stays
    // structurally 0, which is the assertion that would have been a finding
    // rather than a re-pin.
    //
    // Previously re-pinned by #489 (committed actions), in the SAME commit as
    // `gc_data::outfield_ai_baseline`'s v11 -> v12 re-freeze.
    //
    // candidates 9438 -> 9746: the standing-poke tackle now charges and
    // executes instead of resolving the instant it is in reach
    // (`gc_sim::action_slot`, `r#match::advance_tackle_actions`), so a
    // carrier gets more chances to escape a committed defender than it did
    // against the old instant-resolve check -- more possession sequences
    // run longer and reach a keeper before the ball is lost. `disagree_height`
    // 40 -> 25 falls (fewer resolved height disagreements) while
    // `disagree_deferred` 159 -> 268 rises sharply (more one-tick-later
    // resolutions, the RNG-stream-shifting signature this file's own module
    // doc describes) -- `agree_true`/`agree_false` absorb the rest of the
    // candidate-count rise. `new_only` stays structurally 0, which is the
    // assertion that would have been a finding rather than a re-pin.
    //
    // Re-pinned by #517's mechanical transcendental sweep (the dribble-touch
    // and AI-outfield-error cos/sin, the aerial-contact cos/sin, the bot
    // aim-noise cos/sin, and the support-triangle/combat-arc precomputed
    // constants), in the SAME commit as `gc_data::outfield_ai_baseline`'s
    // v10 -> v11 re-freeze, for the reason the paragraph below already
    // gives.
    //
    // candidates 9285 -> 9438: nine call sites across `match.rs`,
    // `aerial.rs`, `bot.rs`, `combat.rs` and `combat_feasibility.rs` moved
    // from `f64::cos`/`f64::sin` (not correctly rounded, and different
    // between native and wasm libm) to `gc_core::deterministic_math::cos_sin`
    // or a precomputed constant, changing every dribble touch, off-ball
    // support run and AI release angle by ULP-scale amounts that compound
    // over a 7200-tick match into materially different possession
    // sequences -- more of them now reach a keeper. `disagree_height`
    // 18 -> 40 and `disagree_deferred` 172 -> 159 move with it; `new_only`
    // stays structurally 0, which is the assertion that would have been a
    // finding rather than a re-pin.
    //
    // Previously re-pinned by #531 phase 2 (the gameplay AI's pass/throw seam), in the
    // SAME commit as `gc_data::outfield_ai_baseline`'s v9 -> v10 re-freeze,
    // for the reason the paragraph below already gives.
    //
    // candidates 9208 -> 9285: the AI now charges a pass/throw over several
    // ticks instead of releasing on the spot, so it is dispossessed via
    // tackle mid-charge more often than it used to be dispossessed
    // instantly -- more of those broken sequences end with the ball
    // reaching a keeper than before, a small rise rather than the large
    // fall #491's own solver-aim change produced. `disagree_height` 10 -> 18
    // and `disagree_deferred` 171 -> 172 move with it; `new_only` stays
    // structurally 0, which is the assertion that would have been a finding
    // rather than a re-pin.
    //
    // Previously re-pinned by #491's passing rework, in the SAME commit as
    // `gc_data::outfield_ai_baseline`'s v8 -> v9 re-freeze, for the reason
    // the paragraph below already gives.
    //
    // candidates 10507 -> 9208: the lead solver aims a driven ground pass at
    // where the receiver WILL be rather than at their feet, so possession
    // sequences run longer between loose balls and fewer of them end in a
    // shot the keeper has to judge. `disagree_height` 27 -> 10 and
    // `disagree_deferred` 210 -> 171 fall roughly in proportion. `new_only`
    // stays structurally 0, which is the assertion that would have been a
    // finding rather than a re-pin.
    //
    // Previously re-pinned by #488's carry-composition fix, in the SAME commit as
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
    // Re-pinned by #572, completing #489's possession invariant, in the SAME
    // commit as `gc_data::outfield_ai_baseline`'s v13 -> v14 re-freeze, for
    // the reason the paragraph above already gives.
    //
    // candidates 9941 -> 9970: seven ownership writes (eight with `combat`'s
    // ball spill) were silently exempt from the rule that a possession change
    // clears the outgoing owner's committed action slot, and now are not. A
    // presser that whiffs a standing poke and then loses the ball no longer
    // serves out its miss recovery, so it re-presses sooner. Unlike this
    // artifact's previous re-pins the candidate count moves only slightly and
    // UPWARD, while the agree/disagree split moves considerably more
    // (agree_true 3490 -> 3307, agree_false 6197 -> 6429, disagree_deferred
    // 230 -> 207, disagree_height 24 -> 27): the change reshuffles WHEN a
    // shot reaches the keeper far more than it changes HOW OFTEN, which is
    // the signature this file's module doc gives for a timing shift rather
    // than a volume one. `new_only` stays structurally 0, which is the
    // assertion that would have been a finding rather than a re-pin.
    // candidates 9970 -> 9775: #578 narrowed the possession invariant so a
    // `Recovering` slot survives the ownership change -- a presser that
    // whiffs a standing poke now serves its full miss recovery even when it
    // touches and loses the loose ball, so it re-presses later and slightly
    // fewer possessions reach a save candidate at all. The split moves back
    // toward the pre-#572 shape and past it on this base (agree_true 3307 ->
    // 3415, agree_false 6429 -> 6066, disagree_deferred 207 -> 268,
    // disagree_height 27 -> 26): a later-served recovery is the same
    // one-tick-earlier-or-later RNG-stream shift this file's module doc
    // names as `disagree_deferred`'s signature, now pointing the other way.
    // `new_only` stays structurally 0, which is the assertion that would
    // have been a finding rather than a re-pin. Re-pinned in the SAME commit
    // as `gc_data::outfield_ai_baseline`'s v14 -> v15 re-freeze, per this
    // file's own coupling rule.
    assert_eq!(total.candidates, 9775);
    assert_eq!(total.agree_true, 3415);
    assert_eq!(total.agree_false, 6066);
    assert_eq!(total.disagree_deferred, 268);
    assert_eq!(total.disagree_height, 26);
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
    // Re-pinned by #491 alongside the counts above. The reconciliation still
    // holds in the same direction: `disagree_height` alone touches 6/60
    // matches (10%), and folding in `disagree_deferred` reaches 24/60 (40%)
    // -- still on the right side of the 17/60 byte-divergent split this
    // paragraph exists to explain, so deferred episodes remain the dominant
    // driver. The shift from 10/22/29 to 6/21/24 tracks the fall in
    // candidates and disturbs nothing about the conclusion.
    // Re-pinned by #531 phase 2 alongside the counts above. The
    // reconciliation still holds in the same direction: `disagree_height`
    // alone touches 6/60 matches (10%), and folding in `disagree_deferred`
    // reaches 25/60 (42%) -- still on the right side of the 17/60
    // byte-divergent split this paragraph exists to explain, so deferred
    // episodes remain the dominant driver. The shift from 6/21/24 to
    // 6/23/25 tracks the rise in candidates and disturbs nothing about the
    // conclusion.
    //
    // Re-pinned by #517's mechanical transcendental sweep alongside the
    // counts above. Unlike every prior re-pin here, this change is not a
    // gameplay AI change to compare against the old-code/new-code control
    // arm from PR #501 -- it is nine call sites moving from native libm
    // `cos`/`sin` to `gc_core::deterministic_math::cos_sin` or a precomputed
    // constant, which is exactly the kind of RNG-stream-shifting,
    // one-tick-earlier-or-later change this file's own module doc describes
    // as `disagree_deferred`'s signature. `disagree_height` alone touches
    // 11/60 matches (18%), and folding in `disagree_deferred` reaches 33/60
    // (55%). Both counts rose along with `disagree_height`'s near-2.5x jump
    // above; deferred episodes remain the larger of the two buckets, so the
    // reconciliation this paragraph exists to explain is unchanged in kind,
    // even though this re-pin has no historical byte-divergent split to
    // compare its own fraction against.
    //
    // Re-pinned by #490 alongside the counts above: the per-match spread barely
    // moves (`disagree_height` still 13/60, `disagree_deferred` 31 -> 32, the
    // union still 37/60) even though the candidate count rose by 195. That is
    // the expected shape for this change and worth stating: the catch band
    // produces MORE save candidates in the same matches, not disagreements in
    // new ones. No historical byte-divergent split to compare against here
    // either.
    //
    // Previously re-pinned by #489 alongside the counts above:
    // `disagree_height` alone touches 13/60 matches (22%), and folding in
    // `disagree_deferred` reaches 37/60 (62%) -- deferred episodes remain the
    // larger and now dominant bucket by an even wider margin, consistent with
    // `disagree_deferred` more than doubling above while `disagree_height`
    // fell.
    //
    // Re-pinned by #572 alongside the counts above: `disagree_height` alone
    // touches 14/60 matches (23%), and folding in `disagree_deferred` reaches
    // 34/60 (57%). Deferred episodes remain the larger bucket, by a narrower
    // margin than the previous re-pin recorded -- consistent with
    // `disagree_deferred` falling and `disagree_height` rising above. An
    // earlier-cleared miss recovery is exactly the one-tick-earlier-or-later
    // RNG-stream shift this file's module doc names as `disagree_deferred`'s
    // signature. No historical byte-divergent split to compare against.
    // Re-pinned by #578 alongside the counts above: `disagree_height` alone
    // touches 15/60 matches (25%), and folding in `disagree_deferred`
    // reaches 38/60 (63%). Deferred episodes remain the dominant bucket, by
    // a wider margin than #572's re-pin recorded -- consistent with
    // `disagree_deferred` rising above as the served recovery shifts when
    // presses happen rather than how often shots reach the keeper. No
    // historical byte-divergent split to compare against.
    assert_eq!(matches_with_disagree, 15);
    assert_eq!(matches_with_deferred, 33);
    assert_eq!(matches_with_either, 38);
}
