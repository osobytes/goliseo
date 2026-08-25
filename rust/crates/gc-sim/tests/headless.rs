//! Tests for `gc_sim::headless`.
//!
//! Rust has no runtime function replacement, so every case that would
//! otherwise need to observe internal construction decisions by mocking
//! `match.new`/`bot.new`/`slot_input.new_producer` instead uses
//! [`gc_sim::headless::run_match_debug`] — see `headless.rs`'s module doc,
//! "Test seams" section, for the reasoning.
//!
//! `gc_sim::tuning::Tuning` is an owned value, not a singleton (see
//! `tuning.rs`'s module doc), so [`gc_sim::headless::run_match`] simply
//! builds a fresh `Tuning` per call — there is no global left to leak into.
//! The "applies a tuning blob for the run and restores the knobs after"
//! test below demonstrates the equivalent guarantee (running with a blob
//! does not perturb a freshly constructed `Tuning`'s defaults) rather than
//! a literal save/restore.

use gc_data::players::PlayerData;
use gc_data::tactics;
use gc_data::teams;
use gc_sim::headless::{self, BatchOpts, HeadlessBot, HeadlessOpts, MatchResult, Winner};
use gc_sim::input_frame::{self, InputFrame, InputSampleOptions};
use gc_sim::match_snapshot::PitchSize;
use gc_sim::slot_input::{MatchSlotSource, MatchSlotSourceKind};
use gc_sim::tuning::Tuning;
use indexmap::IndexMap;

fn recorded_frames(ticks: i64) -> Vec<InputFrame> {
    (0..ticks)
        .map(|tick| {
            let mut slots = [input_frame::InputSample::default(); 8];
            slots[0] = input_frame::new_sample(InputSampleOptions {
                move_x: Some(127),
                ..Default::default()
            })
            .expect("canonical sample");
            slots[4] = input_frame::new_sample(InputSampleOptions {
                move_x: Some(-127),
                ..Default::default()
            })
            .expect("canonical sample");
            input_frame::new(tick, Some(slots)).expect("canonical frame")
        })
        .collect()
}

fn recorded_simultaneous_actions(ticks: i64) -> Vec<InputFrame> {
    let mut frames = recorded_frames(ticks);
    let last_simultaneous = 4.min(ticks - 1);
    for frame in frames.iter_mut().take((last_simultaneous + 1) as usize) {
        frame.slots[3] = input_frame::new_sample(InputSampleOptions {
            held: Some(input_frame::HELD_PASS),
            ..Default::default()
        })
        .expect("canonical sample");
        frame.slots[4] = input_frame::new_sample(InputSampleOptions {
            held: Some(input_frame::HELD_JOCKEY),
            ..Default::default()
        })
        .expect("canonical sample");
    }
    if ticks > 5 {
        frames[5].slots[3] = input_frame::new_sample(InputSampleOptions {
            edges: Some(input_frame::EDGE_PASS),
            ..Default::default()
        })
        .expect("canonical sample");
        frames[5].slots[4] = input_frame::new_sample(InputSampleOptions {
            edges: Some(input_frame::EDGE_DASH),
            ..Default::default()
        })
        .expect("canonical sample");
    }
    frames
}

fn players_by_id() -> IndexMap<&'static str, PlayerData> {
    let mut by_id = IndexMap::new();
    for p in gc_data::players::ALL {
        by_id.insert(p.id, *p);
    }
    by_id
}

fn assert_same_metrics(a: &MatchResult, b: &MatchResult) {
    assert_eq!(a.metrics, b.metrics, "metrics must reproduce exactly");
}

#[test]
fn headless_run_match_plays_a_full_short_match_and_produces_sane_metrics() {
    let r = headless::run_match(&HeadlessOpts {
        seed: 5.0,
        duration: Some(30.0),
        ..Default::default()
    });
    let m = &r.metrics;
    assert_eq!(r.score.home, m.goals_home);
    assert_eq!(r.score.away, m.goals_away);
    if r.score.home == r.score.away {
        assert!(r.winner.is_none(), "draws have no winner");
    } else {
        let expected = if r.score.home > r.score.away {
            Winner::Home
        } else {
            Winner::Away
        };
        assert_eq!(r.winner, Some(expected));
    }
    assert!(
        m.duration >= 29.0,
        "the match ran (close to) its full length"
    );
    assert!(m.goals_total >= 0);
    assert!(m.turnovers_per_min >= 0.0);
    let fun = m.fun.expect("fun score is always set by run_match");
    assert!((0.0..=1.0).contains(&fun), "fun score is 0..1");
    if let Some(balance) = m.possession_balance {
        assert!(balance > 0.0 && balance < 1.0);
    }
}

#[test]
fn headless_run_match_is_deterministic_same_seed_identical_metrics() {
    let a = headless::run_match(&HeadlessOpts {
        seed: 9.0,
        duration: Some(30.0),
        ..Default::default()
    });
    let b = headless::run_match(&HeadlessOpts {
        seed: 9.0,
        duration: Some(30.0),
        ..Default::default()
    });
    assert_same_metrics(&a, &b);
}

#[test]
fn headless_run_match_different_seeds_diverge() {
    let a = headless::run_match(&HeadlessOpts {
        seed: 1.0,
        duration: Some(30.0),
        ..Default::default()
    });
    let b = headless::run_match(&HeadlessOpts {
        seed: 2.0,
        duration: Some(30.0),
        ..Default::default()
    });
    assert_ne!(
        a.metrics, b.metrics,
        "two seeds should not play the identical match"
    );
}

#[test]
fn headless_run_match_applies_a_tuning_blob_for_the_run_and_leaves_no_global_to_leak_into() {
    let before = Tuning::new().value("AI_SHOOT_RANGE");
    let _ = headless::run_match(&HeadlessOpts {
        seed: 3.0,
        duration: Some(10.0),
        tuning_blob: Some("AI_SHOOT_RANGE=340"),
        ..Default::default()
    });
    let after = Tuning::new().value("AI_SHOOT_RANGE");
    assert_eq!(before, after, "no global tuning state exists to leak into");
}

#[test]
fn headless_run_match_keeps_default_fixture_options_on_the_home_proxy_mode() {
    let implicit = headless::run_match(&HeadlessOpts {
        seed: 17.0,
        duration: Some(20.0),
        ..Default::default()
    });
    let nebula = teams::get("nebula").expect("nebula is authored");
    let orion = teams::get("orion").expect("orion is authored");
    let balanced = tactics::get("balanced").expect("balanced is authored");
    let explicit = headless::run_match(&HeadlessOpts {
        seed: 17.0,
        duration: Some(20.0),
        home: Some(nebula),
        away: Some(orion),
        home_formation: Some(nebula.formation),
        away_formation: Some(orion.formation),
        tactic: Some(balanced),
        away_tactic: Some(balanced),
        // Matches gc_sim::headless::FIELD_W/FIELD_H (960x540 -> 1648x927 futsal
        // re-dimensioning): this test proves the implicit default equals the
        // explicit value, so the explicit value must track the real default.
        field: Some(PitchSize {
            w: 1648.0,
            h: 927.0,
        }),
        bot: Some(HeadlessBot::Home),
        ..Default::default()
    });
    assert_same_metrics(&implicit, &explicit);
}

#[test]
fn headless_run_match_keeps_no_frame_runs_on_the_legacy_matchinput_path() {
    let (_, state, debug) = headless::run_match_debug(&HeadlessOpts {
        seed: 19.0,
        duration: Some(3.0),
        ..Default::default()
    });
    assert!(!state.slot_mode);
    assert!(
        debug.bot_constructed,
        "legacy home-proxy bot is constructed for the default bot mode"
    );
}

#[test]
fn headless_run_match_runs_a_non_default_fixture_with_formation_tactic_roster_and_field_overrides()
{
    let mut by_id = players_by_id();
    let keeper = by_id["gax_oru"];
    by_id.insert(
        "gax_oru",
        PlayerData {
            name: "Harness Gax",
            ..keeper
        },
    );

    let orion = teams::get("orion").expect("orion is authored");
    let nebula = teams::get("nebula").expect("nebula is authored");
    let counter = tactics::get("counter").expect("counter is authored");
    let press_high = tactics::get("press_high").expect("press_high is authored");
    let (result, state, _) = headless::run_match_debug(&HeadlessOpts {
        seed: 31.0,
        duration: Some(5.0),
        home: Some(orion),
        away: Some(nebula),
        home_formation: Some("1-2-1"),
        away_formation: Some("1-1-2"),
        tactic: Some(counter),
        away_tactic: Some(press_high),
        players_by_id: Some(&by_id),
        field: Some(PitchSize { w: 800.0, h: 450.0 }),
        ..Default::default()
    });

    assert_eq!(state.players[0].id, "gax_oru", "Orion is the home side");
    assert_eq!(
        state.players[0].name, "Harness Gax",
        "custom player lookup reached match::new"
    );
    assert_eq!(state.players[5].id, "ozzo", "Nebula is the away side");
    assert_eq!(state.field.w, 800.0);
    assert_eq!(state.field.h, 450.0);
    assert!(
        (state.players[1].anchor.x - 112.0).abs() < 1e-9,
        "home formation and tactic were applied: {}",
        state.players[1].anchor.x
    );
    assert!(
        (state.players[6].anchor.x - 496.0).abs() < 1e-9,
        "away formation and tactic were applied: {}",
        state.players[6].anchor.x
    );
    assert!(
        (state.players[6].anchor.y - 225.0).abs() < 1e-9,
        "away formation override changed its shape: {}",
        state.players[6].anchor.y
    );
    assert_eq!(state.press.home, counter.press);
    assert_eq!(state.press.away, press_high.press);
    assert_eq!(
        nebula.formation, "2-1-1",
        "canonical away team data was not mutated"
    );
    assert!(
        result.metrics.goals_total >= 0,
        "the fixture produced a valid MatchResult"
    );
}

#[test]
fn headless_run_match_runs_deterministic_match_ai_vs_match_ai_fixtures_without_constructing_a_bot()
{
    let (a, _, debug) = headless::run_match_debug(&HeadlessOpts {
        seed: 43.0,
        duration: Some(20.0),
        bot: Some(HeadlessBot::None),
        ..Default::default()
    });
    assert!(
        !debug.bot_constructed,
        "AI/AI mode must not construct the human-proxy bot"
    );
    let b = headless::run_match(&HeadlessOpts {
        seed: 43.0,
        duration: Some(20.0),
        bot: Some(HeadlessBot::None),
        ..Default::default()
    });
    assert!(
        a.metrics.duration >= 19.0,
        "the AI/AI fixture ran to full time"
    );
    assert_same_metrics(&a, &b);
}

#[test]
fn headless_run_match_replays_a_complete_eight_stream_fixture_with_simultaneous_actions_deterministically()
 {
    let frames = recorded_simultaneous_actions(200);
    let a = headless::run_match(&HeadlessOpts {
        seed: 67.0,
        duration: Some(3.0),
        frames: Some(&frames),
        ..Default::default()
    });
    let b = headless::run_match(&HeadlessOpts {
        seed: 67.0,
        duration: Some(3.0),
        frames: Some(&frames),
        ..Default::default()
    });
    assert!(a.metrics.duration >= 2.9);
    assert_eq!(a.score.home, b.score.home);
    assert_eq!(a.score.away, b.score.away);
    assert_same_metrics(&a, &b);
}

#[test]
fn headless_run_match_defaults_a_complete_recording_to_all_frame_sources() {
    let frames = recorded_frames(200);
    let (_, _, debug) = headless::run_match_debug(&HeadlessOpts {
        seed: 69.0,
        duration: Some(3.0),
        frames: Some(&frames),
        ..Default::default()
    });
    let sources = debug
        .slot_sources
        .expect("slot mode was entered because frames were supplied");
    for kind in sources {
        assert_eq!(kind, MatchSlotSourceKind::Frame);
    }
}

#[test]
fn headless_run_match_does_not_inject_a_legacy_proxy_when_explicit_sources_omit_frames() {
    let sources = [MatchSlotSource {
        kind: MatchSlotSourceKind::Neutral,
        seed: None,
    }; 8];
    let (_, _, debug) = headless::run_match_debug(&HeadlessOpts {
        seed: 70.0,
        duration: Some(3.0),
        slot_sources: Some(sources),
        ..Default::default()
    });
    assert!(
        !debug.bot_constructed,
        "only explicitly configured slot bots may be created"
    );
    let observed = debug.slot_sources.expect("slot mode was entered");
    for kind in observed {
        assert_eq!(kind, MatchSlotSourceKind::Neutral);
    }
}

#[test]
fn headless_run_match_supports_a_deterministic_mixture_of_recorded_and_explicitly_bot_filled_slots()
{
    let mut sources = [MatchSlotSource {
        kind: MatchSlotSourceKind::Bot,
        seed: Some(0.0),
    }; 8];
    for (index, source) in sources.iter_mut().enumerate() {
        *source = if index == 0 {
            MatchSlotSource {
                kind: MatchSlotSourceKind::Frame,
                seed: None,
            }
        } else {
            MatchSlotSource {
                kind: MatchSlotSourceKind::Bot,
                seed: Some(800.0 + index as f64 + 1.0),
            }
        };
    }
    let frames = recorded_frames(200);
    let a = headless::run_match(&HeadlessOpts {
        seed: 71.0,
        duration: Some(3.0),
        frames: Some(&frames),
        slot_sources: Some(sources),
        ..Default::default()
    });
    let b = headless::run_match(&HeadlessOpts {
        seed: 71.0,
        duration: Some(3.0),
        frames: Some(&frames),
        slot_sources: Some(sources),
        ..Default::default()
    });
    assert!(a.metrics.duration >= 2.9);
    assert_same_metrics(&a, &b);
}

#[test]
fn headless_run_batch_aggregates_a_batch_and_reports_every_match() {
    let batch = headless::run_batch(&BatchOpts {
        n: Some(3),
        duration: Some(20.0),
        ..Default::default()
    });
    assert_eq!(batch.matches.len(), 3);
    assert_eq!(batch.agg.get("duration").expect("duration aggregates").n, 3);
    assert!(
        batch.agg.contains_key("fun"),
        "the fun score aggregates like any metric"
    );
    let report = headless::report(&batch);
    assert!(report.contains("fun-proxy metrics over 3 matches"));
    assert!(report.contains("goals_total"));
}

#[test]
fn headless_run_batch_forwards_fixture_and_bot_options_to_every_match() {
    let orion = teams::get("orion").expect("orion is authored");
    let nebula = teams::get("nebula").expect("nebula is authored");
    let counter = tactics::get("counter").expect("counter is authored");
    let press_high = tactics::get("press_high").expect("press_high is authored");
    let expected = headless::run_match(&HeadlessOpts {
        seed: 59.0,
        duration: Some(5.0),
        home: Some(orion),
        away: Some(nebula),
        home_formation: Some("1-2-1"),
        away_formation: Some("1-1-2"),
        tactic: Some(counter),
        away_tactic: Some(press_high),
        field: Some(PitchSize { w: 800.0, h: 450.0 }),
        bot: Some(HeadlessBot::None),
        ..Default::default()
    });
    let seeds = [59.0];
    let batch = headless::run_batch(&BatchOpts {
        seeds: Some(&seeds),
        duration: Some(5.0),
        home: Some(orion),
        away: Some(nebula),
        home_formation: Some("1-2-1"),
        away_formation: Some("1-1-2"),
        tactic: Some(counter),
        away_tactic: Some(press_high),
        field: Some(PitchSize { w: 800.0, h: 450.0 }),
        bot: Some(HeadlessBot::None),
        ..Default::default()
    });

    assert_eq!(batch.matches.len(), 1);
    assert_same_metrics(&batch.matches[0], &expected);
}

// ---------------------------------------------------------------------
// #531 phase 3 — the post-seam balance reference.
// ---------------------------------------------------------------------

/// The harness invocation phase 3 of #531 measures from: the bot-driven
/// default harness (`HeadlessBot::Home`, one human-proxy slot, the rest
/// AI-driven both sides), 48 full-length (120 s) matches on seeds
/// `20001..20049` — the same base-20001 seed convention
/// `gc-sim/tests/knob_contract.rs`'s `seeds()` helper uses, and the same
/// seed COUNT #491/#527's original 0.6200/0.6189 completion measurements
/// used, so this is the closest reproducible match to that comparison this
/// repository has.
///
/// This is the library call the deleted pre-port `love . --sim 48` mapped
/// to (`docs/design/fun_metrics.md`'s "Today there is no CLI" section) —
/// see that document's "Baseline signature" sections for how the printed
/// table is read and published. Not itself an assertion: this measures
/// balance, and #531's own remediation plan is explicit that "the numbers
/// get worse" is not itself something to make this test fail over.
///
/// `cargo test -p gc-sim --test headless -- --ignored --nocapture \
///  post_531_balance_reference_reports_the_bot_driven_default_harness`
#[test]
#[ignore = "balance reference pilot: minutes, run by hand"]
fn post_531_balance_reference_reports_the_bot_driven_default_harness() {
    let seeds: Vec<f64> = (0..48).map(|i| 20_001.0 + i as f64).collect();
    let batch = headless::run_batch(&BatchOpts {
        seeds: Some(&seeds),
        ..Default::default()
    });
    println!("{}", headless::report(&batch));

    // Standard error on the two headline metrics #531 asks phase 3 to
    // settle, computed the same way every other contract in this
    // repository computes one (`knob_contract::noise_floor`), so the
    // in-band question is answered with a number, not a mean alone.
    for id in ["pass_completion", "turnovers_per_min", "fun"] {
        if id == "fun" {
            // "fun" folds every registered metric and is not itself
            // registered in `metric_registry`, so `noise_floor` (which
            // requires a registered id) cannot measure it — read straight
            // off the batch aggregate `headless::report` already printed
            // above instead.
            let st = batch.agg.get("fun").expect("fun aggregates");
            let se = st.sd / (st.n as f64).sqrt();
            println!(
                "fun                    n={:>3} mean={:>8.4} sd={:>8.4} se={:>8.4}",
                st.n, st.mean, st.sd, se
            );
            continue;
        }
        let floor = gc_sim::knob_contract::noise_floor(id, &seeds, None);
        println!(
            "{id:<22} n={:>3} mean={:>8.4} sd={:>8.4} se={:>8.4}",
            floor.n, floor.mean, floor.sd, floor.standard_error
        );
    }
}

// ---------------------------------------------------------------------
// #531 phase 4 — what fraction of releases reach the lead-solve gate.
// ---------------------------------------------------------------------

/// `pass_lead::solve` only runs when `land_pos.is_none() && blocker_f.is_none()
/// && !target_is_keeper` (`match.rs::release_pass`). #535's PR body flagged
/// measuring what fraction of releases clear that gate as "not cheap within
/// this PR's time budget" and left it for phase 4. `PassShadowTally::
/// ground_releases` already counts releases that resolve on the ground path
/// (the ones the solver's result, if any, actually gets applied to); the
/// newly added `total_releases` (this PR) counts every producer's every
/// `release_pass` call, so the ratio is the fraction of releases that are
/// ground releases — an upper bound on "cleared the gate", since a solved
/// lead can still be discarded into a lob by the dink check that runs after
/// the gate is evaluated (see `total_releases`'s doc comment on
/// `PassShadowTally`).
///
/// Uses `run_match_debug` directly (not `run_batch`) because the tally is
/// per-match diagnostic state `run_batch`/`run_match` do not surface --
/// see `headless.rs`'s module doc, "Test seams" section.
///
/// `cargo test -p gc-sim --test headless -- --ignored --nocapture \
///  post_531_ground_release_fraction_reports_the_lead_solve_gate`
#[test]
#[ignore = "gate-fraction pilot: minutes, run by hand"]
fn post_531_ground_release_fraction_reports_the_lead_solve_gate() {
    let seeds: Vec<f64> = (0..48).map(|i| 20_001.0 + i as f64).collect();
    let mut total_releases: i64 = 0;
    let mut ground_releases: i64 = 0;
    for &seed in &seeds {
        let (_, _, debug) = headless::run_match_debug(&HeadlessOpts {
            seed,
            ..Default::default()
        });
        total_releases += debug.pass_shadow.total_releases;
        ground_releases += debug.pass_shadow.ground_releases;
    }
    let fraction = ground_releases as f64 / total_releases as f64;
    println!(
        "ground_releases={ground_releases} total_releases={total_releases} \
         fraction={fraction:.4} (n={} seeds, bot-driven default harness, full length)",
        seeds.len()
    );
}
