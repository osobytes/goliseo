//! Port of `spec/game/online_match_session_spec.lua`.
//!
//! ## Content is fabricated, not real `data/`-authored content
//!
//! The Lua spec's fixture (`spec/fixtures/online_match_session.lua`) builds
//! everything from `game.online.match_manifest.template`, which reads real
//! `data.teams`/`data.players`/`data.loadouts`/`data.arenas`. Per
//! `gc_netcode::match_manifest`'s module doc comment, this crate has no
//! `gc-data` dependency, so this file cannot reproduce that fixture's real
//! content (`"nebula"`/`"orion"`) — it builds an equally-shaped, entirely
//! fabricated two-team fixture instead (`fixture_team`), and exercises
//! `match_manifest`'s and `match_session`'s actual *logic* against it. Every
//! assertion that only depends on structure (roster shape, digest
//! determinism, ownership/identity agreement, error branches) survives that
//! substitution unchanged; assertions that specifically need the string
//! `"nebula"` etc. are adapted to check the fabricated content's own id
//! instead (still asserting the real property: "resolve returns the content
//! the manifest names").
//!
//! ## What could not be ported at all
//!
//! `gc_sim::match_snapshot::MatchState`/`MatchPlayer` have 100+ fields, no
//! `Default` impl, and no lightweight constructor anywhere in this
//! workspace — every existing match-builder (including `gc-sim`'s own
//! tests, e.g. `crates/gc-sim/tests/match.rs`) goes through
//! `gc_data::teams::get("nebula")` and `gc_sim::r#match::new`. Without
//! `gc-data`, this file cannot build a `gc_sim::match_snapshot::MatchSnapshot`
//! at all, so nothing that needs one — `match_session::finish`,
//! `match_session::request`'s snapshot-touching half,
//! `match_session::player_index`/`carrying`, and every spec assertion that
//! reads `request.initial_snapshot` — is ported as `#[ignore]`, naming the
//! blocker. `gc_netcode::match_session::resolve_identity` exists specifically
//! so the *other* half of `request` (every fallible check) stays fully
//! tested despite this — see that function's doc comment.

use gc_netcode::coordinator::{self, Freeze, Role, SlotDriver};
use gc_netcode::match_manifest::{
    self, BuildIdentity, PlayerPosition, RosterPlayerContent, TeamContent,
};
use gc_netcode::match_session::{self, RequestOptions};
use gc_netcode::protocol::{self, MatchMode, Value};
use gc_sim::input_frame::SlotId;
use indexmap::IndexMap;

fn fixture_team(prefix: &str) -> TeamContent {
    let mut roster = vec![RosterPlayerContent {
        player_id: format!("{prefix}_keeper"),
        position: PlayerPosition::Keeper,
        number: 1,
        presentation_id: format!("presentation.{prefix}_keeper"),
        loadout_id: None,
        loadout_family_id: None,
    }];
    let outfield: &[(PlayerPosition, &str)] = &[
        (PlayerPosition::Defender, "defender"),
        (PlayerPosition::Defender, "defender"),
        (PlayerPosition::Midfielder, "midfielder"),
        (PlayerPosition::Forward, "forward"),
    ];
    for (offset, (position, label)) in outfield.iter().enumerate() {
        let number = offset as i64 + 2;
        roster.push(RosterPlayerContent {
            player_id: format!("{prefix}_{label}_{number}"),
            position: *position,
            number,
            presentation_id: format!("presentation.{prefix}_{label}_{number}"),
            loadout_id: Some(format!("loadout_{prefix}_{number}")),
            loadout_family_id: Some("unarmed".to_string()),
        });
    }
    TeamContent {
        id: format!("team_{prefix}"),
        name: format!("Team {prefix}"),
        formation: "flat_four".to_string(),
        roster,
    }
}

fn fixture_identity() -> BuildIdentity {
    BuildIdentity {
        name: "goliseo-test".to_string(),
        version: "0.0.0-test".to_string(),
        channel: "dev".to_string(),
    }
}

const ARENA_ID: &str = "arena.test";

fn fixture_manifest(mode: MatchMode) -> Value {
    let home = fixture_team("home");
    let away = fixture_team("away");
    match_manifest::template(&home, &away, ARENA_ID, Some(mode), &fixture_identity())
}

fn fixture_peer_ids(mode: MatchMode) -> Vec<String> {
    let shape = protocol::match_mode_shape(mode);
    let mut ids = vec!["host".to_string()];
    for index in 1..shape.humans {
        ids.push(format!("guest_{index}"));
    }
    ids
}

/// Mirrors `spec/fixtures/online_match_session.lua`'s `fixture.freeze`,
/// rebuilt from the same public pieces so this fixture cannot drift from
/// the real one.
fn fixture_freeze(manifest: &Value, peer_ids: &[String]) -> Freeze {
    let assignments = coordinator::plan_assignments(manifest, peer_ids).unwrap();
    coordinator::slot_sources(manifest, &assignments).unwrap();
    let mut owned: IndexMap<String, Vec<SlotId>> = IndexMap::new();
    let mut live: IndexMap<String, SlotId> = IndexMap::new();
    for index in 1..=8i64 {
        let producer = assignments.get_index(index).unwrap();
        if producer.get("producer_kind").and_then(Value::as_str) != Some("peer") {
            continue;
        }
        let producer_id = producer
            .get("producer_id")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        if owned.contains_key(&producer_id) {
            continue;
        }
        let slots: Vec<SlotId> = protocol::owned_slots(&assignments, &producer_id)
            .iter()
            .map(|wire| protocol::slot_id_from_wire(wire).unwrap())
            .collect();
        live.insert(producer_id.clone(), slots[0]);
        owned.insert(producer_id, slots);
    }
    let get_str = |field: &str| {
        manifest
            .get(field)
            .and_then(Value::as_str)
            .unwrap()
            .to_string()
    };
    let get_int = |field: &str| manifest.get(field).and_then(Value::as_int).unwrap();
    Freeze {
        manifest_id: protocol::manifest_id(manifest),
        assignment_id: protocol::assignment_id(&assignments, 1),
        countdown_id: "countdown.1".to_string(),
        first_input_tick: 0,
        seed: get_int("seed"),
        tick_rate: get_int("tick_rate"),
        duration_ticks: get_int("duration_ticks"),
        max_goals: get_int("max_goals"),
        content_id: get_str("content_id"),
        tuning_id: get_str("tuning_id"),
        combat_rules_id: get_str("combat_rules_id"),
        gameplay_ai_policy_id: get_str("gameplay_ai_policy_id"),
        combat_status: get_str("combat_status"),
        match_mode: MatchMode::from_wire_str(&get_str("match_mode")).unwrap(),
        assignments,
        owned,
        live,
    }
}

fn request_options_for(mode: MatchMode, peer_index: usize) -> RequestOptions {
    let manifest = fixture_manifest(mode);
    let peer_ids = fixture_peer_ids(mode);
    let freeze = fixture_freeze(&manifest, &peer_ids);
    RequestOptions {
        role: if peer_index == 0 {
            Role::Host
        } else {
            Role::Guest
        },
        peer_id: peer_ids[peer_index].clone(),
        manifest,
        freeze,
    }
}

// ---------------------------------------------------------------------------
// describe "content-derived online manifest"
// ---------------------------------------------------------------------------

#[test]
fn content_derived_online_manifest_resolves_back_to_the_content_the_simulation_needs() {
    let home = fixture_team("home");
    let away = fixture_team("away");
    let manifest = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &fixture_identity(),
    );
    protocol::validate_manifest(&manifest).unwrap();
    let content = match_manifest::resolve(&manifest, &home, &away, ARENA_ID).unwrap();
    assert_eq!(content.home.id, home.id);
    assert_eq!(content.away.id, away.id);
    assert_eq!(content.arena_id, ARENA_ID);
    assert_eq!(
        content.ownership.len(),
        gc_sim::input_frame::SLOT_COUNT as usize
    );
}

#[test]
fn content_derived_online_manifest_is_identical_across_modes_apart_from_the_mode_itself() {
    let one = fixture_manifest(MatchMode::OneVOne);
    let four = fixture_manifest(MatchMode::FourVFour);
    assert_eq!(one.get("content_id"), four.get("content_id"));
    assert_eq!(one.get("tuning_id"), four.get("tuning_id"));
    let mut one = one;
    one.set("match_mode", Value::str(MatchMode::FourVFour.wire_str()));
    assert_eq!(protocol::manifest_id(&one), protocol::manifest_id(&four));
}

#[test]
fn content_derived_online_manifest_mints_deterministic_protocol_shaped_identities() {
    let home = fixture_team("home");
    let away = fixture_team("away");
    let identity = fixture_identity();
    let ids = [
        match_manifest::content_id(&home, &away, ARENA_ID),
        match_manifest::tuning_id(),
        match_manifest::build_id(&identity),
    ];
    for id in &ids {
        assert!(
            id.bytes().enumerate().all(|(i, b)| {
                if i == 0 {
                    b.is_ascii_alphanumeric()
                } else {
                    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')
                }
            }),
            "identity is not a bounded opaque ASCII id: {id}"
        );
        assert!(id.len() <= 128);
    }
    assert_eq!(
        match_manifest::content_id(&home, &away, ARENA_ID),
        match_manifest::content_id(&home, &away, ARENA_ID)
    );
    assert_eq!(match_manifest::tuning_id(), match_manifest::tuning_id());
    assert_eq!(
        match_manifest::build_id(&identity),
        match_manifest::build_id(&identity)
    );
    let two_v_two_a =
        match_manifest::template(&home, &away, ARENA_ID, Some(MatchMode::TwoVTwo), &identity);
    let two_v_two_b =
        match_manifest::template(&home, &away, ARENA_ID, Some(MatchMode::TwoVTwo), &identity);
    assert_eq!(
        protocol::manifest_id(&two_v_two_a),
        protocol::manifest_id(&two_v_two_b)
    );
}

/// The Lua case monkey-patches `protocol.vocabulary_id` to simulate "the same
/// three `build_info` constants, but a different commit's control
/// vocabulary" — Lua can reassign a module function at runtime; Rust's
/// `protocol::vocabulary_id` is a plain `fn` with no such seam. What
/// *is* portable is the property the monkey-patch was probing for:
/// `build_id` is a pure function of its inputs (never a clock, a counter, or
/// table order), asserted directly below instead.
#[test]
fn content_derived_online_manifest_build_id_is_a_pure_function_of_its_inputs() {
    let a = BuildIdentity {
        name: "goliseo".to_string(),
        version: "1.2.3".to_string(),
        channel: "stable".to_string(),
    };
    let b = BuildIdentity {
        name: "goliseo".to_string(),
        version: "1.2.3".to_string(),
        channel: "stable".to_string(),
    };
    assert_eq!(match_manifest::build_id(&a), match_manifest::build_id(&b));
    let different = BuildIdentity {
        name: "goliseo".to_string(),
        version: "1.2.4".to_string(),
        channel: "stable".to_string(),
    };
    assert_ne!(
        match_manifest::build_id(&a),
        match_manifest::build_id(&different)
    );
}

#[test]
fn content_derived_online_manifest_describes_rosters_the_protocol_and_the_simulation_both_accept() {
    let manifest = fixture_manifest(MatchMode::FourVFour);
    let teams = manifest.get("teams").unwrap();
    for team_index in 1..=2i64 {
        let team = teams.get_index(team_index).unwrap();
        let roster = team.get("roster").unwrap();
        assert_eq!(roster.len(), 5);
        let keeper = roster.get_index(1).unwrap();
        assert_eq!(
            keeper.get("position").and_then(Value::as_str),
            Some("keeper"),
            "a manifest roster places its keeper first"
        );
        assert!(
            keeper.get("loadout_id").is_none(),
            "keepers carry no combat loadout"
        );
        assert!(keeper.get("family_id").is_none());
        for index in 2..=5i64 {
            let player = roster.get_index(index).unwrap();
            assert_ne!(
                player.get("position").and_then(Value::as_str),
                Some("keeper"),
                "only one keeper per roster"
            );
            assert!(
                player.get("loadout_id").is_some(),
                "an online outfielder needs a fixed loadout"
            );
            assert!(
                player.get("family_id").is_some(),
                "an online outfielder needs its family"
            );
        }
    }
}

#[test]
fn content_derived_online_manifest_refuses_a_manifest_naming_content_this_build_does_not_have() {
    let home = fixture_team("home");
    let away = fixture_team("away");
    let identity = fixture_identity();

    let mut wrong_team = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &identity,
    );
    let mut teams = wrong_team.get("teams").unwrap().clone();
    let mut t1 = teams.get_index(1).unwrap().clone();
    t1.set("team_id", Value::str("team_that_does_not_exist"));
    teams = Value::array(vec![t1, teams.get_index(2).unwrap().clone()]);
    wrong_team.set("teams", teams);
    assert!(match_manifest::resolve(&wrong_team, &home, &away, ARENA_ID).is_err());

    let mut wrong_arena = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &identity,
    );
    wrong_arena.set("arena_id", Value::str("arena.that.does.not.exist"));
    assert!(match_manifest::resolve(&wrong_arena, &home, &away, ARENA_ID).is_err());

    let mut wrong_player = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &identity,
    );
    let mut teams = wrong_player.get("teams").unwrap().clone();
    let mut t1 = teams.get_index(1).unwrap().clone();
    let mut roster = t1.get("roster").unwrap().clone();
    let mut p2 = roster.get_index(2).unwrap().clone();
    p2.set("player_id", Value::str("player_that_does_not_exist"));
    let mut items = vec![p2];
    for index in 3..=5i64 {
        items.push(roster.get_index(index).unwrap().clone());
    }
    roster = Value::array(
        std::iter::once(roster.get_index(1).unwrap().clone())
            .chain(items)
            .collect(),
    );
    t1.set("roster", roster);
    teams = Value::array(vec![t1, teams.get_index(2).unwrap().clone()]);
    wrong_player.set("teams", teams);
    assert!(match_manifest::resolve(&wrong_player, &home, &away, ARENA_ID).is_err());
}

#[test]
fn content_derived_online_manifest_refuses_a_manifest_that_disagrees_about_a_player_this_build_has()
{
    let home = fixture_team("home");
    let away = fixture_team("away");
    let identity = fixture_identity();

    let mut manifest = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &identity,
    );
    let mut teams = manifest.get("teams").unwrap().clone();
    let mut t1 = teams.get_index(1).unwrap().clone();
    let mut roster = t1.get("roster").unwrap().clone();
    let mut p2 = roster.get_index(2).unwrap().clone();
    let current = p2.get("position").and_then(Value::as_str).unwrap();
    p2.set(
        "position",
        Value::str(if current == "defender" {
            "midfielder"
        } else {
            "defender"
        }),
    );
    let mut items = vec![roster.get_index(1).unwrap().clone(), p2];
    for index in 3..=5i64 {
        items.push(roster.get_index(index).unwrap().clone());
    }
    roster = Value::array(items);
    t1.set("roster", roster);
    teams = Value::array(vec![t1, teams.get_index(2).unwrap().clone()]);
    manifest.set("teams", teams);
    assert!(match_manifest::resolve(&manifest, &home, &away, ARENA_ID).is_err());

    let mut manifest = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &identity,
    );
    let mut teams = manifest.get("teams").unwrap().clone();
    let mut t1 = teams.get_index(1).unwrap().clone();
    let mut roster = t1.get("roster").unwrap().clone();
    let mut p2 = roster.get_index(2).unwrap().clone();
    p2.set("loadout_id", Value::str("loadout_that_does_not_exist"));
    let mut items = vec![roster.get_index(1).unwrap().clone(), p2];
    for index in 3..=5i64 {
        items.push(roster.get_index(index).unwrap().clone());
    }
    roster = Value::array(items);
    t1.set("roster", roster);
    teams = Value::array(vec![t1, teams.get_index(2).unwrap().clone()]);
    manifest.set("teams", teams);
    assert!(match_manifest::resolve(&manifest, &home, &away, ARENA_ID).is_err());
}

#[test]
fn content_derived_online_manifest_refuses_a_slot_table_that_disagrees_with_locally_computed_ownership()
 {
    let home = fixture_team("home");
    let away = fixture_team("away");
    let identity = fixture_identity();
    let mut manifest = match_manifest::template(
        &home,
        &away,
        ARENA_ID,
        Some(MatchMode::FourVFour),
        &identity,
    );
    let slots = manifest.get("slots").unwrap().clone();
    let mut s1 = slots.get_index(1).unwrap().clone();
    let mut s2 = slots.get_index(2).unwrap().clone();
    let p1 = s1.get("player_id").unwrap().clone();
    let p2 = s2.get("player_id").unwrap().clone();
    s1.set("player_id", p2);
    s2.set("player_id", p1);
    let mut items = vec![s1, s2];
    for index in 3..=slots.len() as i64 {
        items.push(slots.get_index(index).unwrap().clone());
    }
    manifest.set("slots", Value::array(items));
    assert!(match_manifest::resolve(&manifest, &home, &away, ARENA_ID).is_err());
}

// ---------------------------------------------------------------------------
// describe "online match request"
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs a gc_sim::match_snapshot::MatchSnapshot, which needs gc-data (see module doc comment)"]
fn online_match_request_selects_the_combat_bearing_snapshot_and_tape_contracts_explicitly() {
    unimplemented!("needs match_session::finish with a real initial_snapshot");
}

#[test]
#[ignore = "needs a gc_sim::match_snapshot::MatchSnapshot, which needs gc-data (see module doc comment)"]
fn online_match_request_gives_every_peer_a_byte_identical_boundary_zero() {
    unimplemented!("needs match_session::finish with a real initial_snapshot");
}

#[test]
fn online_match_request_sizes_the_owned_set_from_the_frozen_mode_alone() {
    for (mode, slots) in [
        (MatchMode::OneVOne, 4),
        (MatchMode::TwoVTwo, 2),
        (MatchMode::FourVFour, 1),
    ] {
        let identity = match_session::resolve_identity(request_options_for(mode, 0)).unwrap();
        assert_eq!(
            identity.owned.len(),
            slots,
            "{} owned set size",
            mode.wire_str()
        );
        assert_eq!(
            identity.owned[0],
            identity.live,
            "{} opens live on its first slot",
            mode.wire_str()
        );
    }
}

#[test]
#[ignore = "needs a gc_sim::match_snapshot::MatchSnapshot to read state.players[..].is_keeper, which needs gc-data (see module doc comment); structurally guaranteed instead by protocol::owned_slots only ever naming outfield slots"]
fn online_match_request_never_seats_a_keeper_in_any_owned_set_in_any_mode() {
    unimplemented!("needs a real MatchState");
}

#[test]
fn online_match_request_marks_exactly_one_live_human_slot_and_drives_every_other_slot_with_ai() {
    let identity =
        match_session::resolve_identity(request_options_for(MatchMode::OneVOne, 0)).unwrap();
    let humans = identity
        .slot_drivers
        .iter()
        .filter(|driver| **driver == SlotDriver::Human)
        .count();
    assert_eq!(
        humans, 2,
        "a 1v1 has two humans, each live on exactly one slot"
    );
    let live_index = gc_netcode::live_slot::slot_index(identity.live) - 1;
    assert_eq!(
        identity.slot_drivers[live_index as usize],
        SlotDriver::Human
    );
    for slot in &identity.owned[1..] {
        let index = gc_netcode::live_slot::slot_index(*slot) - 1;
        assert_eq!(
            identity.slot_drivers[index as usize],
            SlotDriver::Ai,
            "a non-live owned slot is AI-driven, exactly as in solo play"
        );
    }
}

#[test]
fn online_match_request_refuses_a_freeze_whose_digest_does_not_match_the_manifest() {
    let mut options = request_options_for(MatchMode::FourVFour, 0);
    options.freeze.manifest_id = "manifest.something.else".to_string();
    assert!(match_session::resolve_identity(options).is_err());
}

#[test]
fn online_match_request_refuses_a_freeze_that_disagrees_with_the_manifest_about_the_match() {
    let mut options = request_options_for(MatchMode::FourVFour, 0);
    options.freeze.duration_ticks += 1;
    assert!(match_session::resolve_identity(options).is_err());
}

#[test]
fn online_match_request_refuses_a_peer_the_freeze_does_not_seat() {
    let mut options = request_options_for(MatchMode::FourVFour, 0);
    options.role = Role::Guest;
    options.peer_id = "guest_9".to_string();
    assert!(match_session::resolve_identity(options).is_err());
}

#[test]
fn online_match_request_refuses_a_non_combat_snapshot_contract() {
    let mut options = request_options_for(MatchMode::FourVFour, 0);
    let current = options
        .manifest
        .get("snapshot_version")
        .and_then(Value::as_int)
        .unwrap();
    options
        .manifest
        .set("snapshot_version", Value::int(current - 1));
    assert!(match_session::resolve_identity(options).is_err());
}

#[test]
fn keeper_control_is_off_in_every_online_mode() {
    // Pinned in one place so the deliberate divergence from solo play is not
    // "fixed" as a bug later — see the module doc comment.
    assert!(!match_session::KEEPER_CONTROL);
}
