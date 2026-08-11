//! The canonical fixture data `protocol_conformance.rs` pins golden wire
//! digests against, and that `tests/protocol.rs` differential-tests against
//! frozen cross-language reference values (see
//! `tools/lua_reference/README.md`). See `crate::protocol`'s module doc
//! comment for why manifests and message bodies here are built as
//! [`crate::protocol::Value`] rather than typed structs — this fixture
//! constructs the same dynamically-shaped values that codec expects, rather
//! than a fixed struct per message kind.

use crate::protocol::{self, MessageKind, Value};

/// `fixture.runtime`.
#[must_use]
pub fn runtime() -> Value {
    Value::record(vec![
        ("version", Value::int(protocol::RUNTIME_IDENTITY_VERSION)),
        ("runtime_id", Value::str("lovejs")),
        ("runtime_revision", Value::str("lovejs.11.5.omp0")),
        ("presentation_id", Value::str("presentation.2026-07-25")),
        (
            "capabilities",
            Value::array(vec![
                Value::str("combat_feedback.v1"),
                Value::str("control_channel.v1"),
                Value::str("input_channel.v1"),
            ]),
        ),
    ])
}

fn roster_player(
    player_id: &str,
    position: &str,
    loadout_id: Option<&str>,
    family_id: Option<&str>,
) -> Value {
    let mut fields = vec![
        ("player_id", Value::str(player_id)),
        ("position", Value::str(position)),
    ];
    if let Some(loadout_id) = loadout_id {
        fields.push(("loadout_id", Value::str(loadout_id)));
    }
    if let Some(family_id) = family_id {
        fields.push(("family_id", Value::str(family_id)));
    }
    Value::record(fields)
}

/// `fixture.manifest`: the canonical fixture manifest. `mode` defaults to
/// 4v4, so the pinned conformance vectors and every pre-existing caller
/// describe the same eight one-slot humans they always did.
#[must_use]
pub fn manifest(mode: Option<protocol::MatchMode>) -> Value {
    let home = vec![
        roster_player("ozzo", "keeper", None, None),
        roster_player(
            "zyro_vex",
            "forward",
            Some("loadout_spring_gloves"),
            Some("unarmed"),
        ),
        roster_player(
            "mika_olu",
            "defender",
            Some("loadout_emberguard_shield"),
            Some("guard"),
        ),
        roster_player(
            "rok_tann",
            "midfielder",
            Some("loadout_vector_blade"),
            Some("light_melee"),
        ),
        roster_player(
            "sela_dwin",
            "forward",
            Some("loadout_prism_launcher"),
            Some("ranged"),
        ),
    ];
    let away = vec![
        roster_player("gax_oru", "keeper", None, None),
        roster_player(
            "drell",
            "defender",
            Some("loadout_spring_gloves"),
            Some("unarmed"),
        ),
        roster_player(
            "morv",
            "defender",
            Some("loadout_emberguard_shield"),
            Some("guard"),
        ),
        roster_player(
            "krag",
            "midfielder",
            Some("loadout_vector_blade"),
            Some("light_melee"),
        ),
        roster_player(
            "tox_vren",
            "forward",
            Some("loadout_prism_launcher"),
            Some("ranged"),
        ),
    ];
    let outfield_ids = [
        "zyro_vex",
        "mika_olu",
        "rok_tann",
        "sela_dwin",
        "drell",
        "morv",
        "krag",
        "tox_vren",
    ];
    let mut slots = Vec::with_capacity(8);
    for (offset, player_id) in outfield_ids.iter().enumerate() {
        let index = offset as i64 + 1;
        let slot = gc_sim::input_frame::slot(index).unwrap();
        slots.push(Value::record(vec![
            ("slot", Value::str(protocol::slot_wire_id(slot.id))),
            ("team", Value::str(protocol::team_wire_str(slot.team))),
            ("player_id", Value::str(*player_id)),
        ]));
    }
    let mode = mode.unwrap_or(protocol::MatchMode::FourVFour);
    Value::record(vec![
        ("version", Value::int(protocol::MANIFEST_VERSION)),
        ("session_id", Value::str("session_alpha")),
        ("protocol_version", Value::int(protocol::VERSION)),
        (
            "input_version",
            Value::int(protocol::CURRENT_VERSIONS.input),
        ),
        (
            "snapshot_version",
            Value::int(protocol::CURRENT_VERSIONS.snapshot),
        ),
        ("tape_version", Value::int(protocol::CURRENT_VERSIONS.tape)),
        (
            "combat_schema_version",
            Value::int(protocol::CURRENT_VERSIONS.combat),
        ),
        ("build_id", Value::str("build.97b60ea")),
        ("source_id", Value::str("source.97b60ea")),
        ("content_id", Value::str("content.omp3.v1")),
        ("tuning_id", Value::str("tuning.omp3.v1")),
        ("match_config_id", Value::str("match_config.direct_host.v1")),
        ("fixture_id", Value::str("fixture.default_mixed.v1")),
        ("arena_id", Value::str("arena.goliseo")),
        (
            "combat_rules_id",
            Value::str("combat_interaction.accepted_2026_07_23"),
        ),
        ("gameplay_ai_policy_id", Value::str("gameplay_ai.combat.v1")),
        ("combat_status", Value::str("provisional_114")),
        ("seed", Value::int(20001)),
        ("tick_rate", Value::int(60)),
        ("duration_ticks", Value::int(7200)),
        // 99 is `protocol::MAX_GOALS`: no goal limit, the shipped rule since #268.
        // Spelled as a literal like every other field of this fixture.
        ("max_goals", Value::int(99)),
        ("match_mode", Value::str(mode.wire_str())),
        (
            "teams",
            Value::array(vec![
                Value::record(vec![
                    ("team", Value::str("home")),
                    ("team_id", Value::str("team_nova")),
                    ("roster", Value::array(home)),
                ]),
                Value::record(vec![
                    ("team", Value::str("away")),
                    ("team_id", Value::str("team_void")),
                    ("roster", Value::array(away)),
                ]),
            ]),
        ),
        ("slots", Value::array(slots)),
    ])
}

/// `fixture.assignments`.
#[must_use]
pub fn assignments() -> Value {
    let manifest = manifest(None);
    let slots = manifest.get("slots").unwrap();
    let mut result = Vec::with_capacity(8);
    for index in 1..=8i64 {
        let assignment = slots.get_index(index).unwrap();
        let slot = assignment.get("slot").unwrap().clone();
        let team = assignment.get("team").unwrap().clone();
        let player_id = assignment.get("player_id").unwrap().clone();
        let slot_str = slot.as_str().unwrap().to_string();
        let record = if index <= 6 {
            let producer_id = if index == 1 {
                "host".to_string()
            } else {
                format!("guest_{index}")
            };
            Value::record(vec![
                ("slot", slot),
                ("team", team),
                ("player_id", player_id),
                ("producer_kind", Value::str("peer")),
                ("producer_id", Value::str(producer_id)),
            ])
        } else {
            Value::record(vec![
                ("slot", slot),
                ("team", team),
                ("player_id", player_id),
                ("producer_kind", Value::str("bot")),
                ("producer_id", Value::str(format!("bot_{slot_str}"))),
                ("bot_seed", Value::int(21000 + index)),
            ])
        };
        result.push(record);
    }
    Value::array(result)
}

/// `fixture.pair_slots`: the canonical requested owned set — a 2v2-shaped
/// pair on the home line, in ascending canonical order.
#[must_use]
pub fn pair_slots() -> Value {
    Value::array(vec![Value::str("home_1"), Value::str("home_2")])
}

/// `fixture.messages`.
///
/// # Panics
///
/// Panics if the fixture data itself fails to validate — a bug in this
/// module, not in a caller.
#[must_use]
pub fn messages() -> Vec<protocol::ControlMessage> {
    let manifest_value = manifest(None);
    let manifest_id = protocol::manifest_id(&manifest_value);
    let assignments_value = assignments();
    let assignment_id = protocol::assignment_id(&assignments_value, 1);

    let bodies: Vec<(MessageKind, Value)> = vec![
        // The fixture peer declares exactly the build its fixture manifest
        // names, so the pinned handshake wire is the fullest legal one. A
        // golden that omitted the optional field would leave its encoding
        // unpinned, which is the one thing a wire change must not do.
        (
            MessageKind::Handshake,
            Value::record(vec![
                ("role", Value::str("host")),
                ("runtime", runtime()),
                ("build_id", manifest_value.get("build_id").unwrap().clone()),
            ]),
        ),
        (
            MessageKind::ManifestProposal,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("manifest", manifest_value.clone()),
            ]),
        ),
        (
            MessageKind::ManifestAccept,
            Value::record(vec![("manifest_id", Value::str(manifest_id.clone()))]),
        ),
        (
            MessageKind::PeerAssignment,
            Value::record(vec![
                ("assigned_peer_id", Value::str("host")),
                ("role", Value::str("host")),
            ]),
        ),
        (
            MessageKind::SlotAssignment,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("assignment_id", Value::str(assignment_id.clone())),
                ("assignments", assignments_value.clone()),
            ]),
        ),
        (
            MessageKind::Ready,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("assignment_id", Value::str(assignment_id.clone())),
                ("ready", Value::bool(true)),
            ]),
        ),
        (
            MessageKind::Countdown,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("countdown_id", Value::str("countdown_1")),
                ("remaining_ticks", Value::int(180)),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
        (
            MessageKind::Start,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("countdown_id", Value::str("countdown_1")),
                ("first_input_tick", Value::int(0)),
            ]),
        ),
        (
            MessageKind::MatchPhase,
            Value::record(vec![
                ("phase", Value::str("playing")),
                ("tick", Value::int(1)),
                ("home_score", Value::int(0)),
                ("away_score", Value::int(0)),
            ]),
        ),
        (
            MessageKind::HashReport,
            Value::record(vec![
                ("tick", Value::int(60)),
                ("boundary_hash", Value::str("0123456789abcdef")),
            ]),
        ),
        (
            MessageKind::ResultAck,
            Value::record(vec![
                ("final_tick", Value::int(7200)),
                ("home_score", Value::int(2)),
                ("away_score", Value::int(1)),
                ("final_hash", Value::str("fedcba9876543210")),
            ]),
        ),
        (
            MessageKind::Abort,
            Value::record(vec![("code", Value::str("host_abort"))]),
        ),
        (
            MessageKind::Disconnect,
            Value::record(vec![
                ("target_peer_id", Value::str("guest_2")),
                ("code", Value::str("peer_left")),
            ]),
        ),
        // Appended, never inserted. Every sequence number above is part of
        // its message id and therefore of its pinned wire digest, so a new
        // conformance message goes on the end and moves nothing before it.
        (
            MessageKind::PairPreference,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id.clone())),
                ("assignment_id", Value::str(assignment_id.clone())),
                ("slots", pair_slots()),
            ]),
        ),
        (
            MessageKind::PairPreferenceResult,
            Value::record(vec![
                ("manifest_id", Value::str(manifest_id)),
                ("assignment_id", Value::str(assignment_id)),
                ("slots", pair_slots()),
                ("status", Value::str("granted")),
            ]),
        ),
    ];

    let session_id = manifest_value
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    bodies
        .into_iter()
        .enumerate()
        .map(|(index, (kind, body))| {
            protocol::new(kind, &session_id, "host", index as i64, body)
                .expect("protocol fixture messages must be canonical")
        })
        .collect()
}
