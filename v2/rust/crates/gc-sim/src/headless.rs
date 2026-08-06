//! Port of `sim/headless.lua`.
//!
//! Headless match batches preserve the legacy `MatchInput` fixture by
//! default. Supplying frames or slot sources opts into fixed-slot input
//! production. Both paths fold into fun-proxy metrics ([`crate::metrics`]).
//! Pure — no love, no I/O; [`report`] returns a string and the caller
//! decides where it goes.
//!
//! ## Adapters this module owns (README §5.1)
//!
//! [`crate::bot`], [`crate::metrics`], and [`crate::slot_input`] were each
//! ported before `sim::match` landed. `slot_input` adopted the real
//! `match_snapshot` types directly and needs no adapter; `bot` and
//! `metrics` kept their own narrow, differently-shaped views
//! (`bot::BotMatchView`/`bot::BotPlayerView`,
//! `metrics::MetricsMatchView`/`metrics::MetricsPlayerView`/
//! `metrics::MetricsEventView` — renamed from the generic
//! `MatchStateView`/`MatchPlayerView`/`MatchEventView` every module used
//! before README §5.1's resolution; `crate::aerial`'s third copy needed
//! nearly the whole shape and was folded onto `match_snapshot`'s canonical
//! types instead). This module is the one caller that drives `bot` and
//! `metrics` against a REAL, constructed `MatchState` every tick — exactly
//! `sim/headless.lua`'s own job — so the call-boundary adapters
//! (`to_bot_view`, `to_metrics_view`) belong here. `bot::input` now returns
//! `match_snapshot::MatchInput` directly (its own local `MatchInput` was
//! never legitimately narrow — every field was one the real simulation
//! reads — so it was dropped rather than renamed), which is why there is no
//! `from_bot_input` any more either.
//!
//! ## `tuning.lua`'s global becomes a local value, so "restore" is structural
//!
//! The Lua original saves/restores the global `tuning` singleton around a
//! `tuning_blob` override so a batch never leaks knob overrides into other
//! callers. `sim/tuning.lua`'s Rust port ([`crate::tuning::Tuning`]) is
//! already an explicit, owned value rather than a singleton (AGENTS.md §3;
//! see that module's doc), so [`run_match`] simply builds a fresh
//! [`Tuning`] per call and applies `tuning_blob` on top of it — there is no
//! global left to leak into, so nothing needs restoring afterward.
//!
//! ## Test seams (no function mocking in Rust)
//!
//! `spec/sim/headless_spec.lua` monkey-patches `match.new`, `bot.new`, and
//! `slot_input.new_producer` to observe internal construction decisions
//! (was a bot built exactly once? did the match land in slot mode?). Rust
//! has no runtime function replacement, so [`run_match_debug`] exposes the
//! same observable facts directly — the constructed [`MatchState`] and a
//! [`HeadlessDebug`] summary — instead. `run_match` itself just discards
//! them; every assertion the Lua spec made via a mock is preserved as an
//! assertion against this seam's return value.

use crate::bot;
use crate::fixed_clock;
use crate::input_frame::{self, InputFrame};
use crate::keeper::{KeeperBehaviorState, KeeperShotType};
use crate::r#match as sim_match;
use crate::match_snapshot::{MatchEvent, MatchInput, MatchPlayer, MatchState, PitchSize, Team};
use crate::metrics;
use crate::slot_input;
use crate::tuning::Tuning;
use gc_data::players::PlayerData;
use gc_data::species::SpeciesData;
use gc_data::tactics::TacticData;
use gc_data::teams::{self, TeamData};
use indexmap::IndexMap;

const FIELD_W: f64 = 960.0; // the real game's pitch (game/screens/match.lua)
const FIELD_H: f64 = 540.0;
const DT: f64 = fixed_clock::TICK_SECONDS;
const DEFAULT_DURATION: f64 = 120.0;
/// Overtime guard: a stuck sim must not hang the batch.
const MAX_STEPS_SLACK: u64 = 600;

/// Which population drives the human-controlled slot in a headless match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HeadlessBot {
    /// `sim::bot`'s human-proxy driver controls the slot (the default).
    #[default]
    Home,
    /// No proxy: every player uses the match AI.
    None,
}

/// Construction options for one headless match. Mirrors `HeadlessOpts`.
#[derive(Debug, Default)]
pub struct HeadlessOpts<'a> {
    /// PRNG seed.
    pub seed: f64,
    /// Match duration in seconds; defaults to 120.
    pub duration: Option<f64>,
    /// Goal limit; defaults to `sim::match::NO_GOAL_LIMIT`.
    pub max_goals: Option<i64>,
    /// Bot decision latency override.
    pub reaction: Option<f64>,
    /// Knob overrides in `sim::tuning::Tuning::serialize` format.
    pub tuning_blob: Option<&'a str>,
    /// Home team; defaults to `nebula`.
    pub home: Option<&'a TeamData>,
    /// Away team; defaults to `orion`.
    pub away: Option<&'a TeamData>,
    /// Override for `home`'s formation.
    pub home_formation: Option<&'a str>,
    /// Override for `away`'s formation. `'static` because it is spliced
    /// into a locally constructed `TeamData`, whose `formation` field is
    /// `&'static str` (every authored formation id already is one).
    pub away_formation: Option<&'static str>,
    /// Home tactic; defaults to `tactics::get("balanced")`.
    pub tactic: Option<&'a TacticData>,
    /// Away tactic; defaults to `tactics::get("balanced")`.
    pub away_tactic: Option<&'a TacticData>,
    /// Player pool; defaults to every authored player.
    pub players_by_id: Option<&'a IndexMap<&'a str, PlayerData>>,
    /// Species pool; defaults to every authored species.
    pub species_by_id: Option<&'a IndexMap<&'a str, SpeciesData>>,
    /// Playable pitch dimensions; defaults to 960x540.
    pub field: Option<PitchSize>,
    /// Human-proxy mode; defaults to [`HeadlessBot::Home`].
    pub bot: Option<HeadlessBot>,
    /// Complete canonical recorded rows, indexed by tick.
    pub frames: Option<&'a [InputFrame]>,
    /// Upstream frame/bot/neutral producer policy.
    pub slot_sources: Option<[slot_input::MatchSlotSource; 8]>,
}

/// The winning side of a finished match, or `None` for a draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Winner {
    /// Home won.
    Home,
    /// Away won.
    Away,
}

/// One headless match's outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchResult {
    /// The seed the match was played with.
    pub seed: f64,
    /// Final score.
    pub score: metrics::Score,
    /// The winning side, or `None` for a draw.
    pub winner: Option<Winner>,
    /// Folded fun-proxy metrics, including the composite `fun` score.
    pub metrics: metrics::MatchMetrics,
    /// Per-banded-metric desirability, `0..1`.
    pub desirability: IndexMap<&'static str, f64>,
}

/// Internal introspection [`run_match_debug`] returns alongside a
/// [`MatchResult`], standing in for the Lua spec's function-mock
/// assertions (see the module doc's "Test seams" section).
#[derive(Clone, Debug, PartialEq)]
pub struct HeadlessDebug {
    /// Whether the legacy human-proxy `BotState` was constructed for this
    /// run (`sim::bot::new`'s single call site is taken exactly when this
    /// is `true`).
    pub bot_constructed: bool,
    /// Every canonical slot's source kind, when the match ran in slot
    /// mode.
    pub slot_sources: Option<[slot_input::MatchSlotSourceKind; 8]>,
}

fn default_home() -> &'static TeamData {
    teams::get("nebula").expect("nebula is an authored team")
}

fn default_away() -> &'static TeamData {
    teams::get("orion").expect("orion is an authored team")
}

fn to_bot_player(p: &MatchPlayer) -> bot::BotPlayerView {
    bot::BotPlayerView {
        team: match p.team {
            Team::Home => bot::Team::Home,
            Team::Away => bot::Team::Away,
        },
        is_keeper: p.is_keeper,
        pos: p.pos,
        tackle_timer: p.tackle_timer,
        slide_timer: p.slide_timer,
        dodge_cd: p.dodge_cd,
        sprint_meter: p.sprint_meter,
    }
}

fn to_bot_view(s: &MatchState) -> bot::BotMatchView {
    bot::BotMatchView {
        players: s.players.iter().map(to_bot_player).collect(),
        field: bot::Field {
            w: s.field.w,
            h: s.field.h,
        },
        controlled: (s.controlled - 1) as usize,
        owner: s.owner.map(|i| (i - 1) as usize),
        ball: s.ball,
        ball_vel: s.ball_vel,
        ball_z: s.ball_z,
        ball_vz: s.ball_vz,
    }
}

fn to_metrics_keeper_state(k: KeeperBehaviorState) -> metrics::KeeperState {
    match k {
        KeeperBehaviorState::Base => metrics::KeeperState::Base,
        KeeperBehaviorState::Advance => metrics::KeeperState::Advance,
        KeeperBehaviorState::Contain => metrics::KeeperState::Contain,
        KeeperBehaviorState::Set => metrics::KeeperState::Set,
        KeeperBehaviorState::Retreat => metrics::KeeperState::Retreat,
        KeeperBehaviorState::Recover => metrics::KeeperState::Recover,
    }
}

fn to_metrics_event(e: &MatchEvent) -> metrics::MetricsEventView {
    metrics::MetricsEventView {
        kind: e.kind.wire_str().to_string(),
        player: e.player.clone(),
        shot_type: e.shot_type.map(|t| {
            match t {
                KeeperShotType::Chip => "chip",
                KeeperShotType::Ground => "ground",
            }
            .to_string()
        }),
        on_target: e.on_target,
        keeper_state: e.keeper_state.map(to_metrics_keeper_state),
        keeper_depth: e.keeper_depth,
    }
}

fn to_metrics_view(s: &MatchState) -> metrics::MetricsMatchView {
    metrics::MetricsMatchView {
        players: s
            .players
            .iter()
            .map(|p| metrics::MetricsPlayerView {
                id: p.id.clone(),
                team: match p.team {
                    Team::Home => metrics::MatchTeam::Home,
                    Team::Away => metrics::MatchTeam::Away,
                },
                is_keeper: p.is_keeper,
                vel: p.vel,
                move_speed: p.move_speed,
                sprinting: p.sprinting,
                dodge_timer: p.dodge_timer,
                keeper_state: Some(to_metrics_keeper_state(p.keeper_state)),
            })
            .collect(),
        human_controlled: s.human_controlled,
        controlled: (s.controlled - 1) as usize,
        score: metrics::Score {
            home: s.score.home,
            away: s.score.away,
        },
        owner: s.owner.map(|i| (i - 1) as usize),
        events: s.events.iter().map(to_metrics_event).collect(),
    }
}

/// Play one full match and measure it, exposing internal construction
/// facts a real caller never needs — see the module doc's "Test seams"
/// section. [`run_match`] is this function with the last two return values
/// dropped.
#[must_use]
pub fn run_match_debug(opts: &HeadlessOpts<'_>) -> (MatchResult, MatchState, HeadlessDebug) {
    let mut tune = Tuning::new();
    if let Some(blob) = opts.tuning_blob {
        tune.deserialize(blob);
    }

    let bot_mode = opts.bot.unwrap_or_default();
    let field = opts.field.unwrap_or(PitchSize {
        w: FIELD_W,
        h: FIELD_H,
    });
    let duration = opts.duration.unwrap_or(DEFAULT_DURATION);
    let home_ref: &TeamData = opts.home.unwrap_or(default_home());
    let away_ref: &TeamData = opts.away.unwrap_or(default_away());
    let away_owned;
    let away_team: &TeamData = match opts.away_formation {
        Some(f) => {
            away_owned = TeamData {
                formation: f,
                ..*away_ref
            };
            &away_owned
        }
        None => away_ref,
    };
    let slot_mode = opts.frames.is_some() || opts.slot_sources.is_some();

    let ownership =
        slot_mode.then(|| sim_match::ownership_for_teams(home_ref, away_team, opts.players_by_id));

    let mut s = sim_match::new(sim_match::NewMatchOptions {
        home: home_ref,
        away: away_team,
        field,
        home_formation: opts.home_formation,
        tactic: opts.tactic,
        away_tactic: opts.away_tactic,
        duration: Some(duration),
        max_goals: opts.max_goals,
        seed: Some(opts.seed),
        players_by_id: opts.players_by_id,
        species_by_id: opts.species_by_id,
        showcase_players_by_id: None,
        human_controlled: Some(bot_mode == HeadlessBot::Home),
        input_ownership: ownership,
    });

    let mut producer = slot_mode.then(|| {
        let sources = opts.slot_sources.unwrap_or(
            [slot_input::MatchSlotSource {
                kind: slot_input::MatchSlotSourceKind::Frame,
                seed: None,
            }; 8],
        );
        slot_input::new_producer(sources)
    });
    let slot_sources_debug = producer.as_ref().map(|p| p.sources.map(|src| src.kind));

    let mut bot_state = (!slot_mode && bot_mode == HeadlessBot::Home).then(|| {
        bot::new(bot::BotOptions {
            seed: Some(opts.seed),
            reaction: opts.reaction,
        })
    });
    let bot_constructed = bot_state.is_some();

    let mut collector = metrics::new(&to_metrics_view(&s));
    let mut clock = fixed_clock::new();

    let max_steps = (duration / fixed_clock::TICK_SECONDS).ceil() as u64 + MAX_STEPS_SLACK;
    for _ in 0..max_steps {
        if s.finished {
            break;
        }
        let continue_ = if let Some(producer) = producer.as_mut() {
            let base_frame = if let Some(frames) = opts.frames {
                *frames
                    .get(clock.tick as usize)
                    .unwrap_or_else(|| panic!("recorded frame missing for headless tick"))
            } else {
                input_frame::neutral(clock.tick as i64)
                    .expect("headless tick fits input_frame's tick bound")
            };
            let (frame, _decisions) = slot_input::materialize(producer, &s, &base_frame, None);
            let (_, cont) = fixed_clock::step(&mut clock, &frame, |_, tick_frame: &InputFrame| {
                sim_match::step(
                    &mut s,
                    DT,
                    sim_match::StepInput::Frame(tick_frame),
                    None,
                    &tune,
                );
                let view = to_metrics_view(&s);
                metrics::observe(&mut collector, &view, DT, &tune);
                !s.finished
            });
            cont
        } else {
            let input = if let Some(bot_state) = bot_state.as_mut() {
                let view = to_bot_view(&s);
                bot::input(bot_state, &view, DT, &tune)
            } else {
                MatchInput::default()
            };
            let (_, cont) = fixed_clock::step(&mut clock, &input, |_, tick_input: &MatchInput| {
                sim_match::step(
                    &mut s,
                    DT,
                    sim_match::StepInput::Legacy(*tick_input),
                    None,
                    &tune,
                );
                let view = to_metrics_view(&s);
                metrics::observe(&mut collector, &view, DT, &tune);
                !s.finished
            });
            cont
        };
        if !continue_ {
            break;
        }
    }

    let mut m = metrics::finish(&mut collector, &to_metrics_view(&s));
    let (fun, per) = metrics::fun_score(&m);
    m.fun = Some(fun);
    let winner = if s.score.home > s.score.away {
        Some(Winner::Home)
    } else if s.score.away > s.score.home {
        Some(Winner::Away)
    } else {
        None
    };
    let result = MatchResult {
        seed: opts.seed,
        score: metrics::Score {
            home: s.score.home,
            away: s.score.away,
        },
        winner,
        metrics: m,
        desirability: per,
    };
    let debug = HeadlessDebug {
        bot_constructed,
        slot_sources: slot_sources_debug,
    };
    (result, s, debug)
}

/// Play one full match and measure it. Applies `opts.tuning_blob` on top of
/// the defaults for the run — see the module doc for why nothing needs
/// restoring afterward.
#[must_use]
pub fn run_match(opts: &HeadlessOpts<'_>) -> MatchResult {
    run_match_debug(opts).0
}

/// Construction options for a fixed seed-set batch. Mirrors `BatchOpts`.
#[derive(Debug, Default)]
pub struct BatchOpts<'a> {
    /// Number of matches (seeds `1..=n`) when `seeds` is not given; default
    /// 20.
    pub n: Option<i64>,
    /// Explicit seed set; overrides `n` when present.
    pub seeds: Option<&'a [f64]>,
    /// Forwarded to every match.
    pub duration: Option<f64>,
    /// Forwarded to every match.
    pub max_goals: Option<i64>,
    /// Forwarded to every match.
    pub reaction: Option<f64>,
    /// Forwarded to every match.
    pub tuning_blob: Option<&'a str>,
    /// Forwarded to every match.
    pub home: Option<&'a TeamData>,
    /// Forwarded to every match.
    pub away: Option<&'a TeamData>,
    /// Forwarded to every match.
    pub home_formation: Option<&'a str>,
    /// Forwarded to every match.
    pub away_formation: Option<&'static str>,
    /// Forwarded to every match.
    pub tactic: Option<&'a TacticData>,
    /// Forwarded to every match.
    pub away_tactic: Option<&'a TacticData>,
    /// Forwarded to every match.
    pub players_by_id: Option<&'a IndexMap<&'a str, PlayerData>>,
    /// Forwarded to every match.
    pub species_by_id: Option<&'a IndexMap<&'a str, SpeciesData>>,
    /// Forwarded to every match.
    pub field: Option<PitchSize>,
    /// Forwarded to every match.
    pub bot: Option<HeadlessBot>,
    /// Forwarded to every match.
    pub frames: Option<&'a [InputFrame]>,
    /// Forwarded to every match.
    pub slot_sources: Option<[slot_input::MatchSlotSource; 8]>,
}

/// A seed-set batch's complete results.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchResult {
    /// Every match's outcome, in seed order.
    pub matches: Vec<MatchResult>,
    /// Per-metric distribution stats across the batch.
    pub agg: IndexMap<&'static str, metrics::MetricStats>,
}

/// Play a batch over a fixed seed set. Compare knob configs on the SAME
/// seeds (common random numbers): differences then come from the knobs,
/// not luck.
#[must_use]
pub fn run_batch(opts: &BatchOpts<'_>) -> BatchResult {
    let seeds: Vec<f64> = match opts.seeds {
        Some(seeds) => seeds.to_vec(),
        None => (1..=opts.n.unwrap_or(20)).map(|i| i as f64).collect(),
    };
    let mut matches = Vec::with_capacity(seeds.len());
    let mut all: Vec<IndexMap<&'static str, f64>> = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let r = run_match(&HeadlessOpts {
            seed,
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
        });
        all.push(metrics_to_map(&r.metrics));
        matches.push(r);
    }
    let agg = metrics::aggregate(&all);
    BatchResult { matches, agg }
}

fn metrics_to_map(m: &metrics::MatchMetrics) -> IndexMap<&'static str, f64> {
    let mut map = IndexMap::new();
    map.insert("duration", m.duration);
    map.insert("goals_home", m.goals_home as f64);
    map.insert("goals_away", m.goals_away as f64);
    map.insert("goals_total", m.goals_total as f64);
    map.insert("margin", m.margin as f64);
    map.insert("lead_changes", m.lead_changes as f64);
    map.insert("decided_late", m.decided_late);
    map.insert("shots", m.shots as f64);
    if let Some(v) = m.shots_per_goal {
        map.insert("shots_per_goal", v);
    }
    if let Some(v) = m.save_rate {
        map.insert("save_rate", v);
    }
    map.insert("passes", m.passes as f64);
    if let Some(v) = m.pass_completion {
        map.insert("pass_completion", v);
    }
    map.insert("turnovers_per_min", m.turnovers_per_min);
    if let Some(v) = m.possession_balance {
        map.insert("possession_balance", v);
    }
    map.insert("longest_drought_s", m.longest_drought_s);
    map.insert("controlled_dribble_carry_s", m.controlled_dribble_carry_s);
    if let Some(v) = m.controlled_dribble_close_share {
        map.insert("controlled_dribble_close_share", v);
    }
    if let Some(v) = m.controlled_dribble_sprint_share {
        map.insert("controlled_dribble_sprint_share", v);
    }
    if let Some(v) = m.controlled_dribble_juke_share {
        map.insert("controlled_dribble_juke_share", v);
    }
    map.insert(
        "controlled_dribble_touches_per_min",
        m.controlled_dribble_touches_per_min,
    );
    map.insert(
        "controlled_dribble_heavy_losses_per_min",
        m.controlled_dribble_heavy_losses_per_min,
    );
    map.insert("controlled_jukes", m.controlled_jukes as f64);
    map.insert("ai_dribble_carry_s", m.ai_dribble_carry_s);
    if let Some(v) = m.ai_dribble_close_share {
        map.insert("ai_dribble_close_share", v);
    }
    if let Some(v) = m.ai_dribble_sprint_share {
        map.insert("ai_dribble_sprint_share", v);
    }
    if let Some(v) = m.ai_dribble_juke_share {
        map.insert("ai_dribble_juke_share", v);
    }
    map.insert("ai_dribble_touches_per_min", m.ai_dribble_touches_per_min);
    map.insert(
        "ai_dribble_heavy_losses_per_min",
        m.ai_dribble_heavy_losses_per_min,
    );
    map.insert("ai_jukes", m.ai_jukes as f64);
    map.insert("keeper_base_s", m.keeper_base_s);
    map.insert("keeper_advance_s", m.keeper_advance_s);
    map.insert("keeper_contain_s", m.keeper_contain_s);
    map.insert("keeper_set_s", m.keeper_set_s);
    map.insert("keeper_retreat_s", m.keeper_retreat_s);
    map.insert("keeper_recover_s", m.keeper_recover_s);
    if let Some(v) = m.keeper_shot_depth_mean {
        map.insert("keeper_shot_depth_mean", v);
    }
    map.insert("chip_shots", m.chip_shots as f64);
    map.insert("chip_on_target", m.chip_on_target as f64);
    map.insert("chip_goals", m.chip_goals as f64);
    if let Some(v) = m.chip_conversion {
        map.insert("chip_conversion", v);
    }
    map.insert("keeper_saves_base", m.keeper_saves_base as f64);
    map.insert("keeper_saves_advance", m.keeper_saves_advance as f64);
    map.insert("keeper_saves_contain", m.keeper_saves_contain as f64);
    map.insert("keeper_saves_set", m.keeper_saves_set as f64);
    map.insert("keeper_saves_retreat", m.keeper_saves_retreat as f64);
    map.insert("keeper_saves_recover", m.keeper_saves_recover as f64);
    map.insert(
        "keeper_saves_unclassified",
        m.keeper_saves_unclassified as f64,
    );
    map.insert("keeper_goals_base", m.keeper_goals_base as f64);
    map.insert("keeper_goals_advance", m.keeper_goals_advance as f64);
    map.insert("keeper_goals_contain", m.keeper_goals_contain as f64);
    map.insert("keeper_goals_set", m.keeper_goals_set as f64);
    map.insert("keeper_goals_retreat", m.keeper_goals_retreat as f64);
    map.insert("keeper_goals_recover", m.keeper_goals_recover as f64);
    map.insert(
        "keeper_goals_unclassified",
        m.keeper_goals_unclassified as f64,
    );
    if let Some(v) = m.fun {
        map.insert("fun", v);
    }
    map
}

// Display order: the banded fun-proxy metrics first, context counts after.
const REPORT_ROWS: &[&str] = &[
    "fun",
    "goals_total",
    "shots_per_goal",
    "save_rate",
    "pass_completion",
    "turnovers_per_min",
    "possession_balance",
    "longest_drought_s",
    "decided_late",
    "lead_changes",
    "margin",
    "shots",
    "passes",
    "controlled_dribble_carry_s",
    "controlled_dribble_close_share",
    "controlled_dribble_sprint_share",
    "controlled_dribble_juke_share",
    "controlled_dribble_touches_per_min",
    "controlled_dribble_heavy_losses_per_min",
    "controlled_jukes",
    "ai_dribble_carry_s",
    "ai_dribble_close_share",
    "ai_dribble_sprint_share",
    "ai_dribble_juke_share",
    "ai_dribble_touches_per_min",
    "ai_dribble_heavy_losses_per_min",
    "ai_jukes",
    "keeper_base_s",
    "keeper_advance_s",
    "keeper_contain_s",
    "keeper_set_s",
    "keeper_retreat_s",
    "keeper_recover_s",
    "keeper_shot_depth_mean",
    "chip_shots",
    "chip_on_target",
    "chip_goals",
    "chip_conversion",
    "keeper_saves_base",
    "keeper_saves_advance",
    "keeper_saves_contain",
    "keeper_saves_set",
    "keeper_saves_retreat",
    "keeper_saves_recover",
    "keeper_saves_unclassified",
    "keeper_goals_base",
    "keeper_goals_advance",
    "keeper_goals_contain",
    "keeper_goals_set",
    "keeper_goals_retreat",
    "keeper_goals_recover",
    "keeper_goals_unclassified",
    "duration",
];

// A private, display-only duplicate of `metrics.rs`'s (also private)
// `BAND_*` constants (`sim/metrics.lua`'s public `metrics.bands` table).
// This module does not own `metrics.rs`, so it cannot make them `pub`
// there; these values only ever feed formatted report text, never a
// determinism-path computation, so the duplication is display-layer, not
// content-layer (AGENTS.md §8 is about game content, not report labels).
fn band_for(key: &str) -> Option<[f64; 4]> {
    Some(match key {
        "goals_total" => [0.0, 2.0, 5.0, 8.0],
        "shots_per_goal" => [1.0, 2.5, 6.0, 25.0],
        "save_rate" => [0.15, 0.45, 0.75, 0.95],
        "pass_completion" => [0.25, 0.55, 0.85, 1.001],
        "turnovers_per_min" => [0.3, 1.0, 5.0, 10.0],
        "possession_balance" => [0.1, 0.35, 0.65, 0.9],
        "longest_drought_s" => [-1.0, 0.0, 35.0, 80.0],
        "decided_late" => [0.05, 0.4, 1.0, f64::INFINITY],
        _ => return None,
    })
}

/// Format `value` the way C's `%g` would, for the small magnitude range
/// every band edge lives in. See `sim/tuning.rs`'s `format_g6` for the same
/// technique at a different fixed precision; duplicated here because it is
/// a private helper there.
fn format_g(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 17) as usize;
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Render a batch's fun-proxy metrics as a fixed-width table.
#[must_use]
pub fn report(batch: &BatchResult) -> String {
    let n = batch.matches.len();
    let mut desir: IndexMap<&'static str, (f64, i64)> = IndexMap::new();
    for r in &batch.matches {
        for (&k, &d) in &r.desirability {
            let entry = desir.entry(k).or_insert((0.0, 0));
            entry.0 += d;
            entry.1 += 1;
        }
    }

    let mut out = Vec::new();
    out.push(format!(
        "fun-proxy metrics over {n} matches (mean +/- sd [min .. max])"
    ));
    out.push(format!(
        "{:<20} {:>10} {:>9} {:>9} {:>9} {:>7}  {}",
        "metric", "mean", "sd", "min", "max", "desir", "band"
    ));
    for &key in REPORT_ROWS {
        let Some(st) = batch.agg.get(key) else {
            continue;
        };
        let band_str = band_for(key)
            .map(|b| format!("[{} .. {}]", format_g(b[1]), format_g(b[2])))
            .unwrap_or_default();
        let d_str = desir
            .get(key)
            .map(|&(sum, n)| format!("{:>7.2}", sum / n as f64))
            .unwrap_or_else(|| format!("{:>7}", "-"));
        let miss = if st.n < n as i64 {
            format!(" (n={})", st.n)
        } else {
            String::new()
        };
        out.push(format!(
            "{:<20} {:>10.3} {:>9.3} {:>9.3} {:>9.3} {d_str}  {band_str}{miss}",
            key, st.mean, st.sd, st.min, st.max
        ));
    }
    out.join("\n")
}
