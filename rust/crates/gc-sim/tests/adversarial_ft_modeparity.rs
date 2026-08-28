//! Adversarial coverage for the "shoot on first touch" feature (#623),
//! dimension: INPUT-MODE AND RE-SIMULATION PARITY.
//!
//! Every legacy `first_touch.rs` test drives `StepInput::Legacy` — a
//! non-slot-mode fixture with one human-controlled player. That is not the
//! path a real online match runs on: online play is `StepInput::Frame`
//! through `input_ownership`/slot mode, and rollback resimulates a tail of
//! ticks from a restored snapshot whenever a late-arriving authoritative
//! input disagrees with a prediction. A mechanism proven correct only on
//! the legacy path is unproven on the path players actually experience
//! online, and unproven under resimulation, which has its own failure mode
//! (a source of nondeterminism — uninitialized state, a stray timestamp,
//! iteration over an unordered collection — that a single straight-line run
//! can never surface, because it never runs the same ticks twice).
//!
//! Three cases, matching the assignment:
//! 1. THE BIG ONE — the staged first-touch scenario built and driven
//!    entirely through slot mode (`input_ownership`, `StepInput::Frame`),
//!    with the shot held via the real `MatchInput -> InputSample` quantizer
//!    (`slot_input::to_sample`) rather than a hand-built bitmask — proving
//!    the event fires, denies possession, and (aftermath) the resulting
//!    shot survives 30+ held ticks exactly as it does on the legacy path.
//! 2. Snapshot/restore safety: capture mid-approach, run the live match
//!    through the first-touch event and 30 ticks of aftermath, then
//!    restore from the earlier snapshot and replay the identical recorded
//!    frames — the two runs must land on a byte-identical final state
//!    (`match_snapshot::hash_canonical`), not merely "close".
//! 3. Slot-mode keeper exclusion: a keeper can never be mapped to an input
//!    slot at all (`match.rs`'s `new` asserts it), so a back-pass to the
//!    keeper is driven through a real `InputFrame` and must still resolve
//!    with the feet — parity with `first_touch.rs`'s legacy keeper case,
//!    confirming the `is_keeper` guard in `resolve_first_touch_shot` is not
//!    accidentally bypassable by the slot-routing branch in
//!    `first_touch_requested`.

use gc_core::vec2::Vec2;
use gc_sim::aerial::AerialOutcome;
use gc_sim::input_frame::{self, InputFrame, InputSample};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{self, MatchEventKind, MatchInput, MatchState, PitchSize, Team};
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

const DT: f64 = 1.0 / 60.0;
const RECEIVER_POS: Vec2 = Vec2 { x: 700.0, y: 200.0 };

/// A slot-mode fixture: same nebula/orion fixture as `first_touch.rs`, but
/// with `input_ownership` populated so `s.slot_mode` is true and the
/// simulation requires `StepInput::Frame` — the online path.
fn new_match_seeded_slot(seed: f64) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let ownership = sim_match::ownership_for_teams(home, away, None);
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
        input_ownership: Some(ownership),
    })
}

/// Identical staging to `first_touch.rs`'s `stage_arrival`: `receiver`
/// (one-based) is the designated receiver of a pass arriving at
/// `ball_speed` from the east, already inside possession reach, everyone
/// else parked far away. Kept as an exact copy — one new file, no shared
/// helper module — rather than a diverging near-copy.
fn stage_arrival(s: &mut MatchState, receiver: i64, receiver_pos: Vec2, ball_speed: f64) {
    for (i, p) in s.players.iter_mut().enumerate() {
        let row = 40.0 + 30.0 * i as f64;
        p.pos = if p.team == Team::Home {
            Vec2::new(90.0, row)
        } else {
            Vec2::new(480.0, row)
        };
        p.vel = Vec2::new(0.0, 0.0);
        p.run_vel = Vec2::new(0.0, 0.0);
        p.receive_timer = 0.0;
    }
    let rp = &mut s.players[(receiver - 1) as usize];
    rp.pos = receiver_pos;
    rp.receive_timer = 1.0;
    rp.facing = Vec2::new(1.0, 0.0);
    s.owner = None;
    s.kickoff_hold = 0.0;
    s.pickup_cd = 0.0;
    s.aerial_lock = 0.0;
    s.ball = receiver_pos.add(Vec2::new(10.0, 0.0));
    s.ball_vel = Vec2::new(-ball_speed, 0.0);
    s.ball_z = 0.0;
    s.ball_vz = 0.0;
    s.ball_spin = 0.0;
}

/// The legacy `MatchInput` shape a held space bar produces (jockey AND
/// aerial_strike together — see `first_touch.rs`'s `strike_input` doc
/// comment for why both bits matter to the real input pipeline even though
/// `aerial::strike_requested` only reads `aerial_strike`).
fn strike_input(aim: Vec2) -> MatchInput {
    MatchInput {
        r#move: aim,
        jockey: true,
        aerial_strike: Some(true),
        aerial_acrobatic: Some(false),
        ..MatchInput::default()
    }
}

/// One canonical `InputFrame` with `slot` (one-based) holding the strike at
/// `aim` and every other slot neutral. Routes the held input through
/// `slot_input::to_sample`, the SAME quantizer the online client's real
/// input pipeline uses, rather than hand-assembling the held-bit mask —
/// this is the house way to build a slot-mode frame that models a real
/// held control, not merely a frame that happens to flip bit 32.
fn strike_frame(tick: i64, slot: i64, aim: Vec2) -> InputFrame {
    let mut slots = [InputSample::default(); 8];
    slots[(slot - 1) as usize] = slot_input::to_sample(&strike_input(aim));
    input_frame::new(tick, Some(slots)).expect("canonical slot sample always validates")
}

/// A fully neutral `InputFrame` — every slot at rest, nobody holding
/// anything.
fn neutral_frame(tick: i64) -> InputFrame {
    input_frame::neutral(tick).expect("neutral frame is always valid")
}

fn first_touch_event(s: &MatchState) -> Option<&gc_sim::match_snapshot::MatchEvent> {
    s.events
        .iter()
        .find(|e| e.kind == MatchEventKind::FirstTouchShot)
}

// ---------------------------------------------------------------------
// 1. THE BIG ONE: does the staged first-touch scenario fire through slot
//    mode the same way it fires through the legacy path?
// ---------------------------------------------------------------------

#[test]
fn a_slot_owned_receiver_holding_strike_fires_the_first_touch_through_a_real_input_frame() {
    // Same seed-scan-for-Clean house pattern as first_touch.rs's own
    // "a_receiver_holding_strike_shoots_first_time_instead_of_trapping" —
    // this is that exact test, ported to the slot-mode/online path.
    let aim = Vec2::new(0.0, -1.0);
    let tune = Tuning::new();
    for seed in 0..40 {
        let mut s = new_match_seeded_slot(seed as f64);
        let receiver = s.slot_players[0].expect("slot 1 is always mapped to a home outfielder");
        stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
        s.players[(receiver - 1) as usize].volley_skill = 1.0;

        let tick = s.input_tick;
        let frame = strike_frame(tick, 1, aim);
        sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);

        let Some(event) = first_touch_event(&s) else {
            panic!(
                "a slot-owned receiver holding strike must attempt the first-touch shot \
                 through a real InputFrame, exactly as the legacy path does"
            );
        };
        assert_eq!(
            s.owner, None,
            "a first-touch shot must never grant possession in slot mode either"
        );
        if event.outcome != Some(AerialOutcome::Clean) {
            continue;
        }

        let dir = s.ball_vel.normalized();
        assert!(
            dir.x * aim.x + dir.y * aim.y > 0.995,
            "a clean slot-mode first touch must follow the held aim (got {dir:?})"
        );
        let launch_speed = s.ball_vel.length();
        assert!(
            launch_speed > 250.0,
            "a clean one-timer must still outpace the pass that fed it in slot mode"
        );

        // Aftermath: the player would still be holding the same button.
        // Keep feeding the identical held InputFrame for 40 more ticks and
        // watch what a player would actually see — the ball keeps moving
        // away under the shot's own pace, nobody re-claims it, and no
        // stray event (Block, a second FirstTouchShot, a possession grant)
        // appears along the way.
        for _ in 0..40 {
            let tick = s.input_tick;
            let frame = strike_frame(tick, 1, aim);
            sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);
            assert_eq!(
                s.owner, None,
                "over the following second nobody may claim the struck ball back \
                 as if the shot had never left"
            );
            assert!(
                !s.events.iter().any(|e| e.kind == MatchEventKind::Block),
                "the striker's own body must not swallow the release in slot mode either"
            );
        }
        assert!(
            s.ball_vel.length() > launch_speed * 0.3,
            "40 ticks later the shot must still be carrying real pace, not have been \
             quietly zeroed out somewhere on the slot-mode path"
        );
        return;
    }
    panic!("no seed in 0..40 produced a Clean slot-mode first touch at volley skill 1.0");
}

// ---------------------------------------------------------------------
// 2. Rollback resimulation safety: restore from an earlier snapshot and
//    replay the identical held input — must land byte-identical to the
//    live run that never restored at all.
// ---------------------------------------------------------------------

#[test]
fn restoring_five_ticks_before_the_first_touch_and_replaying_reaches_the_identical_outcome() {
    let aim = Vec2::new(0.0, -1.0);
    let tune = Tuning::new();
    let seed = 11.0;
    let mut s = new_match_seeded_slot(seed);
    let receiver = s.slot_players[0].expect("slot 1 is always mapped to a home outfielder");
    stage_arrival(&mut s, receiver, RECEIVER_POS, 250.0);
    s.players[(receiver - 1) as usize].volley_skill = 1.0;
    // Push the ball out further than the default staged 10 px so the
    // approach genuinely spans several ticks: the snapshot below must be
    // captured strictly BEFORE arrival, not merely before the resulting
    // event log entry on the very same tick.
    s.ball.x += 250.0 * DT * 24.0;

    // Pre-roll five ticks of the held input. The scenario is built to keep
    // the ball outside pickup reach for well past this window (asserted
    // below) so "5 ticks before arrival" is actually true, not incidental.
    for _ in 0..5 {
        let tick = s.input_tick;
        let frame = strike_frame(tick, 1, aim);
        sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);
        assert!(
            first_touch_event(&s).is_none(),
            "test setup invariant: the ball must not have arrived during the pre-roll, \
             or the snapshot below would not actually precede the event"
        );
    }
    let snapshot_before_arrival = match_snapshot::capture(&s, None);

    // Live run: keep holding the same input, recording every frame, until
    // the first-touch event fires, then 30 more ticks of aftermath with
    // the same held input.
    let mut frames_after_snapshot = Vec::new();
    let mut fired = false;
    for _ in 0..200 {
        let tick = s.input_tick;
        let frame = strike_frame(tick, 1, aim);
        frames_after_snapshot.push(frame);
        sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);
        if first_touch_event(&s).is_some() {
            fired = true;
            break;
        }
    }
    assert!(
        fired,
        "the staged approach must reach the first-touch event within budget \
         (seed {seed} desyncs the fixture, or friction on the flight slowed the \
         ball more than the added lead accounted for)"
    );
    for _ in 0..30 {
        let tick = s.input_tick;
        let frame = strike_frame(tick, 1, aim);
        frames_after_snapshot.push(frame);
        sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);
    }
    let live_final_ball_vel = s.ball_vel;
    let live_final_owner = s.owner;
    let live_final_hash = match_snapshot::hash_canonical(&match_snapshot::capture(&s, None));

    // Restore-and-resimulate: a rollback correction landing exactly here
    // replays the SAME recorded frames from the SAME restored point. A
    // player must never be able to tell the difference — the resimulated
    // outcome (ball flight, event log, ownership, and every other hashed
    // field) must be byte-identical to the run that never rewound at all.
    let (mut s2, combat2) = match_snapshot::restore(&snapshot_before_arrival);
    assert!(combat2.is_none(), "this fixture never carries combat state");
    for frame in &frames_after_snapshot {
        sim_match::step(&mut s2, DT, StepInput::Frame(frame), None, &tune);
    }
    assert_eq!(
        s2.ball_vel, live_final_ball_vel,
        "restore-and-resimulate must reproduce the exact same ball velocity, \
         not merely a close one"
    );
    assert_eq!(
        s2.owner, live_final_owner,
        "restore-and-resimulate must reproduce the exact same possession outcome"
    );
    let resimulated_hash = match_snapshot::hash_canonical(&match_snapshot::capture(&s2, None));
    assert_eq!(
        resimulated_hash, live_final_hash,
        "a first-touch shot resimulated from an earlier snapshot must reach a \
         byte-identical final state to the run that never restored — any \
         divergence here is exactly the desync shape rollback exists to prevent"
    );
}

// ---------------------------------------------------------------------
// 3. Slot-mode keeper exclusion: a keeper can never be a slot player at
//    all, so this drives the legacy keeper case through a real InputFrame
//    and confirms the exclusion is not bypassable via the slot-routing
//    branch in `first_touch_requested`.
// ---------------------------------------------------------------------

#[test]
fn a_slot_mode_keeper_receiving_a_back_pass_never_first_touches_it() {
    let mut s = new_match_seeded_slot(3.0);
    let keeper = (1..=s.players.len() as i64)
        .find(|&i| {
            let p = &s.players[(i - 1) as usize];
            p.team == Team::Home && p.is_keeper
        })
        .expect("home keeper");
    // The keeper is never a slot player — `match.rs`'s `new` asserts a
    // keeper cannot be mapped to an input slot, so this is a structural
    // property of the fixture, not an assumption this test has to enforce
    // itself.
    assert_eq!(
        s.slot_for_player[(keeper - 1) as usize],
        None,
        "test setup invariant: a keeper must never carry a slot mapping"
    );
    let keeper_pos = s.players[(keeper - 1) as usize].pos;
    stage_arrival(&mut s, keeper, keeper_pos, 200.0);

    // Even with the AI range covering the whole pitch, and even though
    // `is_human_player` in slot mode is true for every SLOT-mapped player
    // regardless of what drives that slot, the keeper carries no slot
    // mapping at all -- so it takes the AI branch of
    // `first_touch_requested`, which the `is_keeper` guard at the top of
    // `resolve_first_touch_shot` short-circuits before it is even
    // evaluated. A fully neutral frame (nobody on the pitch holding
    // anything) is enough to prove the exclusion does not depend on what
    // any human happens to be pressing this tick.
    let mut tune = Tuning::new();
    tune.set("AI_FIRST_TOUCH_RANGE", 2000.0);
    let tick = s.input_tick;
    let frame = neutral_frame(tick);
    sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);

    assert_eq!(first_touch_event(&s), None);
    assert_eq!(
        s.owner,
        Some(keeper),
        "a keeper meeting a teammate's pass takes it with the feet in slot mode too"
    );

    // Aftermath: keep stepping with nobody pressing anything. An
    // AI-controlled keeper (never slot-mapped, so always AI-driven here)
    // legitimately distributes the ball on its own timer, which routinely
    // leaves it owner-less in flight for a few ticks exactly like any
    // other release (pass, shot, punt) — that is ordinary keeper
    // behaviour, not a first-touch concern, so this does NOT assert
    // anything about who holds the ball afterwards or whether it is briefly
    // loose. What the feature actually claims is narrower and holds no
    // matter what the keeper's own AI does next: the keeper's OWN
    // reception of the back-pass must never retroactively, or belatedly,
    // manufacture a first-touch attempt.
    for _ in 0..30 {
        let tick = s.input_tick;
        let frame = neutral_frame(tick);
        sim_match::step(&mut s, DT, StepInput::Frame(&frame), None, &tune);
        assert_eq!(
            first_touch_event(&s),
            None,
            "the keeper's reception must never retroactively turn into a first-touch attempt"
        );
    }
}
