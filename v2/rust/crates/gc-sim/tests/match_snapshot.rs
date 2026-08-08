//! Port of `spec/sim/match_snapshot_spec.lua`.
//!
//! `gc_sim::r#match` (`sim/match.lua`'s port) has landed, so every case in
//! the Lua spec is now expressible against real fixtures built through
//! [`r#match::new`]/[`r#match::step`], mirroring the Lua spec's
//! `new_state()` / `new_ai_state()` / `new_attacking_ai_state()` helpers.
//!
//! ## Suspected defects found while porting
//!
//! Comparing `sim/match_snapshot.lua` against `crates/gc-sim/src/match_snapshot.rs`
//! line by line surfaced three validation calls the Lua original makes that
//! the Rust port never makes. None of these were introduced by this file —
//! they live in `match_snapshot.rs`, which is out of scope here (this is a
//! test-only port) — but several ported cases below only make sense in
//! light of them, so they are recorded once, here, rather than repeated in
//! every affected test's `#[ignore]` reason:
//!
//! 1. **Per-player `outfield_decision` structural validation is missing.**
//!    `sim/match_snapshot.lua`'s `copy_state`/`copy_owned_player` call
//!    `outfield_decision.copy_state(source.outfield_decision)` for *every*
//!    player (`sim/match_snapshot.lua:384`, `:765`), which runs
//!    `outfield_decision`'s own `validate_state` (version match,
//!    context/intent/target coherence, run-expiry pairing) regardless of
//!    that player's intent. `match_snapshot.rs`'s `validate()` only calls
//!    `validate_run_relations`, which walks players and skips any whose
//!    `intent` isn't one of the three run intents
//!    (`outfield_decision::is_run_intent`) — so a structurally invalid
//!    decision with a *non*-run intent (e.g. `context = Carrier, intent =
//!    Move`, or a stale `version`) is never checked at all.
//! 2. **Per-team `outfield_press` structural validation is missing.**
//!    `sim/match_snapshot.lua:648` and `:868-869` call
//!    `outfield_press.copy_state(source.outfield_press[team])`, which runs
//!    `outfield_press`'s own `validate_state` (version match, and
//!    mode/presser_index/reason coherence — e.g. `Contain` requires a
//!    presser). `match_snapshot.rs`'s `validate_press_eligibility` only
//!    checks the *relational* eligibility of a presser that is already
//!    present (team, keeper, fixed-slot, human-control) — it never checks
//!    `OutfieldPressState`'s own internal coherence.
//! 3. **The combat/run cross-check is missing entirely.** `sim/match_snapshot.lua`
//!    defines `assert_combat_run_relations(state, combat_state, path)`
//!    (`sim/match_snapshot.lua:715-724`), which asserts every player with an
//!    active run intent has `combat_state.players[index].phase == "ready"`
//!    and `forced_ticks <= 0`, and calls it from *both* `capture`
//!    (`sim/match_snapshot.lua:910`) and `restore` (`:964`). No equivalent
//!    call exists anywhere in `match_snapshot.rs` — `capture`/`restore`
//!    never look at `combat_state` when validating `outfield_decision` runs
//!    at all.
//!
//! Each is reproducible with a real, still-passing fixture (no Lua-only
//! mechanism involved), so these are recorded as `#[ignore = "suspected
//! defect: ..."]` on the affected tests below, per this port's brief, rather
//! than silently adjusted to match the current (wrong) behavior.
//!
//! ## Determinism coverage note
//!
//! The determinism-critical part of this module — canonical scalar
//! encoding, the wire format, and the FNV-1a-64 hash, exercised across a
//! soccer-only and a combat-active snapshot — is differential-tested
//! against the real Lua implementation separately, in
//! `match_snapshot_differential.rs`; that coverage is not duplicated here.

use gc_core::vec2::Vec2;
use gc_data::action_families::ActionFamilyId;
use gc_data::omp2_rollback_validation;
use gc_data::teams;
use gc_sim::aerial::{AerialOutcome, AerialStyle};
use gc_sim::brain::PressReason;
use gc_sim::combat;
use gc_sim::combat_snapshot::{
    self, CombatContactResult, CombatEncounterTerminal, CombatEvent, CombatEventKind,
    CombatForcedState, CombatPhase, CombatProjectile, CombatRequestOutcome,
    CombatRequestRejectionReason,
};
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputSampleOptions};
use gc_sim::keeper::{KeeperBehaviorState, KeeperShotType, SaveStyle};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{
    self, MatchEvent, MatchEventKind, MatchPlayer, MatchSnapshot, MatchState, PitchSize,
    Team as MatchTeam, WindupShot,
};
use gc_sim::outfield_decision::{
    self, OutfieldDecisionContext, OutfieldDecisionState, OutfieldIntent,
};
use gc_sim::outfield_press::{self, OutfieldPressContext, OutfieldPressState, StablePressMode};
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

// ---------------------------------------------------------------------
// Fixtures, mirroring the Lua spec's `new_state()` / `new_ai_state()` /
// `new_attacking_ai_state()`.
// ---------------------------------------------------------------------

fn new_state() -> MatchState {
    let home = teams::get("nebula").expect("nebula team is authored");
    let away = teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(2.0),
        max_goals: Some(3),
        seed: Some(38.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(sim_match::ownership_for_teams(home, away, None)),
    })
}

fn new_ai_state() -> MatchState {
    let home = teams::get("nebula").expect("nebula team is authored");
    let away = teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(2.0),
        max_goals: Some(3),
        seed: Some(38.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: Some(false),
        input_ownership: None,
    })
}

fn new_attacking_ai_state() -> MatchState {
    let mut state = new_ai_state();
    state.kickoff_hold = 0.0;
    state
}

/// Mirrors the Lua spec's inline `match.new({ home = ..., away = ... })`
/// human-controlled fixture (default `human_controlled`, no
/// `input_ownership`).
fn new_human_state() -> MatchState {
    let home = teams::get("nebula").expect("nebula team is authored");
    let away = teams::get("orion").expect("orion team is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(2.0),
        max_goals: Some(3),
        seed: Some(38.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

/// `t.is_true(not pcall(f))`.
fn fails<F: FnOnce()>(f: F) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
}

// ---------------------------------------------------------------------
// Local re-derivations of match_snapshot.rs's private wire-spelling
// functions, needed for the byte-accounting tests below (`outfield_decision_context_wire`,
// `outfield_intent_wire`, `stable_press_mode_wire`, `press_reason_wire` are
// not `pub`).
// ---------------------------------------------------------------------

fn wire_context(c: OutfieldDecisionContext) -> &'static str {
    match c {
        OutfieldDecisionContext::Ineligible => "ineligible",
        OutfieldDecisionContext::Offball => "offball",
        OutfieldDecisionContext::Carrier => "carrier",
    }
}

fn wire_intent(i: OutfieldIntent) -> &'static str {
    match i {
        OutfieldIntent::None => "none",
        OutfieldIntent::Move => "move",
        OutfieldIntent::InBehind => "in_behind",
        OutfieldIntent::ComeShort => "come_short",
        OutfieldIntent::HoldWidth => "hold_width",
        OutfieldIntent::Shoot => "shoot",
        OutfieldIntent::Cross => "cross",
        OutfieldIntent::Pass => "pass",
        OutfieldIntent::Dribble => "dribble",
    }
}

fn wire_press_mode(m: StablePressMode) -> &'static str {
    match m {
        StablePressMode::Inactive => "inactive",
        StablePressMode::Contain => "contain",
        StablePressMode::Commit => "commit",
    }
}

fn wire_press_reason(r: PressReason) -> &'static str {
    match r {
        PressReason::HeavyTouch => "heavy_touch",
        PressReason::ExposedBall => "exposed_ball",
        PressReason::Cover => "cover",
        PressReason::BoxDesperation => "box_desperation",
        PressReason::LowDiscipline => "low_discipline",
        PressReason::NoTrigger => "no_trigger",
    }
}

#[test]
fn canonical_match_snapshots_pins_matchstate_and_matchplayer_additions_to_explicit_versioned_allowlists()
 {
    // The Lua case reads `sim/match.lua`'s and `sim/combat.lua`'s *source
    // text* at test time (`love.filesystem.read`) and regex-extracts
    // `---@field` annotations to compare against
    // `match_snapshot.PLAYER_FIELDS` / `MATCH_FIELDS` and
    // `combat_snapshot.STATE_FIELDS` / `PLAYER_FIELDS` / `PROJECTILE_FIELDS`
    // / `EVENT_FIELDS`. That mechanism has no Rust analogue for two
    // independent reasons, either one sufficient on its own:
    //
    // - Rust has no runtime reflection over a struct's field set, so there
    //   is nothing to "extract" the way the Lua case regexes doc comments.
    // - `v2/README.md` §1 states plainly: "Nothing in `v2/` may `require` or
    //   read the Lua sources at runtime." Reading `sim/match.lua`'s text at
    //   test time, even just to diff field names, is exactly the dependency
    //   that rule forbids.
    //
    // Separately, the four allowlist constants being compared against
    // (`match_snapshot.PLAYER_FIELDS`/`MATCH_FIELDS`,
    // `combat_snapshot.STATE_FIELDS`/`PLAYER_FIELDS`/`PROJECTILE_FIELDS`/`EVENT_FIELDS`)
    // do not exist in the Rust port at all — `match_snapshot.rs` and
    // `combat_snapshot.rs` replace the Lua "table of field-name strings"
    // idiom with real typed structs (`MatchState`, `MatchPlayer`,
    // `CombatMatchState`, `CombatPlayerState`, `CombatProjectile`,
    // `CombatEvent`) and enums (`CombatEventKind`, `CombatContactResult`,
    // ...) with `wire_str()` methods. There is no allowlist value left to
    // pin.
    //
    // What already guards field-list drift in Rust, in place of this case:
    //
    // - Every one of these structs' fields is populated via ordinary struct
    //   literal syntax (`MatchPlayer { id: ..., name: ..., ... }`) at every
    //   construction site in the crate (chiefly `match.rs` and `combat.rs`).
    //   Rust struct literals must name every field — there is no partial or
    //   optional-by-default construction — so adding a field to
    //   `MatchPlayer`/`MatchState`/`CombatPlayerState`/... is a compile
    //   error at every one of those call sites until it is addressed. This
    //   is a stronger, load-bearing guarantee than the Lua case's read of
    //   doc comments: the Lua check verifies a *comment* matches a
    //   constant, while the Rust compiler verifies every *value* actually
    //   carries the field.
    // - `match_snapshot_differential.rs` pins the canonical encoder's wire
    //   bytes and FNV-1a-64 hash against reference vectors computed by the
    //   real running Lua (`v2/tools/lua_reference/`). A field that starts
    //   getting serialized (or stops) changes the wire length/hash, so that
    //   differential test is the safety net for "the encoder's own field
    //   list drifted from the struct's", the concern this case's schema-pin
    //   was really guarding against.
    //
    // Neither mechanism is exercised by *this* file (both live outside a
    // single test's scope), so there is no meaningful substitute assertion
    // to write here — the case is dropped in full.
}

#[test]
fn canonical_match_snapshots_captures_combat_as_one_owned_canonical_versioned_boundary() {
    let mut state = new_state();
    state.kickoff_hold = 0.0;
    let mut combat_state = combat::new_state(&mut state, None);

    let mut source_index: Option<usize> = None;
    for (index, runtime) in combat_state.players.iter().enumerate() {
        if runtime.family_id == Some(ActionFamilyId::Ranged) {
            source_index = Some(index + 1);
            break;
        }
    }
    let source_index = source_index.expect("fixture requires one ranged loadout");

    {
        let runtime = &mut combat_state.players[source_index - 1];
        runtime.phase = CombatPhase::Windup;
        runtime.phase_ticks = 7;
        runtime.cooldown_ticks = 42;
        runtime.source_sequence = Some(7);
        runtime.control_held = true;
    }
    combat_state.projectiles.push(CombatProjectile {
        family_id: ActionFamilyId::Ranged,
        source_index: source_index as i64,
        source_sequence: 7,
        pos: Vec2::new(111.0, 222.0),
        dir: Vec2::new(0.5, -0.5),
        remaining_ticks: 12,
    });
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::ProjectileSpawn,
        tick: 0,
        family_id: Some(ActionFamilyId::Ranged),
        source_index: Some(source_index as i64),
        target_index: None,
        source_sequence: Some(7),
        result: None,
        outcome: None,
        reason: None,
        terminal: None,
        x: 111.0,
        y: 222.0,
        interruption_ticks: None,
        displacement_px: None,
    });
    combat_state.next_source_sequence = 8;
    state.input_tick = 1;
    combat_state.tick = 1;

    let runtime_loadout_id = combat_state.players[source_index - 1].loadout_id.clone();

    let snapshot = match_snapshot::capture(&state, Some(&combat_state));
    assert_eq!(snapshot.version, match_snapshot::COMBAT_VERSION);
    assert_eq!(
        snapshot.combat.as_ref().unwrap().version,
        combat_snapshot::VERSION
    );
    assert_eq!(
        snapshot.combat.as_ref().unwrap().players[source_index - 1].loadout_id,
        runtime_loadout_id
    );
    let (restored_state, restored_combat) = match_snapshot::restore(&snapshot);
    let restored_combat = restored_combat.expect("combat companion required");
    assert_eq!(restored_combat.players[source_index - 1].phase_ticks, 7);
    assert_eq!(restored_combat.projectiles[0].pos.x, 111.0);
    assert_eq!(restored_combat.events[0].source_sequence, Some(7));
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(
            &restored_state,
            Some(&restored_combat)
        )),
        match_snapshot::hash(&snapshot)
    );

    state.players[source_index - 1].pos.x = -1.0;
    combat_state.players[source_index - 1].phase_ticks = 1;
    combat_state.projectiles[0].pos.x = -2.0;
    combat_state.events[0].x = -3.0;
    assert!(snapshot.state.players[source_index - 1].pos.x != -1.0);
    assert_eq!(
        snapshot.combat.as_ref().unwrap().players[source_index - 1].phase_ticks,
        7
    );
    assert_eq!(
        snapshot.combat.as_ref().unwrap().projectiles[0].pos.x,
        111.0
    );
    assert_eq!(snapshot.combat.as_ref().unwrap().events[0].x, 111.0);

    let mut snapshot = snapshot;
    snapshot.combat.as_mut().unwrap().players[source_index - 1].phase_ticks = 6;
    let changed = match_snapshot::capture(&restored_state, Some(&restored_combat));
    let found = match_snapshot::first_difference(&snapshot, &changed).expect("difference expected");
    assert_eq!(
        found.path,
        format!("combat.players.{source_index}.phase_ticks")
    );

    // The Lua case also sets `malformed_player.unknown = true` (an
    // undeclared field via `rawset`) and expects `restore` to reject it.
    // `CombatPlayerState` is a fixed-field struct with no room for an extra
    // key, so there is no way to construct that value — see this file's
    // module doc for the same principle applied to the schema-pin case.
    // The still-constructible half (a marked-unsupported plain state cannot
    // be captured without its combat companion) is kept below.
    assert!(fails(|| {
        let _ = match_snapshot::capture(&restored_state, None);
    }));
}

#[test]
fn canonical_match_snapshots_rejects_holes_in_authoritative_combat_projectile_and_event_arrays() {
    // The Lua case builds `values = { v1, v1, v1 }` then sets `values[2] =
    // nil`, leaving a HOLE in the middle of a Lua array (`#values == 4`
    // with index 2 missing while 3 and 4 remain) and expects
    // `combat_snapshot.copy`'s density check to reject it. `CombatMatchState`'s
    // `projectiles`/`events` fields are `Vec<CombatProjectile>` /
    // `Vec<CombatEvent>` — not `Vec<Option<T>>` — so there is no way to
    // construct a Rust value with a "hole": removing an element from a
    // `Vec` always yields a shorter, still-dense sequence. The invariant
    // this case pins (retained combat arrays never skip an index) is
    // therefore enforced unconditionally by the type itself rather than by
    // a runtime check, the same "structurally impossible to violate"
    // category `tests/content_validation.rs`'s squad-hole case documents.
    // There is no still-constructible partial version of this case: any
    // `Vec<CombatProjectile>`/`Vec<CombatEvent>` a test could build is, by
    // construction, already dense.
}

#[test]
fn canonical_match_snapshots_replays_combat_phase_boundaries_exactly_after_restore() {
    let cases = [
        "windup",
        "active",
        "guard",
        "projectile",
        "stagger",
        "knockback",
        "immunity",
    ];
    let tune = Tuning::new();
    for case in cases {
        let mut state = new_state();
        state.kickoff_hold = 0.0;
        let mut combat_state = combat::new_state(&mut state, None);

        let mut family_guard: Option<i64> = None;
        let mut family_ranged: Option<i64> = None;
        let mut family_light_melee: Option<i64> = None;
        let mut family_unarmed: Option<i64> = None;
        for (i, p) in combat_state.players.iter().enumerate() {
            if let Some(fid) = p.family_id {
                let idx = (i + 1) as i64;
                match fid {
                    ActionFamilyId::Guard if family_guard.is_none() => family_guard = Some(idx),
                    ActionFamilyId::Ranged if family_ranged.is_none() => family_ranged = Some(idx),
                    ActionFamilyId::LightMelee if family_light_melee.is_none() => {
                        family_light_melee = Some(idx);
                    }
                    ActionFamilyId::Unarmed if family_unarmed.is_none() => {
                        family_unarmed = Some(idx);
                    }
                    _ => {}
                }
            }
        }

        match case {
            "windup" => {
                let idx = family_light_melee.expect("fixture has a light-melee loadout");
                let runtime = &mut combat_state.players[(idx - 1) as usize];
                runtime.phase = CombatPhase::Windup;
                runtime.phase_ticks = 3;
                runtime.cooldown_ticks = 20;
                runtime.source_sequence = Some(1);
            }
            "active" => {
                let idx = family_light_melee.expect("fixture has a light-melee loadout");
                let runtime = &mut combat_state.players[(idx - 1) as usize];
                runtime.phase = CombatPhase::Active;
                runtime.phase_ticks = 2;
                runtime.cooldown_ticks = 20;
                runtime.source_sequence = Some(1);
            }
            "guard" => {
                let idx = family_guard.expect("fixture has a guard loadout");
                let runtime = &mut combat_state.players[(idx - 1) as usize];
                runtime.phase = CombatPhase::Guard;
                runtime.control_held = true;
                runtime.source_sequence = Some(1);
            }
            "projectile" => {
                let idx = family_ranged.expect("fixture has a ranged loadout");
                let pos = state.players[(idx - 1) as usize].pos;
                combat_state.projectiles.push(CombatProjectile {
                    family_id: ActionFamilyId::Ranged,
                    source_index: idx,
                    source_sequence: 1,
                    pos,
                    dir: Vec2::new(1.0, 0.0),
                    remaining_ticks: 2,
                });
                combat_state.next_source_sequence = 2;
            }
            "stagger" => {
                let idx = family_unarmed.expect("fixture has an unarmed loadout");
                let runtime = &mut combat_state.players[(idx - 1) as usize];
                runtime.forced_state = Some(CombatForcedState::Stagger);
                runtime.forced_ticks = 2;
                runtime.chain_ticks = 4;
            }
            "knockback" => {
                let idx = family_light_melee.expect("fixture has a light-melee loadout");
                let runtime = &mut combat_state.players[(idx - 1) as usize];
                runtime.forced_state = Some(CombatForcedState::Knockback);
                runtime.forced_ticks = 2;
                runtime.chain_ticks = 4;
            }
            "immunity" => {
                let idx = family_ranged.expect("fixture has a ranged loadout");
                combat_state.players[(idx - 1) as usize].immunity_ticks = 2;
            }
            _ => unreachable!("unknown case {case}"),
        }

        let frame = input_frame::neutral(0).expect("neutral frame");
        let start = match_snapshot::capture(&state, Some(&combat_state));
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(&frame),
            Some(&mut combat_state),
            &tune,
        );
        let expected = match_snapshot::capture(&state, Some(&combat_state));
        let (mut restored, restored_combat) = match_snapshot::restore(&start);
        let mut restored_combat = restored_combat.expect("combat companion required");
        sim_match::step(
            &mut restored,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(&frame),
            Some(&mut restored_combat),
            &tune,
        );
        assert_eq!(
            match_snapshot::hash(&match_snapshot::capture(&restored, Some(&restored_combat))),
            match_snapshot::hash(&expected),
            "case {case}"
        );
    }
}

#[test]
fn canonical_match_snapshots_persists_active_ai_runs_through_a_v10_soccer_boundary_and_continuation()
 {
    let mut state = new_attacking_ai_state();
    let owner = state.owner.expect("owner assigned");
    let owner_team = state.players[(owner - 1) as usize].team;
    let mut runner_indices: Vec<i64> = Vec::new();
    for (index, player) in state.players.iter().enumerate() {
        let idx = (index + 1) as i64;
        if player.team == owner_team
            && !player.is_keeper
            && idx != owner
            && runner_indices.len() < 2
        {
            runner_indices.push(idx);
        }
    }
    assert_eq!(runner_indices.len(), 2);
    for (ordinal, &runner_index) in runner_indices.iter().enumerate() {
        let player = &state.players[(runner_index - 1) as usize];
        let intent = if ordinal == 0 {
            OutfieldIntent::ComeShort
        } else {
            OutfieldIntent::InBehind
        };
        let (x, y) = if ordinal == 0 {
            (420.0, 180.0)
        } else {
            (700.0, 300.0)
        };
        let refreshed = outfield_decision::refresh(
            &player.outfield_decision,
            OutfieldDecisionContext::Offball,
            intent,
            player.scan_rate,
            Some(x),
            Some(y),
            None,
            Some(-0.2),
        );
        state.players[(runner_index - 1) as usize].outfield_decision = refreshed;
    }

    let boundary = match_snapshot::capture(&state, None);
    assert_eq!(boundary.version, match_snapshot::VERSION);
    let (mut restored, restored_combat) = match_snapshot::restore(&boundary);
    assert!(restored_combat.is_none());
    for &runner_index in &runner_indices {
        let expected = state.players[(runner_index - 1) as usize].outfield_decision;
        let actual = restored.players[(runner_index - 1) as usize].outfield_decision;
        assert_eq!(actual.intent, expected.intent);
        assert_eq!(actual.run_expires_at, expected.run_expires_at);
        assert_eq!(actual.remaining, expected.remaining);
        assert_eq!(actual.generation, expected.generation);
    }
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(&restored, None)),
        match_snapshot::hash(&boundary)
    );

    let tune = Tuning::new();
    for _ in 0..2 {
        let live_input = slot_input::neutral_match_input();
        let restored_input = slot_input::neutral_match_input();
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(live_input),
            None,
            &tune,
        );
        sim_match::step(
            &mut restored,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(restored_input),
            None,
            &tune,
        );
        assert_eq!(
            match_snapshot::hash(&match_snapshot::capture(&restored, None)),
            match_snapshot::hash(&match_snapshot::capture(&state, None))
        );
    }
}

#[test]
fn canonical_match_snapshots_persists_active_ai_runs_through_a_v11_combat_boundary_and_continuation()
 {
    let mut state = new_attacking_ai_state();
    let owner = state.owner.expect("owner assigned");
    let owner_team = state.players[(owner - 1) as usize].team;
    let mut runner_indices: Vec<i64> = Vec::new();
    for (index, player) in state.players.iter().enumerate() {
        let idx = (index + 1) as i64;
        if player.team == owner_team
            && !player.is_keeper
            && idx != owner
            && runner_indices.len() < 2
        {
            runner_indices.push(idx);
        }
    }
    assert_eq!(runner_indices.len(), 2);
    for (ordinal, &runner_index) in runner_indices.iter().enumerate() {
        let player = &state.players[(runner_index - 1) as usize];
        let intent = if ordinal == 0 {
            OutfieldIntent::ComeShort
        } else {
            OutfieldIntent::InBehind
        };
        let (x, y) = if ordinal == 0 {
            (420.0, 180.0)
        } else {
            (700.0, 300.0)
        };
        let refreshed = outfield_decision::refresh(
            &player.outfield_decision,
            OutfieldDecisionContext::Offball,
            intent,
            player.scan_rate,
            Some(x),
            Some(y),
            None,
            Some(-0.2),
        );
        state.players[(runner_index - 1) as usize].outfield_decision = refreshed;
    }

    let mut combat_state = combat::new_state(&mut state, None);
    let boundary = match_snapshot::capture(&state, Some(&combat_state));
    assert_eq!(boundary.version, match_snapshot::COMBAT_VERSION);
    let (mut restored, restored_combat) = match_snapshot::restore(&boundary);
    let mut restored_combat = restored_combat.expect("combat companion required");
    for &runner_index in &runner_indices {
        let expected = state.players[(runner_index - 1) as usize].outfield_decision;
        let actual = restored.players[(runner_index - 1) as usize].outfield_decision;
        assert_eq!(actual.intent, expected.intent);
        assert_eq!(actual.run_expires_at, expected.run_expires_at);
        assert_eq!(actual.remaining, expected.remaining);
        assert_eq!(actual.generation, expected.generation);
    }
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(&restored, Some(&restored_combat))),
        match_snapshot::hash(&boundary)
    );

    let tune = Tuning::new();
    for _ in 0..2 {
        let live_input = slot_input::neutral_match_input();
        let restored_input = slot_input::neutral_match_input();
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(live_input),
            Some(&mut combat_state),
            &tune,
        );
        sim_match::step(
            &mut restored,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(restored_input),
            Some(&mut restored_combat),
            &tune,
        );
        assert_eq!(
            match_snapshot::hash(&match_snapshot::capture(&restored, Some(&restored_combat))),
            match_snapshot::hash(&match_snapshot::capture(&state, Some(&combat_state)))
        );
    }
}

#[test]
fn canonical_match_snapshots_rejects_a_combat_blocked_active_run_as_a_malformed_v11_boundary() {
    let mut state = new_attacking_ai_state();
    let owner = state.owner.expect("owner assigned");
    let owner_team = state.players[(owner - 1) as usize].team;
    let mut runner_index: Option<i64> = None;
    for (index, player) in state.players.iter().enumerate() {
        let idx = (index + 1) as i64;
        if player.team == owner_team && !player.is_keeper && idx != owner {
            runner_index = Some(idx);
            break;
        }
    }
    let runner_index = runner_index.expect("fixture requires an ordinary teammate runner");
    let runner = &state.players[(runner_index - 1) as usize];
    let refreshed = outfield_decision::refresh(
        &runner.outfield_decision,
        OutfieldDecisionContext::Offball,
        OutfieldIntent::InBehind,
        runner.scan_rate,
        Some(700.0),
        Some(240.0),
        None,
        Some(-0.2),
    );
    state.players[(runner_index - 1) as usize].outfield_decision = refreshed;

    let mut combat_state = combat::new_state(&mut state, None);

    let valid_boundary = match_snapshot::capture(&state, Some(&combat_state));
    {
        let runtime = &mut combat_state.players[(runner_index - 1) as usize];
        runtime.forced_state = Some(CombatForcedState::Stagger);
        runtime.forced_ticks = 2;
        runtime.chain_ticks = 4;
    }
    assert!(
        fails(|| {
            let _ = match_snapshot::capture(&state, Some(&combat_state));
        }),
        "capture accepted a forced player retaining a run"
    );

    let mut valid_boundary = valid_boundary;
    {
        let runtime =
            &mut valid_boundary.combat.as_mut().unwrap().players[(runner_index - 1) as usize];
        runtime.forced_state = Some(CombatForcedState::Stagger);
        runtime.forced_ticks = 2;
        runtime.chain_ticks = 4;
    }
    assert!(
        fails(|| {
            let _ = match_snapshot::restore(&valid_boundary);
        }),
        "restore accepted a forced player retaining a run"
    );
    {
        let runtime =
            &mut valid_boundary.combat.as_mut().unwrap().players[(runner_index - 1) as usize];
        runtime.forced_state = None;
        runtime.forced_ticks = 0;
        runtime.chain_ticks = 0;
        runtime.phase = CombatPhase::Recovery;
        runtime.phase_ticks = 1;
    }
    assert!(
        fails(|| {
            let _ = match_snapshot::restore(&valid_boundary);
        }),
        "restore accepted an action-committed player retaining a run"
    );

    // `match::sanitize_run_states` (the Lua original: `match._sanitize_run_states`)
    // is a private function in `match.rs`, not `pub`/`pub(crate)`, and it is
    // called from exactly one place: near the end of `step()`, after combat
    // resolution for the tick. There is no way to call it in isolation from
    // this integration test without widening its visibility (which this
    // test-only port must not do — see the report). A full `step()` call
    // exercises it as a side effect and settles the same invariant the Lua
    // case checks directly: an outfield decision that can no longer sustain
    // a legal run (here, because its owner is combat-forced) gets cleared.
    let tune = Tuning::new();
    sim_match::step(
        &mut state,
        fixed_clock::TICK_SECONDS,
        StepInput::Legacy(slot_input::neutral_match_input()),
        Some(&mut combat_state),
        &tune,
    );
    let cleared = state.players[(runner_index - 1) as usize].outfield_decision;
    assert!(!outfield_decision::is_run_intent(cleared.intent));
    assert_eq!(cleared.run_expires_at, None);
    let cleared_boundary = match_snapshot::capture(&state, Some(&combat_state));
    let (cleared_state, cleared_combat) = match_snapshot::restore(&cleared_boundary);
    let cleared_combat = cleared_combat.expect("combat companion required");
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(
            &cleared_state,
            Some(&cleared_combat)
        )),
        match_snapshot::hash(&cleared_boundary)
    );
}

#[test]
fn canonical_match_snapshots_captures_and_restores_every_nested_payload_as_independent_state() {
    let mut state = new_attacking_ai_state();
    state.players[1].outfield_decision = OutfieldDecisionState {
        version: outfield_decision::VERSION,
        generation: 7,
        rng_state: 53,
        remaining: 0.25,
        context: OutfieldDecisionContext::Offball,
        intent: OutfieldIntent::InBehind,
        target_x: Some(410.0),
        target_y: Some(220.0),
        target_player: None,
        run_expires_at: Some(-0.2),
    };
    state.players[1].dive_target = Some(Vec2::new(10.0, 20.0));
    state.players[0].keeper_state = KeeperBehaviorState::Retreat;
    state.players[0].keeper_state_timer = 0.15;
    state.players[0].keeper_release_state = Some(KeeperBehaviorState::Advance);
    state.players[0].keeper_release_motion = 0.75;
    state.players[0].keeper_release_kind = Some(KeeperShotType::Chip);
    state.players[0].keeper_release_depth = 40.0;
    state.players[2].windup_shot = Some(WindupShot {
        dir: Vec2::new(0.25, -0.75),
        speed: 456.0,
        vz: 123.0,
        spin: -8.0,
        shot_type: KeeperShotType::Chip,
    });
    state.players[1].save_style = Some(SaveStyle::Stretch);
    state.players[1].save_tip_emitted = true;
    state.players[1].keeper_anticipation = 0.75;
    state.players[1].keeper_set = 0.125;
    state.events.push(MatchEvent {
        kind: MatchEventKind::Header,
        x: 44.0,
        y: 55.0,
        player: Some(state.players[1].id.clone()),
        save_style: None,
        style: Some(AerialStyle::Header),
        outcome: Some(AerialOutcome::Clean),
        jumping: Some(true),
        difficulty: Some(0.4),
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    state.events.push(MatchEvent {
        kind: MatchEventKind::Catch,
        x: 66.0,
        y: 77.0,
        player: Some(state.players[0].id.clone()),
        save_style: Some(SaveStyle::Spread),
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: Some(KeeperBehaviorState::Set),
        keeper_depth: Some(12.0),
        on_target: None,
    });
    let snapshot = match_snapshot::capture(&state, None);
    let mut snapshot = snapshot;
    let (mut restored, _) = match_snapshot::restore(&snapshot);

    state.players[1].pos.x = -100.0;
    state.players[1].outfield_decision.generation = 8;
    state.players[1].outfield_decision.target_x = Some(-101.0);
    state.players[0].keeper_state = KeeperBehaviorState::Base;
    state.players[0].keeper_release_depth = -101.0;
    state.players[2].windup_shot.as_mut().unwrap().dir.y = 99.0;
    state.events[0].x = 999.0;
    state.events[1].save_style = Some(SaveStyle::Central);
    assert!(snapshot.state.players[1].pos.x != -100.0);
    assert_eq!(snapshot.state.players[1].outfield_decision.generation, 7);
    assert_eq!(
        snapshot.state.players[1].outfield_decision.target_x,
        Some(410.0)
    );
    assert_eq!(
        snapshot.state.players[1].outfield_decision.run_expires_at,
        Some(-0.2)
    );
    assert_eq!(
        snapshot.state.players[0].keeper_state,
        KeeperBehaviorState::Retreat
    );
    assert_eq!(
        snapshot.state.players[0].keeper_release_state,
        Some(KeeperBehaviorState::Advance)
    );
    assert_eq!(snapshot.state.players[0].keeper_release_motion, 0.75);
    assert_eq!(
        snapshot.state.players[0].keeper_release_kind,
        Some(KeeperShotType::Chip)
    );
    assert_eq!(snapshot.state.players[0].keeper_release_depth, 40.0);
    assert_eq!(
        snapshot.state.players[2]
            .windup_shot
            .as_ref()
            .unwrap()
            .dir
            .y,
        -0.75
    );
    assert_eq!(
        snapshot.state.players[1].save_style,
        Some(SaveStyle::Stretch)
    );
    assert!(snapshot.state.players[1].save_tip_emitted);
    assert_eq!(snapshot.state.players[1].keeper_anticipation, 0.75);
    assert_eq!(snapshot.state.players[1].keeper_set, 0.125);
    assert_eq!(snapshot.state.events[0].x, 44.0);
    assert_eq!(snapshot.state.events[1].save_style, Some(SaveStyle::Spread));

    snapshot.state.players[1].pos.y = -200.0;
    snapshot.state.players[1].outfield_decision.target_y = Some(-201.0);
    snapshot.state.players[0].keeper_state = KeeperBehaviorState::Recover;
    snapshot.state.players[2]
        .windup_shot
        .as_mut()
        .unwrap()
        .speed = 1.0;
    assert!(restored.players[1].pos.y != -200.0);
    assert_eq!(restored.players[1].outfield_decision.generation, 7);
    assert_eq!(restored.players[1].outfield_decision.target_y, Some(220.0));
    assert_eq!(
        restored.players[1].outfield_decision.run_expires_at,
        Some(-0.2)
    );
    assert_eq!(
        restored.players[0].keeper_state,
        KeeperBehaviorState::Retreat
    );
    assert_eq!(restored.players[0].keeper_state_timer, 0.15);
    assert_eq!(
        restored.players[0].keeper_release_state,
        Some(KeeperBehaviorState::Advance)
    );
    assert_eq!(restored.players[0].keeper_release_motion, 0.75);
    assert_eq!(
        restored.players[0].keeper_release_kind,
        Some(KeeperShotType::Chip)
    );
    assert_eq!(restored.players[0].keeper_release_depth, 40.0);
    assert!(
        (restored.players[0].keeper_aggression - state.players[0].keeper_aggression).abs() < 1e-6
    );
    assert!(
        (restored.players[0].keeper_anticipation - state.players[0].keeper_anticipation).abs()
            < 1e-6
    );
    assert_eq!(
        restored.players[2].windup_shot.as_ref().unwrap().speed,
        456.0
    );
    assert_eq!(
        restored.players[2].windup_shot.as_ref().unwrap().shot_type,
        KeeperShotType::Chip
    );
    assert!(
        (restored.players[2]
            .windup_shot
            .as_ref()
            .unwrap()
            .dir
            .length()
            - 0.625_f64.sqrt())
        .abs()
            < 1e-6
    );
    assert_eq!(restored.players[1].keeper_anticipation, 0.75);
    assert_eq!(restored.players[1].keeper_set, 0.125);
    assert_eq!(restored.events[1].save_style, Some(SaveStyle::Spread));
    assert_eq!(
        restored.events[1].keeper_state,
        Some(KeeperBehaviorState::Set)
    );
    assert_eq!(restored.events[1].keeper_depth, Some(12.0));

    let _ = &mut restored;
}

#[test]
fn canonical_match_snapshots_keeps_trusted_rollback_copies_exact_and_independently_owned() {
    let mut state = new_state();
    state.ball_z = -0.0;
    state.players[1].dash_cd = -0.0;
    state.players[1].dive_target = Some(Vec2::new(10.0, 20.0));
    state.players[1].windup_shot = Some(WindupShot {
        dir: Vec2::new(0.25, -0.75),
        speed: 456.0,
        vz: 123.0,
        spin: -8.0,
        shot_type: KeeperShotType::Chip,
    });
    state.events.push(MatchEvent {
        kind: MatchEventKind::Header,
        x: 44.0,
        y: 55.0,
        player: Some(state.players[1].id.clone()),
        save_style: None,
        style: None,
        outcome: None,
        jumping: None,
        difficulty: None,
        shot_type: None,
        keeper_state: None,
        keeper_depth: None,
        on_target: None,
    });
    let validated = match_snapshot::capture(&state, None);
    let mut owned = match_snapshot::capture_owned(&state, None);

    assert_eq!(
        match_snapshot::encode_canonical(&owned),
        match_snapshot::encode(&validated)
    );
    assert_eq!(
        match_snapshot::hash(&owned),
        match_snapshot::hash(&validated)
    );
    assert_eq!(
        match_snapshot::encoded_size_canonical(&owned),
        match_snapshot::encoded_size_canonical(&validated)
    );

    state.players[1].pos.x = -100.0;
    state.players[1].dive_target.as_mut().unwrap().y = -200.0;
    state.players[1].windup_shot.as_mut().unwrap().dir.x = -300.0;
    state.events[0].x = -400.0;
    state.field.w = -500.0;
    state.goal_home.x = -600.0;
    state.score.home = 7;
    state.press.away = 99;
    state.marking.home.standoff = -123.0;
    if state.marks.home.is_empty() {
        state.marks.home.push(None);
    }
    state.marks.home[0] = Some(9);
    state.input_ownership.as_mut().unwrap().rosters.home[0] = "mutated".to_string();
    state.input_ownership.as_mut().unwrap().slots[0].player_id = "mutated".to_string();
    state.slot_players[0] = Some(9);
    state.slot_for_player[0] = Some(9);
    assert!(owned.state.players[1].pos.x != -100.0);
    assert!(owned.state.players[1].dive_target.unwrap().y != -200.0);
    assert!(owned.state.players[1].windup_shot.unwrap().dir.x != -300.0);
    assert!(owned.state.events[0].x != -400.0);
    assert!(owned.state.field.w != -500.0);
    assert!(owned.state.goal_home.x != -600.0);
    assert!(owned.state.score.home != 7);
    assert!(owned.state.press.away != 99);
    assert!(owned.state.marking.home.standoff != -123.0);
    assert!(owned.state.marks.home[0] != Some(9));
    assert!(owned.state.input_ownership.as_ref().unwrap().rosters.home[0] != "mutated");
    assert!(owned.state.input_ownership.as_ref().unwrap().slots[0].player_id != "mutated");
    assert!(owned.state.slot_players[0] != Some(9));
    assert!(owned.state.slot_for_player[0] != Some(9));

    let (public_restored, _) = match_snapshot::restore(&owned);
    let owned_restored = match_snapshot::restore_owned(&owned);
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture_owned(&owned_restored.0, None)),
        match_snapshot::hash(&match_snapshot::capture(&public_restored, None))
    );

    owned.state.players[1].pos.y = -500.0;
    owned.state.players[1].dive_target.as_mut().unwrap().x = -600.0;
    owned.state.players[1].windup_shot.as_mut().unwrap().speed = -700.0;
    owned.state.events[0].y = -800.0;
    assert!(owned_restored.0.players[1].pos.y != -500.0);
    assert!(owned_restored.0.players[1].dive_target.unwrap().x != -600.0);
    assert!(owned_restored.0.players[1].windup_shot.unwrap().speed != -700.0);
    assert!(owned_restored.0.events[0].y != -800.0);
    assert!(
        (owned_restored.0.players[1].pos.length() - public_restored.players[1].pos.length()).abs()
            < 1e-6
    );
}

#[test]
fn canonical_match_snapshots_guards_the_shallow_trusted_copy_ownership_contract() {
    // Three of the Lua case's four sub-cases pass `nil` where a
    // `MatchState`/`MatchSnapshot` is required: `capture_owned(nil)`,
    // `restore_owned(nil)`, and `restore_owned({ version = VERSION, state =
    // nil })`. `capture_owned` takes `&MatchState` and `restore_owned` takes
    // `&MatchSnapshot { state: MatchState, .. }` — neither reference can be
    // null in safe Rust, and `MatchSnapshot.state` is a plain `MatchState`,
    // never an `Option<MatchState>` — so none of the three bad calls can
    // even be written, let alone compiled. The one sub-case that survives —
    // `restore_owned` rejecting a snapshot whose `version` is neither
    // `VERSION` nor `COMBAT_VERSION` — is fully constructible and ported
    // below.
    let wrong_version = MatchSnapshot {
        version: match_snapshot::VERSION - 1,
        state: new_state(),
        combat: None,
    };
    assert!(fails(|| {
        let _ = match_snapshot::restore_owned(&wrong_version);
    }));
}

#[test]
fn canonical_match_snapshots_canonically_restores_a_v10_keeper_state_through_goal_and_kickoff() {
    let mut live = new_state();
    {
        let keeper = &mut live.players[5];
        keeper.keeper_state = KeeperBehaviorState::Retreat;
        keeper.keeper_state_timer = 0.1;
        keeper.keeper_release_state = Some(KeeperBehaviorState::Advance);
        keeper.keeper_release_motion = 0.5;
        keeper.keeper_release_kind = Some(KeeperShotType::Chip);
        keeper.keeper_release_depth = 42.0;
        keeper.receive_timer = 1.0;
    }
    live.owner = None;
    live.ball = Vec2::new(965.0, 270.0);
    live.ball_vel = Vec2::new(600.0, 0.0);
    live.ball_z = 0.0;
    live.ball_vz = 0.0;
    live.pickup_cd = 1.0;
    live.block_grace = 1.0;

    let boundary = match_snapshot::capture(&live, None);
    let (mut restored, _) = match_snapshot::restore(&boundary);
    assert_eq!(
        restored.players[5].keeper_state,
        KeeperBehaviorState::Retreat
    );
    assert_eq!(
        restored.players[5].keeper_release_kind,
        Some(KeeperShotType::Chip)
    );
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(&restored, None)),
        match_snapshot::hash(&boundary)
    );

    let frame = input_frame::neutral(live.input_tick).expect("neutral frame");
    let tune = Tuning::new();
    sim_match::step(
        &mut live,
        fixed_clock::TICK_SECONDS,
        StepInput::Frame(&frame),
        None,
        &tune,
    );
    sim_match::step(
        &mut restored,
        fixed_clock::TICK_SECONDS,
        StepInput::Frame(&frame),
        None,
        &tune,
    );

    assert_eq!(live.score.home, 1);
    assert!(live.kickoff_hold > 0.0);
    let owner_idx = live.owner.expect("kickoff assigns an owner");
    assert_eq!(live.players[(owner_idx - 1) as usize].team, MatchTeam::Away);
    assert_eq!(live.players[5].keeper_state, KeeperBehaviorState::Base);
    assert_eq!(live.players[5].keeper_release_kind, None);
    assert_eq!(
        match_snapshot::hash(&match_snapshot::capture(&restored, None)),
        match_snapshot::hash(&match_snapshot::capture(&live, None))
    );
}

#[test]
fn canonical_match_snapshots_converges_snapshot_advance_restore_and_replay_at_every_boundary() {
    let mut live = new_state();
    let initial = match_snapshot::capture(&live, None);
    let mut frames = [
        input_frame::neutral(0).expect("neutral frame"),
        input_frame::neutral(1).expect("neutral frame"),
        input_frame::neutral(2).expect("neutral frame"),
    ];
    frames[0].slots[0] = input_frame::new_sample(InputSampleOptions {
        move_x: Some(127),
        ..Default::default()
    })
    .expect("sample");
    frames[1].slots[4] = input_frame::new_sample(InputSampleOptions {
        move_y: Some(-127),
        ..Default::default()
    })
    .expect("sample");

    let tune = Tuning::new();
    let mut hashes = vec![match_snapshot::hash(&initial)];
    for frame in &frames {
        sim_match::step(
            &mut live,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(frame),
            None,
            &tune,
        );
        hashes.push(match_snapshot::hash(&match_snapshot::capture(&live, None)));
    }

    let (mut restored, _) = match_snapshot::restore(&initial);
    // Rust's ownership model makes the Lua case's `restored ~= live` check
    // (reference-identity inequality) moot: `restore` always returns an
    // owned value, so `restored` can never alias `live` in the first place
    // -- there is no way to even express that comparison. The closest
    // still-meaningful check is that the two states' *content* differs at
    // this point (one has been stepped three times already, the other is
    // freshly restored from the pre-step snapshot), which is what the Lua
    // case's surrounding narrative (an independently steppable copy) is
    // really about.
    assert_ne!(restored, live);
    for (index, frame) in frames.iter().enumerate() {
        sim_match::step(
            &mut restored,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(frame),
            None,
            &tune,
        );
        assert_eq!(
            match_snapshot::hash(&match_snapshot::capture(&restored, None)),
            hashes[index + 1],
            "restored boundary {}",
            index + 1
        );
    }
    assert!(
        match_snapshot::first_difference(
            &match_snapshot::capture(&live, None),
            &match_snapshot::capture(&restored, None)
        )
        .is_none()
    );
}

#[test]
fn canonical_match_snapshots_serializes_independent_of_table_insertion_order() {
    // The Lua case's core mechanism re-inserts `MATCH_FIELDS` in reverse
    // order into a fresh table and checks the encoding is byte-identical --
    // proving the canonical encoder doesn't depend on Lua's table iteration
    // order. `MatchState` in Rust is a typed struct with a fixed field
    // layout; there is no `MATCH_FIELDS` allowlist to reorder (see the
    // schema-pin case's retirement above) and no way to construct a struct
    // value whose fields were "inserted" in a different order -- a struct
    // literal has no runtime notion of insertion order to vary in the first
    // place. So the reordering half of this case is structurally
    // guaranteed rather than merely already-passing, and is dropped; the
    // surviving assertions (canonical encoding agrees with restore-then-encode,
    // and the counting encoder agrees with the materializing one) are
    // ported.
    let snapshot = match_snapshot::capture(&new_state(), None);
    assert_eq!(
        match_snapshot::encode_canonical(&snapshot),
        match_snapshot::encode(&snapshot)
    );
    assert_eq!(
        match_snapshot::encoded_size_canonical(&snapshot),
        match_snapshot::encode(&snapshot).len()
    );
    assert_eq!(
        match_snapshot::hash_canonical(&snapshot),
        match_snapshot::hash(&snapshot)
    );
}

#[test]
fn canonical_match_snapshots_compares_owned_canonical_snapshots_without_normalizing_them_again() {
    let left = match_snapshot::capture(&new_state(), None);
    let mut right = match_snapshot::capture(&new_state(), None);
    assert!(match_snapshot::first_difference_canonical(&left, &right).is_none());
    right.state.score.home = 1;

    let expected = match_snapshot::first_difference(&left, &right).expect("difference expected");

    // The Lua case additionally monkey-patches `match_snapshot.capture`/
    // `.restore` to `error(...)` for the duration of the call, proving
    // `first_difference_canonical` never normalizes its inputs by calling
    // them. Rust has no equivalent of reassigning a module function at
    // runtime -- `match_snapshot::capture`/`::restore` are free functions,
    // not mutable fields on a value that could be monkey-patched. The fact
    // itself is verified by inspection instead: reading `match_snapshot.rs`
    // shows `first_difference_canonical` calls `first_difference_canonical_inner`
    // directly on its two arguments, with no `capture`/`restore` call
    // anywhere in that path. The still-expressible half of this case (the
    // canonical comparison finds the same difference `first_difference`
    // does) is kept.
    let actual =
        match_snapshot::first_difference_canonical(&left, &right).expect("difference expected");
    assert_eq!(actual.path, expected.path);
    assert_eq!(actual.expected, expected.expected);
    assert_eq!(actual.actual, expected.actual);
}

#[test]
fn canonical_match_snapshots_compares_every_canonical_windup_shot_field() {
    let mut state = new_state();
    state.players[2].windup_shot = Some(WindupShot {
        dir: Vec2::new(0.25, -0.75),
        speed: 456.0,
        vz: 123.0,
        spin: -8.0,
        shot_type: KeeperShotType::Chip,
    });
    let left = match_snapshot::capture(&state, None);
    let mut right = match_snapshot::capture(&state, None);
    right.state.players[2]
        .windup_shot
        .as_mut()
        .unwrap()
        .shot_type = KeeperShotType::Ground;

    let found =
        match_snapshot::first_difference_canonical(&left, &right).expect("difference expected");
    assert_eq!(found.path, "state.players.3.windup_shot.shot_type");
    // `MatchSnapshotDifference.expected`/`.actual` render via Rust's derived
    // `Debug` ("rendered for display", per the struct's own doc), not the
    // wire spelling -- so these read "Chip"/"Ground" (the enum variant
    // names) rather than the Lua original's `"chip"`/`"ground"` wire
    // strings. The path, and that a difference is found at all, are the
    // load-bearing parts of this case; both match the Lua original exactly.
    assert_eq!(found.expected, "Chip");
    assert_eq!(found.actual, "Ground");
}

#[test]
fn canonical_match_snapshots_compares_every_canonical_outfield_decision_field() {
    let state = new_ai_state();
    let left = match_snapshot::capture(&state, None);
    let mut right = match_snapshot::capture(&state, None);
    let base = right.state.players[1].outfield_decision;
    right.state.players[1].outfield_decision = outfield_decision::refresh(
        &base,
        OutfieldDecisionContext::Offball,
        OutfieldIntent::HoldWidth,
        0.5,
        Some(450.0),
        Some(20.0),
        None,
        Some(-0.3),
    );
    let found =
        match_snapshot::first_difference_canonical(&left, &right).expect("difference expected");
    assert_eq!(found.path, "state.players.2.outfield_decision.generation");

    let mut left = match_snapshot::capture(&state, None);
    let mut right = match_snapshot::capture(&state, None);
    let left_base = left.state.players[1].outfield_decision;
    left.state.players[1].outfield_decision = outfield_decision::refresh(
        &left_base,
        OutfieldDecisionContext::Offball,
        OutfieldIntent::HoldWidth,
        0.5,
        Some(450.0),
        Some(20.0),
        None,
        Some(-0.3),
    );
    let right_base = right.state.players[1].outfield_decision;
    right.state.players[1].outfield_decision = outfield_decision::refresh(
        &right_base,
        OutfieldDecisionContext::Offball,
        OutfieldIntent::HoldWidth,
        0.5,
        Some(450.0),
        Some(20.0),
        None,
        Some(-0.2),
    );
    let found =
        match_snapshot::first_difference_canonical(&left, &right).expect("difference expected");
    assert_eq!(
        found.path,
        "state.players.2.outfield_decision.run_expires_at"
    );
}

#[test]
fn canonical_match_snapshots_restores_and_diffs_both_authoritative_formation_identities() {
    let state = new_state();
    let snapshot = match_snapshot::capture(&state, None);
    let (restored, _) = match_snapshot::restore(&snapshot);
    assert_eq!(restored.formation.home, "2-1-1");
    assert_eq!(restored.formation.away, "1-1-2");

    let mut changed = match_snapshot::capture(&state, None);
    changed.state.formation.home = "1-2-1".to_string();
    let found = match_snapshot::first_difference_canonical(&snapshot, &changed)
        .expect("difference expected");
    assert_eq!(found.path, "state.formation.home");

    // The Lua case also `rawset`s an undeclared `extra` key onto the
    // formation table and expects `restore` to reject it. `ByTeam<String>`
    // (this module's `formation` field type) is a fixed two-field struct
    // with no room for an extra key -- there is no way to construct that
    // value -- so that half of the case is dropped; the still-constructible
    // half (an unauthored formation string) is kept below.
    let mut unknown = match_snapshot::capture(&state, None);
    unknown.state.formation.away = "future-shape".to_string();
    assert!(fails(|| {
        let _ = match_snapshot::restore(&unknown);
    }));
}

#[test]
fn canonical_match_snapshots_restores_and_hashes_team_press_state_across_soccer_and_combat_boundaries()
 {
    for combat_active in [false, true] {
        let mut state = new_ai_state();
        state.outfield_press.home = outfield_press::resolve(
            2,
            &OutfieldPressContext {
                heavy_touch: true,
                exposed_ball: false,
                cover_available: false,
                box_desperation: false,
                press_discipline: 1.0,
            },
        );
        let companion = if combat_active {
            Some(combat::new_state(&mut state, None))
        } else {
            None
        };
        let snapshot = match_snapshot::capture(&state, companion.as_ref());
        let (restored, restored_combat) = match_snapshot::restore(&snapshot);
        assert_eq!(restored.outfield_press.home.presser_index, Some(2));
        assert_eq!(restored.outfield_press.home.mode, StablePressMode::Commit);
        assert_eq!(restored.outfield_press.home.reason, PressReason::HeavyTouch);
        assert_eq!(
            match_snapshot::hash(&match_snapshot::capture(
                &restored,
                restored_combat.as_ref()
            )),
            match_snapshot::hash(&snapshot)
        );
    }
}

#[test]
fn canonical_match_snapshots_diffs_and_strictly_validates_nested_press_state_relations() {
    let mut state = new_ai_state();
    state.outfield_press.home = outfield_press::contain(2);
    let left = match_snapshot::capture(&state, None);
    let mut right = match_snapshot::capture(&state, None);
    right.state.outfield_press.home = outfield_press::resolve(
        2,
        &OutfieldPressContext {
            heavy_touch: false,
            exposed_ball: false,
            cover_available: true,
            box_desperation: false,
            press_discipline: 1.0,
        },
    );
    let found =
        match_snapshot::first_difference_canonical(&left, &right).expect("difference expected");
    assert_eq!(found.path, "state.outfield_press.home.mode");

    // The Lua case's sixth mutation, `press.unknown = true`, adds an
    // undeclared field via `rawset`. `OutfieldPressState` is a four-field
    // struct with no room for an extra key -- there is no way to construct
    // that value -- so it is dropped; the five still-constructible
    // mutations are ported below.
    type PressMutation = Box<dyn Fn(&mut OutfieldPressState)>;
    let mutations: Vec<PressMutation> = vec![
        Box::new(|press: &mut OutfieldPressState| press.version = outfield_press::VERSION + 1),
        Box::new(|press: &mut OutfieldPressState| press.presser_index = None),
        Box::new(|press: &mut OutfieldPressState| press.presser_index = Some(99)),
        Box::new(|press: &mut OutfieldPressState| press.presser_index = Some(1)),
        Box::new(|press: &mut OutfieldPressState| press.presser_index = Some(7)),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut malformed = match_snapshot::capture(&state, None);
        mutate(&mut malformed.state.outfield_press.home);
        assert!(
            fails(|| {
                let _ = match_snapshot::restore(&malformed);
            }),
            "malformed press state {} was accepted",
            index + 1
        );
    }

    let fixed = new_state();
    let mut fixed_snapshot = match_snapshot::capture(&fixed, None);
    fixed_snapshot.state.outfield_press.home = outfield_press::contain(2);
    assert!(
        fails(|| {
            let _ = match_snapshot::restore(&fixed_snapshot);
        }),
        "fixed-slot presser relation was accepted"
    );

    let human = new_human_state();
    let mut human_snapshot = match_snapshot::capture(&human, None);
    human_snapshot.state.outfield_press.home = outfield_press::contain(human.controlled as u32);
    assert!(
        fails(|| {
            let _ = match_snapshot::restore(&human_snapshot);
        }),
        "human-controlled presser relation was accepted"
    );
}

#[test]
fn canonical_match_snapshots_rejects_malformed_v10_and_v11_decision_contracts_during_restore() {
    let mut state = new_state();
    let mut soccer = match_snapshot::capture(&state, None);
    {
        let d = &mut soccer.state.players[1].outfield_decision;
        d.context = OutfieldDecisionContext::Offball;
        d.intent = OutfieldIntent::Shoot;
        d.remaining = 0.2;
    }
    assert!(fails(|| {
        let _ = match_snapshot::restore(&soccer);
    }));

    let combat_for_boundary = combat::new_state(&mut state, None);
    let mut combat_boundary = match_snapshot::capture(&state, Some(&combat_for_boundary));
    {
        let d = &mut combat_boundary.state.players[1].outfield_decision;
        d.context = OutfieldDecisionContext::Carrier;
        d.intent = OutfieldIntent::Move;
        d.remaining = 0.2;
        d.target_x = Some(100.0);
        d.target_y = Some(200.0);
    }
    assert!(fails(|| {
        let _ = match_snapshot::restore(&combat_boundary);
    }));

    let state = new_attacking_ai_state();
    let mut valid_run = match_snapshot::capture(&state, None);
    let base = valid_run.state.players[1].outfield_decision;
    valid_run.state.players[1].outfield_decision = outfield_decision::refresh(
        &base,
        OutfieldDecisionContext::Offball,
        OutfieldIntent::InBehind,
        0.5,
        Some(500.0),
        Some(220.0),
        None,
        Some(-0.2),
    );
    assert!(!fails(|| {
        let _ = match_snapshot::restore(&valid_run);
    }));
    let active = valid_run.state.players[1].outfield_decision;
    let cancelled = outfield_decision::cancel_run(&active, 420.0, 220.0);
    assert_eq!(cancelled.generation, active.generation);
    assert_eq!(cancelled.remaining, active.remaining);
    assert_eq!(cancelled.intent, OutfieldIntent::Move);
    assert_eq!(cancelled.run_expires_at, None);
    valid_run.state.players[1].outfield_decision = cancelled;
    assert!(!fails(|| {
        let _ = match_snapshot::restore(&valid_run);
    }));

    type DecisionMutation = Box<dyn Fn(&mut OutfieldDecisionState)>;
    let mutations: Vec<DecisionMutation> = vec![
        Box::new(|d: &mut OutfieldDecisionState| d.version = outfield_decision::VERSION - 1),
        Box::new(|d: &mut OutfieldDecisionState| d.run_expires_at = None),
        Box::new(|d: &mut OutfieldDecisionState| d.run_expires_at = Some(f64::NAN)),
        Box::new(|d: &mut OutfieldDecisionState| d.run_expires_at = Some(-2.0)),
        Box::new(|d: &mut OutfieldDecisionState| d.run_expires_at = Some(0.0)),
        Box::new(|d: &mut OutfieldDecisionState| {
            d.target_x = None;
            d.target_y = None;
        }),
        Box::new(|d: &mut OutfieldDecisionState| d.target_player = Some(3)),
        Box::new(|d: &mut OutfieldDecisionState| d.context = OutfieldDecisionContext::Carrier),
        Box::new(|d: &mut OutfieldDecisionState| d.intent = OutfieldIntent::Move),
        // The Lua case's tenth mutation, `decision.unknown_run_field =
        // true`, adds an undeclared field via `rawset`. `OutfieldDecisionState`
        // is a fixed-field struct with no room for an extra key, so it is
        // dropped; the nine still-constructible mutations above are ported.
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut malformed = match_snapshot::capture(&state, None);
        let base = malformed.state.players[1].outfield_decision;
        malformed.state.players[1].outfield_decision = outfield_decision::refresh(
            &base,
            OutfieldDecisionContext::Offball,
            OutfieldIntent::ComeShort,
            0.5,
            Some(300.0),
            Some(220.0),
            None,
            Some(-0.2),
        );
        mutate(&mut malformed.state.players[1].outfield_decision);
        assert!(
            fails(|| {
                let _ = match_snapshot::restore(&malformed);
            }),
            "malformed v9 run decision {} was accepted",
            index + 1
        );
    }

    let mut malformed_combat_state = state.clone();
    let combat_for_malformed = combat::new_state(&mut malformed_combat_state, None);
    let mut malformed_combat =
        match_snapshot::capture(&malformed_combat_state, Some(&combat_for_malformed));
    let base = malformed_combat.state.players[6].outfield_decision;
    malformed_combat.state.players[6].outfield_decision = outfield_decision::refresh(
        &base,
        OutfieldDecisionContext::Offball,
        OutfieldIntent::HoldWidth,
        0.5,
        Some(500.0),
        Some(30.0),
        None,
        Some(-0.2),
    );
    malformed_combat.state.players[6]
        .outfield_decision
        .run_expires_at = Some(f64::INFINITY);
    assert!(fails(|| {
        let _ = match_snapshot::restore(&malformed_combat);
    }));

    let too_many = new_attacking_ai_state();
    let mut too_many_snapshot = match_snapshot::capture(&too_many, None);
    for &index in &[1usize, 2, 3] {
        let base = too_many_snapshot.state.players[index].outfield_decision;
        let scan_rate = too_many_snapshot.state.players[index].scan_rate;
        too_many_snapshot.state.players[index].outfield_decision = outfield_decision::refresh(
            &base,
            OutfieldDecisionContext::Offball,
            OutfieldIntent::InBehind,
            scan_rate,
            Some(500.0 + (index + 1) as f64),
            Some(200.0 + (index + 1) as f64),
            None,
            Some(-0.2),
        );
    }
    assert!(
        fails(|| {
            let _ = match_snapshot::restore(&too_many_snapshot);
        }),
        "third same-team run was accepted"
    );

    fn set_ordinary_teammate_owner(state: &mut MatchState, runner_index: usize) {
        let runner_team = state.players[runner_index].team;
        for (index, player) in state.players.iter().enumerate() {
            if index != runner_index && player.team == runner_team && !player.is_keeper {
                state.owner = Some((index + 1) as i64);
                state.kickoff_hold = 0.0;
                state.finished = false;
                return;
            }
        }
        panic!("relation fixture requires an ordinary teammate owner");
    }

    struct RelationCase {
        name: &'static str,
        state: MatchState,
        player_index: usize,
        target_x: f64,
        target_y: f64,
    }

    let mut fixed_case_state = new_state();
    fixed_case_state.human_controlled = false;
    set_ordinary_teammate_owner(&mut fixed_case_state, 1);

    let mut human_case_state = new_human_state();
    let human_controlled_index = (human_case_state.controlled - 1) as usize;
    set_ordinary_teammate_owner(&mut human_case_state, human_controlled_index);

    let relation_cases = vec![
        RelationCase {
            name: "keeper",
            state: new_attacking_ai_state(),
            player_index: 0,
            target_x: 500.0,
            target_y: 200.0,
        },
        RelationCase {
            name: "fixed slot",
            state: fixed_case_state,
            player_index: 1,
            target_x: 500.0,
            target_y: 200.0,
        },
        RelationCase {
            name: "outside field",
            state: new_attacking_ai_state(),
            player_index: 1,
            target_x: -1.0,
            target_y: 200.0,
        },
        RelationCase {
            name: "human control",
            state: human_case_state,
            player_index: human_controlled_index,
            target_x: 500.0,
            target_y: 200.0,
        },
    ];
    for case in relation_cases {
        if case.name == "fixed slot" || case.name == "human control" {
            let relation_state = &case.state;
            let relation_player = &relation_state.players[case.player_index];
            let owner_index = relation_state.owner.expect("relation fixture has an owner");
            let relation_owner = &relation_state.players[(owner_index - 1) as usize];
            assert!(
                !relation_player.is_keeper,
                "{} runner is a keeper",
                case.name
            );
            assert_ne!(
                owner_index as usize - 1,
                case.player_index,
                "{} runner owns the ball",
                case.name
            );
            assert_eq!(
                relation_owner.team, relation_player.team,
                "{} owner belongs to the wrong team",
                case.name
            );
            assert!(!relation_owner.is_keeper, "{} owner is a keeper", case.name);
            assert!(
                relation_state.kickoff_hold <= 0.0 && !relation_state.finished,
                "{} fixture is outside ordinary attack",
                case.name
            );
            if case.name == "fixed slot" {
                assert!(
                    relation_state.slot_for_player[case.player_index].is_some(),
                    "fixed-slot fixture runner has no slot"
                );
                assert!(
                    !relation_state.human_controlled
                        || relation_state.controlled != (case.player_index + 1) as i64,
                    "fixed-slot fixture also violates human control"
                );
            } else {
                assert_eq!(
                    relation_state.slot_for_player[case.player_index], None,
                    "human fixture runner owns a fixed slot"
                );
                assert!(
                    relation_state.human_controlled
                        && relation_state.controlled == (case.player_index + 1) as i64,
                    "human fixture runner is not human-controlled"
                );
            }
        }
        let mut snapshot = match_snapshot::capture(&case.state, None);
        let base = snapshot.state.players[case.player_index].outfield_decision;
        let scan_rate = snapshot.state.players[case.player_index].scan_rate;
        snapshot.state.players[case.player_index].outfield_decision = outfield_decision::refresh(
            &base,
            OutfieldDecisionContext::Offball,
            OutfieldIntent::InBehind,
            scan_rate,
            Some(case.target_x),
            Some(case.target_y),
            None,
            Some(-0.2),
        );
        assert!(
            fails(|| {
                let _ = match_snapshot::restore(&snapshot);
            }),
            "{} run relation was accepted",
            case.name
        );
    }

    fn ordinary_run_boundary() -> MatchSnapshot {
        let mut snapshot = match_snapshot::capture(&new_attacking_ai_state(), None);
        let base = snapshot.state.players[1].outfield_decision;
        let scan_rate = snapshot.state.players[1].scan_rate;
        snapshot.state.players[1].outfield_decision = outfield_decision::refresh(
            &base,
            OutfieldDecisionContext::Offball,
            OutfieldIntent::ComeShort,
            scan_rate,
            Some(360.0),
            Some(220.0),
            None,
            Some(-0.2),
        );
        snapshot
    }

    type SnapshotMutation = (&'static str, Box<dyn Fn(&mut MatchSnapshot)>);
    let possession_mutations: Vec<SnapshotMutation> = vec![
        (
            "loose ball",
            Box::new(|s: &mut MatchSnapshot| s.state.owner = None),
        ),
        (
            "opponent possession",
            Box::new(|s: &mut MatchSnapshot| s.state.owner = Some(7)),
        ),
        (
            "keeper build-up",
            Box::new(|s: &mut MatchSnapshot| s.state.owner = Some(1)),
        ),
        (
            "kickoff hold",
            Box::new(|s: &mut MatchSnapshot| s.state.kickoff_hold = 0.2),
        ),
        (
            "finished match",
            Box::new(|s: &mut MatchSnapshot| s.state.finished = true),
        ),
    ];
    for (name, mutate) in &possession_mutations {
        let mut malformed = ordinary_run_boundary();
        mutate(&mut malformed);
        assert!(
            fails(|| {
                let _ = match_snapshot::restore(&malformed);
            }),
            "{name} retained an active run"
        );
    }

    type PlayerMutation = (&'static str, Box<dyn Fn(&mut MatchPlayer)>);
    let commitment_mutations: Vec<PlayerMutation> = vec![
        ("stun", Box::new(|p: &mut MatchPlayer| p.stun_timer = 0.2)),
        ("slide", Box::new(|p: &mut MatchPlayer| p.slide_timer = 0.2)),
        (
            "tackle",
            Box::new(|p: &mut MatchPlayer| p.tackle_timer = 0.2),
        ),
        ("dodge", Box::new(|p: &mut MatchPlayer| p.dodge_timer = 0.2)),
        (
            "jockey",
            Box::new(|p: &mut MatchPlayer| p.jockey_timer = 0.2),
        ),
        (
            "wind-up timer",
            Box::new(|p: &mut MatchPlayer| p.windup_timer = 0.2),
        ),
        (
            "wind-up payload",
            Box::new(|p: &mut MatchPlayer| {
                p.windup_shot = Some(WindupShot {
                    dir: Vec2::new(1.0, 0.0),
                    speed: 300.0,
                    vz: 0.0,
                    spin: 0.0,
                    shot_type: KeeperShotType::Ground,
                });
            }),
        ),
        (
            "aerial action",
            Box::new(|p: &mut MatchPlayer| p.aerial_timer = 0.2),
        ),
        (
            "aerial recovery",
            Box::new(|p: &mut MatchPlayer| p.aerial_recovery = 0.2),
        ),
        (
            "reception",
            Box::new(|p: &mut MatchPlayer| p.receive_timer = 0.2),
        ),
    ];
    for (name, mutate) in &commitment_mutations {
        let mut malformed = ordinary_run_boundary();
        mutate(&mut malformed.state.players[1]);
        assert!(
            fails(|| {
                let _ = match_snapshot::restore(&malformed);
            }),
            "{name} retained an active run"
        );
    }
}

#[test]
fn canonical_match_snapshots_encodes_decision_children_positionally_with_exact_no_run_v11_arithmetic()
 {
    let legacy_fields = [
        "version",
        "generation",
        "rng_state",
        "remaining",
        "context",
        "intent",
        "target_x",
        "target_y",
        "target_player",
    ];
    let mut legacy_key_bytes = 0usize;
    for field in legacy_fields {
        legacy_key_bytes += format!("k{}:{};", field.len(), field).len();
    }
    assert_eq!(legacy_key_bytes, 115);

    let home = teams::get("nebula").expect("nebula team is authored");
    let away = teams::get("orion").expect("orion team is authored");
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(120.0),
        max_goals: Some(3),
        seed: Some(38.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: Some(sim_match::ownership_for_teams(home, away, None)),
    });
    let tune = Tuning::new();
    for tick in 0..120i64 {
        let frame = input_frame::neutral(tick).expect("neutral frame");
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Frame(&frame),
            None,
            &tune,
        );
    }
    // A slot-mode neutral fixture never turns the ball over, so the
    // transition block prices its established-but-idle spelling exactly.
    assert_eq!(
        state.transition.last_team,
        Some(gc_sim::possession_transition::TransitionTeam::Home)
    );
    assert_eq!(
        state.transition.holding_team,
        Some(gc_sim::possession_transition::TransitionTeam::Home)
    );
    assert_eq!(
        state.transition.hold,
        gc_sim::possession_transition::ESTABLISH_SECONDS
    );
    assert_eq!(state.transition.turnover_team, None);
    assert_eq!(state.transition.elapsed, 0.0);

    let balanced_window = format!("n{};", match_snapshot::number_bytes(2.5)).len();
    let transition_bytes = "k18:transition_windows;".len()
        + 2 * ("k12:counterpress;".len() + "k13:counterattack;".len() + 2 * balanced_window)
        + "k10:transition;".len()
        + "k7:version;".len()
        + format!("n{};", match_snapshot::number_bytes(1.0)).len()
        + "k9:last_team;".len()
        + "s4:home;".len()
        + "k12:holding_team;".len()
        + "s4:home;".len()
        + "k4:hold;".len()
        + format!(
            "n{};",
            match_snapshot::number_bytes(gc_sim::possession_transition::ESTABLISH_SECONDS)
        )
        .len()
        + "k13:turnover_team;".len()
        + "z;".len()
        + "k7:elapsed;".len()
        + "nz;".len();
    assert_eq!(transition_bytes, 311);

    let encoded = match_snapshot::encode(&match_snapshot::capture(&state, None));
    let expected = 21343 - 10 * legacy_key_bytes
        + 10 * "z;".len()
        + "k9:formation;".len()
        + "s5:2-1-1;".len()
        + "s5:1-1-2;".len()
        + 10 * "k19:keeper_get_up_timer;nz;".len()
        + transition_bytes;
    assert_eq!(encoded.len(), expected);
    assert_eq!(encoded.len(), 20825);

    let decision_marker = "k17:outfield_decision;d;";
    let next_field_marker = "k9:is_keeper;";
    let mut search_at = 0usize;
    let mut count = 0;
    while let Some(rel) = encoded[search_at..].find(decision_marker) {
        let start_at = search_at + rel;
        let decision_start = start_at + decision_marker.len();
        let decision_end = decision_start
            + encoded[decision_start..]
                .find(next_field_marker)
                .expect("next field marker present");
        let decision_wire = &encoded[decision_start..decision_end];
        assert!(
            !decision_wire.contains('k'),
            "decision child emitted a named key"
        );
        count += 1;
        search_at = decision_end + next_field_marker.len();
    }
    assert_eq!(count, 10);
}

fn scalar_bytes_num(n: f64) -> usize {
    match_snapshot::number_bytes(n).len() + 2
}
fn scalar_bytes_opt_num(n: Option<f64>) -> usize {
    match n {
        None => "z;".len(),
        Some(v) => scalar_bytes_num(v),
    }
}
fn scalar_bytes_opt_i(n: Option<i64>) -> usize {
    scalar_bytes_opt_num(n.map(|v| v as f64))
}
fn scalar_bytes_str(s: &str) -> usize {
    format!("s{}:{};", s.len(), s).len()
}

fn decision_field_bytes(d: &OutfieldDecisionState) -> [usize; 10] {
    [
        scalar_bytes_num(f64::from(d.version)),
        scalar_bytes_num(f64::from(d.generation)),
        scalar_bytes_num(f64::from(d.rng_state)),
        scalar_bytes_num(d.remaining),
        scalar_bytes_str(wire_context(d.context)),
        scalar_bytes_str(wire_intent(d.intent)),
        scalar_bytes_opt_num(d.target_x),
        scalar_bytes_opt_num(d.target_y),
        scalar_bytes_opt_i(d.target_player.map(i64::from)),
        scalar_bytes_opt_num(d.run_expires_at),
    ]
}

fn press_field_bytes(p: &OutfieldPressState) -> [usize; 4] {
    [
        scalar_bytes_num(f64::from(p.version)),
        scalar_bytes_opt_i(p.presser_index.map(i64::from)),
        scalar_bytes_str(wire_press_mode(p.mode)),
        scalar_bytes_str(wire_press_reason(p.reason)),
    ]
}

#[test]
fn canonical_match_snapshots_prices_four_hypothetical_runs_on_the_valid_high_overhead_slot_mode_base()
 {
    let mut state = new_state();
    assert!(state.slot_mode);
    assert!(state.input_ownership.is_some());
    for slot_index in 1..=input_frame::SLOT_COUNT {
        let player_index =
            state.slot_players[(slot_index - 1) as usize].expect("slot is filled in slot mode");
        assert_eq!(
            state.slot_for_player[(player_index - 1) as usize],
            Some(slot_index)
        );
    }

    let soccer = match_snapshot::capture(&state, None);
    let combat_state = combat::new_state(&mut state, None);
    let combat_boundary = match_snapshot::capture(&state, Some(&combat_state));
    let soccer_bytes = match_snapshot::encoded_size_canonical(&soccer) as i64;
    let combat_bytes = match_snapshot::encoded_size_canonical(&combat_boundary) as i64;
    assert_eq!(
        soccer.state.outfield_press.home.version,
        outfield_press::VERSION
    );
    assert_eq!(
        soccer.state.outfield_press.away.version,
        outfield_press::VERSION
    );

    let run_records: [(usize, OutfieldIntent, f64, f64); 4] = [
        (1, OutfieldIntent::ComeShort, 350.0, 120.0),
        (2, OutfieldIntent::InBehind, 700.0, 180.0),
        (6, OutfieldIntent::HoldWidth, 610.0, 510.0),
        (7, OutfieldIntent::InBehind, 220.0, 300.0),
    ];
    let mut four_run_delta: i64 = 0;
    for (idx, intent, x, y) in run_records {
        let player = &soccer.state.players[idx];
        let base_decision = player.outfield_decision;
        let active_decision = outfield_decision::refresh(
            &base_decision,
            OutfieldDecisionContext::Offball,
            intent,
            player.scan_rate,
            Some(x),
            Some(y),
            None,
            Some(-0.2),
        );
        assert_eq!(active_decision.generation, base_decision.generation + 1);
        assert!(active_decision.remaining > 0.0);
        let active_bytes = decision_field_bytes(&active_decision);
        let base_bytes = decision_field_bytes(&base_decision);
        for i in 0..10 {
            four_run_delta += active_bytes[i] as i64 - base_bytes[i] as i64;
        }
    }

    let mut press_delta: i64 = 0;
    for (base_press, active_press) in [
        (
            &soccer.state.outfield_press.home,
            outfield_press::contain(2),
        ),
        (
            &soccer.state.outfield_press.away,
            outfield_press::contain(7),
        ),
    ] {
        let base_bytes = press_field_bytes(base_press);
        let active_bytes = press_field_bytes(&active_press);
        for i in 0..4 {
            press_delta += active_bytes[i] as i64 - base_bytes[i] as i64;
        }
    }

    let combined_delta = four_run_delta + press_delta;
    let budget = omp2_rollback_validation::DATA.budgets.snapshot_bytes;
    let boundaries = omp2_rollback_validation::DATA.budgets.snapshot_count;
    let soccer_window = (soccer_bytes + combined_delta) * boundaries;
    let combat_window = (combat_bytes + combined_delta) * boundaries;

    assert_eq!(boundaries, 31);
    assert_eq!(soccer_bytes, 20697);
    assert_eq!(combat_bytes, 24373);
    assert_eq!(four_run_delta, 346);
    assert_eq!(press_delta, 26);
    assert_eq!(combined_delta, 372);
    assert_eq!(soccer_window, 653139);
    assert_eq!(combat_window, 767095);
    assert_eq!(budget - soccer_window, 264365);
    assert_eq!(budget - combat_window, 150409);
    assert!(soccer_window < budget);
    assert!(combat_window < budget);
}

fn pick_longest<T: Copy>(values: &[T], wire: impl Fn(T) -> &'static str) -> T {
    let mut best = values[0];
    let mut best_str = wire(best);
    for &v in &values[1..] {
        let s = wire(v);
        if s.len() > best_str.len() || (s.len() == best_str.len() && s < best_str) {
            best = v;
            best_str = s;
        }
    }
    best
}

#[test]
fn canonical_match_snapshots_prices_the_worst_case_combat_event_row_against_the_retained_window() {
    let mut state = new_state();
    let combat_state = combat::new_state(&mut state, None);
    let snapshot = match_snapshot::capture(&state, Some(&combat_state));
    assert_eq!(snapshot.combat.as_ref().unwrap().events.len(), 0);

    let budget = omp2_rollback_validation::DATA.budgets.snapshot_bytes;
    let boundaries = omp2_rollback_validation::DATA.budgets.snapshot_count;
    let combat_bytes = match_snapshot::encoded_size_canonical(&snapshot) as i64;
    assert_eq!(combat_bytes, 24373);
    // The active-AI delta priced by the measurement above.
    let combined_delta: i64 = 372;
    let combat_window = (combat_bytes + combined_delta) * boundaries;
    assert_eq!(combat_window, 767095);

    let worst_kind = pick_longest(
        &[
            CombatEventKind::RequestRejected,
            CombatEventKind::Commit,
            CombatEventKind::ProjectileSpawn,
            CombatEventKind::ProjectileExpire,
            CombatEventKind::Contact,
            CombatEventKind::BallSpill,
            CombatEventKind::Forced,
            CombatEventKind::GuardRecoil,
            CombatEventKind::Miss,
            CombatEventKind::Interrupted,
            CombatEventKind::Cancelled,
            CombatEventKind::MatchTerminated,
        ],
        CombatEventKind::wire_str,
    );
    let worst_family = pick_longest(
        &[
            ActionFamilyId::Unarmed,
            ActionFamilyId::Guard,
            ActionFamilyId::LightMelee,
            ActionFamilyId::Ranged,
        ],
        combat_snapshot::action_family_wire_id,
    );
    let worst_result = pick_longest(
        &[
            CombatContactResult::Hit,
            CombatContactResult::Extended,
            CombatContactResult::Guarded,
            CombatContactResult::Immune,
            CombatContactResult::Superseded,
        ],
        CombatContactResult::wire_str,
    );
    let worst_outcome = pick_longest(
        &[
            CombatRequestOutcome::Accepted,
            CombatRequestOutcome::Rejected,
        ],
        CombatRequestOutcome::wire_str,
    );
    let worst_reason = pick_longest(
        &[
            CombatRequestRejectionReason::ProtectedKeeperOrNoLoadout,
            CombatRequestRejectionReason::KickoffHold,
            CombatRequestRejectionReason::SoccerCommitment,
            CombatRequestRejectionReason::AerialStateOrRecovery,
            CombatRequestRejectionReason::ForcedState,
            CombatRequestRejectionReason::AlreadyCommitted,
            CombatRequestRejectionReason::Cooldown,
            CombatRequestRejectionReason::MissingPressEdge,
            CombatRequestRejectionReason::MalformedInput,
        ],
        CombatRequestRejectionReason::wire_str,
    );
    let worst_terminal = pick_longest(
        &[
            CombatEncounterTerminal::Miss,
            CombatEncounterTerminal::Expire,
            CombatEncounterTerminal::Guarded,
            CombatEncounterTerminal::Immune,
            CombatEncounterTerminal::Superseded,
            CombatEncounterTerminal::Hit,
            CombatEncounterTerminal::Interrupted,
            CombatEncounterTerminal::Cancelled,
            CombatEncounterTerminal::MatchTerminated,
        ],
        CombatEncounterTerminal::wire_str,
    );

    assert_eq!(worst_kind.wire_str(), "projectile_expire");
    assert_eq!(worst_reason.wire_str(), "protected_keeper_or_no_loadout");
    assert_eq!(worst_terminal.wire_str(), "match_terminated");
    assert_eq!(worst_result.wire_str(), "superseded");
    assert_eq!(
        combat_snapshot::action_family_wire_id(worst_family),
        "light_melee"
    );

    // Worst-case rows in one tick, from the emitter analysis recorded in
    // docs/online/omp2_rollback_validation.md: P + 2*O + 1 + (1 + J) * R
    // with P = 10 players, O = 8 non-keeper outfielders, R <= O ranged
    // players and J = 2 concurrent projectiles per ranged player. It sums
    // independently maximised terms, so it is a ceiling rather than a
    // constructed witness.
    let worst_row = CombatEvent {
        kind: worst_kind,
        tick: 3599,
        family_id: Some(worst_family),
        source_index: Some(10),
        target_index: Some(10),
        source_sequence: Some(9999),
        result: Some(worst_result),
        outcome: Some(worst_outcome),
        reason: Some(worst_reason),
        terminal: Some(worst_terminal),
        x: 959.999_999_999_999_9,
        y: 539.999_999_999_999_9,
        interruption_ticks: Some(30),
        displacement_px: Some(123.456_789_012_345_67),
    };

    // Mutating the captured snapshot bypasses combat_snapshot::copy's
    // validators on purpose -- this measures the encoder, not the
    // simulator, and a valid row of this exact width is not constructible
    // from one tick of play.
    let mut snapshot = snapshot;
    snapshot
        .combat
        .as_mut()
        .unwrap()
        .events
        .push(worst_row.clone());
    let one_row = match_snapshot::encoded_size_canonical(&snapshot) as i64;
    snapshot.combat.as_mut().unwrap().events.push(worst_row);
    let two_rows = match_snapshot::encoded_size_canonical(&snapshot) as i64;
    let row_bytes = two_rows - one_row;

    let event_fields = [
        "kind",
        "tick",
        "family_id",
        "source_index",
        "target_index",
        "source_sequence",
        "result",
        "outcome",
        "reason",
        "terminal",
        "x",
        "y",
        "interruption_ticks",
        "displacement_px",
    ];
    let key_bytes: i64 = event_fields
        .iter()
        .map(|f| format!("k{}:{};", f.len(), f).len() as i64)
        .sum();

    let worst_rows_per_tick: i64 = 10 + 2 * 8 + 1 + 3 * 8;
    assert_eq!(worst_rows_per_tick, 51);

    let headroom = budget - combat_window;
    let rows_per_boundary = headroom / boundaries / row_bytes;
    let worst_tick_window =
        (combat_bytes + combined_delta + worst_rows_per_tick * row_bytes) * boundaries;

    assert_eq!(key_bytes, 179);
    assert_eq!(row_bytes, 456);
    assert_eq!(rows_per_boundary, 10);

    let sustained_window = (combat_bytes + combined_delta + 8 * row_bytes) * boundaries;
    assert!(sustained_window < budget);
    assert_eq!(sustained_window, 880183);

    assert!(worst_tick_window > budget);
    assert_eq!(worst_tick_window, 1488031);
}

#[test]
fn canonical_match_snapshots_rejects_unhandled_state_and_player_fields() {
    // Both of the Lua case's sub-cases `rawset` an undeclared field onto a
    // live `MatchState`/`MatchPlayer` table (`future_match_field`,
    // `future_player_field`) and expect `capture` to reject it. `MatchState`
    // and `MatchPlayer` are fixed-field structs -- exactly the allowlists
    // this file's schema-pin case (the first test above) explains have no
    // separate runtime representation in Rust, because the struct itself
    // *is* the closed field set. There is no way to construct a `MatchState`
    // or `MatchPlayer` value carrying an extra field, so both sub-cases are
    // structurally impossible to violate rather than merely already-passing,
    // and this case is dropped in full.
}

#[test]
fn canonical_match_snapshots_rejects_the_prior_snapshot_schema_instead_of_inventing_keeper_state() {
    let mut snapshot = match_snapshot::capture(&new_state(), None);
    snapshot.version = match_snapshot::VERSION - 1;
    assert!(fails(|| {
        let _ = match_snapshot::restore(&snapshot);
    }));
}

#[test]
fn canonical_match_snapshots_uses_exact_canonical_finite_number_spelling() {
    assert_eq!(match_snapshot::number_bytes(0.0), "z");
    assert_eq!(match_snapshot::number_bytes(-0.0), "Z");
    assert_eq!(match_snapshot::number_bytes(0.5), "p:0:33554432:0");
    assert_eq!(match_snapshot::number_bytes(-1.0), "m:1:33554432:0");
    assert_ne!(
        match_snapshot::number_bytes(0.1),
        match_snapshot::number_bytes(0.10000000000000002)
    );
    assert!(fails(|| {
        let _ = match_snapshot::number_bytes(f64::NAN);
    }));
    assert!(fails(|| {
        let _ = match_snapshot::number_bytes(f64::INFINITY);
    }));
}
