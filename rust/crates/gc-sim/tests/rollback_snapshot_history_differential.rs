//! Differential test against reference vectors captured from the Lua
//! implementation this simulation was originally validated against (README
//! rule 5.9, `tools/lua_reference/README.md`), required for
//! `rollback_snapshot_history`: it is the ring the rollback re-simulation
//! restores from, and an off-by-one in its ring index or eviction floor is a
//! desync, not a cosmetic difference.
//!
//! `tests/fixtures/rollback_snapshot_history_lua_reference.txt` is the
//! captured stdout of running the real Lua `sim/rollback_snapshot_history.lua`
//! (via `sim/match.lua` for a real fixture, plus its `sim/match_snapshot.lua`
//! dependency) under headless `love` (no display, no `xvfb`), via a scratch
//! `conf.lua`/`main.lua` harness built per that README (not committed —
//! scratch dirs are session-local). This file reconstructs the identical
//! sequence of calls in Rust and asserts every captured line matches.
//!
//! Every captured value is a boolean, an integer, `nil`, or a plain ASCII
//! status/error-code string — never a float — so plain equality is already
//! bit-exact comparison.
//!
//! `store`/`status`/`truncate_after`/eviction never read anything from the
//! `MatchState` payload except `input_tick` (see the module doc comment on
//! [`gc_sim::rollback_snapshot_history`]), so this reconstructs the scenario
//! against the same hand-built `MatchState` fixture
//! `tests/rollback_snapshot_history.rs` uses rather than the real Lua
//! `match.new` fixture the harness used — the ring's behavior is a pure
//! function of the tick sequence and the configured window, not of what the
//! snapshot payload contains.
//!
//! ## What this covers
//!
//! **Scenario A** — the default window (`max_rollback_ticks = 30`,
//! `capacity = 31`): push ticks `0..=40` (past a full wrap of the 31-slot
//! ring), read back across the whole window, the exact eviction floor
//! (`tick == oldest_supported_tick`, retained; `tick == floor - 1`,
//! evicted), and a real ring-slot reuse at tick 41 (`41 % 31 == 10 == 10 %
//! 31`) that evicts tick 10's entry via actual wraparound rather than a
//! window-floor sweep alone.
//!
//! **Scenario B** — a small window (`max_rollback_ticks = 3`, `capacity =
//! 4`): sparse ticks that force eviction on almost every store, a ring gap
//! that must read `missing` rather than `outside_window`, `truncate_after`
//! shortening the retained tail, and all three [`RollbackSnapshotHistoryErrorCode`]
//! branches (`outside_window` on a stale store, `missing` and
//! `outside_window` on two different rejected truncations).

use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::match_snapshot::{
    self, ByTeam, MatchPlayer, MatchSnapshot, MatchState, PitchSize, Rect,
};
use gc_sim::possession_transition::TransitionWindows;
use gc_sim::rollback_snapshot_history::{self, RollbackSnapshotLookupStatus};
use gc_sim::{outfield_decision, outfield_press, possession_transition};
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/rollback_snapshot_history_lua_reference.txt");

fn reference() -> IndexMap<&'static str, &'static str> {
    FIXTURE
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("malformed fixture line: {line}"));
            (key, value)
        })
        .collect()
}

fn expect<'a>(reference: &IndexMap<&'a str, &'a str>, key: &str) -> &'a str {
    reference
        .get(key)
        .unwrap_or_else(|| panic!("missing reference value for {key}"))
}

fn status_wire(status: RollbackSnapshotLookupStatus) -> &'static str {
    match status {
        RollbackSnapshotLookupStatus::Present => "present",
        RollbackSnapshotLookupStatus::Retained => "retained",
        RollbackSnapshotLookupStatus::Missing => "missing",
        RollbackSnapshotLookupStatus::OutsideWindow => "outside_window",
    }
}

fn error_code_wire(
    code: rollback_snapshot_history::RollbackSnapshotHistoryErrorCode,
) -> &'static str {
    use rollback_snapshot_history::RollbackSnapshotHistoryErrorCode::*;
    match code {
        Malformed => "malformed",
        OutsideWindow => "outside_window",
        Missing => "missing",
    }
}

#[allow(clippy::too_many_arguments)]
fn make_player(
    id: &str,
    team: match_snapshot::Team,
    is_keeper: bool,
    x: f64,
    y: f64,
) -> MatchPlayer {
    MatchPlayer {
        id: id.to_string(),
        name: format!("{id}_name"),
        team,
        pos: gc_core::vec2::Vec2::new(x, y),
        vel: gc_core::vec2::Vec2::new(0.0, 0.0),
        run_vel: gc_core::vec2::Vec2::new(0.0, 0.0),
        facing: gc_core::vec2::Vec2::new(
            if team == match_snapshot::Team::Home {
                1.0
            } else {
                -1.0
            },
            0.0,
        ),
        anchor: gc_core::vec2::Vec2::new(x, y),
        species_id: "human_base".to_string(),
        owned_verb: gc_data::species::SimVerb::None,
        move_speed: 180.0,
        shot_speed: 500.0,
        dribble: 0.5,
        strength: 0.5,
        first_touch: 0.5,
        header_skill: 0.5,
        volley_skill: 0.5,
        bicycle_skill: 0.5,
        scan_rate: 0.5,
        composure: 0.5,
        outfield_decision: outfield_decision::new_state(None),
        is_keeper,
        radius: 12.0,
        dash_cd: 0.0,
        dodge_cd: 0.0,
        dodge_timer: 0.0,
        dodge_dir: gc_core::vec2::Vec2::new(0.0, 0.0),
        reach: if is_keeper { 30.0 } else { 0.0 },
        handling: if is_keeper { 0.5 } else { 0.0 },
        keeper_aggression: if is_keeper { 40.0 } else { 0.0 },
        keeper_anticipation: if is_keeper { 0.5 } else { 0.0 },
        keeper_state: gc_sim::keeper::KeeperBehaviorState::Base,
        keeper_state_timer: 0.0,
        keeper_release_state: None,
        keeper_release_motion: 0.0,
        keeper_release_kind: None,
        keeper_release_depth: 0.0,
        keeper_set: 0.0,
        dive_timer: 0.0,
        dive_dir: gc_core::vec2::Vec2::new(0.0, 0.0),
        dive_delay: 0.0,
        dive_target: None,
        keeper_get_up_timer: 0.0,
        hold_timer: 0.0,
        feet_ball: false,
        slide_timer: 0.0,
        slide_dir: gc_core::vec2::Vec2::new(0.0, 0.0),
        slide_vel: 0.0,
        tackle_timer: 0.0,
        tackle_cd: 0.0,
        stun_timer: 0.0,
        grab_timer: 0.0,
        throw_timer: 0.0,
        receive_timer: 0.0,
        sprint_meter: 1.0,
        sprint_dur: 3.0,
        sprinting: false,
        save_pending: None,
        save_timer: 0.0,
        save_vx: 0.0,
        save_style: None,
        save_tip_emitted: false,
        settle_timer: 0.0,
        header_cd: 0.0,
        aerial_timer: 0.0,
        aerial_style: None,
        aerial_outcome: None,
        aerial_jump: 0.0,
        aerial_recovery: 0.0,
        charge: 0.0,
        pass_charge: 0.0,
        pass_target: None,
        pass_intent: gc_sim::pass_intent::new_state(),
        windup_timer: 0.0,
        windup_shot: None,
        jockey_timer: 0.0,
    }
}

fn make_players() -> Vec<MatchPlayer> {
    use match_snapshot::Team::{Away, Home};
    vec![
        make_player("h_keeper", Home, true, 20.0, 270.0),
        make_player("h1", Home, false, 200.0, 150.0),
        make_player("h2", Home, false, 200.0, 390.0),
        make_player("h3", Home, false, 400.0, 200.0),
        make_player("h4", Home, false, 400.0, 340.0),
        make_player("a_keeper", Away, true, 940.0, 270.0),
        make_player("a1", Away, false, 760.0, 150.0),
        make_player("a2", Away, false, 760.0, 390.0),
        make_player("a3", Away, false, 560.0, 200.0),
        make_player("a4", Away, false, 560.0, 340.0),
    ]
}

fn new_state() -> MatchState {
    MatchState {
        field: PitchSize { w: 960.0, h: 540.0 },
        goal_home: Rect {
            x: 0.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        goal_away: Rect {
            x: 950.0,
            y: 200.0,
            w: 10.0,
            h: 140.0,
        },
        players: make_players(),
        ball: gc_core::vec2::Vec2::new(480.0, 270.0),
        ball_vel: gc_core::vec2::Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner: None,
        controlled: 2,
        human_controlled: false,
        score: ByTeam { home: 0, away: 0 },
        time_left: 120.0,
        max_goals: 3,
        finished: false,
        pickup_cd: 0.0,
        press: ByTeam { home: 1, away: 1 },
        marking: ByTeam {
            home: MarkingConfig {
                scheme: MarkingScheme::Hybrid,
                man_marks: 1,
                standoff: 32.0,
                compactness: 0.5,
                support: 0.5,
            },
            away: MarkingConfig {
                scheme: MarkingScheme::Zonal,
                man_marks: 0,
                standoff: 40.0,
                compactness: 0.6,
                support: 0.4,
            },
        },
        marks: ByTeam {
            home: vec![None; 10],
            away: vec![None; 10],
        },
        outfield_press: ByTeam {
            home: outfield_press::new_state(),
            away: outfield_press::new_state(),
        },
        transition_windows: TransitionWindows {
            home: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
            away: TransitionConfig {
                counterpress: 4.0,
                counterattack: 3.0,
            },
        },
        transition: possession_transition::new_state(),
        formation: ByTeam {
            home: "2-1-1".to_string(),
            away: "2-1-1".to_string(),
        },
        ball_spin: 0.0,
        rng: gc_core::rng::seed(4242.0),
        block_grace: 0.0,
        aerial_lock: 0.0,
        kickoff_hold: 0.0,
        events: Vec::new(),
        slot_mode: false,
        input_ownership: None,
        slot_players: vec![None; 8],
        slot_for_player: vec![None; 10],
        input_tick: 0,
        unsupported_reason: None,
    }
}

fn boundary(state: &mut MatchState, tick: i64) -> MatchSnapshot {
    state.input_tick = tick;
    match_snapshot::capture(state, None)
}

#[test]
fn rollback_snapshot_history_matches_lua_reference_across_wraparound_and_out_of_range_ticks() {
    let reference = reference();

    // Scenario A.
    let mut history = rollback_snapshot_history::new(None);
    let mut state = new_state();
    for tick in 0..=40i64 {
        let result =
            rollback_snapshot_history::store(&mut history, &boundary(&mut state, tick)).unwrap();
        assert_eq!(
            result.replaced.to_string(),
            expect(&reference, &format!("a.store.{tick}.replaced")),
            "tick {tick} replaced"
        );
        assert_eq!(
            result.evicted.to_string(),
            expect(&reference, &format!("a.store.{tick}.evicted")),
            "tick {tick} evicted"
        );
    }
    let diagnostics = rollback_snapshot_history::diagnostics(&history);
    assert_eq!(
        diagnostics.capacity.to_string(),
        expect(&reference, "a.diag.capacity")
    );
    assert_eq!(
        diagnostics.retained_boundary_count.to_string(),
        expect(&reference, "a.diag.retained_boundary_count")
    );
    assert_eq!(
        diagnostics.oldest_supported_tick.unwrap().to_string(),
        expect(&reference, "a.diag.oldest_supported_tick")
    );
    assert_eq!(
        diagnostics.oldest_boundary_tick.unwrap().to_string(),
        expect(&reference, "a.diag.oldest_boundary_tick")
    );
    assert_eq!(
        diagnostics.latest_tick.unwrap().to_string(),
        expect(&reference, "a.diag.latest_tick")
    );

    for tick in [0i64, 1, 9, 10, 11, 30, 31, 39, 40] {
        assert_eq!(
            status_wire(rollback_snapshot_history::status(&history, tick)),
            expect(&reference, &format!("a.status.{tick}")),
            "status at tick {tick}"
        );
    }
    assert_eq!(
        status_wire(rollback_snapshot_history::status(&history, 10)),
        expect(&reference, "a.status.exact_floor.10")
    );
    assert_eq!(
        status_wire(rollback_snapshot_history::status(&history, 9)),
        expect(&reference, "a.status.one_before_floor.9")
    );

    let wrap_result =
        rollback_snapshot_history::store(&mut history, &boundary(&mut state, 41)).unwrap();
    assert_eq!(
        wrap_result.replaced.to_string(),
        expect(&reference, "a.wrap.store.41.replaced")
    );
    assert_eq!(
        wrap_result.evicted.to_string(),
        expect(&reference, "a.wrap.store.41.evicted")
    );
    assert_eq!(
        status_wire(rollback_snapshot_history::status(&history, 10)),
        expect(&reference, "a.wrap.status.10")
    );
    assert_eq!(
        status_wire(rollback_snapshot_history::status(&history, 41)),
        expect(&reference, "a.wrap.status.41")
    );
    assert_eq!(
        rollback_snapshot_history::diagnostics(&history)
            .oldest_supported_tick
            .unwrap()
            .to_string(),
        expect(&reference, "a.wrap.diag.oldest_supported_tick")
    );

    // Scenario B.
    let mut history = rollback_snapshot_history::new(Some(3));
    let mut state = new_state();
    for tick in [0i64, 1, 4, 6, 7, 9] {
        let result =
            rollback_snapshot_history::store(&mut history, &boundary(&mut state, tick)).unwrap();
        assert_eq!(
            result.replaced.to_string(),
            expect(&reference, &format!("b.store.{tick}.replaced")),
            "b tick {tick} replaced"
        );
        assert_eq!(
            result.evicted.to_string(),
            expect(&reference, &format!("b.store.{tick}.evicted")),
            "b tick {tick} evicted"
        );
    }
    for tick in 0..=9i64 {
        assert_eq!(
            status_wire(rollback_snapshot_history::status(&history, tick)),
            expect(&reference, &format!("b.status.{tick}")),
            "b status at tick {tick}"
        );
    }

    let truncated = rollback_snapshot_history::truncate_after(&mut history, 7).unwrap();
    assert_eq!(
        truncated.removed.to_string(),
        expect(&reference, "b.truncate.removed")
    );
    assert_eq!(
        truncated.diagnostics.latest_tick.unwrap().to_string(),
        expect(&reference, "b.truncate.latest_tick")
    );
    for tick in 0..=9i64 {
        assert_eq!(
            status_wire(rollback_snapshot_history::status(&history, tick)),
            expect(&reference, &format!("b.status_after_truncate.{tick}")),
            "b status after truncate at tick {tick}"
        );
    }

    let stale = rollback_snapshot_history::store(&mut history, &boundary(&mut state, 3));
    let stale_err = stale.unwrap_err();
    assert_eq!("true", expect(&reference, "b.store_stale.rejected"));
    assert_eq!(
        error_code_wire(stale_err.code),
        expect(&reference, "b.store_stale.code")
    );

    let missing_truncate = rollback_snapshot_history::truncate_after(&mut history, 8);
    let missing_err = missing_truncate.unwrap_err();
    assert_eq!("true", expect(&reference, "b.truncate_missing.rejected"));
    assert_eq!(
        error_code_wire(missing_err.code),
        expect(&reference, "b.truncate_missing.code")
    );

    let outside_truncate = rollback_snapshot_history::truncate_after(&mut history, 3);
    let outside_err = outside_truncate.unwrap_err();
    assert_eq!("true", expect(&reference, "b.truncate_outside.rejected"));
    assert_eq!(
        error_code_wire(outside_err.code),
        expect(&reference, "b.truncate_outside.code")
    );
}
