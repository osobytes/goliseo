//! #491's presentation-half acceptance criterion: measure the achievable
//! preview-accuracy ceiling across the authored network profiles, rather
//! than trusting the issue's own un-measured 0.90 floor.
//!
//! ## What "preview accuracy" means here
//!
//! While a locally controlled outfielder holds the pass button, the render
//! layer draws a marker over `RenderFrameControl::pass_target` — read
//! straight off `MatchState`, recomputed fresh every tick
//! (`gc_sim::r#match::update_pass_target_outfield`). That value is a pure
//! function of the LOCALLY PREDICTED state, so under rollback it can show a
//! different receiver than the one the corrected, authoritative timeline
//! eventually resolves at release. Preview accuracy is the fraction of
//! releases where those two agree:
//!
//! - **previewed receiver**: the last non-`None` `pass_target` the LOCAL
//!   client's own predicted session showed for that slot before release.
//! - **authoritative receiver**: the last non-`None` `pass_target` the
//!   INDEPENDENT REFERENCE (`RollbackLabRunState::reference`) showed for the
//!   same slot before its own `pass_target` reset because it fired — i.e.
//!   what `r#match::try_pass` actually resolved, since it is the exact same
//!   pure `select_pass_target` call the reference just finished making with
//!   the same aim, one tick earlier.
//!
//! Both quantities are read straight off the two `MatchState`s the
//! `rollback_lab::RollbackLabOptions::observer` seam (#495, `e2ae786`)
//! already hands the observer every tick, mirroring
//! `tests/pass_release_budget.rs`'s use of the same seam. No thread-local
//! hook and no manual re-invocation of `select_pass_target` is needed.
//!
//! ## Why this file builds its own tapes instead of reusing an existing one
//!
//! `select_pass_target` is reached only from the human/bot INPUT path
//! (`r#match::is_human_player`); the match AI picks receivers through
//! `outfield_decision` and never consults the cone (PR #527's own finding).
//! An all-AI match therefore never sets `pass_target` at all —
//! `pass_aim_error` arms on 0 of 60 such matches, per the sim-half worker's
//! own measurement.
//!
//! That rules out an all-AI fixture, but it does not by itself supply a
//! working one: the SLOT-MODE bot fill every existing `rollback_lab` fixture
//! is built from (`slot_input::bot_input`, which backs
//! `determinism_evidence::fixture_tape` and therefore the whole nine-
//! scenario `rollback_validation` matrix `tests/pass_release_budget.rs`
//! already trusts) never sets `pass: true` or `pass_held: true` at all — it
//! shoots, dashes and sprints, but never passes deliberately. Measured
//! directly: the full 7,201-tick OMP-1 fixture contains exactly one `Pass`
//! event in the whole match (a keeper distribution, which does not go
//! through `select_pass_target` either), and `pass_target` is `Some` on
//! zero ticks. So neither the all-AI fixture NOR the existing bot-driven
//! rollback fixture can measure this metric — a third, purpose-built
//! fixture is required, and this file is it.
//!
//! [`build_pass_tape`] hand-scripts ONE local outfielder's hold-then-release
//! pass (`MatchSlotSourceKind::Frame` for its slot) while every other slot
//! keeps running the real `MatchSlotSourceKind::Bot` fill — so the
//! candidates' positions still drift exactly as they would in a genuine
//! bot-driven match, which is what gives rollback something real to
//! mispredict. The aim is swept around the SCORE-computed tie point between
//! two teammates (see [`find_tie_aim_t`]) rather than a geometric midpoint —
//! not every nearby pair of teammates has a reachable tie at all (measured:
//! canonical slots 3/4 never tie across the whole aim range at kickoff;
//! slots 2/3 do), so the tie has to be located, not assumed.

use gc_core::vec2::Vec2;
use gc_data::teams;
use gc_sim::determinism_evidence;
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputFrame, InputSample};
use gc_sim::input_tape::{self, InputTape, InputTapeIdentity};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{self, MatchInput, MatchState, PitchSize};
use gc_sim::passing;
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_lab::{self, RollbackLabObserver, RollbackLabOptions, RollbackLabRunState};
use gc_sim::rollback_session;
use gc_sim::rollback_validation;
use gc_sim::slot_input::{self, MatchSlotSource, MatchSlotSourceKind};
use gc_sim::tuning::Tuning;
use std::cell::RefCell;
use std::rc::Rc;

const SLOTS: usize = 8;
const DT: f64 = fixed_clock::TICK_SECONDS;
/// Short deliberately, not just "under the auto-fire cap": measured
/// directly (see this file's git history), a client's locally predicted
/// position for a remote candidate resyncs with the reference within 2-3
/// ticks of a fresh network-impaired guess, on every authored profile. A
/// 10-tick hold gives that correction time to land BEFORE release, so
/// "last shown locally" is already the corrected value and every case
/// measures a trivial 1.0 -- which is a real fact about a long, deliberate
/// charge, but says nothing about a quick tap-release, which is exactly
/// when a fresh misprediction is still open when the button comes up. This
/// fixture measures the quick-tap case; the PR body reports the long-hold
/// number too, gathered by hand at `HOLD_TICKS = 10` for comparison.
const HOLD_TICKS: i64 = 3;
const POST_TICKS: i64 = 30;

/// Confirms the checked-in rollback fixture that backs the rest of the
/// nine-scenario matrix cannot measure this metric -- the claim this file's
/// module doc makes. Discriminating: this must FAIL the moment
/// `determinism_evidence::fixture_tape` (or the bot fill behind it) starts
/// exercising a deliberate pass, which is exactly the day this file's
/// hand-scripted tape stops being the only option and its module doc goes
/// stale.
#[test]
fn the_existing_bot_driven_fixture_never_exercises_the_soft_cone() {
    let tune = Tuning::new();
    let tape = determinism_evidence::fixture_tape(&tune).expect("fixture tape is valid");
    let (mut state, mut combat) = match_snapshot::restore(&tape.initial);
    let mut any_preview = false;
    for frame in &tape.frames {
        sim_match::step(
            &mut state,
            DT,
            StepInput::Frame(frame),
            combat.as_mut(),
            &tune,
        );
        if state.players.iter().any(|p| p.pass_target.is_some()) {
            any_preview = true;
            break;
        }
    }
    assert!(
        !any_preview,
        "the checked-in bot-driven fixture now exercises pass_target -- this file's \
         hand-scripted tape may no longer be the only fixture that can measure preview \
         accuracy; update the module doc accordingly"
    );
}

/// Interpolate two unit-ish directions and renormalize -- close enough for
/// sweeping an aim between two nearby candidates; it does not need to be an
/// exact angular interpolation.
fn lerp_dir(a: Vec2, b: Vec2, t: f64) -> Vec2 {
    Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t).normalized()
}

fn neutral_input() -> MatchInput {
    slot_input::neutral_match_input()
}

/// Canonical slot indices this file scripts: the local passer (slot 1) and
/// two teammates (slots 2 and 3) whose kickoff geometry has a reachable
/// score tie across the aim range -- see [`fresh_kickoff_state`]'s
/// candidate-selection comment for why not just any nearby pair works.
struct Roster {
    passer_idx: i64,
    candidate_a_idx: i64,
    candidate_b_idx: i64,
    ownership: input_frame::InputOwnership,
    kickoff_a_dir: Vec2,
    kickoff_b_dir: Vec2,
}

fn fresh_kickoff_state(seed: f64) -> (MatchState, Roster) {
    let home = teams::get("nebula").expect("nebula is authored");
    let away = teams::get("orion").expect("orion is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(10.0),
        max_goals: Some(99),
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(ownership.clone()),
    });
    state.kickoff_hold = 0.0;

    let passer_idx = state.slot_players[0].expect("canonical slot 1 is mapped");
    // Canonical slots 2 and 3, NOT 3/4: measured directly (score every pair
    // of home outfielders at kickoff across the whole aim range), slots
    // 3/4's score gap keeps the SAME sign end to end -- one candidate
    // dominates everywhere, no tie exists to be sensitive to. Slots 2/3 DO
    // cross zero, which is required for `find_tie_aim_t` to have anything
    // to find.
    let candidate_a_idx = state.slot_players[1].expect("canonical slot 2 is mapped");
    let candidate_b_idx = state.slot_players[2].expect("canonical slot 3 is mapped");
    assert!(
        !state.players[(passer_idx - 1) as usize].is_keeper,
        "canonical slot 1 must be an outfielder for this fixture to say anything about \
         select_pass_target"
    );
    let passer_pos = state.players[(passer_idx - 1) as usize].pos;
    let a_pos = state.players[(candidate_a_idx - 1) as usize].pos;
    let b_pos = state.players[(candidate_b_idx - 1) as usize].pos;
    state.owner = Some(passer_idx);
    state.ball = passer_pos;
    state.ball_vel = Vec2::default();
    state.ball_z = 0.0;
    state.ball_vz = 0.0;
    state.pickup_cd = 0.0;
    state.block_grace = 0.0;

    let roster = Roster {
        passer_idx,
        candidate_a_idx,
        candidate_b_idx,
        ownership,
        kickoff_a_dir: a_pos.sub(passer_pos),
        kickoff_b_dir: b_pos.sub(passer_pos),
    };
    (state, roster)
}

fn aim_for(roster: &Roster, aim_t: f64) -> Vec2 {
    lerp_dir(roster.kickoff_a_dir, roster.kickoff_b_dir, aim_t)
}

/// Simulate [`HOLD_TICKS`] ticks of a held pass at a fixed `aim_t`, local
/// slot Frame-sourced and every other slot the real bot fill, and return
/// the soft-cone score GAP between the two candidates using the ACTUAL
/// positions and the ACTUAL accumulated `pass_charge` at the tick release
/// would fire on -- i.e. exactly what `r#match::try_pass` would score, not
/// a closed-form estimate from kickoff positions. A closed-form estimate
/// was tried first and was wrong: the passer's own aim also steers their
/// OWN movement while charging (confirmed by direct trace), so both sides
/// of the score move between kickoff and release, and a kickoff-only
/// estimate landed the "tie" well off the boundary the real release
/// resolves against -- which is why an earlier version of this file swept
/// a grid around the WRONG point and measured a flat 1.0 on every profile.
fn simulated_gap(seed: f64, aim_t: f64, tune: &Tuning) -> f64 {
    let (mut state, roster) = fresh_kickoff_state(seed);
    let aim = aim_for(&roster, aim_t);
    let mut sources = [MatchSlotSource {
        kind: MatchSlotSourceKind::Frame,
        seed: None,
    }; SLOTS];
    for (i, source) in sources.iter_mut().enumerate().skip(1) {
        source.kind = MatchSlotSourceKind::Bot;
        source.seed = Some(1_000.0 + i as f64 * 37.0 + seed);
    }
    let mut producer = slot_input::new_producer(sources);
    for tick in 0..HOLD_TICKS {
        let local_input = MatchInput {
            r#move: aim,
            pass_held: true,
            ..neutral_input()
        };
        let mut base_slots = [InputSample::default(); SLOTS];
        base_slots[0] = slot_input::to_sample(&local_input);
        let base_frame = input_frame::new(tick, Some(base_slots)).expect("valid base frame");
        let (materialized, _decisions) =
            slot_input::materialize(&mut producer, &state, &base_frame, None);
        sim_match::step(&mut state, DT, StepInput::Frame(&materialized), None, tune);
    }
    let passer = &state.players[(roster.passer_idx - 1) as usize];
    let passer_pos = passer.pos;
    let charge = passer.pass_charge;
    let range = (charge > 0.12).then(|| {
        tune.value("PASS_RANGE_MIN")
            + charge * (tune.value("PASS_RANGE_MAX") - tune.value("PASS_RANGE_MIN"))
    });
    let a_pos = state.players[(roster.candidate_a_idx - 1) as usize].pos;
    let b_pos = state.players[(roster.candidate_b_idx - 1) as usize].pos;
    let weight = passing::SelectionKnobs::of(tune).angular_weight;
    passing::score(passer_pos, aim, a_pos, range, weight)
        - passing::score(passer_pos, aim, b_pos, range, weight)
}

/// Binary-search `aim_t` for the point where [`simulated_gap`] crosses
/// zero -- see that function's doc for why this must be simulated rather
/// than computed in closed form from kickoff positions.
fn find_tie_aim_t(seed: f64, tune: &Tuning) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    let (mut lo_v, hi_v) = (simulated_gap(seed, lo, tune), simulated_gap(seed, hi, tune));
    if lo_v == 0.0 {
        return lo;
    }
    if hi_v == 0.0 {
        return hi;
    }
    if lo_v.signum() == hi_v.signum() {
        return if lo_v.abs() < hi_v.abs() { 0.0 } else { 1.0 };
    }
    for _ in 0..16 {
        let mid = (lo + hi) / 2.0;
        let mid_v = simulated_gap(seed, mid, tune);
        if mid_v == 0.0 {
            return mid;
        }
        if mid_v.signum() == lo_v.signum() {
            lo = mid;
            lo_v = mid_v;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// Build one hand-scripted tape. The local slot's possession is forced ON
/// THE INITIAL SNAPSHOT, at kickoff, before any frame exists -- the same
/// precedent `rollback_validation::synthetic_goal_tape` sets for hand-built
/// scenarios. That is the ONLY state mutation outside ordinary input this
/// function makes: a tape's `boundary_hashes` come from an independent
/// replay of `initial` through nothing but the frames themselves
/// (`input_tape::new`'s own doc), so any mid-tape mutation not expressed as
/// input would silently desync the tape from what this function actually
/// intended.
///
/// The local slot holds the pass immediately from kickoff, aimed between
/// two teammates who start close together in the shipped formation. Every
/// other slot runs the real `MatchSlotSourceKind::Bot` fill throughout, so
/// their positions are genuinely bot-driven from the first tick --
/// deliberately including the opening acceleration out of a standing
/// start, which is where a "repeat the last known input" remote-prediction
/// model is most likely to be wrong.
///
/// `tie_t` is [`find_tie_aim_t`]'s result for this `seed`, computed ONCE by
/// the caller and reused across every `tie_offset` in the sweep (repeating
/// the search per offset would be redundant -- the tie point does not move
/// with the offset being swept around it). `tie_offset` is added to it and
/// clamped to `[0, 1]`, so the caller sweeps a range of offsets around the
/// boundary the soft cone is actually sensitive to.
fn build_pass_tape(seed: f64, tie_t: f64, tie_offset: f64, tune: &Tuning) -> InputTape {
    let (mut state, roster) = fresh_kickoff_state(seed);
    let aim_t = (tie_t + tie_offset).clamp(0.0, 1.0);
    let aim = aim_for(&roster, aim_t);

    let mut sources = [MatchSlotSource {
        kind: MatchSlotSourceKind::Frame,
        seed: None,
    }; SLOTS];
    for (i, source) in sources.iter_mut().enumerate().skip(1) {
        source.kind = MatchSlotSourceKind::Bot;
        source.seed = Some(1_000.0 + i as f64 * 37.0 + seed);
    }
    let mut producer = slot_input::new_producer(sources);

    let initial = match_snapshot::capture_owned(&state, None);
    let total_ticks = HOLD_TICKS + 1 + POST_TICKS;
    let mut frames: Vec<InputFrame> = Vec::with_capacity(total_ticks as usize);
    for tick in 0..total_ticks {
        let local_input = if tick < HOLD_TICKS {
            MatchInput {
                r#move: aim,
                pass_held: true,
                ..neutral_input()
            }
        } else if tick == HOLD_TICKS {
            MatchInput {
                r#move: aim,
                pass: true,
                pass_held: false,
                ..neutral_input()
            }
        } else {
            neutral_input()
        };
        let mut base_slots = [InputSample::default(); SLOTS];
        base_slots[0] = slot_input::to_sample(&local_input);
        let base_frame = input_frame::new(tick, Some(base_slots)).expect("valid base frame");
        let (materialized, _decisions) =
            slot_input::materialize(&mut producer, &state, &base_frame, None);
        sim_match::step(&mut state, DT, StepInput::Frame(&materialized), None, tune);
        frames.push(materialized);
    }
    let ownership = roster.ownership;

    let identity = InputTapeIdentity {
        tape_version: input_tape::VERSION,
        input_version: input_frame::VERSION,
        snapshot_version: match_snapshot::VERSION,
        build: "gc491-preview-accuracy-v1".to_string(),
        source: "gc491-scripted-pass-preview-v1".to_string(),
        content: "nebula-orion-showcase-content-v1".to_string(),
        tuning: tune.serialize(),
        config: format!("field=960x540;duration=10;max_goals=99;tick_rate=60;aim_t={aim_t}"),
        fixture: "gc491-scripted-pass-preview".to_string(),
        seed,
        tick_rate: fixed_clock::TICK_RATE as i64,
        ownership,
        combat: None,
    };
    input_tape::new(&identity, &initial, &frames, tune).expect("scripted pass tape is well formed")
}

/// Per-slot release-tracking state threaded through the observer closure.
#[derive(Clone, Copy, Debug, Default)]
struct SlotTrack {
    last_client_preview: Option<i64>,
    last_ref_preview: Option<i64>,
}

/// One profile/case's tally.
#[derive(Clone, Copy, Debug, Default)]
struct AccuracyTally {
    measured: i64,
    hits: i64,
    unpreviewed: i64,
}

impl AccuracyTally {
    fn fold(&mut self, other: &AccuracyTally) {
        self.measured += other.measured;
        self.hits += other.hits;
        self.unpreviewed += other.unpreviewed;
    }

    fn accuracy(&self) -> Option<f64> {
        (self.measured > 0).then(|| self.hits as f64 / self.measured as f64)
    }
}

fn run_case(tape: &InputTape, profile_name: &str, network_seed: i64) -> AccuracyTally {
    let tally: Rc<RefCell<AccuracyTally>> = Rc::new(RefCell::new(AccuracyTally::default()));
    let tracks: Rc<RefCell<[SlotTrack; SLOTS]>> =
        Rc::new(RefCell::new([SlotTrack::default(); SLOTS]));

    let tally_obs = Rc::clone(&tally);
    let tracks_obs = Rc::clone(&tracks);

    let observer: RollbackLabObserver = Box::new(move |_tick, state: &RollbackLabRunState| {
        let client = rollback_session::state(&state.session);
        let mut tracks = tracks_obs.borrow_mut();
        let mut tally = tally_obs.borrow_mut();
        for slot in 0..SLOTS {
            if state.sources[slot] != RollbackInputSource::Local {
                continue;
            }
            // Canonical slot index is NOT a player array index: slot 1
            // (index 0) maps to whichever roster position
            // `slot_players` names (a keeper occupies array index 0, so
            // this is load-bearing, not defensive).
            let Some(player_idx) = state.reference.slot_players[slot] else {
                continue;
            };
            let player_index = (player_idx - 1) as usize;
            let client_player = &client.players[player_index];
            let ref_player = &state.reference.players[player_index];
            if client_player.is_keeper || ref_player.is_keeper {
                continue;
            }

            let track = &mut tracks[slot];
            if let Some(t) = client_player.pass_target {
                track.last_client_preview = Some(t);
            }

            let prev_ref = track.last_ref_preview;
            let ref_now = ref_player.pass_target;
            let ref_shot_charging = ref_player.charge > 0.001;

            if prev_ref.is_some() && ref_now.is_none() {
                if ref_shot_charging {
                    track.last_ref_preview = None;
                    track.last_client_preview = None;
                } else {
                    match track.last_client_preview {
                        Some(shown) => {
                            tally.measured += 1;
                            if Some(shown) == prev_ref {
                                tally.hits += 1;
                            }
                        }
                        None => tally.unpreviewed += 1,
                    }
                    track.last_ref_preview = None;
                    track.last_client_preview = None;
                }
            } else if ref_now.is_some() {
                track.last_ref_preview = ref_now;
            }
        }
    });

    let options = RollbackLabOptions {
        profile_name: Some(profile_name.to_string()),
        network_seed: Some(network_seed),
        sources: Some(rollback_validation::sources()),
        prevalidated_tape: true,
        observer: Some(observer),
        ..Default::default()
    };
    let result = rollback_lab::run(tape.clone(), options);
    assert!(
        result.success,
        "{profile_name}/{network_seed}: campaign did not converge"
    );
    *tally.borrow()
}

/// Independent base seeds. Each is expanded across the whole
/// [`TIE_OFFSETS`], so the total tape count is
/// `SEEDS.len() * TIE_OFFSETS.len()`.
const SEEDS: &[f64] = &[9_001.0, 9_002.0, 9_003.0, 9_004.0, 9_005.0];
/// Offsets from the SCORE-computed tie point (see [`find_tie_aim_t`]),
/// dense near zero -- exactly where a small rollback-induced position error
/// can flip the winner -- and a couple of wider points so the sample is not
/// entirely boundary cases.
const TIE_OFFSETS: &[f64] = &[-0.2, -0.02, -0.005, 0.0, 0.005, 0.02, 0.2];

fn build_tapes(tune: &Tuning) -> Vec<InputTape> {
    let mut tapes = Vec::with_capacity(SEEDS.len() * TIE_OFFSETS.len());
    for &seed in SEEDS {
        let tie_t = find_tie_aim_t(seed, tune);
        for &offset in TIE_OFFSETS {
            tapes.push(build_pass_tape(seed, tie_t, offset, tune));
        }
    }
    tapes
}

/// The measurement the PR body reports, not merely asserts: preview
/// accuracy's achievable ceiling per authored network profile.
#[test]
fn preview_accuracy_across_network_profiles() {
    let tune = Tuning::new();
    let config = rollback_validation::config();

    let tapes = build_tapes(&tune);

    let mut overall = AccuracyTally::default();
    for &profile_name in config.full_profiles {
        let mut profile_tally = AccuracyTally::default();
        for &network_seed in config.network_seeds {
            for tape in &tapes {
                profile_tally.fold(&run_case(tape, profile_name, network_seed));
            }
        }
        println!(
            "GC_PREVIEW_ACCURACY|profile={profile_name}|seeds={:?}|measured={}|hits={}\
             |unpreviewed={}|accuracy={}",
            config.network_seeds,
            profile_tally.measured,
            profile_tally.hits,
            profile_tally.unpreviewed,
            profile_tally
                .accuracy()
                .map_or_else(|| "n/a".to_string(), |a| format!("{a:.4}"))
        );
        assert!(
            profile_tally.measured > 0,
            "{profile_name}: no previewed release was observed -- the sweep did not arm the \
             metric"
        );
        overall.fold(&profile_tally);
    }
    assert!(
        overall.measured >= tapes.len() as i64,
        "expected at least one measured release per sweep point across the whole run"
    );
}
