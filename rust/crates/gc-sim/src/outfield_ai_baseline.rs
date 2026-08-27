//! Frozen combat-disabled Outfield AI common-seed baseline (#59).
//!
//! What this is: a checked-in, versioned recording of how the frozen
//! gameplay-AI policy ([`crate::outfield_ai_policy`]) plays a declared
//! fixture over a declared seed set, with full identity so later work can
//! cite it instead of copying it. It is the fixture-A control every
//! "combat changed X" claim is measured against; without it such a claim is
//! unfalsifiable.
//!
//! What this is NOT: a tolerance band over human-proxy play. This tree used
//! to carry one alongside this artifact — the soccer fun tripwire,
//! `gc_sim::tripwire` plus `gc_data::fun_baseline`, a 30-seed bot-driven
//! smoke test with a 5% band — and #630 deleted it: nothing ever called it,
//! and its frozen values described a 960×540 pitch and thirty drift-log
//! entries of sim change ago. This artifact was separate from it on purpose
//! and is now the only frozen balance control: declared seeds, a declared
//! fixture, all-AI sides rather than a bot in one slot, and an EXACT
//! comparison — see [`compare`]. Relative per-knob claims are
//! [`crate::knob_contract`]'s job; it measures its own noise floor on the
//! caller's seed set instead of assuming a band, which is what a human-proxy
//! instrument can honestly support.
//!
//! What blocks. Only a moved tracked metric fails the check
//! ([`OutfieldAiBaselineComparison::metrics_ok`]). Identity covers more than
//! play does — all 40 knob defaults and every authored roster, formation and
//! tactic — so it moves for edits that provably cannot change combat-disabled
//! play. Those are reported loudly as STALE
//! ([`OutfieldAiBaselineComparison::stale`]) and still owe a drift-log entry
//! and a re-freeze, but they do not fail a shared gate.
//!
//! Comparison is EXACT. The batch is deterministic per seed, values
//! round-trip through Rust's shortest-round-trip float formatting (see
//! [`serialize`]), and this is a frozen control rather than a drift band, so
//! any movement is a real finding.
//!
//! Pure module: no I/O. The caller decides where the report/serialized text
//! go.
//!
//! ## Two record types, one shape
//!
//! `data::outfield_ai_baseline::OutfieldAiBaselineRecord`
//! (`gc_data::outfield_ai_baseline`) is a `'static`, compile-time-frozen
//! literal — its string fields are `&'static str`. A freshly [`measure`]d
//! record computes its strings at runtime, so it cannot reuse that exact
//! type; [`OutfieldAiBaselineRecord`] here is the same shape with owned
//! `String` fields instead. [`OutfieldAiBaselineRecord::from`] converts the
//! frozen record into this shape so [`compare`] can treat "the frozen
//! control" and "what this build just measured" uniformly.

use crate::headless::{self, HeadlessBot};
use crate::match_snapshot::PitchSize;
use crate::metrics::MetricStats;
use crate::{fixed_clock, input_frame, match_snapshot, outfield_ai_policy, tuning};
use gc_core::fnv1a64;
use gc_data::formations::{self, FormationRole};
use gc_data::players::{self, Position};
use gc_data::showcase_player_compatibility;
use gc_data::species;
use gc_data::tactics::{self, MarkingScheme};
use gc_data::teams;
use indexmap::IndexMap;

/// Baseline record schema name.
pub const SCHEMA: &str = "outfield_ai_baseline";
/// Baseline record schema version.
pub const SCHEMA_VERSION: i64 = 1;
/// Fixture name.
pub const FIXTURE: &str = "combat_disabled_control_a";

/// First seed of the locked paired calibration/common-seed block from the
/// accepted evidence contract (`docs/design/combat_fun_evidence_contract.md`
/// §3.3). Combat-active arms run their own fixture on these same seeds, so
/// this control is a paired control under common random numbers rather than
/// an independent sample. Seeds 1..30 — the retired soccer tripwire's block,
/// spent whether or not the tripwire still exists — and the historical
/// evaluation seeds 1001..1060 stay out of it.
pub const SEED_FIRST: i64 = 20001;
/// Number of seeds in the declared block.
pub const SEED_COUNT: i64 = 60;

/// Match duration, in seconds.
pub const DURATION_SECONDS: f64 = 120.0;
/// Goal cap.
pub const MAX_GOALS: i64 = 3;
/// Fixture pitch size.
pub const FIELD: PitchSize = PitchSize {
    w: 1648.0,
    h: 927.0,
};

/// All-AI sides. The human-proxy bot in [`crate::bot`] is a separate policy
/// with its own weaknesses; mixing it in would make this a baseline of the
/// proxy, not of the gameplay AI.
pub const BOT: &str = "none";

/// Tracked metrics: the soccer-integrity family the combat calibration must
/// not damage, plus the AI dribble diagnostics. Ordered; the order is
/// hashed.
pub const TRACKED: &[&str] = &[
    "fun",
    "goals_total",
    "goals_home",
    "goals_away",
    "shots",
    "shots_per_goal",
    "save_rate",
    "passes",
    "pass_completion",
    "turnovers_per_min",
    "possession_balance",
    "longest_drought_s",
    "decided_late",
    "lead_changes",
    "margin",
    "duration",
    "ai_dribble_carry_s",
    "ai_dribble_close_share",
    "ai_dribble_sprint_share",
    "ai_dribble_juke_share",
    "ai_dribble_touches_per_min",
    "ai_dribble_heavy_losses_per_min",
    "ai_jukes",
];

/// Per-metric fields, in hash and comparison order.
pub const STAT_FIELDS: &[&str] = &["n", "mean", "sd", "min", "max"];

/// Identity fields compared field by field, in report order.
pub const IDENTITY_FIELDS: &[&str] = &[
    "schema",
    "schema_version",
    "policy_id",
    "fixture",
    "config",
    "config_hash",
    "content_hash",
    "tuning_hash",
    "snapshot_version",
    "input_version",
    "tick_rate",
    "seed_first",
    "seed_count",
    "seed_hash",
    "fixture_hash",
];

const STAT_KEYS: &[&str] = &["pace", "strength", "technique", "stamina", "mental"];

fn append_num(parts: &mut String, value: f64) {
    parts.push('n');
    parts.push_str(&match_snapshot::number_bytes(value));
    parts.push(';');
}

fn append_str(parts: &mut String, value: &str) {
    parts.push('s');
    parts.push_str(&value.len().to_string());
    parts.push(':');
    parts.push_str(value);
    parts.push(';');
}

fn position_wire(position: Position) -> &'static str {
    match position {
        Position::Keeper => "keeper",
        Position::Defender => "defender",
        Position::Midfielder => "midfielder",
        Position::Forward => "forward",
    }
}

fn formation_role_wire(role: FormationRole) -> &'static str {
    match role {
        FormationRole::Def => "def",
        FormationRole::Mid => "mid",
        FormationRole::Wide => "wide",
        FormationRole::Fwd => "fwd",
    }
}

fn marking_scheme_wire(scheme: MarkingScheme) -> &'static str {
    match scheme {
        MarkingScheme::Zonal => "zonal",
        MarkingScheme::Man => "man",
        MarkingScheme::Hybrid => "hybrid",
    }
}

/// The declared seed set, materialized.
#[must_use]
pub fn seeds() -> Vec<i64> {
    (0..SEED_COUNT).map(|i| SEED_FIRST + i).collect()
}

/// Everything about the run that is not the AI policy or the content.
#[must_use]
pub fn config() -> String {
    format!(
        "field={}x{};duration={};max_goals={};tick_rate={};bot={};combat={};tactic={}",
        FIELD.w as i64,
        FIELD.h as i64,
        DURATION_SECONDS as i64,
        MAX_GOALS,
        fixed_clock::TICK_RATE as i64,
        BOT,
        outfield_ai_policy::COMBAT_MODE,
        tactics::get("balanced")
            .expect("balanced is an authored tactic")
            .id
    )
}

/// Hash of the authored content the fixture actually instantiates: both
/// teams, every rostered player's mechanical stats, the species modifiers
/// applied to them, both formations' anchors, and the shared tactic. A
/// content edit that changes play therefore invalidates the baseline even
/// though the AI policy is untouched — which is the honest outcome, since
/// the recorded numbers moved.
#[must_use]
pub fn content_hash() -> String {
    let by_id: IndexMap<&str, &players::PlayerData> =
        players::ALL.iter().map(|p| (p.id, p)).collect();
    let mut parts = String::from("GCOAC;1;");
    let sides = [
        (
            "home",
            teams::get("nebula").expect("nebula is an authored team"),
        ),
        (
            "away",
            teams::get("orion").expect("orion is an authored team"),
        ),
    ];
    for (key, team) in sides {
        append_str(&mut parts, key);
        append_str(&mut parts, team.id);
        append_str(&mut parts, team.formation);
        append_num(&mut parts, team.roster.len() as f64);
        for &player_id in team.roster {
            let player = by_id
                .get(player_id)
                .unwrap_or_else(|| panic!("baseline roster names an unknown player: {player_id}"));
            append_str(&mut parts, player.id);
            append_num(&mut parts, player.number as f64);
            append_str(&mut parts, position_wire(player.position));
            let stat_values = [
                player.stats.pace,
                player.stats.strength,
                player.stats.technique,
                player.stats.stamina,
                player.stats.mental,
            ];
            for (name, value) in STAT_KEYS.iter().zip(stat_values.iter()) {
                append_str(&mut parts, name);
                append_num(&mut parts, *value as f64);
            }
            let species_id =
                showcase_player_compatibility::get(player_id).map_or("neutral", |c| c.species);
            let species_data = species::get(species_id).unwrap_or_else(|| {
                panic!("baseline player names an unknown species: {species_id}")
            });
            append_str(&mut parts, "species");
            append_str(&mut parts, species_data.id);
            let modifier_values = [
                species_data.modifiers.pace,
                species_data.modifiers.strength,
                species_data.modifiers.technique,
                species_data.modifiers.stamina,
                species_data.modifiers.mental,
            ];
            for value in modifier_values {
                append_num(&mut parts, value as f64);
            }
        }
        let formation = formations::get(team.formation).unwrap_or_else(|| {
            panic!(
                "baseline team names an unknown formation: {}",
                team.formation
            )
        });
        append_str(&mut parts, formation.id);
        append_num(&mut parts, formation.keeper.x);
        append_num(&mut parts, formation.keeper.y);
        append_num(&mut parts, formation.outfield.len() as f64);
        for anchor in formation.outfield {
            append_str(&mut parts, formation_role_wire(anchor.role));
            append_num(&mut parts, anchor.x);
            append_num(&mut parts, anchor.y);
        }
    }
    let tactic = tactics::get("balanced").expect("balanced is an authored tactic");
    append_str(&mut parts, tactic.id);
    append_num(&mut parts, tactic.press as f64);
    append_num(&mut parts, tactic.line_shift);
    append_num(&mut parts, tactic.stamina_drain);
    append_str(&mut parts, marking_scheme_wire(tactic.marking.scheme));
    append_num(&mut parts, tactic.marking.man_marks as f64);
    append_num(&mut parts, tactic.marking.standoff);
    append_num(&mut parts, tactic.marking.compactness);
    append_num(&mut parts, tactic.marking.support);
    append_num(&mut parts, tactic.transition.counterpress);
    append_num(&mut parts, tactic.transition.counterattack);
    fnv1a64::hash(parts.as_bytes())
}

/// Hash over every shipped knob default, including the ones the policy id
/// deliberately excludes. Movement and dribble defaults are world rules
/// rather than AI decisions, but they still move the recorded numbers.
#[must_use]
pub fn tuning_hash() -> String {
    let mut parts = String::from("GCOAT;1;");
    for knob in tuning::KNOBS.iter() {
        append_str(&mut parts, knob.key);
        append_num(&mut parts, knob.default);
    }
    fnv1a64::hash(parts.as_bytes())
}

/// Everything about a recorded run that is not the AI policy or the content.
///
/// Same shape as `gc_data::outfield_ai_baseline::OutfieldAiBaselineIdentity`,
/// but with owned `String` fields — see the module doc's "Two record types"
/// section.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldAiBaselineIdentity {
    /// Baseline record schema name.
    pub schema: String,
    /// Baseline record schema version.
    pub schema_version: i64,
    /// Frozen AI policy id.
    pub policy_id: String,
    /// Fixture name.
    pub fixture: String,
    /// Hash of the fixture's declared shape.
    pub fixture_hash: String,
    /// Everything about the run that is not the AI policy or the content.
    pub config: String,
    /// Hash of `config`.
    pub config_hash: String,
    /// Hash of the authored content the fixture instantiates.
    pub content_hash: String,
    /// Hash of the tuning knobs in effect.
    pub tuning_hash: String,
    /// Match snapshot format version.
    pub snapshot_version: i64,
    /// Input frame format version.
    pub input_version: i64,
    /// Simulation tick rate.
    pub tick_rate: i64,
    /// First seed in the declared seed set.
    pub seed_first: i64,
    /// Count of seeds in the declared seed set.
    pub seed_count: i64,
    /// Hash over the exact seed list, not just first/count.
    pub seed_hash: String,
}

/// A frozen combat-disabled Outfield AI baseline recording, with owned
/// identity strings — see the module doc.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldAiBaselineRecord {
    /// Bumped by every deliberate re-freeze.
    pub baseline_version: i64,
    /// Everything about the recorded run that is not the AI policy or the
    /// content.
    pub identity: OutfieldAiBaselineIdentity,
    /// Per-metric summary statistics.
    pub stats: gc_data::outfield_ai_baseline::OutfieldAiBaselineStats,
    /// Hash over identity + stats; excludes `baseline_version`.
    pub signature: String,
}

impl From<&gc_data::outfield_ai_baseline::OutfieldAiBaselineRecord> for OutfieldAiBaselineRecord {
    fn from(r: &gc_data::outfield_ai_baseline::OutfieldAiBaselineRecord) -> Self {
        OutfieldAiBaselineRecord {
            baseline_version: r.baseline_version,
            identity: OutfieldAiBaselineIdentity {
                schema: r.identity.schema.to_string(),
                schema_version: r.identity.schema_version,
                policy_id: r.identity.policy_id.to_string(),
                fixture: r.identity.fixture.to_string(),
                fixture_hash: r.identity.fixture_hash.to_string(),
                config: r.identity.config.to_string(),
                config_hash: r.identity.config_hash.to_string(),
                content_hash: r.identity.content_hash.to_string(),
                tuning_hash: r.identity.tuning_hash.to_string(),
                snapshot_version: r.identity.snapshot_version,
                input_version: r.identity.input_version,
                tick_rate: r.identity.tick_rate,
                seed_first: r.identity.seed_first,
                seed_count: r.identity.seed_count,
                seed_hash: r.identity.seed_hash.to_string(),
            },
            stats: r.stats,
            signature: r.signature.to_string(),
        }
    }
}

/// The complete citable identity of the fixture, resolved from live
/// modules. `seeds` exists so a cheap probe run records what it ACTUALLY
/// ran: its identity then differs from the frozen 60-seed one and can never
/// be mistaken for the freeze.
///
/// # Panics
///
/// Panics if `seeds` is `Some(&[])` (a baseline needs at least one seed).
#[must_use]
pub fn identity(seeds: Option<&[i64]>) -> OutfieldAiBaselineIdentity {
    let default_seeds;
    let seed_list: &[i64] = match seeds {
        Some(s) => s,
        None => {
            default_seeds = self::seeds();
            &default_seeds
        }
    };
    assert!(!seed_list.is_empty(), "a baseline needs at least one seed");
    let mut seed_parts = String::from("GCOAS;1;");
    for &seed in seed_list {
        append_num(&mut seed_parts, seed as f64);
    }
    let config = config();
    let mut identity = OutfieldAiBaselineIdentity {
        schema: SCHEMA.to_string(),
        schema_version: SCHEMA_VERSION,
        policy_id: outfield_ai_policy::id(),
        fixture: FIXTURE.to_string(),
        fixture_hash: String::new(),
        config_hash: fnv1a64::hash(config.as_bytes()),
        config,
        content_hash: content_hash(),
        tuning_hash: tuning_hash(),
        snapshot_version: match_snapshot::VERSION,
        input_version: input_frame::VERSION,
        tick_rate: fixed_clock::TICK_RATE as i64,
        seed_first: seed_list[0],
        seed_count: seed_list.len() as i64,
        seed_hash: fnv1a64::hash(seed_parts.as_bytes()),
    };
    let mut parts = String::from("GCOAF;1;");
    append_str(&mut parts, &identity.policy_id);
    append_str(&mut parts, &identity.fixture);
    append_str(&mut parts, &identity.config_hash);
    append_str(&mut parts, &identity.content_hash);
    append_str(&mut parts, &identity.tuning_hash);
    append_num(&mut parts, identity.snapshot_version as f64);
    append_num(&mut parts, identity.input_version as f64);
    append_str(&mut parts, &identity.seed_hash);
    identity.fixture_hash = fnv1a64::hash(parts.as_bytes());
    identity
}

fn stat_field(
    stats: &gc_data::outfield_ai_baseline::OutfieldAiBaselineStats,
    key: &str,
) -> gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
    match key {
        "fun" => stats.fun,
        "goals_total" => stats.goals_total,
        "goals_home" => stats.goals_home,
        "goals_away" => stats.goals_away,
        "shots" => stats.shots,
        "shots_per_goal" => stats.shots_per_goal,
        "save_rate" => stats.save_rate,
        "passes" => stats.passes,
        "pass_completion" => stats.pass_completion,
        "turnovers_per_min" => stats.turnovers_per_min,
        "possession_balance" => stats.possession_balance,
        "longest_drought_s" => stats.longest_drought_s,
        "decided_late" => stats.decided_late,
        "lead_changes" => stats.lead_changes,
        "margin" => stats.margin,
        "duration" => stats.duration,
        "ai_dribble_carry_s" => stats.ai_dribble_carry_s,
        "ai_dribble_close_share" => stats.ai_dribble_close_share,
        "ai_dribble_sprint_share" => stats.ai_dribble_sprint_share,
        "ai_dribble_juke_share" => stats.ai_dribble_juke_share,
        "ai_dribble_touches_per_min" => stats.ai_dribble_touches_per_min,
        "ai_dribble_heavy_losses_per_min" => stats.ai_dribble_heavy_losses_per_min,
        "ai_jukes" => stats.ai_jukes,
        _ => panic!("unknown tracked outfield AI baseline metric: {key}"),
    }
}

fn stat_field_value(
    stat: &gc_data::outfield_ai_baseline::OutfieldAiBaselineStat,
    field: &str,
) -> f64 {
    match field {
        "n" => stat.n as f64,
        "mean" => stat.mean,
        "sd" => stat.sd,
        "min" => stat.min,
        "max" => stat.max,
        _ => panic!("unknown outfield AI baseline stat field: {field}"),
    }
}

/// Content hash of the evidence itself. Deliberately excludes
/// `baseline_version`, so a re-freeze that changes nothing shows up in git
/// as a lone version bump instead of hiding inside a churned file.
#[must_use]
pub fn signature(record: &OutfieldAiBaselineRecord) -> String {
    let mut parts = String::new();
    parts.push_str("GCOAB;");
    parts.push_str(&SCHEMA_VERSION.to_string());
    parts.push(';');
    let id = &record.identity;
    append_str(&mut parts, &id.schema);
    append_num(&mut parts, id.schema_version as f64);
    append_str(&mut parts, &id.policy_id);
    append_str(&mut parts, &id.fixture);
    append_str(&mut parts, &id.config);
    append_str(&mut parts, &id.config_hash);
    append_str(&mut parts, &id.content_hash);
    append_str(&mut parts, &id.tuning_hash);
    append_num(&mut parts, id.snapshot_version as f64);
    append_num(&mut parts, id.input_version as f64);
    append_num(&mut parts, id.tick_rate as f64);
    append_num(&mut parts, id.seed_first as f64);
    append_num(&mut parts, id.seed_count as f64);
    append_str(&mut parts, &id.seed_hash);
    append_str(&mut parts, &id.fixture_hash);
    for &key in TRACKED {
        let stat = stat_field(&record.stats, key);
        append_str(&mut parts, key);
        for &field in STAT_FIELDS {
            append_num(&mut parts, stat_field_value(&stat, field));
        }
    }
    fnv1a64::hash(parts.as_bytes())
}

/// [`measure`]'s options.
#[derive(Debug, Default)]
pub struct MeasureOpts<'a> {
    /// Overrides the recorded `baseline_version`; defaults to 1.
    pub baseline_version: Option<i64>,
    /// Probe override; the identity records what actually ran.
    pub seeds: Option<&'a [i64]>,
    /// Knob overrides applied on top of the defaults, in `sweep::parse_blob`'s
    /// `KEY=value` form. `None` means the empty blob — every knob at its
    /// default, which is what a freeze measures.
    ///
    /// Exposed so a caller can measure a *candidate* tuning against the frozen
    /// one, which is the tool's natural job and is also how the "detects a
    /// real policy change" test case works: a `static` frozen record can't
    /// have its knob defaults reassigned at runtime, but setting the same
    /// knob through the blob changes how the match is actually played in
    /// exactly the same way.
    pub tuning_blob: Option<&'a str>,
}

fn baseline_stat(
    agg: &IndexMap<&'static str, MetricStats>,
    key: &str,
) -> gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
    // A metric with no denominator in any match (shots per goal across
    // goalless matches) is absent from the aggregate. Record it as a
    // zero-support row rather than dropping it: `n` carries the fact, and
    // the schema stays the same width whatever the seeds produced.
    match agg.get(key) {
        Some(s) => gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
            n: s.n,
            mean: s.mean,
            sd: s.sd,
            min: s.min,
            max: s.max,
        },
        None => gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
            n: 0,
            mean: 0.0,
            sd: 0.0,
            min: 0.0,
            max: 0.0,
        },
    }
}

fn build_stats(
    agg: &IndexMap<&'static str, MetricStats>,
) -> gc_data::outfield_ai_baseline::OutfieldAiBaselineStats {
    gc_data::outfield_ai_baseline::OutfieldAiBaselineStats {
        fun: baseline_stat(agg, "fun"),
        goals_total: baseline_stat(agg, "goals_total"),
        goals_home: baseline_stat(agg, "goals_home"),
        goals_away: baseline_stat(agg, "goals_away"),
        shots: baseline_stat(agg, "shots"),
        shots_per_goal: baseline_stat(agg, "shots_per_goal"),
        save_rate: baseline_stat(agg, "save_rate"),
        passes: baseline_stat(agg, "passes"),
        pass_completion: baseline_stat(agg, "pass_completion"),
        turnovers_per_min: baseline_stat(agg, "turnovers_per_min"),
        possession_balance: baseline_stat(agg, "possession_balance"),
        longest_drought_s: baseline_stat(agg, "longest_drought_s"),
        decided_late: baseline_stat(agg, "decided_late"),
        lead_changes: baseline_stat(agg, "lead_changes"),
        margin: baseline_stat(agg, "margin"),
        duration: baseline_stat(agg, "duration"),
        ai_dribble_carry_s: baseline_stat(agg, "ai_dribble_carry_s"),
        ai_dribble_close_share: baseline_stat(agg, "ai_dribble_close_share"),
        ai_dribble_sprint_share: baseline_stat(agg, "ai_dribble_sprint_share"),
        ai_dribble_juke_share: baseline_stat(agg, "ai_dribble_juke_share"),
        ai_dribble_touches_per_min: baseline_stat(agg, "ai_dribble_touches_per_min"),
        ai_dribble_heavy_losses_per_min: baseline_stat(agg, "ai_dribble_heavy_losses_per_min"),
        ai_jukes: baseline_stat(agg, "ai_jukes"),
    }
}

/// Run the declared fixture over the declared seeds and record it.
#[must_use]
pub fn measure(opts: &MeasureOpts<'_>) -> OutfieldAiBaselineRecord {
    let default_seeds;
    let seed_list: &[i64] = match opts.seeds {
        Some(s) => s,
        None => {
            default_seeds = seeds();
            &default_seeds
        }
    };
    let seeds_f64: Vec<f64> = seed_list.iter().map(|&s| s as f64).collect();
    let batch = headless::run_batch(&headless::BatchOpts {
        seeds: Some(&seeds_f64),
        duration: Some(DURATION_SECONDS),
        max_goals: Some(MAX_GOALS),
        field: Some(FIELD),
        bot: Some(HeadlessBot::None),
        // Empty blob = every knob at its default, applied and restored per
        // match, so a stray in-process nudge cannot leak into the freeze.
        tuning_blob: Some(opts.tuning_blob.unwrap_or("")),
        ..Default::default()
    });
    let stats = build_stats(&batch.agg);
    let mut record = OutfieldAiBaselineRecord {
        baseline_version: opts.baseline_version.unwrap_or(1),
        identity: identity(Some(seed_list)),
        stats,
        signature: String::new(),
    };
    record.signature = signature(&record);
    record
}

/// One identity field's comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldAiBaselineIdentityRow {
    /// The identity field name (one of [`IDENTITY_FIELDS`]).
    pub key: &'static str,
    /// The frozen value, rendered for display.
    pub base: String,
    /// The measured value, rendered for display.
    pub cur: String,
    /// Whether `base == cur`.
    pub ok: bool,
}

/// One tracked metric's comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldAiBaselineMetricRow {
    /// The metric name (one of [`TRACKED`]).
    pub key: &'static str,
    /// The frozen mean.
    pub base: f64,
    /// The measured mean.
    pub cur: f64,
    /// `cur - base`.
    pub delta: f64,
    /// Stat fields that differ, e.g. `["mean", "sd"]`.
    pub moved: Vec<&'static str>,
    /// Whether `moved` is empty.
    pub ok: bool,
}

/// A frozen-vs-measured comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfieldAiBaselineComparison {
    /// Blocking: `false` only when a tracked metric actually moved.
    pub ok: bool,
    /// Whether every tracked metric matched exactly.
    pub metrics_ok: bool,
    /// Non-blocking: a stale identity is warned, not failed.
    pub identity_ok: bool,
    /// Whether the whole-record signature matched.
    pub signature_ok: bool,
    /// Identity drifted while every tracked metric held.
    pub stale: bool,
    /// Every identity field's comparison.
    pub identity_rows: Vec<OutfieldAiBaselineIdentityRow>,
    /// Every tracked metric's comparison.
    pub rows: Vec<OutfieldAiBaselineMetricRow>,
}

fn identity_field_display(identity: &OutfieldAiBaselineIdentity, field: &str) -> String {
    match field {
        "schema" => identity.schema.clone(),
        "schema_version" => identity.schema_version.to_string(),
        "policy_id" => identity.policy_id.clone(),
        "fixture" => identity.fixture.clone(),
        "config" => identity.config.clone(),
        "config_hash" => identity.config_hash.clone(),
        "content_hash" => identity.content_hash.clone(),
        "tuning_hash" => identity.tuning_hash.clone(),
        "snapshot_version" => identity.snapshot_version.to_string(),
        "input_version" => identity.input_version.to_string(),
        "tick_rate" => identity.tick_rate.to_string(),
        "seed_first" => identity.seed_first.to_string(),
        "seed_count" => identity.seed_count.to_string(),
        "seed_hash" => identity.seed_hash.clone(),
        "fixture_hash" => identity.fixture_hash.clone(),
        _ => panic!("unknown outfield AI baseline identity field: {field}"),
    }
}

/// Like [`identity_field_display`], but rendered as a Rust literal (strings
/// quoted via `Debug`, which is also valid `&'static str` source text) for
/// [`serialize`].
fn identity_field_literal(identity: &OutfieldAiBaselineIdentity, field: &str) -> String {
    match field {
        "schema" => format!("{:?}", identity.schema),
        "schema_version" => identity.schema_version.to_string(),
        "policy_id" => format!("{:?}", identity.policy_id),
        "fixture" => format!("{:?}", identity.fixture),
        "config" => format!("{:?}", identity.config),
        "config_hash" => format!("{:?}", identity.config_hash),
        "content_hash" => format!("{:?}", identity.content_hash),
        "tuning_hash" => format!("{:?}", identity.tuning_hash),
        "snapshot_version" => identity.snapshot_version.to_string(),
        "input_version" => identity.input_version.to_string(),
        "tick_rate" => identity.tick_rate.to_string(),
        "seed_first" => identity.seed_first.to_string(),
        "seed_count" => identity.seed_count.to_string(),
        "seed_hash" => format!("{:?}", identity.seed_hash),
        "fixture_hash" => format!("{:?}", identity.fixture_hash),
        _ => panic!("unknown outfield AI baseline identity field: {field}"),
    }
}

/// Compare a frozen record against a fresh measurement. Exact, per the
/// module doc's "Comparison is EXACT" section.
///
/// Only `metrics_ok` blocks. `content_hash` covers every authored roster,
/// formation and tactic and `tuning_hash` covers all 40 knob defaults, so
/// identity moves for edits that provably do not change combat-disabled
/// play — registering an unrelated knob, renaming a reserve player. Failing
/// a shared gate on those would tax every unrelated branch with the
/// deliberately awkward re-freeze ceremony and teach people to run it
/// reflexively, which is the exact habit this artifact exists to prevent. A
/// stale identity is therefore loud and still owes a drift-log entry; it
/// just is not a hard stop.
#[must_use]
pub fn compare(
    baseline: &OutfieldAiBaselineRecord,
    current: &OutfieldAiBaselineRecord,
) -> OutfieldAiBaselineComparison {
    let mut identity_rows = Vec::with_capacity(IDENTITY_FIELDS.len());
    let mut identity_ok = true;
    for &field in IDENTITY_FIELDS {
        let base = identity_field_display(&baseline.identity, field);
        let cur = identity_field_display(&current.identity, field);
        let row_ok = base == cur;
        identity_ok = identity_ok && row_ok;
        identity_rows.push(OutfieldAiBaselineIdentityRow {
            key: field,
            base,
            cur,
            ok: row_ok,
        });
    }

    let mut rows = Vec::with_capacity(TRACKED.len());
    let mut metrics_ok = true;
    for &key in TRACKED {
        let base = stat_field(&baseline.stats, key);
        let cur = stat_field(&current.stats, key);
        let mut moved = Vec::new();
        for &field in STAT_FIELDS {
            if stat_field_value(&base, field) != stat_field_value(&cur, field) {
                moved.push(field);
            }
        }
        let row_ok = moved.is_empty();
        metrics_ok = metrics_ok && row_ok;
        rows.push(OutfieldAiBaselineMetricRow {
            key,
            base: base.mean,
            cur: cur.mean,
            delta: cur.mean - base.mean,
            moved,
            ok: row_ok,
        });
    }

    let signature_ok = baseline.signature == current.signature;
    OutfieldAiBaselineComparison {
        ok: metrics_ok,
        metrics_ok,
        identity_ok,
        signature_ok,
        stale: metrics_ok && !identity_ok,
        identity_rows,
        rows,
    }
}

/// Render a human-readable comparison report.
#[must_use]
pub fn report(
    comparison: &OutfieldAiBaselineComparison,
    baseline: &OutfieldAiBaselineRecord,
    current: &OutfieldAiBaselineRecord,
) -> String {
    let mut lines = vec![
        format!(
            "outfield AI baseline: {FIXTURE}, seeds {SEED_FIRST}..{}, combat {}",
            SEED_FIRST + SEED_COUNT - 1,
            outfield_ai_policy::COMBAT_MODE
        ),
        format!(
            "frozen v{} vs data::outfield_ai_baseline",
            baseline.baseline_version
        ),
        format!("policy   {}", current.identity.policy_id),
        format!("fixture  {}", current.identity.fixture_hash),
        format!(
            "signature base={} now={}",
            baseline.signature, current.signature
        ),
    ];
    if !comparison.identity_ok {
        lines.push("IDENTITY MISMATCH — the frozen fixture is not the one measured:".to_string());
        for row in &comparison.identity_rows {
            if !row.ok {
                lines.push(format!(
                    "  {:<18} base={} now={}",
                    row.key, row.base, row.cur
                ));
            }
        }
    }
    if !comparison.signature_ok {
        lines.push("signature differs (it covers identity as well as the statistics)".to_string());
    }
    lines.push(format!(
        "{:<32} {:>14} {:>14} {:>14}",
        "metric", "base mean", "now mean", "delta"
    ));
    for row in &comparison.rows {
        let status = if row.ok {
            "ok".to_string()
        } else {
            format!("MOVED[{}]", row.moved.join(","))
        };
        lines.push(format!(
            "{:<32} {:>14.6} {:>14.6} {:>+14.6}  {status}",
            row.key, row.base, row.cur, row.delta
        ));
    }
    if comparison.stale {
        // Warning, not a failure: play is provably identical, only the
        // description of the fixture is out of date.
        lines.push(
            "AI BASELINE STALE — every tracked metric is unchanged, so this build".to_string(),
        );
        lines.push(
            "still plays the frozen control exactly; only the recorded identity is".to_string(),
        );
        lines.push("out of date. Not a failure, but it does owe a drift-log entry in".to_string());
        lines.push("docs/design/fun_metrics.md naming what moved, then a deliberate".to_string());
        lines.push(
            "re-freeze that bumps baseline_version (this repository has no runner".to_string(),
        );
        lines.push("to drive that automatically -- see the module doc). Until then".to_string());
        lines.push("dependent evidence is citing an identity that no longer describes".to_string());
        lines.push("this build.".to_string());
    } else if comparison.ok {
        lines.push("AI BASELINE OK".to_string());
    } else {
        lines.push(
            "AI BASELINE MOVED — the frozen combat-disabled control is no longer".to_string(),
        );
        lines.push(
            "what this build produces. This is a finding, not a chore: dependent".to_string(),
        );
        lines.push(
            "evidence cites this artifact, so refreshing it to go green deletes the evidence."
                .to_string(),
        );
        lines.push("Confirm the change is intended, record it in the drift log".to_string());
        lines.push("of docs/design/fun_metrics.md, then re-freeze deliberately by".to_string());
        lines.push(
            "bumping baseline_version (this repository has no runner to drive that".to_string(),
        );
        lines.push("automatically -- see the module doc).".to_string());
    }
    lines.join("\n")
}

/// Render an `f64` as valid Rust float-literal source: Rust's shortest
/// round-trippable `Display` form, guaranteed to parse back to the exact
/// same bit pattern, but with a decimal point forced in when `Display`
/// would otherwise emit a bare integer (`0`, `5`) — which is NOT a valid
/// `f64` literal in Rust (a bare integer literal does not coerce to `f64`,
/// even where one is expected) and would make [`serialize`]'s output fail
/// to compile.
fn f64_literal(value: f64) -> String {
    let text = value.to_string();
    if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    }
}

/// Serialize a record as loadable baseline source text: the exact `//!`
/// module header, `#![allow(clippy::excessive_precision)]`, and
/// `pub const RECORD: OutfieldAiBaselineRecord = OutfieldAiBaselineRecord
/// { ... };` literal that belong in `gc_data::outfield_ai_baseline`
/// (`rust/crates/gc-data/src/outfield_ai_baseline.rs`), so the output can
/// be pasted directly over that file's header and const. Numbers are
/// rendered with Rust's shortest round-trippable `Display` form, which —
/// unlike a fixed decimal precision — is guaranteed to parse back to the
/// exact same `f64` bit pattern, so exact comparison after a round trip
/// stays sound.
#[must_use]
pub fn serialize(record: &OutfieldAiBaselineRecord) -> String {
    let mut lines = vec![
        "//! Frozen combat-disabled Outfield AI baseline (#59). DO NOT hand-edit and".to_string(),
        "//! DO NOT refresh to silence a failing baseline check: #148/#149 cite".to_string(),
        "//! this artifact as their control, so a moved baseline is evidence.".to_string(),
        "//!".to_string(),
        "//! A deliberate re-freeze is:".to_string(),
        "//!   1. confirm the change is intended and record it in the drift log of".to_string(),
        "//!      docs/design/fun_metrics.md;".to_string(),
        "//!   2. re-record with the runner, which bumps `baseline_version` itself:".to_string(),
        "//!".to_string(),
        "//!      cd rust".to_string(),
        "//!      cargo test -p gc-sim --test outfield_ai_baseline -- --ignored --nocapture record_outfield_ai_baseline".to_string(),
        "//!".to_string(),
        "//!      then splice its `pub const RECORD` block over this file's. The".to_string(),
        "//!      runner emits this doc header and that block only -- the type".to_string(),
        "//!      definitions between them live here and are not regenerated, so".to_string(),
        "//!      do not overwrite the whole file with its output.".to_string(),
        "//!".to_string(),
        "//!      Until #488 no such runner existed, and this paragraph said so;".to_string(),
        "//!      `measure` and `serialize` both existed and nothing drove them".to_string(),
        "//!      together, so every re-freeze until then was the hand edit the".to_string(),
        "//!      line above warns against.".to_string(),
        "//!".to_string(),
        "//!      Regenerating by hand defeats the purpose of a frozen control, so".to_string(),
        "//!      treat a moved baseline as a finding to investigate first, not a".to_string(),
        "//!      check to clear.".to_string(),
        "//!".to_string(),
        "//! See `sim::outfield_ai_baseline` and docs/design/fun_metrics.md.".to_string(),
        "//!".to_string(),
        "//! The recorded means/standard-deviations below are kept at full precision,".to_string(),
        "//! matching the frozen evidence contract's `%.17g` round-trip requirement".to_string(),
        "//! (see `gc_sim::outfield_ai_baseline::serialize`). Clippy's".to_string(),
        "//! `excessive_precision` lint would otherwise ask for a shorter —".to_string(),
        "//! bit-identical — decimal form; this file keeps the full literal digit".to_string(),
        "//! sequence instead, so a reviewer diffing a re-frozen version against this".to_string(),
        "//! one sees every digit.".to_string(),
        "#![allow(clippy::excessive_precision)]".to_string(),
        String::new(),
        "/// The frozen baseline recording.".to_string(),
        "pub const RECORD: OutfieldAiBaselineRecord = OutfieldAiBaselineRecord {".to_string(),
        format!("    baseline_version: {},", record.baseline_version),
        "    identity: OutfieldAiBaselineIdentity {".to_string(),
    ];
    let id = &record.identity;
    for &field in IDENTITY_FIELDS {
        lines.push(format!(
            "        {field}: {},",
            identity_field_literal(id, field)
        ));
    }
    lines.push("    },".to_string());
    lines.push("    stats: OutfieldAiBaselineStats {".to_string());
    for &key in TRACKED {
        let stat = stat_field(&record.stats, key);
        // One field per line: a wide inline struct literal would need
        // reformatting anyway, and this mirrors the checked-in baseline
        // file's own layout.
        lines.push(format!("        {key}: OutfieldAiBaselineStat {{"));
        lines.push(format!("            n: {},", stat.n));
        lines.push(format!("            mean: {},", f64_literal(stat.mean)));
        lines.push(format!("            sd: {},", f64_literal(stat.sd)));
        lines.push(format!("            min: {},", f64_literal(stat.min)));
        lines.push(format!("            max: {},", f64_literal(stat.max)));
        lines.push("        },".to_string());
    }
    lines.push("    },".to_string());
    lines.push(format!("    signature: {:?},", record.signature));
    lines.push("};".to_string());
    lines.push(String::new());
    lines.join("\n")
}
