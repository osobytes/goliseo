//! Port of `sim/research_session.lua`.
//!
//! Versioned research session envelope and withdrawal tombstone.
//!
//! The envelope is the *only* place participant, protocol, and condition
//! metadata lives. It links to gameplay traces by content-addressed id, so a
//! session can reference many traces and a trace can be referenced by many
//! sessions without either side rewriting the other.
//!
//! Deliberate exclusions (see `docs/design/player_evidence_contracts.md`):
//!   * no direct identifiers and no agreement prose — the envelope records
//!     which agreement version the participant accepted and when, never its
//!     text; and
//!   * no hand-entered experience label. Experience is stored as continuous
//!     and ordinal source measures, and any "beginner/intermediate/experienced"
//!     label must name the versioned calibration that derived it.
//!
//! `validate_against_traces` (the orphan-join guard against
//! `ResearchTraceManifest`) is not ported: it needs `sim::research_trace`
//! (`sim/research_trace.lua`), which this port explicitly defers — see
//! `v2/README.md`'s porting notes for this crate. Every other public
//! function is here.

use crate::research_schema::{
    self, ResearchField, ResearchFieldKind, ResearchShape, Result, Value,
};

/// Reader version for the envelope/tombstone wire shapes.
pub const VERSION: i64 = 1;
/// Serialization versions this reader accepts.
pub const SUPPORTED_VERSIONS: &[i64] = &[1];
/// `manifest_kind` for a session envelope.
pub const KIND: &str = "research_session_envelope";
/// `manifest_kind` for a withdrawal tombstone.
pub const TOMBSTONE_KIND: &str = "research_withdrawal_tombstone";

/// What this validator actually enforces for participant and session ids:
/// the bounded `id` charset, this minimum length, uniqueness inside a
/// payload, and that a session id differs from its participant id. It does
/// **not** measure entropy or guessability — generating unguessable ids is
/// the recorder's job. These ids are join keys, not secrets.
pub const MIN_PSEUDONYM_LENGTH: usize = 16;

/// Declared session lifecycle statuses.
#[must_use]
pub fn statuses() -> Vec<String> {
    research_schema::enum_values(&[
        "in_progress",
        "completed",
        "interrupted",
        "abandoned",
        "withdrawn",
        "excluded",
    ])
}

/// Declared structured-missingness reasons.
#[must_use]
pub fn missingness_reasons() -> Vec<String> {
    research_schema::enum_values(&[
        "participant_skipped",
        "participant_withdrew",
        "instrument_not_administered",
        "operator_error",
        "technical_failure",
        "time_limit",
    ])
}

/// Declared exclusion reasons.
#[must_use]
pub fn exclusion_reasons() -> Vec<String> {
    research_schema::enum_values(&[
        "protocol_violation",
        "technical_failure",
        "practice_block",
        "operator_intervention",
        "participant_request",
        "identity_mismatch",
    ])
}

fn pseudonym_field(name: &str) -> ResearchField {
    ResearchField::new(ResearchFieldKind::Id)
        .named(name)
        .min_length(MIN_PSEUDONYM_LENGTH)
}

fn assignment_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("build_id"),
        ResearchField::new(ResearchFieldKind::Id).named("tuning_config_id"),
        ResearchField::new(ResearchFieldKind::Integer).named("seed"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("side")
            .values(research_schema::enum_values(&["home", "away"])),
        ResearchField::new(ResearchFieldKind::Id).named("role_id"),
        ResearchField::new(ResearchFieldKind::Id)
            .named("loadout_id")
            .optional(),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("opponent_population")
            .values(research_schema::enum_values(&[
                "bot",
                "human_proxy",
                "human",
                "adversarial_policy",
            ])),
        ResearchField::new(ResearchFieldKind::Id).named("opponent_policy_id"),
        ResearchField::new(ResearchFieldKind::Boolean).named("practice_block"),
    ]
}

fn environment_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Enum)
            .named("device_class")
            .values(research_schema::enum_values(&[
                "desktop", "laptop", "browser", "handheld",
            ])),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("control_device")
            .values(research_schema::enum_values(&[
                "keyboard", "gamepad", "touch", "mixed",
            ])),
        ResearchField::new(ResearchFieldKind::Id).named("control_mapping_id"),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("viewport_w")
            .min(1.0),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("viewport_h")
            .min(1.0),
        ResearchField::new(ResearchFieldKind::Number)
            .named("render_hz")
            .min(1.0)
            .max(1000.0),
        ResearchField::new(ResearchFieldKind::Number)
            .named("input_latency_ms")
            .min(0.0)
            .max(1000.0),
        ResearchField::new(ResearchFieldKind::Id).named("language_tag"),
        // Readability/accessibility settings are recorded because they
        // change what a participant could see, which is a validity concern
        // for cue readability.
        ResearchField::new(ResearchFieldKind::Map)
            .named("readability_settings")
            .optional()
            .element(ResearchField::new(ResearchFieldKind::Str).named("setting")),
    ]
}

fn interruption_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Enum)
            .named("kind")
            .values(research_schema::enum_values(&[
                "pause",
                "technical_failure",
                "participant_break",
                "operator_stop",
                "process_exit",
            ])),
        ResearchField::new(ResearchFieldKind::Number)
            .named("wall_clock_ms")
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Number)
            .named("duration_ms")
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("canonical_tick")
            .optional()
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("reason_code")
            .optional()
            .values(missingness_reasons()),
    ]
}

fn missingness_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("target_id"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("target_kind")
            .values(research_schema::enum_values(&[
                "instrument",
                "item",
                "block",
                "trace",
                "annotation",
            ])),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("reason_code")
            .values(missingness_reasons()),
    ]
}

fn exclusion_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("exclusion_id"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("scope")
            .values(research_schema::enum_values(&[
                "session",
                "block",
                "trial",
                "trace",
                "instrument",
            ])),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("reason_code")
            .values(exclusion_reasons()),
        ResearchField::new(ResearchFieldKind::Boolean).named("preregistered"),
    ]
}

fn lifecycle_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Enum)
            .named("status")
            .values(statuses()),
        ResearchField::new(ResearchFieldKind::Number)
            .named("started_wall_clock_ms")
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Number)
            .named("ended_wall_clock_ms")
            .optional()
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("observer_mode")
            .values(research_schema::enum_values(&[
                "none",
                "same_room",
                "remote",
                "recorded",
            ])),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("assistance")
            .values(research_schema::enum_values(&[
                "none",
                "verbal_prompt",
                "operator_input",
                "restart",
            ])),
        ResearchField::new(ResearchFieldKind::Array)
            .named("interruptions")
            .max_length(256)
            .element(
                ResearchField::new(ResearchFieldKind::Record)
                    .named("interruption")
                    .fields(interruption_fields()),
            ),
        ResearchField::new(ResearchFieldKind::Array)
            .named("exclusions")
            .max_length(256)
            .element(
                ResearchField::new(ResearchFieldKind::Record)
                    .named("exclusion")
                    .fields(exclusion_fields()),
            ),
        ResearchField::new(ResearchFieldKind::Array)
            .named("missingness")
            .max_length(1024)
            .element(
                ResearchField::new(ResearchFieldKind::Record)
                    .named("missing")
                    .fields(missingness_fields()),
            ),
    ]
}

fn experience_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Number)
            .named("play_hours_per_week")
            .min(0.0)
            .max(168.0),
        ResearchField::new(ResearchFieldKind::Number)
            .named("years_playing")
            .min(0.0)
            .max(100.0),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("football_games_ordinal")
            .min(1.0)
            .max(5.0),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("action_games_ordinal")
            .min(1.0)
            .max(5.0),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("self_reported_familiarity_ordinal")
            .min(1.0)
            .max(5.0),
        ResearchField::new(ResearchFieldKind::Record)
            .named("derived_label")
            .optional()
            .fields(vec![
                ResearchField::new(ResearchFieldKind::Enum)
                    .named("label")
                    .values(research_schema::enum_values(&[
                        "beginner",
                        "intermediate",
                        "experienced",
                    ])),
                ResearchField::new(ResearchFieldKind::Id).named("calibration_id"),
                ResearchField::new(ResearchFieldKind::Integer)
                    .named("calibration_version")
                    .min(1.0),
            ]),
    ]
}

fn agreement_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("agreement_version"),
        ResearchField::new(ResearchFieldKind::Boolean).named("accepted"),
        ResearchField::new(ResearchFieldKind::Number)
            .named("accepted_wall_clock_ms")
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Boolean).named("model_use_covered"),
    ]
}

fn trace_link_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Hash).named("trace_id"),
        ResearchField::new(ResearchFieldKind::Id).named("game_instance_id"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("role")
            .values(research_schema::enum_values(&[
                "condition_block",
                "practice",
                "replay_probe",
            ])),
    ]
}

/// The `research_session_envelope/v1` record shape.
#[must_use]
pub fn shape() -> ResearchShape {
    research_schema::record(
        "research_session_envelope/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Integer)
                .named("schema_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("manifest_kind")
                .values(research_schema::enum_values(&[KIND])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("digest")
                .values(research_schema::enum_values(&[research_schema::DIGEST])),
            pseudonym_field("session_id"),
            pseudonym_field("participant_id"),
            ResearchField::new(ResearchFieldKind::Id).named("study_id"),
            ResearchField::new(ResearchFieldKind::Id).named("protocol_version"),
            ResearchField::new(ResearchFieldKind::Id).named("cohort_id"),
            ResearchField::new(ResearchFieldKind::Id).named("recruitment_channel"),
            ResearchField::new(ResearchFieldKind::Id).named("block_id"),
            ResearchField::new(ResearchFieldKind::Id).named("trial_id"),
            ResearchField::new(ResearchFieldKind::Id).named("condition_id"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("condition_order")
                .min(1.0)
                .max(32.0),
            ResearchField::new(ResearchFieldKind::Id).named("sequence_label"),
            ResearchField::new(ResearchFieldKind::Id).named("counterbalancing_cell"),
            ResearchField::new(ResearchFieldKind::Record)
                .named("assignment")
                .fields(assignment_fields()),
            ResearchField::new(ResearchFieldKind::Record)
                .named("environment")
                .fields(environment_fields()),
            ResearchField::new(ResearchFieldKind::Record)
                .named("lifecycle")
                .fields(lifecycle_fields()),
            ResearchField::new(ResearchFieldKind::Record)
                .named("experience")
                .fields(experience_fields()),
            ResearchField::new(ResearchFieldKind::Record)
                .named("agreement")
                .fields(agreement_fields()),
            ResearchField::new(ResearchFieldKind::Array)
                .named("trace_links")
                .max_length(256)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("trace_link")
                        .fields(trace_link_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Id)
                .named("tombstone_id")
                .optional(),
        ],
    )
}

/// The `research_withdrawal_tombstone/v1` record shape.
#[must_use]
pub fn tombstone_shape() -> ResearchShape {
    research_schema::record(
        "research_withdrawal_tombstone/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Integer)
                .named("schema_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("manifest_kind")
                .values(research_schema::enum_values(&[TOMBSTONE_KIND])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("digest")
                .values(research_schema::enum_values(&[research_schema::DIGEST])),
            ResearchField::new(ResearchFieldKind::Id).named("tombstone_id"),
            pseudonym_field("session_id"),
            pseudonym_field("participant_id"),
            ResearchField::new(ResearchFieldKind::Number)
                .named("request_wall_clock_ms")
                .min(0.0),
            // There is exactly one withdrawal path: delete the session and
            // regenerate anything derived from it. Partial scopes were
            // removed because nothing enforced them, and an unenforced
            // promise is worse than no promise.
            ResearchField::new(ResearchFieldKind::Array)
                .named("revoked_payload_hashes")
                .max_length(4096)
                .element(ResearchField::new(ResearchFieldKind::Hash).named("payload_hash")),
            ResearchField::new(ResearchFieldKind::Boolean).named("rebuild_required"),
        ],
    )
}

fn text_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a str {
    Value::record_get(entries, name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("validated field {name} missing or not text"))
}

fn opt_text_field<'a>(entries: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    Value::record_get(entries, name).and_then(Value::as_str)
}

fn number_field(entries: &[(String, Value)], name: &str) -> f64 {
    Value::record_get(entries, name)
        .and_then(Value::as_number)
        .unwrap_or_else(|| panic!("validated field {name} missing or not a number"))
}

fn bool_field(entries: &[(String, Value)], name: &str) -> bool {
    Value::record_get(entries, name)
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("validated field {name} missing or not boolean"))
}

fn array_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [Value] {
    Value::record_get(entries, name)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("validated field {name} missing or not an array"))
}

fn record_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    Value::record_get(entries, name)
        .and_then(Value::as_record)
        .unwrap_or_else(|| panic!("validated field {name} missing or not a record"))
}

/// Validate a session envelope against [`shape`] plus every cross-field
/// invariant.
pub fn validate(envelope: &Value) -> Result<()> {
    let entries_for_version = envelope
        .as_record()
        .ok_or_else(|| "research session envelope must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&shape(), envelope)?;
    let entries = envelope.as_record().expect("validated record");
    let lifecycle = record_field(entries, "lifecycle");

    // Recording only happens after the participant accepts the agreement,
    // so an envelope that was not accepted, or was accepted after recording
    // started, is a contract violation rather than a preference to honour
    // later.
    let agreement = record_field(entries, "agreement");
    if !bool_field(agreement, "accepted") {
        return Err(
            "research_session_envelope.agreement must be accepted before recording".to_string(),
        );
    }
    let started = number_field(lifecycle, "started_wall_clock_ms");
    if number_field(agreement, "accepted_wall_clock_ms") > started {
        return Err(
            "research_session_envelope.agreement was accepted after the session started"
                .to_string(),
        );
    }

    let ended = Value::record_get(lifecycle, "ended_wall_clock_ms").and_then(Value::as_number);
    let status = text_field(lifecycle, "status");
    if let Some(ended) = ended {
        if ended < started {
            return Err("research_session_envelope.lifecycle ends before it starts".to_string());
        }
    } else if status != "in_progress" {
        return Err(format!(
            "research_session_envelope.lifecycle.{status} requires an end timestamp"
        ));
    }

    let interruptions = array_field(lifecycle, "interruptions");
    let exclusions = array_field(lifecycle, "exclusions");
    if status == "completed" {
        for (index, interruption) in interruptions.iter().enumerate() {
            let kind = text_field(interruption.as_record().unwrap(), "kind");
            if kind == "process_exit" || kind == "operator_stop" {
                return Err(format!(
                    "research_session_envelope.lifecycle.interruptions.{} contradicts a completed session",
                    index + 1
                ));
            }
        }
    } else if status == "interrupted" && interruptions.is_empty() {
        return Err(
            "research_session_envelope.lifecycle interrupted sessions must record why".to_string(),
        );
    } else if status == "excluded" && exclusions.is_empty() {
        return Err(
            "research_session_envelope.lifecycle excluded sessions must record why".to_string(),
        );
    }

    let tombstone_id = opt_text_field(entries, "tombstone_id");
    let missingness = array_field(lifecycle, "missingness");
    if status == "withdrawn" {
        if tombstone_id.is_none() {
            return Err(
                "research_session_envelope withdrawn sessions require a tombstone_id".to_string(),
            );
        }
        if !array_field(entries, "trace_links").is_empty() {
            return Err(
                "research_session_envelope withdrawn sessions must not retain payload links"
                    .to_string(),
            );
        }
        let withdrawal_recorded = missingness.iter().any(|missing| {
            text_field(missing.as_record().unwrap(), "reason_code") == "participant_withdrew"
        });
        if !withdrawal_recorded {
            return Err(
                "research_session_envelope withdrawn sessions must record participant_withdrew missingness"
                    .to_string(),
            );
        }
    } else if tombstone_id.is_some() {
        return Err(
            "research_session_envelope.tombstone_id requires a withdrawn session".to_string(),
        );
    }

    let mut seen_traces: Vec<String> = Vec::new();
    let assignment = record_field(entries, "assignment");
    for (index, link) in array_field(entries, "trace_links").iter().enumerate() {
        let link_entries = link.as_record().unwrap();
        let key = format!(
            "{}/{}",
            text_field(link_entries, "trace_id"),
            text_field(link_entries, "game_instance_id")
        );
        if seen_traces.contains(&key) {
            return Err(format!(
                "research_session_envelope.trace_links.{} is duplicated",
                index + 1
            ));
        }
        seen_traces.push(key);
        if text_field(link_entries, "role") == "practice"
            && !bool_field(assignment, "practice_block")
        {
            return Err(format!(
                "research_session_envelope.trace_links.{} is a practice trace outside a practice block",
                index + 1
            ));
        }
    }

    let mut seen_missing: Vec<String> = Vec::new();
    for (index, missing) in missingness.iter().enumerate() {
        let missing_entries = missing.as_record().unwrap();
        let key = format!(
            "{}/{}",
            text_field(missing_entries, "target_kind"),
            text_field(missing_entries, "target_id")
        );
        if seen_missing.contains(&key) {
            return Err(format!(
                "research_session_envelope.lifecycle.missingness.{} repeats {key}",
                index + 1
            ));
        }
        seen_missing.push(key);
    }

    let mut seen_exclusions: Vec<&str> = Vec::new();
    for (index, exclusion) in exclusions.iter().enumerate() {
        let exclusion_entries = exclusion.as_record().unwrap();
        let exclusion_id = text_field(exclusion_entries, "exclusion_id");
        if seen_exclusions.contains(&exclusion_id) {
            return Err(format!(
                "research_session_envelope.lifecycle.exclusions.{} is duplicated",
                index + 1
            ));
        }
        seen_exclusions.push(exclusion_id);
    }

    if text_field(entries, "session_id") == text_field(entries, "participant_id") {
        return Err(
            "research_session_envelope.session_id must differ from participant_id".to_string(),
        );
    }
    Ok(())
}

/// Validate a withdrawal tombstone against [`tombstone_shape`] plus its
/// cross-field invariants.
pub fn validate_tombstone(tombstone: &Value) -> Result<()> {
    let entries_for_version = tombstone
        .as_record()
        .ok_or_else(|| "research withdrawal tombstone must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(TOMBSTONE_KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&tombstone_shape(), tombstone)?;
    let entries = tombstone.as_record().expect("validated record");

    let mut seen: Vec<&str> = Vec::new();
    let hashes = array_field(entries, "revoked_payload_hashes");
    for (index, hash) in hashes.iter().enumerate() {
        let hash = hash.as_str().expect("validated hash");
        if seen.contains(&hash) {
            return Err(format!(
                "research_withdrawal_tombstone.revoked_payload_hashes.{} is duplicated",
                index + 1
            ));
        }
        seen.push(hash);
    }
    if hashes.is_empty() {
        return Err("research_withdrawal_tombstone must name the payloads it revokes".to_string());
    }
    if !bool_field(entries, "rebuild_required") {
        return Err(
            "research_withdrawal_tombstone must require derived rebuilds; derived tables are never hand-patched"
                .to_string(),
        );
    }
    Ok(())
}

/// A tombstoned session and its tombstone must agree, and the tombstone
/// must cover every payload the session still names.
pub fn validate_withdrawal(envelope: &Value, tombstone: &Value) -> Result<()> {
    validate(envelope)?;
    validate_tombstone(tombstone)?;
    let entries = envelope.as_record().expect("validated record");
    let lifecycle = record_field(entries, "lifecycle");
    if text_field(lifecycle, "status") != "withdrawn" {
        return Err("research withdrawal requires a withdrawn session envelope".to_string());
    }
    let tombstone_entries = tombstone.as_record().expect("validated record");
    if opt_text_field(entries, "tombstone_id")
        != Some(text_field(tombstone_entries, "tombstone_id"))
    {
        return Err(
            "research withdrawal tombstone id does not match the session envelope".to_string(),
        );
    }
    if text_field(entries, "session_id") != text_field(tombstone_entries, "session_id") {
        return Err(
            "research withdrawal tombstone session id does not match the envelope".to_string(),
        );
    }
    if text_field(entries, "participant_id") != text_field(tombstone_entries, "participant_id") {
        return Err(
            "research withdrawal tombstone participant id does not match the envelope".to_string(),
        );
    }
    Ok(())
}

/// Content hash of a valid session envelope.
pub fn content_hash(envelope: &Value) -> Result<String> {
    validate(envelope)?;
    research_schema::content_hash(&shape(), envelope)
}

/// Canonical bytes of a valid session envelope.
pub fn encode(envelope: &Value) -> Result<Vec<u8>> {
    validate(envelope)?;
    research_schema::encode(&shape(), envelope)
}

/// Decode and re-validate a session envelope.
pub fn decode(bytes: &[u8]) -> Result<Value> {
    let envelope = research_schema::decode(&shape(), bytes)?;
    validate(&envelope)?;
    Ok(envelope)
}

/// A model trained on this session may ship or be shared only if the
/// agreement version the participant accepted covered that use. Permission
/// is never granted retroactively, so this reads the recorded acceptance
/// and nothing else.
pub fn allows_model_use(envelope: &Value) -> Result<()> {
    validate(envelope)?;
    let entries = envelope.as_record().expect("validated record");
    let agreement = record_field(entries, "agreement");
    if bool_field(agreement, "model_use_covered") {
        Ok(())
    } else {
        Err(format!(
            "agreement version {} did not cover training or shipping a model",
            text_field(agreement, "agreement_version")
        ))
    }
}
