//! Port of `sim/lever_metrics.lua`.
//!
//! Match-grain liveness for manager levers. Each comparison runs fixture A
//! and fixture B on the same seeds (common random numbers), then reports
//! effect sizes: home win-rate percentage points and mean shifts normalized
//! by each match metric's good-band width. No standard-error threshold is
//! used.
//!
//! This is only half 1 of manager-mode's headless-first ship-gate. Passing
//! here means a lever is perceptible, not that it is a decision: a lever is
//! ships-eligible only after decision_contingency (#0005) passes as well.
//!
//! ## Band table duplication (README §5.1 precedent)
//!
//! [`crate::headless`]'s `band_for` already duplicates `crate::metrics`'s
//! private `BAND_*` constants for the same reason documented there: this
//! module does not own `metrics.rs` and cannot make them `pub`. [`band_width`]
//! duplicates the same eight good-band widths a second time; both
//! duplicates are display/analysis-layer reads of authored balance
//! constants, not game content (AGENTS.md §8 is about content, not these).

use crate::headless::{self, BatchOpts, BatchResult, HeadlessBot, Winner};
use crate::metrics::MatchMetrics;
use gc_data::players::PlayerData;
use gc_data::tactics::TacticData;
use gc_data::teams::TeamData;
use gc_data::{tactics, teams, tuning_presets};
use indexmap::IndexMap;

/// Lower edge of the good |dWin| band, in percentage points.
pub const WIN_GOOD_LO: f64 = 3.0;
/// Upper edge of the good |dWin| band, in percentage points.
pub const WIN_GOOD_HI: f64 = 20.0;
/// Minimum |band-widths| shift for a metric to count as "moved".
pub const METRIC_MOVE_MIN: f64 = 0.5;

// Sorted (matches the Lua `table.sort(keys)` over `metrics.bands`'s keys)
// good-band widths for the eight metrics `sim/metrics.lua` bands. Values are
// `good_hi - good_lo` from `crate::metrics`'s private `BAND_*` constants.
const BANDED_KEYS: &[&str] = &[
    "decided_late",
    "goals_total",
    "longest_drought_s",
    "pass_completion",
    "possession_balance",
    "save_rate",
    "shots_per_goal",
    "turnovers_per_min",
];

fn band_width(key: &str) -> Option<f64> {
    Some(match key {
        "decided_late" => 1.0 - 0.4,
        "goals_total" => 5.0 - 2.0,
        "longest_drought_s" => 35.0 - 0.0,
        "pass_completion" => 0.85 - 0.55,
        "possession_balance" => 0.65 - 0.35,
        "save_rate" => 0.75 - 0.45,
        "shots_per_goal" => 6.0 - 2.5,
        "turnovers_per_min" => 5.0 - 1.0,
        _ => return None,
    })
}

/// Read the observed value for one of [`BANDED_KEYS`] off a finished
/// match's metrics. `None` mirrors the Lua `nil` a rate metric has when its
/// denominator did not occur.
fn banded_value(m: &MatchMetrics, key: &str) -> Option<f64> {
    match key {
        "decided_late" => Some(m.decided_late),
        "goals_total" => Some(m.goals_total as f64),
        "longest_drought_s" => Some(m.longest_drought_s),
        "pass_completion" => m.pass_completion,
        "possession_balance" => m.possession_balance,
        "save_rate" => m.save_rate,
        "shots_per_goal" => m.shots_per_goal,
        "turnovers_per_min" => Some(m.turnovers_per_min),
        _ => None,
    }
}

/// One banded metric's paired-seed comparison between fixture A and B.
#[derive(Clone, Debug, PartialEq)]
pub struct LeverMetricDelta {
    /// The metric name (one of [`BANDED_KEYS`]).
    pub key: &'static str,
    /// Seeds with finite observations for both A and B.
    pub n: i64,
    /// Fixture A's mean over the paired seeds.
    pub mean_a: f64,
    /// Fixture B's mean over the paired seeds.
    pub mean_b: f64,
    /// Signed raw mean shift, A - B.
    pub delta: f64,
    /// `good_hi - good_lo` for this metric.
    pub band_width: f64,
    /// Signed normalized shift, `delta / band_width`.
    pub band_widths: f64,
}

/// One lever comparison's outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct LeverLivenessResult {
    /// Seed-set size the comparison ran on.
    pub seeds: i64,
    /// Home-win share for fixture A, `0..1`.
    pub win_rate_a: f64,
    /// Home-win share for fixture B, `0..1`.
    pub win_rate_b: f64,
    /// Signed percentage points, A home wins - B home wins.
    pub dwin_pts: f64,
    /// Whether `|dwin_pts|` is in the 3..20 pp good band.
    pub win_in_band: bool,
    /// Every banded metric's comparison, in [`BANDED_KEYS`] order.
    pub metric_deltas: Vec<LeverMetricDelta>,
    /// The subset of `metric_deltas` with `|band_widths| >= METRIC_MOVE_MIN`.
    pub moved_metrics: Vec<LeverMetricDelta>,
    /// `win_in_band` AND at least one moved metric.
    pub passes: bool,
}

fn home_win_rate(batch: &BatchResult) -> f64 {
    let wins = batch
        .matches
        .iter()
        .filter(|m| m.winner == Some(Winner::Home))
        .count();
    wins as f64 / batch.matches.len() as f64
}

fn metric_deltas(a: &BatchResult, b: &BatchResult) -> Vec<LeverMetricDelta> {
    let mut deltas = Vec::new();
    for &key in BANDED_KEYS {
        let Some(width) = band_width(key) else {
            continue;
        };
        if !(width.is_finite() && width > 0.0) {
            continue;
        }
        let mut n: i64 = 0;
        let mut sum_a = 0.0;
        let mut sum_b = 0.0;
        for i in 0..a.matches.len() {
            let value_a = banded_value(&a.matches[i].metrics, key);
            let value_b = banded_value(&b.matches[i].metrics, key);
            // Rate metrics can be `None` when their denominator did not
            // occur. Keep the common-seed pairing honest by admitting a
            // seed only when both fixture observations are valid.
            if let (Some(va), Some(vb)) = (value_a, value_b)
                && va.is_finite()
                && vb.is_finite()
            {
                n += 1;
                sum_a += va;
                sum_b += vb;
            }
        }
        if n > 0 {
            let mean_a = sum_a / n as f64;
            let mean_b = sum_b / n as f64;
            let delta = mean_a - mean_b;
            deltas.push(LeverMetricDelta {
                key,
                n,
                mean_a,
                mean_b,
                delta,
                band_width: width,
                band_widths: delta / width,
            });
        }
    }
    deltas
}

fn with_seeds<'a>(opts: &BatchOpts<'a>, seeds: &'a [f64]) -> BatchOpts<'a> {
    BatchOpts {
        n: opts.n,
        seeds: Some(seeds),
        duration: opts.duration,
        max_goals: opts.max_goals,
        reaction: opts.reaction,
        tuning_blob: opts.tuning_blob,
        home: opts.home,
        away: opts.away,
        home_formation: opts.home_formation,
        away_formation: opts.away_formation,
        tactic: opts.tactic,
        away_tactic: opts.away_tactic,
        players_by_id: opts.players_by_id,
        species_by_id: opts.species_by_id,
        field: opts.field,
        bot: opts.bot,
        frames: opts.frames,
        slot_sources: opts.slot_sources,
    }
}

/// Compare two alternatives applied to the home side. The sign is always
/// fixture A minus fixture B: draws count as no home win, so `dwin_pts` is
/// `100 * (share of seeds A's home side wins - share B's home side wins)`.
/// Liveness is direction-agnostic and gates on `|dwin_pts|`.
///
/// # Panics
///
/// Panics if `seeds` is empty, or if the two fixtures produced batches of
/// different sizes (a programmer error — both must run the same seed set).
#[must_use]
pub fn lever_liveness<'a>(
    fixture_a: &BatchOpts<'a>,
    fixture_b: &BatchOpts<'a>,
    seeds: &'a [f64],
) -> LeverLivenessResult {
    assert!(!seeds.is_empty(), "lever_liveness needs at least one seed");
    let a = headless::run_batch(&with_seeds(fixture_a, seeds));
    let b = headless::run_batch(&with_seeds(fixture_b, seeds));
    assert_eq!(
        a.matches.len(),
        b.matches.len(),
        "paired fixtures produced different batch sizes"
    );

    let win_rate_a = home_win_rate(&a);
    let win_rate_b = home_win_rate(&b);
    let dwin_pts = (win_rate_a - win_rate_b) * 100.0;
    let win_magnitude = dwin_pts.abs();
    let win_in_band = (WIN_GOOD_LO..=WIN_GOOD_HI).contains(&win_magnitude);
    let deltas = metric_deltas(&a, &b);
    let moved: Vec<LeverMetricDelta> = deltas
        .iter()
        .filter(|d| d.band_widths.abs() >= METRIC_MOVE_MIN)
        .cloned()
        .collect();
    let passes = win_in_band && !moved.is_empty();

    LeverLivenessResult {
        seeds: seeds.len() as i64,
        win_rate_a,
        win_rate_b,
        dwin_pts,
        win_in_band,
        moved_metrics: moved,
        passes,
        metric_deltas: deltas,
    }
}

/// One built-in lever's two owned fixture configurations. Owned by value
/// (every field is `'static`-content and `Copy`) so a [`LeverDefinition`]
/// never borrows from the frame that built it — [`built_ins`] can return a
/// `Vec` of these directly. A [`headless::BatchOpts`] is built from a
/// fixture just-in-time, at each call site that needs one.
#[derive(Clone, Debug, PartialEq)]
pub struct LeverFixture {
    /// Home team.
    pub home: TeamData,
    /// Away team.
    pub away: TeamData,
    /// Home tactic.
    pub tactic: TacticData,
    /// Away tactic.
    pub away_tactic: TacticData,
    /// Override for the home team's formation.
    pub home_formation: Option<&'static str>,
    /// Knob overrides in `sim::tuning::Tuning::serialize` format.
    pub tuning_blob: &'static str,
    /// Human-proxy mode.
    pub bot: HeadlessBot,
}

fn to_batch_opts<'a>(
    f: &'a LeverFixture,
    players: &'a IndexMap<&'static str, PlayerData>,
) -> BatchOpts<'a> {
    BatchOpts {
        home: Some(&f.home),
        away: Some(&f.away),
        tactic: Some(&f.tactic),
        away_tactic: Some(&f.away_tactic),
        home_formation: f.home_formation,
        tuning_blob: Some(f.tuning_blob),
        bot: Some(f.bot),
        players_by_id: Some(players),
        ..Default::default()
    }
}

/// A manager lever: two named alternatives, each an owned [`LeverFixture`].
#[derive(Clone, Debug, PartialEq)]
pub struct LeverDefinition {
    /// Persistent identity.
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Fixture A's display label.
    pub option_a: &'static str,
    /// Fixture B's display label.
    pub option_b: &'static str,
    /// Fixture A.
    pub fixture_a: LeverFixture,
    /// Fixture B.
    pub fixture_b: LeverFixture,
}

fn players_by_id_map() -> IndexMap<&'static str, PlayerData> {
    gc_data::players::ALL.iter().map(|p| (p.id, *p)).collect()
}

fn nebula_with_roster(roster: &'static [&'static str]) -> TeamData {
    let nebula = teams::get("nebula").expect("nebula is an authored team");
    TeamData {
        id: nebula.id,
        name: nebula.name,
        color: nebula.color,
        formation: nebula.formation,
        roster,
        squad: None,
    }
}

/// The built-in lever set: three comparisons run against tuning preset
/// `candidate_a` (defaults have a broken scoring signature, so a
/// dead-looking lever there would not be valid liveness evidence).
#[must_use]
pub fn built_ins() -> (&'static str, Vec<LeverDefinition>) {
    let preset =
        tuning_presets::get("candidate_a").expect("candidate_a is an authored tuning preset");
    let nebula = *teams::get("nebula").expect("nebula is an authored team");
    let orion = *teams::get("orion").expect("orion is an authored team");
    let balanced = *tactics::get("balanced").expect("balanced is an authored tactic");
    let press_high = *tactics::get("press_high").expect("press_high is an authored tactic");
    let counter = *tactics::get("counter").expect("counter is an authored tactic");

    let base = LeverFixture {
        home: nebula,
        away: orion,
        tactic: balanced,
        away_tactic: balanced,
        home_formation: None,
        tuning_blob: preset.blob,
        bot: HeadlessBot::None,
    };
    let star_in = nebula_with_roster(&["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"]);
    let star_benched = nebula_with_roster(&["ozzo", "brakka", "veil_nyx", "rok_tann", "mika_olu"]);

    let levers = vec![
        LeverDefinition {
            id: "formation",
            name: "Formation",
            option_a: "2-1-1 Balanced",
            option_b: "1-1-2 Aggressive",
            fixture_a: LeverFixture {
                home_formation: Some("2-1-1"),
                ..base.clone()
            },
            fixture_b: LeverFixture {
                home_formation: Some("1-1-2"),
                ..base.clone()
            },
        },
        LeverDefinition {
            id: "tactic",
            name: "Tactic",
            option_a: "Press High",
            option_b: "Counter Attack",
            fixture_a: LeverFixture {
                tactic: press_high,
                ..base.clone()
            },
            fixture_b: LeverFixture {
                tactic: counter,
                ..base.clone()
            },
        },
        LeverDefinition {
            id: "star_swap",
            name: "Star swap",
            option_a: "Zyro Vex starts",
            option_b: "Mika Olu starts",
            fixture_a: LeverFixture {
                home: star_in,
                ..base.clone()
            },
            fixture_b: LeverFixture {
                home: star_benched,
                ..base.clone()
            },
        },
    ];
    (preset.name, levers)
}

/// One built-in lever's outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct LeverRun {
    /// The lever that was compared.
    pub lever: LeverDefinition,
    /// Its liveness result.
    pub result: LeverLivenessResult,
}

/// Run every built-in lever ([`built_ins`]) over `seeds`, optionally
/// reporting progress through `log`.
#[must_use]
pub fn run_built_ins(
    seeds: &[f64],
    mut log: Option<&mut dyn FnMut(&str)>,
) -> (&'static str, Vec<LeverRun>) {
    let (config_name, levers) = built_ins();
    let players = players_by_id_map();
    let total = levers.len();
    let mut runs = Vec::with_capacity(total);
    for (i, lever) in levers.into_iter().enumerate() {
        if let Some(log) = log.as_deref_mut() {
            log(&format!("levers: {}/{} {}", i + 1, total, lever.name));
        }
        let result = lever_liveness(
            &to_batch_opts(&lever.fixture_a, &players),
            &to_batch_opts(&lever.fixture_b, &players),
            seeds,
        );
        runs.push(LeverRun { lever, result });
    }
    (config_name, runs)
}

/// Render a human-readable liveness report for a batch of lever runs.
#[must_use]
pub fn report(config_name: &str, runs: &[LeverRun]) -> String {
    let mut out = vec![
        format!("lever liveness — {config_name}; AI/AI; paired common seeds"),
        "dWin is home win-rate percentage points (A - B); the gate uses |dWin| 3..20 pp."
            .to_string(),
        "PASS here is only ship-gate half 1; decision_contingency (#0005) is still required."
            .to_string(),
        format!(
            "{:<12} {:<21} {:<21} {:>9} {:>17} {:>6}",
            "lever", "A", "B", "dWin pp", "moved (band-w)", "gate"
        ),
    ];
    for run in runs {
        let moved: Vec<String> = run
            .result
            .moved_metrics
            .iter()
            .map(|d| format!("{} {:+.2}", d.key, d.band_widths))
            .collect();
        let moved_str = if moved.is_empty() {
            "none".to_string()
        } else {
            moved.join(",")
        };
        out.push(format!(
            "{:<12} {:<21} {:<21} {:>+9.1} {:>17} {:>6}",
            run.lever.name,
            run.lever.option_a,
            run.lever.option_b,
            run.result.dwin_pts,
            moved_str,
            if run.result.passes { "PASS" } else { "FAIL" }
        ));
        let all_deltas: Vec<String> = run
            .result
            .metric_deltas
            .iter()
            .map(|d| format!("{}={:+.2}(n={})", d.key, d.band_widths, d.n))
            .collect();
        out.push(format!(
            "  home win rates A/B: {:.1}% / {:.1}%; all band-width deltas A-B: {}",
            run.result.win_rate_a * 100.0,
            run.result.win_rate_b * 100.0,
            all_deltas.join(", ")
        ));
    }
    out.join("\n")
}
