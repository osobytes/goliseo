//! Differential test of `match_snapshot` against reference vectors captured
//! from the real Lua implementation (ARCHITECTURE.md §3 rule 7,
//! `tools/lua_reference/README.md`). This is the required determinism
//! evidence for a hashed snapshot: a spec-based unit test proves this Rust
//! implementation satisfies written-down assertions, not that two clients
//! hash to the same bits.
//!
//! `tests/fixtures/match_snapshot_case_a_lua_reference.txt` and
//! `..._case_b_lua_reference.txt` are the captured canonical wire encoding
//! of `sim/match_snapshot.lua`'s `encode_canonical`/`hash_canonical`, run
//! under headless `love` (no display, no `xvfb`) against a hand-built
//! fixture (mirroring the same "minimal MatchState-shaped table" pattern
//! `metrics_spec.lua` uses, since `sim/match.lua` had not yet been ported to
//! Rust at the time and so could not build one) — a scratch
//! `conf.lua`/`main.lua` harness per that README, not committed (scratch
//! dirs are session-local).
//!
//! Case A is a soccer-only snapshot (`VERSION`) with a negative position
//! component, a zero `time_left`, and non-default per-team marking configs
//! (one `hybrid`, one `zonal`) to exercise the sign/zero/enum paths. Case B
//! adds a combat companion (`COMBAT_VERSION`) with one player mid-`windup`
//! on a `light_melee` loadout and one causal `commit` event, exercising
//! `combat_snapshot::append`'s wire path through
//! `match_snapshot::append_state`.
//!
//! Both cases reconstruct the identical fixture in Rust and assert
//! `match_snapshot::encode_canonical` matches the captured wire string byte
//! for byte, and `match_snapshot::hash_canonical` matches the captured
//! FNV-1a-64 hex digest — proof the canonical scalar encoding
//! (`number_bytes`, its `frexp` bit-manipulation, and every enum/string
//! wire mapping in this layer) agrees with the Lua original, not merely
//! that this Rust implementation satisfies its own spec-derived unit tests.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::tactics::{MarkingConfig, MarkingScheme, TransitionConfig};
use gc_sim::combat_feasibility::CombatActionPhase;
use gc_sim::combat_intent;
use gc_sim::combat_snapshot::{
    CombatEvent, CombatEventKind, CombatMatchState, CombatPlayerState, CombatRequestOutcome,
};
use gc_sim::match_snapshot::{self, ByTeam, MatchPlayer, MatchState, PitchSize, Rect};
use gc_sim::outfield_decision;
use gc_sim::outfield_press;
use gc_sim::possession_transition::{self, TransitionWindows};

const CASE_A: &str = include_str!("fixtures/match_snapshot_case_a_lua_reference.txt");
const CASE_B: &str = include_str!("fixtures/match_snapshot_case_b_lua_reference.txt");

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
        pos: Vec2::new(x, y),
        vel: Vec2::new(0.0, 0.0),
        run_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(
            if team == match_snapshot::Team::Home {
                1.0
            } else {
                -1.0
            },
            0.0,
        ),
        anchor: Vec2::new(x, y),
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
        dodge_dir: Vec2::new(0.0, 0.0),
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
        dive_dir: Vec2::new(0.0, 0.0),
        dive_delay: 0.0,
        dive_target: None,
        keeper_get_up_timer: 0.0,
        hold_timer: 0.0,
        feet_ball: false,
        slide_timer: 0.0,
        slide_dir: Vec2::new(0.0, 0.0),
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

fn base_state() -> MatchState {
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
        ball: Vec2::new(480.0, 270.0),
        ball_vel: Vec2::new(0.0, 0.0),
        ball_z: 0.0,
        ball_vz: 0.0,
        owner: None,
        controlled: 2,
        human_controlled: false,
        score: ByTeam { home: 0, away: 0 },
        time_left: 300.0,
        max_goals: 5,
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
        rng: gc_core::rng::seed(42.0),
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

fn parse_reference(text: &str) -> (&str, &str) {
    // fixture files: "<hash-irrelevant>" not stored; the wire string is the
    // whole file contents (trailing newline stripped). Hash is verified by
    // hashing this exact string with the Rust FNV-1a-64 implementation and
    // comparing to the value embedded in the test below.
    (text.trim_end_matches('\n'), "")
}

#[test]
fn case_a_soccer_only_matches_lua_wire_and_hash() {
    let mut state = base_state();
    state.players[1].pos = Vec2::new(-5.5, 0.0);
    state.ball_spin = -1.25;
    state.time_left = 0.0;

    let snapshot = match_snapshot::capture(&state, None);
    let encoded = match_snapshot::encode_canonical(&snapshot);
    let (expected_wire, _) = parse_reference(CASE_A);
    assert_eq!(encoded, expected_wire, "case A canonical wire mismatch");

    let hash = match_snapshot::hash_canonical(&snapshot);
    assert_eq!(hash, "38b6fb814d580964", "case A hash mismatch");
}

#[test]
fn case_b_combat_active_matches_lua_wire_and_hash() {
    let state = base_state();
    let player_ids: Vec<String> = state.players.iter().map(|p| p.id.clone()).collect();

    let mut players: Vec<CombatPlayerState> = (0..10)
        .map(|_| CombatPlayerState {
            loadout_id: None,
            family_id: None,
            phase: CombatActionPhase::Ready,
            phase_ticks: 0,
            cooldown_ticks: 0,
            source_sequence: None,
            contacted: false,
            release_latched: false,
            control_held: false,
            projectile_spawned: false,
            forced_state: None,
            forced_ticks: 0,
            chain_ticks: 0,
            immunity_ticks: 0,
            intent: combat_intent::new_state(),
        })
        .collect();
    players[1].loadout_id = Some("loadout_vector_blade".to_string());
    players[1].family_id = Some(ActionFamilyId::LightMelee);
    players[1].phase = CombatActionPhase::Windup;
    players[1].phase_ticks = 3;
    players[1].source_sequence = Some(1);
    players[1].control_held = true;

    let combat_state = CombatMatchState {
        version: gs_combat_snapshot_version(),
        tick: 5,
        player_ids,
        players,
        projectiles: Vec::new(),
        events: vec![CombatEvent {
            kind: CombatEventKind::Commit,
            tick: 4,
            family_id: Some(ActionFamilyId::LightMelee),
            source_index: Some(2),
            target_index: None,
            source_sequence: Some(1),
            result: None,
            outcome: Some(CombatRequestOutcome::Accepted),
            reason: None,
            terminal: None,
            x: 200.0,
            y: 150.0,
            interruption_ticks: None,
            displacement_px: None,
        }],
        next_source_sequence: 2,
    };

    let snapshot = match_snapshot::capture(&state, Some(&combat_state));
    let encoded = match_snapshot::encode_canonical(&snapshot);
    let (expected_wire, _) = parse_reference(CASE_B);
    assert_eq!(encoded, expected_wire, "case B canonical wire mismatch");

    let hash = match_snapshot::hash_canonical(&snapshot);
    assert_eq!(hash, "60416704b0578215", "case B hash mismatch");
}

fn gs_combat_snapshot_version() -> i64 {
    gc_sim::combat_snapshot::VERSION
}
