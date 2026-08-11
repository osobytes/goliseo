//! Tests that `gc_sim::combat_observation::CombatObservation` leaks nothing
//! outside its declared contract.
//!
//! Section 4.7 of `docs/design/combat_fun_evidence_contract.md` excludes
//! pixels, cue visibility, render timing, theme, presentation identity,
//! viewport, frame rate, unresolved outcomes, hidden collision results,
//! future input, and RNG state. These tests assert the absence of all of
//! that, and each negative assertion is paired with a POSITIVE CONTROL that
//! feeds the same scanner something that genuinely leaks: a scan that
//! stopped scanning would then pass the negative test and fail the
//! control, instead of passing quietly.
//!
//! [`CombatObservation`] is a typed struct tree with a closed,
//! `pub`-documented field set, so the scan below walks the `{:?}` `Debug`
//! rendering: a forbidden field can only appear in that text if some type
//! in the tree actually declares it (as `name: `) or actually holds it as a
//! string value (as `"value"`).

use gc_data::action_families::ActionFamilyId;
use gc_data::players::PlayerData;
use gc_sim::combat;
use gc_sim::combat_observation::{self, CombatObservation};
use gc_sim::combat_policy;
use gc_sim::combat_snapshot::{
    CombatContactResult, CombatEvent, CombatEventKind, CombatMatchState,
};
use gc_sim::fixed_clock;
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{self, MatchState, PitchSize};
use gc_sim::slot_input;
use gc_sim::tuning::Tuning;

/// This spec's own broader forbidden-key set, layered on top of
/// [`combat_observation::FORBIDDEN_FIELDS`] (taken from the module rather
/// than restated: two lists that can drift apart is the failure mode).
fn forbidden_keys() -> Vec<&'static str> {
    let mut keys = vec![
        "equipment_presentation_id",
        "species_id",
        "theme",
        "viewport",
        "pixels",
        "frame_time",
        "render_tick",
        "rng",
        "rng_state",
        "seed",
        "snapshot",
        "state",
        "combat_state",
        "events",
        "contacted",
        "projectile_spawned",
        "chain_ticks",
        "result",
        "outcome",
        "future_input",
        "intent",
        "outfield_decision",
        "outfield_press",
        "marks",
        "marking",
        "press",
        "windup_shot",
        "pass_target",
        "save_pending",
        "composure",
        "scan_rate",
    ];
    keys.extend(combat_observation::FORBIDDEN_FIELDS);
    keys
}

/// Every forbidden key that appears as a field name in a `Debug`
/// rendering. `Derive`d struct `Debug` always opens a field list with
/// `"TypeName { "` and separates subsequent fields with `", "`, so a field
/// name is only genuinely present when immediately preceded by one of
/// those two markers — a plain substring search would also match `state`
/// inside `forced_state`, which is a real, harmless field.
fn forbidden_key_in(text: &str) -> Option<&'static str> {
    forbidden_keys()
        .into_iter()
        .find(|key| text.contains(&format!("{{ {key}: ")) || text.contains(&format!(", {key}: ")))
}

fn new_match(overrides: &[(&str, &'static str)]) -> (MatchState, CombatMatchState) {
    let home = gc_data::teams::get("nebula").expect("nebula team is authored");
    let away = gc_data::teams::get("orion").expect("orion team is authored");
    let mut state = sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: None,
        max_goals: None,
        seed: Some(4242.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    });
    state.kickoff_hold = 0.0;
    let mut pool: Vec<PlayerData> = gc_data::players::ALL.to_vec();
    for player in &mut pool {
        if let Some((_, loadout)) = overrides.iter().find(|(id, _)| *id == player.id) {
            player.loadout_id = Some(loadout);
        }
    }
    let combat_state = combat::new_state(&mut state, Some(&pool));
    (state, combat_state)
}

fn observe(state: &MatchState, combat_state: &CombatMatchState) -> CombatObservation {
    combat_observation::build(state, Some(combat_state), 2, combat_policy::POLICY_ID, None)
}

#[test]
fn combat_observation_leakage_carries_no_presentation_hidden_or_authority_key() {
    let (mut state, mut combat_state) = new_match(&[]);
    state.rng = 999_983;
    combat_state.players[6].contacted = true; // canonical player index 7
    let observation = observe(&state, &combat_state);
    let text = format!("{observation:?}");
    let offender = forbidden_key_in(&text);
    assert!(offender.is_none(), "observation leaked {offender:?}");
}

#[test]
fn combat_observation_leakage_positive_control_the_same_scanner_rejects_the_authoritative_snapshot()
{
    let (state, combat_state) = new_match(&[]);
    let snapshot = match_snapshot::capture(&state, Some(&combat_state));
    let text = format!("{snapshot:?}");
    let offender = forbidden_key_in(&text);
    assert!(
        offender.is_some(),
        "the leakage scanner found nothing in a full snapshot, so it proves nothing"
    );
}

#[test]
fn combat_observation_leakage_carries_no_presentation_or_loadout_string_value() {
    let (state, combat_state) = new_match(&[]);
    let text = format!("{:?}", observe(&state, &combat_state));
    for player in gc_data::players::ALL {
        assert!(
            !text.contains(&format!("\"{}\"", player.presentation_id)),
            "leaked a presentation id"
        );
        if let Some(loadout_id) = player.loadout_id {
            assert!(
                !text.contains(&format!("\"{loadout_id}\"")),
                "leaked a loadout id"
            );
        }
        if let Some(cosmetic_variant_id) = player.cosmetic_variant_id {
            assert!(
                !text.contains(&format!("\"{cosmetic_variant_id}\"")),
                "leaked a cosmetic variant"
            );
        }
    }
    // Positive control: the strings are genuinely reachable from the
    // authored content, so the assertions above are testing something.
    let authored_text = format!("{:?}", gc_data::players::ALL);
    assert!(authored_text.contains(&format!("\"{}\"", gc_data::players::ALL[0].presentation_id)));
}

#[test]
fn combat_observation_leakage_carries_no_rng_state_and_no_resolver_verdict() {
    let (mut state, mut combat_state) = new_match(&[]);
    state.rng = 123_457;
    combat_state.events.push(CombatEvent {
        kind: CombatEventKind::Contact,
        tick: 0,
        family_id: Some(ActionFamilyId::Unarmed),
        source_index: Some(2),
        target_index: Some(7),
        source_sequence: Some(1),
        result: Some(CombatContactResult::Hit),
        outcome: None,
        reason: None,
        terminal: None,
        x: 1.0,
        y: 2.0,
        interruption_ticks: Some(10),
        displacement_px: Some(8.0),
    });
    let text = format!("{:?}", observe(&state, &combat_state));
    assert!(!text.contains("\"hit\""), "leaked a resolver verdict");
    assert!(!text.contains("\"superseded\""));
    assert!(forbidden_key_in(&text).is_none());
    assert!(!text.contains("result: "));
}

#[test]
fn combat_observation_leakage_produces_an_identical_digest_for_a_presentation_swap_inside_one_family()
 {
    // `loadout_tournament_sword` and `loadout_vector_blade` are both
    // `light_melee` and differ only in equipment presentation.
    let (baseline_state, baseline_combat) = new_match(&[]);
    let (swapped_state, swapped_combat) = new_match(&[("veil_nyx", "loadout_vector_blade")]);
    let baseline = observe(&baseline_state, &baseline_combat);
    let swapped = observe(&swapped_state, &swapped_combat);
    assert_eq!(
        combat_observation::encode(&swapped),
        combat_observation::encode(&baseline)
    );
    assert_eq!(swapped.digest, baseline.digest);

    // Positive control: a swap that changes the FAMILY is a real
    // mechanical change and must move the digest.
    let (family_swap_state, family_swap_combat) =
        new_match(&[("veil_nyx", "loadout_pulse_blaster")]);
    let family_swap = observe(&family_swap_state, &family_swap_combat);
    assert_ne!(family_swap.digest, baseline.digest);
}

/// Runs 120 fixed ticks of neutral input and concatenates every
/// `combat_*` match event plus every player's materialized intent tape
/// into one string, so two runs can be compared for byte-identical AI
/// behavior.
fn run(overrides: &[(&str, &'static str)]) -> String {
    let (mut state, mut combat_state) = new_match(overrides);
    let tune = Tuning::new();
    let mut parts: Vec<String> = Vec::new();
    for _ in 0..120 {
        sim_match::step(
            &mut state,
            fixed_clock::TICK_SECONDS,
            StepInput::Legacy(slot_input::neutral_match_input()),
            Some(&mut combat_state),
            &tune,
        );
        for event in &state.events {
            let kind = event.kind.wire_str();
            if let Some(stripped) = kind.strip_prefix("combat_") {
                let _ = stripped;
                parts.push(format!("{kind}/{:?}", event.player));
            }
        }
        for (index, runtime) in combat_state.players.iter().enumerate() {
            parts.push(format!(
                "{}{:?}{:?}{:?}",
                index + 1,
                runtime.intent.stage,
                runtime.intent.reason,
                runtime.intent.target_player
            ));
        }
    }
    parts.join(";")
}

#[test]
fn combat_observation_leakage_keeps_ai_decisions_and_materialized_tapes_identical_across_that_swap()
{
    let baseline = run(&[]);
    assert_eq!(run(&[("veil_nyx", "loadout_vector_blade")]), baseline);
    assert!(!baseline.is_empty());
}
