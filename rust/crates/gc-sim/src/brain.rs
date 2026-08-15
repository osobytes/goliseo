//! Pure outfield decision and assignment helpers. Callers own all clocks,
//! state, candidate construction, and action execution; this module only
//! resolves serializable values.
//!
//! `player_index` fields throughout this module are opaque, caller-owned
//! identities (never used to index a Rust-side collection inside this
//! module), so — like the stable slot identities in `sim::input_frame` —
//! they keep their authored 1-based value rather than being remapped to a
//! 0-based array offset.

use gc_core::deterministic_math;
use gc_core::rng;
use indexmap::IndexMap;
use std::cmp::Ordering;

/// The high-level phase a team is playing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeamPhase {
    /// The team's own settled possession.
    Attack,
    /// The opponent's settled possession.
    Defend,
    /// Nobody has settled possession.
    Loose,
    /// Inside the window after losing the ball.
    Counterpress,
    /// Inside the window after winning the ball.
    Counterattack,
}

/// Coarse ball ownership as seen by [`phase`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainPossession {
    /// This team has settled possession.
    Team,
    /// The opponent has settled possession.
    Opponent,
    /// Nobody has settled possession.
    Loose,
}

/// Which side of a turnover a live transition represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainTransition {
    /// This team just won the ball.
    Won,
    /// This team just lost the ball.
    Lost,
}

/// An off-ball run shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RunType {
    /// A run in behind the last defender.
    InBehind,
    /// A run back short to offer an outlet.
    ComeShort,
    /// A run that holds the width of the pitch.
    HoldWidth,
}

/// The two press intensities a player can be told to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressMode {
    /// Shadow the ball without diving in.
    Contain,
    /// Close down and try to win the ball.
    Commit,
}

/// Why [`press_mode`] chose the mode it did, in stable trigger-precedence
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressReason {
    /// The carrier just took a heavy touch.
    HeavyTouch,
    /// The ball is exposed and can be won cleanly.
    ExposedBall,
    /// A covering teammate makes the commit safe.
    Cover,
    /// The team is desperate enough to commit regardless.
    BoxDesperation,
    /// Team discipline has slipped below its threshold.
    LowDiscipline,
    /// Nothing triggered a commit.
    NoTrigger,
}

/// Inputs to [`phase`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainPhaseContext {
    /// Coarse ball ownership.
    pub possession: BrainPossession,
    /// The live transition this team is inside, if any.
    pub transition: Option<BrainTransition>,
    /// Seconds elapsed since that transition started.
    pub transition_elapsed: f64,
    /// Seconds a "lost" transition stays a counterpress.
    pub counterpress_window: f64,
    /// Seconds a "won" transition stays a counterattack.
    pub counterattack_window: f64,
}

/// One candidate for [`assign_presser`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresserCandidate {
    /// The candidate's stable player identity.
    pub player_index: u32,
    /// Non-negative cost; lower wins.
    pub distance_cost: f64,
    /// `Some(false)` excludes the candidate; `None`/`Some(true)` include it.
    pub eligible: Option<bool>,
}

/// A single scored run target, as offered by one player for one run type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainRunTarget {
    /// The target's score; higher is more attractive.
    pub score: f64,
    /// Target x position.
    pub x: f64,
    /// Target y position.
    pub y: f64,
    /// How long the run should be granted for, in seconds. Must be positive.
    pub duration: f64,
}

/// One player's run offers for [`run_candidates`].
#[derive(Clone, Debug, PartialEq)]
pub struct BrainRunPlayer {
    /// The player's stable identity.
    pub player_index: u32,
    /// `Some(false)` excludes the player entirely; otherwise included.
    pub eligible: Option<bool>,
    /// This player's in-behind offer, if any.
    pub in_behind: Option<BrainRunTarget>,
    /// This player's come-short offer, if any.
    pub come_short: Option<BrainRunTarget>,
    /// This player's hold-width offer, if any.
    pub hold_width: Option<BrainRunTarget>,
}

/// Inputs to [`run_candidates`].
#[derive(Clone, Debug, PartialEq)]
pub struct BrainRunContext {
    /// Every player's run offers this tick.
    pub players: Vec<BrainRunPlayer>,
}

/// One ranked run offer, produced by [`run_candidates`] and consumed by
/// [`grant_runs`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunCandidate {
    /// The offering player's stable identity.
    pub player_index: u32,
    /// Which run shape this is.
    pub run_type: RunType,
    /// The offer's score; higher is more attractive.
    pub score: f64,
    /// Target x position.
    pub target_x: f64,
    /// Target y position.
    pub target_y: f64,
    /// How long the run should be granted for, in seconds.
    pub duration: f64,
}

/// One live granted run, as tracked by the caller between ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunSlot {
    /// The runner's stable identity.
    pub player_index: u32,
    /// Which run shape this is.
    pub run_type: RunType,
    /// The offer's score at the time it was granted.
    pub score: f64,
    /// Target x position.
    pub target_x: f64,
    /// Target y position.
    pub target_y: f64,
    /// The absolute time the run was granted.
    pub granted_at: f64,
    /// The absolute time the run expires.
    pub expires_at: f64,
}

/// Inputs to [`press_mode`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrainPressContext {
    /// The carrier just took a heavy touch.
    pub heavy_touch: bool,
    /// The ball is loose and immediately winnable.
    pub exposed_ball: bool,
    /// A teammate is covering behind the presser.
    pub cover_available: bool,
    /// The team is desperate enough to commit regardless of discipline.
    pub box_desperation: bool,
    /// This team's current press discipline, in `[0, 1]`.
    pub press_discipline: f64,
    /// The discipline floor below which pressing commits anyway, in `[0, 1]`.
    pub low_discipline_threshold: f64,
}

/// A value carried in a [`BrainScoredOption`] payload.
#[derive(Clone, Debug, PartialEq)]
pub enum BrainPayloadValue {
    /// A boolean payload value.
    Bool(bool),
    /// A numeric payload value.
    Number(f64),
    /// A text payload value.
    Text(String),
}

/// What a scored option refers back to: a caller-owned identity, either
/// textual or a stable index.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionReference {
    /// A textual reference.
    Text(String),
    /// A stable-index reference (e.g. a player or run-slot identity).
    Index(u32),
}

/// One option in a seeded softmax selection ([`select_scored_option`]).
#[derive(Clone, Debug, PartialEq)]
pub struct BrainScoredOption {
    /// The option's identity within its `kind`. Must be non-empty.
    pub id: String,
    /// The option's category. Must be non-empty.
    pub kind: String,
    /// The option's score; higher is more attractive.
    pub score: f64,
    /// Arbitrary extra data the caller attaches to this option.
    pub payload: Option<IndexMap<String, BrainPayloadValue>>,
    /// What this option refers back to.
    pub reference: Option<OptionReference>,
}

/// An alias for [`BrainScoredOption`] used at carrier decision call sites.
pub type CarrierOption = BrainScoredOption;

const RUN_LABEL_IN_BEHIND: &str = "in-behind run";
const RUN_LABEL_COME_SHORT: &str = "come-short run";
const RUN_LABEL_HOLD_WIDTH: &str = "hold-width run";

fn assert_positive_index(value: u32, label: &str) {
    assert!(value >= 1, "{label} must be a positive integer");
}

/// Clamp any finite number into `[0, 1]`; non-finite values saturate toward
/// the nearer bound (`NaN` and `-inf` -> `0`, `+inf` -> `1`).
fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return 0.0;
    }
    if value == f64::INFINITY {
        return 1.0;
    }
    value.clamp(0.0, 1.0)
}

/// Derive this team's high-level phase from possession and any live
/// transition window.
#[must_use]
pub fn phase(context: &BrainPhaseContext) -> TeamPhase {
    assert!(
        context.transition_elapsed.is_finite(),
        "phase transition elapsed must be finite"
    );
    assert!(
        context.transition_elapsed >= 0.0,
        "phase transition elapsed must be non-negative"
    );
    assert!(
        context.counterpress_window.is_finite(),
        "counterpress window must be finite"
    );
    assert!(
        context.counterpress_window >= 0.0,
        "counterpress window must be non-negative"
    );
    assert!(
        context.counterattack_window.is_finite(),
        "counterattack window must be finite"
    );
    assert!(
        context.counterattack_window >= 0.0,
        "counterattack window must be non-negative"
    );

    if context.transition == Some(BrainTransition::Lost)
        && context.transition_elapsed < context.counterpress_window
    {
        return TeamPhase::Counterpress;
    }
    if context.transition == Some(BrainTransition::Won)
        && context.transition_elapsed < context.counterattack_window
    {
        return TeamPhase::Counterattack;
    }
    match context.possession {
        BrainPossession::Team => TeamPhase::Attack,
        BrainPossession::Opponent => TeamPhase::Defend,
        BrainPossession::Loose => TeamPhase::Loose,
    }
}

/// Maps a normalized scan rate to a personal interval. Invalid scan-rate
/// scalars are saturated safely; interval endpoints remain caller-owned
/// programmer inputs.
#[must_use]
pub fn refresh_interval(scan_rate: f64, slow_seconds: f64, fast_seconds: f64) -> f64 {
    assert!(
        slow_seconds.is_finite(),
        "slow refresh interval must be finite"
    );
    assert!(
        slow_seconds >= 0.0,
        "slow refresh interval must be non-negative"
    );
    assert!(
        fast_seconds.is_finite(),
        "fast refresh interval must be finite"
    );
    assert!(
        fast_seconds >= 0.0,
        "fast refresh interval must be non-negative"
    );
    assert!(
        slow_seconds >= fast_seconds,
        "slow refresh interval must not be shorter than fast refresh interval"
    );
    let rate = clamp_unit(scan_rate);
    slow_seconds + (fast_seconds - slow_seconds) * rate
}

/// Pick the presser to commit: nearest eligible candidate, with a hysteresis
/// band that keeps the current presser unless a rival clears both a real
/// improvement and the switch ratio.
#[must_use]
pub fn assign_presser(
    candidates: &[PresserCandidate],
    current: Option<u32>,
    switch_ratio: f64,
) -> Option<u32> {
    struct Eligible {
        player_index: u32,
        distance_cost: f64,
    }

    let mut eligible: Vec<Eligible> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let label = format!("presser candidate {}", index + 1);
        assert_positive_index(candidate.player_index, &format!("{label} player"));
        assert!(
            !seen.contains(&candidate.player_index),
            "presser candidate player indices must be unique"
        );
        seen.push(candidate.player_index);
        assert!(
            candidate.distance_cost.is_finite(),
            "{label} distance cost must be finite"
        );
        assert!(
            candidate.distance_cost >= 0.0,
            "presser distance cost must be non-negative"
        );
        if candidate.eligible != Some(false) {
            eligible.push(Eligible {
                player_index: candidate.player_index,
                distance_cost: candidate.distance_cost,
            });
        }
    }
    eligible.sort_by(|a, b| {
        if a.distance_cost != b.distance_cost {
            a.distance_cost
                .partial_cmp(&b.distance_cost)
                .expect("distance costs must be comparable")
        } else {
            a.player_index.cmp(&b.player_index)
        }
    });

    let (best_player_index, best_distance_cost) = match eligible.first() {
        Some(best) => (best.player_index, best.distance_cost),
        None => return None,
    };

    let current_candidate = current.and_then(|value| {
        assert_positive_index(value, "current presser");
        eligible.iter().find(|c| c.player_index == value)
    });

    let current_candidate = match current_candidate {
        None => return Some(best_player_index),
        Some(c) if c.player_index == best_player_index => return Some(best_player_index),
        Some(c) => c,
    };

    let ratio = clamp_unit(switch_ratio);
    let is_real_improvement = best_distance_cost < current_candidate.distance_cost;
    let clears_hysteresis = best_distance_cost <= current_candidate.distance_cost * (1.0 - ratio);
    if is_real_improvement && clears_hysteresis {
        Some(best_player_index)
    } else {
        Some(current_candidate.player_index)
    }
}

fn sort_run_candidates(candidates: &mut [RunCandidate]) {
    candidates.sort_by(|a, b| {
        if a.score != b.score {
            b.score
                .partial_cmp(&a.score)
                .expect("run candidate scores must be comparable")
        } else if a.player_index != b.player_index {
            a.player_index.cmp(&b.player_index)
        } else {
            a.run_type.cmp(&b.run_type)
        }
    });
}

fn append_run_candidate(
    candidates: &mut Vec<RunCandidate>,
    player_index: u32,
    run_type: RunType,
    target: &BrainRunTarget,
    label: &str,
) {
    assert!(target.score.is_finite(), "{label} score must be finite");
    assert!(target.x.is_finite(), "{label} target x must be finite");
    assert!(target.y.is_finite(), "{label} target y must be finite");
    assert!(
        target.duration.is_finite(),
        "{label} duration must be finite"
    );
    assert!(target.duration > 0.0, "{label} duration must be positive");
    candidates.push(RunCandidate {
        player_index,
        run_type,
        score: target.score,
        target_x: target.x,
        target_y: target.y,
        duration: target.duration,
    });
}

/// Build the deterministic, fully ranked run-candidate list from every
/// player's offers: sorted by score descending, then player index, then run
/// type, so the order is a total order regardless of caller iteration order.
#[must_use]
pub fn run_candidates(context: &BrainRunContext) -> Vec<RunCandidate> {
    let mut candidates = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    for (index, player) in context.players.iter().enumerate() {
        assert_positive_index(player.player_index, &format!("run player {}", index + 1));
        assert!(
            !seen.contains(&player.player_index),
            "run player indices must be unique"
        );
        seen.push(player.player_index);
        if player.eligible != Some(false) {
            if let Some(target) = &player.in_behind {
                append_run_candidate(
                    &mut candidates,
                    player.player_index,
                    RunType::InBehind,
                    target,
                    RUN_LABEL_IN_BEHIND,
                );
            }
            if let Some(target) = &player.come_short {
                append_run_candidate(
                    &mut candidates,
                    player.player_index,
                    RunType::ComeShort,
                    target,
                    RUN_LABEL_COME_SHORT,
                );
            }
            if let Some(target) = &player.hold_width {
                append_run_candidate(
                    &mut candidates,
                    player.player_index,
                    RunType::HoldWidth,
                    target,
                    RUN_LABEL_HOLD_WIDTH,
                );
            }
        }
    }
    sort_run_candidates(&mut candidates);
    candidates
}

/// Reconcile active run slots against fresh candidates: unexpired slots are
/// kept (earliest grants win when a lowered cap can't preserve them all),
/// then remaining capacity is filled from the ranked candidate list.
#[must_use]
pub fn grant_runs(
    candidates: &[RunCandidate],
    active: &[RunSlot],
    maximum: u32,
    now: f64,
) -> Vec<RunSlot> {
    assert!(now.is_finite(), "run grant time must be finite");

    let mut kept: Vec<RunSlot> = Vec::new();
    let mut assigned: Vec<u32> = Vec::new();
    for (index, slot) in active.iter().enumerate() {
        let label = format!("active run slot {}", index + 1);
        assert_positive_index(slot.player_index, &format!("{label} player"));
        assert!(
            !assigned.contains(&slot.player_index),
            "active run slot players must be unique"
        );
        assigned.push(slot.player_index);
        assert!(slot.score.is_finite(), "{label} score must be finite");
        assert!(slot.target_x.is_finite(), "{label} target x must be finite");
        assert!(slot.target_y.is_finite(), "{label} target y must be finite");
        assert!(
            slot.granted_at.is_finite(),
            "{label} grant time must be finite"
        );
        assert!(slot.expires_at.is_finite(), "{label} expiry must be finite");
        assert!(
            slot.expires_at >= slot.granted_at,
            "{label} expires before it was granted"
        );
        if slot.expires_at > now {
            kept.push(*slot);
        }
    }
    kept.sort_by(|a, b| {
        if a.granted_at != b.granted_at {
            a.granted_at
                .partial_cmp(&b.granted_at)
                .expect("granted_at must be comparable")
        } else if a.player_index != b.player_index {
            a.player_index.cmp(&b.player_index)
        } else {
            a.run_type.cmp(&b.run_type)
        }
    });
    // A lowered cap cannot preserve every prior slot. Earlier grants remain
    // authoritative and later grants yield deterministically.
    while kept.len() as u32 > maximum {
        kept.pop();
    }

    let mut assigned: Vec<u32> = kept.iter().map(|s| s.player_index).collect();

    let mut ranked: Vec<RunCandidate> = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        assert_positive_index(
            candidate.player_index,
            &format!("run candidate {} player", index + 1),
        );
        assert!(
            candidate.score.is_finite(),
            "run candidate score must be finite"
        );
        assert!(
            candidate.target_x.is_finite(),
            "run candidate target x must be finite"
        );
        assert!(
            candidate.target_y.is_finite(),
            "run candidate target y must be finite"
        );
        assert!(
            candidate.duration.is_finite(),
            "run candidate duration must be finite"
        );
        assert!(
            candidate.duration > 0.0,
            "run candidate duration must be positive"
        );
        ranked.push(*candidate);
    }
    sort_run_candidates(&mut ranked);

    for candidate in &ranked {
        if kept.len() as u32 >= maximum {
            break;
        }
        if !assigned.contains(&candidate.player_index) {
            kept.push(RunSlot {
                player_index: candidate.player_index,
                run_type: candidate.run_type,
                score: candidate.score,
                target_x: candidate.target_x,
                target_y: candidate.target_y,
                granted_at: now,
                expires_at: now + candidate.duration,
            });
            assigned.push(candidate.player_index);
        }
    }
    kept
}

/// Resolve the press mode and its reason, in a stable trigger-precedence
/// order.
#[must_use]
pub fn press_mode(context: &BrainPressContext) -> (PressMode, PressReason) {
    if context.heavy_touch {
        return (PressMode::Commit, PressReason::HeavyTouch);
    }
    if context.exposed_ball {
        return (PressMode::Commit, PressReason::ExposedBall);
    }
    if context.cover_available {
        return (PressMode::Commit, PressReason::Cover);
    }
    if context.box_desperation {
        return (PressMode::Commit, PressReason::BoxDesperation);
    }
    let discipline = clamp_unit(context.press_discipline);
    let threshold = clamp_unit(context.low_discipline_threshold);
    if discipline < threshold {
        return (PressMode::Commit, PressReason::LowDiscipline);
    }
    (PressMode::Contain, PressReason::NoTrigger)
}

fn option_order(a: &BrainScoredOption, b: &BrainScoredOption) -> Ordering {
    a.kind.cmp(&b.kind).then_with(|| a.id.cmp(&b.id))
}

fn canonical_options(options: &[BrainScoredOption]) -> Vec<BrainScoredOption> {
    let mut ordered: Vec<BrainScoredOption> = Vec::with_capacity(options.len());
    let mut ids_by_kind: IndexMap<String, Vec<String>> = IndexMap::new();
    for (index, option) in options.iter().enumerate() {
        assert!(!option.id.is_empty(), "option {} needs an id", index + 1);
        assert!(!option.kind.is_empty(), "option {} needs a kind", index + 1);
        assert!(
            option.score.is_finite(),
            "option {} score must be finite",
            index + 1
        );
        let kind_ids = ids_by_kind.entry(option.kind.clone()).or_default();
        assert!(
            !kind_ids.contains(&option.id),
            "option kind and id pairs must be unique"
        );
        kind_ids.push(option.id.clone());
        ordered.push(option.clone());
    }
    assert!(
        !ordered.is_empty(),
        "scored option selection needs at least one option"
    );
    ordered.sort_by(option_order);
    ordered
}

fn assert_rng_state(rng_state: u32) {
    assert_eq!(
        rng::seed(f64::from(rng_state)),
        rng_state,
        "option RNG state must be a canonical positive integer"
    );
}

/// Seeded softmax selection over a canonical option order. At zero
/// temperature, exact argmax is deterministic and consumes no RNG.
#[must_use]
pub fn select_scored_option(
    options: &[BrainScoredOption],
    temperature: f64,
    rng_state: u32,
) -> (BrainScoredOption, u32) {
    let ordered = canonical_options(options);
    assert_rng_state(rng_state);
    let effective_temperature = if temperature.is_nan() || temperature < 0.0 {
        0.0
    } else {
        temperature
    };

    let mut maximum = ordered[0].score;
    for option in &ordered[1..] {
        maximum = maximum.max(option.score);
    }
    if effective_temperature == 0.0 {
        for option in &ordered {
            if option.score == maximum {
                return (option.clone(), rng_state);
            }
        }
    }

    let mut weights: Vec<f64> = Vec::with_capacity(ordered.len());
    let mut total = 0.0_f64;
    for option in &ordered {
        let weight = if effective_temperature == f64::INFINITY {
            1.0
        } else {
            deterministic_math::exp((option.score - maximum) / effective_temperature)
        };
        weights.push(weight);
        total += weight;
    }
    let (next_state, sample) = rng::roll(rng_state);
    let target = sample * total;
    let mut cumulative = 0.0_f64;
    for (index, option) in ordered.iter().enumerate() {
        cumulative += weights[index];
        if target < cumulative {
            return (option.clone(), next_state);
        }
    }
    (ordered[ordered.len() - 1].clone(), next_state)
}

/// Composure sharpens ordinary selection while pressure increases
/// temperature only in proportion to missing composure.
#[must_use]
pub fn decide_carrier(
    options: &[CarrierOption],
    composure: f64,
    pressure: f64,
    base_temperature: f64,
    rng_state: u32,
) -> (CarrierOption, u32) {
    let calm = clamp_unit(composure);
    let danger = clamp_unit(pressure);
    let base = if base_temperature.is_nan() || base_temperature < 0.0 {
        0.0
    } else {
        base_temperature
    };
    let temperature = if calm == 1.0 {
        0.0
    } else {
        base * (1.0 - calm) * (1.0 + danger)
    };
    select_scored_option(options, temperature, rng_state)
}
