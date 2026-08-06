//! Port of `spec/sim/research_session_spec.lua`.
//!
//! `example_package.lua`'s `completed_session`/`withdrawn_session`/
//! `withdrawal_tombstone` fixtures are reproduced locally: this spec never
//! calls the trace/rollback-derived fixture functions, only the
//! envelope/tombstone builders (which take a bare `trace_id` string, not a
//! real trace manifest).

use gc_sim::research_schema::Value;
use gc_sim::research_session;

const TRACE_ID: &str = "0123456789abcdef";
const SESSION_ID: &str = "s-1c9d4f7a3b6e2058";
const PARTICIPANT_ID: &str = "p-7f3a9c21b8e45d06";
const WITHDRAWN_SESSION_ID: &str = "s-58f7a3b6e20591c9";

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn set_field(value: &Value, name: &str, new_value: Value) -> Value {
    let entries = value.as_record().expect("record value").to_vec();
    let mut updated: Vec<(String, Value)> =
        entries.into_iter().filter(|(key, _)| key != name).collect();
    updated.push((name.to_string(), new_value));
    Value::Record(updated)
}

fn remove_field(value: &Value, name: &str) -> Value {
    let entries = value.as_record().expect("record value").to_vec();
    Value::Record(entries.into_iter().filter(|(key, _)| key != name).collect())
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    Value::record_get(value.as_record().expect("record value"), name).expect("field present")
}

/// Mirrors `spec/fixtures/research/example_package.lua`'s `base_envelope`.
fn base_envelope(trace_id: &str) -> Value {
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_session_envelope")),
        ("digest", Value::str("fnv1a64/v1")),
        ("session_id", Value::str(SESSION_ID)),
        ("participant_id", Value::str(PARTICIPANT_ID)),
        ("study_id", Value::str("m11-combat-fun")),
        ("protocol_version", Value::str("m11-protocol-v3")),
        ("cohort_id", Value::str("confirmatory-wave-1")),
        ("recruitment_channel", Value::str("local-football-club")),
        ("block_id", Value::str("block-b")),
        ("trial_id", Value::str("trial-2")),
        ("condition_id", Value::str("condition-b-combat-on")),
        ("condition_order", Value::Number(2.0)),
        ("sequence_label", Value::str("ab")),
        ("counterbalancing_cell", Value::str("cell-ab-home-seed38")),
        (
            "assignment",
            record(vec![
                ("build_id", Value::str("spec-build")),
                ("tuning_config_id", Value::str("tuning-default-v1")),
                ("seed", Value::Number(38.0)),
                ("side", Value::str("home")),
                ("role_id", Value::str("outfield-slot-1")),
                ("loadout_id", Value::str("loadout-spring-gloves")),
                ("opponent_population", Value::str("bot")),
                ("opponent_policy_id", Value::str("bot-baseline-v1")),
                ("practice_block", Value::Bool(false)),
            ]),
        ),
        (
            "environment",
            record(vec![
                ("device_class", Value::str("desktop")),
                ("control_device", Value::str("keyboard")),
                ("control_mapping_id", Value::str("kb-default-v2")),
                ("viewport_w", Value::Number(1280.0)),
                ("viewport_h", Value::Number(720.0)),
                ("render_hz", Value::Number(60.0)),
                ("input_latency_ms", Value::Number(24.5)),
                ("language_tag", Value::str("en-gb")),
                (
                    "readability_settings",
                    Value::Map(vec![
                        ("high_contrast_cues".to_string(), Value::str("on")),
                        ("cue_scale".to_string(), Value::str("1.25")),
                    ]),
                ),
            ]),
        ),
        (
            "lifecycle",
            record(vec![
                ("status", Value::str("completed")),
                ("started_wall_clock_ms", Value::Number(1000.0)),
                ("ended_wall_clock_ms", Value::Number(900_000.0)),
                ("observer_mode", Value::str("same_room")),
                ("assistance", Value::str("none")),
                ("interruptions", Value::Array(vec![])),
                ("exclusions", Value::Array(vec![])),
                ("missingness", Value::Array(vec![])),
            ]),
        ),
        (
            "experience",
            record(vec![
                ("play_hours_per_week", Value::Number(6.5)),
                ("years_playing", Value::Number(12.0)),
                ("football_games_ordinal", Value::Number(4.0)),
                ("action_games_ordinal", Value::Number(2.0)),
                ("self_reported_familiarity_ordinal", Value::Number(3.0)),
                (
                    "derived_label",
                    record(vec![
                        ("label", Value::str("intermediate")),
                        ("calibration_id", Value::str("experience-calibration-v1")),
                        ("calibration_version", Value::Number(1.0)),
                    ]),
                ),
            ]),
        ),
        (
            "agreement",
            record(vec![
                ("agreement_version", Value::str("playtest-agreement-v3")),
                ("accepted", Value::Bool(true)),
                ("accepted_wall_clock_ms", Value::Number(500.0)),
                ("model_use_covered", Value::Bool(true)),
            ]),
        ),
        (
            "trace_links",
            Value::Array(vec![record(vec![
                ("trace_id", Value::str(trace_id)),
                ("game_instance_id", Value::str("gi-0001")),
                ("role", Value::str("condition_block")),
            ])]),
        ),
    ])
}

fn envelope() -> Value {
    base_envelope(TRACE_ID)
}

fn withdrawn_session() -> Value {
    let mut envelope = base_envelope("0000000000000000");
    envelope = set_field(&envelope, "session_id", Value::str(WITHDRAWN_SESSION_ID));
    envelope = set_field(
        &envelope,
        "lifecycle",
        record(vec![
            ("status", Value::str("withdrawn")),
            ("started_wall_clock_ms", Value::Number(1000.0)),
            ("ended_wall_clock_ms", Value::Number(500_000.0)),
            ("observer_mode", Value::str("same_room")),
            ("assistance", Value::str("none")),
            ("interruptions", Value::Array(vec![])),
            ("exclusions", Value::Array(vec![])),
            (
                "missingness",
                Value::Array(vec![record(vec![
                    ("target_id", Value::str("session-payload")),
                    ("target_kind", Value::str("trace")),
                    ("reason_code", Value::str("participant_withdrew")),
                ])]),
            ),
        ]),
    );
    envelope = set_field(&envelope, "trace_links", Value::Array(vec![]));
    envelope = set_field(&envelope, "tombstone_id", Value::str("tombstone-4d7c1a9e"));
    envelope
}

fn withdrawal_tombstone(revoked_payload_hashes: Option<Vec<&str>>) -> Value {
    let hashes = revoked_payload_hashes.unwrap_or_else(|| vec!["1111111111111111"]);
    record(vec![
        ("schema_version", Value::Number(1.0)),
        ("manifest_kind", Value::str("research_withdrawal_tombstone")),
        ("digest", Value::str("fnv1a64/v1")),
        ("tombstone_id", Value::str("tombstone-4d7c1a9e")),
        ("session_id", Value::str(WITHDRAWN_SESSION_ID)),
        ("participant_id", Value::str(PARTICIPANT_ID)),
        ("request_wall_clock_ms", Value::Number(1_200_000.0)),
        (
            "revoked_payload_hashes",
            Value::Array(hashes.into_iter().map(Value::str).collect()),
        ),
        ("rebuild_required", Value::Bool(true)),
    ])
}

#[test]
fn research_session_envelope_accepts_the_completed_example_and_round_trips_byte_for_byte() {
    let value = envelope();
    assert!(research_session::validate(&value).is_ok());
    let bytes = research_session::encode(&value).unwrap();
    let decoded = research_session::decode(&bytes).unwrap();
    assert_eq!(research_session::encode(&decoded).unwrap(), bytes);
    assert_eq!(
        field(&decoded, "participant_id").as_str(),
        Some(PARTICIPANT_ID)
    );
    let experience = field(&decoded, "experience");
    let derived_label = field(experience, "derived_label");
    assert_eq!(field(derived_label, "label").as_str(), Some("intermediate"));
    let environment = field(&decoded, "environment");
    let readability = field(environment, "readability_settings").as_map().unwrap();
    assert_eq!(
        Value::record_get(readability, "cue_scale").and_then(Value::as_str),
        Some("1.25")
    );
    let agreement = field(&decoded, "agreement");
    assert_eq!(
        field(agreement, "agreement_version").as_str(),
        Some("playtest-agreement-v3")
    );
}

#[test]
fn research_session_envelope_requires_bounded_distinct_pseudonymous_ids() {
    let short_participant = set_field(&envelope(), "participant_id", Value::str("p-1"));
    assert!(research_session::validate(&short_participant).is_err());

    let shared = set_field(&envelope(), "session_id", Value::str(PARTICIPANT_ID));
    let err = research_session::validate(&shared).unwrap_err();
    assert!(err.contains("must differ"), "{err}");
}

#[test]
fn research_session_envelope_rejects_direct_identifiers_anywhere_in_a_join_key() {
    let email = set_field(
        &envelope(),
        "recruitment_channel",
        Value::str("friend@example.com"),
    );
    assert!(research_session::validate(&email).is_err());

    let url = set_field(
        &envelope(),
        "cohort_id",
        Value::str("https://example.com/cohort"),
    );
    assert!(research_session::validate(&url).is_err());
}

#[test]
fn research_session_envelope_rejects_unknown_fields_and_unsupported_schema_versions() {
    let unknown = set_field(&envelope(), "participant_email", Value::str("nope"));
    let err = research_session::validate(&unknown).unwrap_err();
    assert!(err.contains("unknown field"), "{err}");

    let future = set_field(
        &envelope(),
        "schema_version",
        Value::Number((research_session::VERSION + 1) as f64),
    );
    let err = research_session::validate(&future).unwrap_err();
    assert!(err.contains("no migration"), "{err}");
}

#[test]
fn research_session_agreement_requires_an_accepted_agreement_recorded_before_the_session_started() {
    let base = envelope();
    let agreement = field(&base, "agreement").clone();

    let declined_agreement = set_field(&agreement, "accepted", Value::Bool(false));
    let declined = set_field(&base, "agreement", declined_agreement);
    let err = research_session::validate(&declined).unwrap_err();
    assert!(err.contains("must be accepted"), "{err}");

    let lifecycle = field(&base, "lifecycle");
    let started = Value::record_get(lifecycle.as_record().unwrap(), "started_wall_clock_ms")
        .and_then(Value::as_number)
        .unwrap();
    let late_agreement = set_field(
        &agreement,
        "accepted_wall_clock_ms",
        Value::Number(started + 1.0),
    );
    let late = set_field(&base, "agreement", late_agreement);
    let err = research_session::validate(&late).unwrap_err();
    assert!(err.contains("after the session started"), "{err}");

    let missing = remove_field(&base, "agreement");
    assert!(research_session::validate(&missing).is_err());
}

#[test]
fn research_session_agreement_answers_the_model_use_question_from_the_accepted_version_only() {
    let value = envelope();
    assert!(research_session::allows_model_use(&value).is_ok());

    let agreement = field(&value, "agreement").clone();
    let uncovered_agreement = set_field(&agreement, "model_use_covered", Value::Bool(false));
    let uncovered = set_field(&value, "agreement", uncovered_agreement);
    let err = research_session::allows_model_use(&uncovered).unwrap_err();
    assert!(err.contains("did not cover"), "{err}");
    assert!(err.contains("playtest-agreement-v3"), "{err}");
}

#[test]
fn research_session_agreement_keeps_readability_settings_optional_and_free_of_a_permission_matrix()
{
    let base = envelope();
    let environment = field(&base, "environment").clone();

    let without_env = remove_field(&environment, "readability_settings");
    let without = set_field(&base, "environment", without_env);
    assert!(research_session::validate(&without).is_ok());

    let unknown_key_env = set_field(
        &environment,
        "readability_settings",
        Value::Map(vec![("High Contrast".to_string(), Value::str("on"))]),
    );
    let unknown_key = set_field(&base, "environment", unknown_key_env);
    assert!(research_session::validate(&unknown_key).is_err());
}

#[test]
fn research_session_lifecycle_requires_an_end_timestamp_for_every_terminal_status() {
    let base = envelope();
    let lifecycle = field(&base, "lifecycle").clone();

    let unfinished_lifecycle = remove_field(&lifecycle, "ended_wall_clock_ms");
    let unfinished = set_field(&base, "lifecycle", unfinished_lifecycle);
    assert!(research_session::validate(&unfinished).is_err());

    let mut in_progress_lifecycle = remove_field(&lifecycle, "ended_wall_clock_ms");
    in_progress_lifecycle = set_field(&in_progress_lifecycle, "status", Value::str("in_progress"));
    let in_progress = set_field(&base, "lifecycle", in_progress_lifecycle);
    assert!(research_session::validate(&in_progress).is_ok());

    let reversed_lifecycle = set_field(&lifecycle, "ended_wall_clock_ms", Value::Number(999.0));
    let reversed = set_field(&base, "lifecycle", reversed_lifecycle);
    assert!(research_session::validate(&reversed).is_err());
}

#[test]
fn research_session_lifecycle_refuses_a_completed_session_that_also_reports_a_stop() {
    let base = envelope();
    let lifecycle = field(&base, "lifecycle").clone();

    let contradictory_lifecycle = set_field(
        &lifecycle,
        "interruptions",
        Value::Array(vec![record(vec![
            ("kind", Value::str("process_exit")),
            ("wall_clock_ms", Value::Number(5000.0)),
            ("duration_ms", Value::Number(0.0)),
        ])]),
    );
    let contradictory = set_field(&base, "lifecycle", contradictory_lifecycle);
    assert!(research_session::validate(&contradictory).is_err());

    let paused_lifecycle = set_field(
        &lifecycle,
        "interruptions",
        Value::Array(vec![record(vec![
            ("kind", Value::str("pause")),
            ("wall_clock_ms", Value::Number(5000.0)),
            ("duration_ms", Value::Number(12000.0)),
        ])]),
    );
    let paused = set_field(&base, "lifecycle", paused_lifecycle);
    assert!(research_session::validate(&paused).is_ok());
}

#[test]
fn research_session_lifecycle_requires_a_recorded_reason_for_interrupted_and_excluded_sessions() {
    let base = envelope();
    let lifecycle = field(&base, "lifecycle").clone();

    let interrupted_lifecycle = set_field(&lifecycle, "status", Value::str("interrupted"));
    let interrupted = set_field(&base, "lifecycle", interrupted_lifecycle);
    assert!(research_session::validate(&interrupted).is_err());

    let excluded_lifecycle = set_field(&lifecycle, "status", Value::str("excluded"));
    let excluded = set_field(&base, "lifecycle", excluded_lifecycle.clone());
    assert!(research_session::validate(&excluded).is_err());

    let excluded_with_reason_lifecycle = set_field(
        &excluded_lifecycle,
        "exclusions",
        Value::Array(vec![record(vec![
            ("exclusion_id", Value::str("ex-1")),
            ("scope", Value::str("session")),
            ("reason_code", Value::str("protocol_violation")),
            ("preregistered", Value::Bool(true)),
        ])]),
    );
    let excluded_with_reason = set_field(&base, "lifecycle", excluded_with_reason_lifecycle);
    assert!(research_session::validate(&excluded_with_reason).is_ok());
}

#[test]
fn research_session_lifecycle_rejects_duplicate_missingness_rows_and_duplicate_trace_links() {
    let base = envelope();
    let lifecycle = field(&base, "lifecycle").clone();

    let duplicated_missing_lifecycle = set_field(
        &lifecycle,
        "missingness",
        Value::Array(vec![
            record(vec![
                ("target_id", Value::str("pxi-enjoyment-addon")),
                ("target_kind", Value::str("instrument")),
                ("reason_code", Value::str("participant_skipped")),
            ]),
            record(vec![
                ("target_id", Value::str("pxi-enjoyment-addon")),
                ("target_kind", Value::str("instrument")),
                ("reason_code", Value::str("operator_error")),
            ]),
        ]),
    );
    let duplicated_missing = set_field(&base, "lifecycle", duplicated_missing_lifecycle);
    assert!(research_session::validate(&duplicated_missing).is_err());

    let trace_link = field(&base, "trace_links").as_array().unwrap()[0].clone();
    let duplicated_link = set_field(
        &base,
        "trace_links",
        Value::Array(vec![trace_link.clone(), trace_link]),
    );
    assert!(research_session::validate(&duplicated_link).is_err());
}

#[test]
fn research_session_lifecycle_keeps_practice_traces_inside_practice_blocks() {
    let base = envelope();
    let trace_link = field(&base, "trace_links").as_array().unwrap()[0].clone();
    let misdeclared_link = set_field(&trace_link, "role", Value::str("practice"));
    let misdeclared = set_field(
        &base,
        "trace_links",
        Value::Array(vec![misdeclared_link.clone()]),
    );
    assert!(research_session::validate(&misdeclared).is_err());

    let assignment = field(&base, "assignment").clone();
    let practice_assignment = set_field(&assignment, "practice_block", Value::Bool(true));
    let fixed = set_field(&misdeclared, "assignment", practice_assignment);
    assert!(research_session::validate(&fixed).is_ok());
}

#[test]
fn research_session_experience_measures_stores_continuous_and_ordinal_sources_never_a_bare_label() {
    let base = envelope();
    let experience = field(&base, "experience").clone();
    assert_eq!(
        Value::record_get(experience.as_record().unwrap(), "football_games_ordinal")
            .and_then(Value::as_number),
        Some(4.0)
    );

    let unbounded_experience = set_field(&experience, "football_games_ordinal", Value::Number(6.0));
    let unbounded = set_field(&base, "experience", unbounded_experience);
    assert!(research_session::validate(&unbounded).is_err());

    let uncalibrated_experience = set_field(
        &experience,
        "derived_label",
        record(vec![("label", Value::str("experienced"))]),
    );
    let uncalibrated = set_field(&base, "experience", uncalibrated_experience);
    let err = research_session::validate(&uncalibrated).unwrap_err();
    assert!(err.contains("calibration_id"), "{err}");

    let unlabelled_experience = remove_field(&experience, "derived_label");
    let unlabelled = set_field(&base, "experience", unlabelled_experience);
    assert!(research_session::validate(&unlabelled).is_ok());
}

#[test]
fn research_session_experience_measures_rejects_non_finite_numbers_in_this_family() {
    let base = envelope();
    let experience = field(&base, "experience").clone();
    let nan_experience = set_field(&experience, "play_hours_per_week", Value::Number(f64::NAN));
    let nan = set_field(&base, "experience", nan_experience);
    assert!(research_session::validate(&nan).is_err());

    let lifecycle = field(&base, "lifecycle").clone();
    let infinite_lifecycle = set_field(
        &lifecycle,
        "started_wall_clock_ms",
        Value::Number(f64::INFINITY),
    );
    let infinite = set_field(&base, "lifecycle", infinite_lifecycle);
    assert!(research_session::validate(&infinite).is_err());
}

#[test]
fn research_withdrawal_tombstone_requires_a_tombstone_and_an_emptied_envelope_for_a_withdrawal() {
    let withdrawn = withdrawn_session();
    assert!(research_session::validate(&withdrawn).is_ok());

    let without_tombstone = remove_field(&withdrawn, "tombstone_id");
    assert!(research_session::validate(&without_tombstone).is_err());

    let lifecycle = field(&withdrawn, "lifecycle").clone();
    let without_missingness_lifecycle = set_field(&lifecycle, "missingness", Value::Array(vec![]));
    let without_missingness = set_field(&withdrawn, "lifecycle", without_missingness_lifecycle);
    assert!(research_session::validate(&without_missingness).is_err());

    let stray_tombstone = set_field(
        &envelope(),
        "tombstone_id",
        Value::str("tombstone-4d7c1a9e"),
    );
    assert!(research_session::validate(&stray_tombstone).is_err());
}

#[test]
fn research_withdrawal_tombstone_requires_the_tombstone_to_name_payloads_and_force_a_rebuild() {
    let tombstone = withdrawal_tombstone(None);
    assert!(research_session::validate_tombstone(&tombstone).is_ok());

    let empty = withdrawal_tombstone(Some(vec![]));
    assert!(research_session::validate_tombstone(&empty).is_err());

    let hand_patched = set_field(&tombstone, "rebuild_required", Value::Bool(false));
    let err = research_session::validate_tombstone(&hand_patched).unwrap_err();
    assert!(err.contains("rebuild"), "{err}");

    let duplicated = withdrawal_tombstone(Some(vec!["1111111111111111", "1111111111111111"]));
    assert!(research_session::validate_tombstone(&duplicated).is_err());
}

#[test]
fn research_withdrawal_tombstone_only_pairs_a_tombstone_with_its_own_session_and_participant() {
    let withdrawn = withdrawn_session();
    let tombstone = withdrawal_tombstone(None);
    assert!(research_session::validate_withdrawal(&withdrawn, &tombstone).is_ok());

    let other_participant = set_field(
        &tombstone,
        "participant_id",
        Value::str("p-000000000000000a"),
    );
    assert!(research_session::validate_withdrawal(&withdrawn, &other_participant).is_err());

    let live_session = remove_field(&envelope(), "tombstone_id");
    assert!(research_session::validate_withdrawal(&live_session, &tombstone).is_err());
}
