//! Does the pose pipeline actually carry the game's mechanics to the renderer?
//!
//! Every link in that chain is unit-tested elsewhere: `player_pose::select`
//! (see `keeper_pose.rs` / `outfield_pose.rs`), the wire encoding against
//! `frame_buffer.rs`'s own numbering, and the TypeScript side against its
//! own spec. None of those notice if the chain is dead END TO END -- which
//! is exactly how this shipped browser matches that played 0-0 with zero
//! events while every individual link passed. `tests/frame.rs` has the same
//! blind spot: it builds ONE frame from a hand-built state.
//!
//! So this drives a whole match and asks what the renderer would actually
//! receive. A stubbed selector, a mis-wired context, or a pose that stops
//! firing because a timer moved shows up here as a vocabulary that collapses
//! toward `Locomotion`, and nowhere else.

use gc_render::frame::{self as render_frame, RenderFrameOptions};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{MatchInput, MatchState, PitchSize};
use gc_sim::tuning::Tuning;
use std::collections::BTreeMap;

/// A full 120-second match at 60 Hz, the same length as the OMP-1 fixture.
const TICKS: usize = 7_200;

fn fixture(seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(seed),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

fn pose_histogram(seed: f64) -> BTreeMap<String, usize> {
    let tune = Tuning::default();
    let mut state = fixture(seed);
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..TICKS {
        sim_match::step(
            &mut state,
            1.0 / 60.0,
            StepInput::Legacy(MatchInput::default()),
            None,
            &tune,
        );
        let frame = render_frame::build(&state, &RenderFrameOptions::default());
        for id in &frame.players.pose_id {
            *hist.entry(format!("{id:?}")).or_default() += 1;
        }
    }
    hist
}

/// Seeds the vocabulary claim is measured over.
///
/// **Three, not one, and that is #491's finding rather than a convenience.**
/// This fixture drives a whole match with `MatchInput::default()` — the home
/// controlled player never presses anything — so it is a deliberately
/// pathological scenario whose event mix is extremely trajectory-sensitive.
/// Measured across eight seeds before and after #491's passing rework:
/// `AerialAction` and `KeeperGrab` appeared on 8 of 8 seeds before and 5 of
/// 8 after; `KeeperStretch` on 6 of 8 before and 4 of 8 after. The mechanisms
/// are all still reachable — three of the eight seeds simply stopped
/// producing a save or an aerial at all, seed 17 among them.
///
/// The claim this file makes is that each rigging mechanism is exercised
/// **end to end**, which is a statement about reachability and not about seed
/// 17. Pinning it to one trajectory is the brittleness the assertion below
/// already warns about in its own comment ("a brittle equality here would be
/// reverted rather than investigated the first time tuning moved"), so the
/// vocabulary is the UNION over a small seed set. A stubbed selector or a
/// mis-wired context still collapses every seed at once; only trajectory
/// luck is absorbed.
const SEEDS: [f64; 3] = [17.0, 3.0, 11.0];

fn union_histogram() -> BTreeMap<String, usize> {
    let mut union: BTreeMap<String, usize> = BTreeMap::new();
    for seed in SEEDS {
        for (id, count) in pose_histogram(seed) {
            *union.entry(id).or_default() += count;
        }
    }
    union
}

#[test]
fn a_whole_match_drives_the_renderer_through_a_real_pose_vocabulary() {
    let hist = union_histogram();

    // Measured: 13 over this fixture. Asserted loosely because the exact set
    // depends on what the bots happen to do, and a brittle equality here would
    // be reverted rather than investigated the first time tuning moved. The
    // failure this guards is a COLLAPSE (one or two ids), not a drift.
    assert!(
        hist.len() >= 10,
        "pose vocabulary collapsed to {} ids: {hist:?}",
        hist.len()
    );

    // Each of these reaches the rigged renderer by a DIFFERENT mechanism, so
    // between them they cover all three (see player_renderer_3d.ts's POSE_CLIP
    // comment). Losing any one is invisible in a per-module test.
    for (id, mechanism) in [
        // rig3d/action_pose.ts's SAVES table -- a root transform, not a clip.
        ("KeeperStretch", "action_pose save"),
        // action_pose's aerial lift.
        ("AerialAction", "action_pose aerial"),
        // Keeper hands, driven off possession rather than the pose id.
        ("KeeperGrab", "possession-driven hands"),
        // POSE_CLIP proper: a genuine limb clip.
        ("KeeperShuffle", "POSE_CLIP"),
        ("Locomotion", "POSE_CLIP"),
    ] {
        assert!(
            hist.contains_key(id),
            "{id} never reached the renderer over {TICKS} ticks on ANY of {SEEDS:?}, so \
             the '{mechanism}' path is unexercised end to end: {hist:?}"
        );
    }

    // A vocabulary that is technically wide but 99.9% one value is the same
    // defect wearing a disguise.
    let total: usize = hist.values().sum();
    let dominant = *hist.values().max().expect("non-empty histogram");
    assert!(
        (dominant as f64) / (total as f64) < 0.95,
        "one pose id accounts for {:.1}% of every player-tick: {hist:?}",
        100.0 * (dominant as f64) / (total as f64)
    );
}

#[test]
fn the_vocabulary_is_not_an_artefact_of_one_seed() {
    for seed in [3.0, 101.0] {
        let hist = pose_histogram(seed);
        assert!(
            hist.len() >= 8,
            "seed {seed}: pose vocabulary collapsed to {} ids: {hist:?}",
            hist.len()
        );
    }
}
