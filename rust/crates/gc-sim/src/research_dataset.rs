//! Port of `sim/research_dataset.lua`.
//!
//! Dataset, split, and lineage manifests.
//!
//! A dataset manifest is the only artifact allowed to claim "these rows are
//! what we analysed". It therefore names its immutable sources by content
//! hash, the exact feature versions and register hash used, its
//! transformations, the agreement versions its sources were recorded under,
//! and its parent dataset.
//!
//! `dataset_hash` and the pinned `trace_manifest_hash` values are integrity
//! checks against accidental corruption and hand-edits. They are not a
//! tamper-proof audit trail: anyone who can rewrite a manifest can recompute
//! them.
//!
//! Splits fail closed on overlap. Held-out *participants* stop leakage
//! between people; held-out *builds and tuning configurations* stop leakage
//! between the very configurations a model would be asked to generalize
//! across. A split may require both, and coverage is checked in each
//! direction so a silently dropped participant is an error rather than an
//! unnoticed exclusion.
//!
//! Like [`crate::research_session`], every manifest here is a
//! [`research_schema::Value`] built and read through small private field
//! helpers, never a bespoke Rust struct — see that module's doc comment for
//! the rationale this file follows. README rule 4 (never `HashMap`/
//! `HashSet`) means every membership/coverage set below is a `Vec`, checked
//! with linear scans; these sets are small (participants and builds per
//! split, not per match tick) and never hashed or serialized themselves, so
//! this costs nothing on any hot path.

use crate::research_features;
use crate::research_schema::{
    self, ResearchField, ResearchFieldKind, ResearchShape, Result, TuplePart, Value,
};
use crate::research_session;

/// Reader version for the dataset manifest wire shape.
pub const VERSION: i64 = 1;
/// Serialization versions this reader accepts.
pub const SUPPORTED_VERSIONS: &[i64] = &[1];
/// `manifest_kind` this module writes and reads.
pub const KIND: &str = "research_dataset_manifest";
/// Tuple-hash label for [`derive_hash`].
pub const HASH_LABEL: &str = "research-dataset/v1";

/// Declared dataset-split fold roles.
#[must_use]
pub fn fold_roles() -> Vec<String> {
    research_schema::enum_values(&["train", "validation", "test", "holdout"])
}

/// Declared dataset-split groupings.
#[must_use]
pub fn groupings() -> Vec<String> {
    research_schema::enum_values(&["participant", "build", "participant_and_build"])
}

fn source_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Hash).named("trace_id"),
        ResearchField::new(ResearchFieldKind::Hash).named("trace_manifest_hash"),
        ResearchField::new(ResearchFieldKind::Id)
            .named("session_id")
            .min_length(16),
        ResearchField::new(ResearchFieldKind::Id)
            .named("participant_id")
            .min_length(16),
        ResearchField::new(ResearchFieldKind::Id).named("condition_id"),
        ResearchField::new(ResearchFieldKind::Id).named("build_id"),
        ResearchField::new(ResearchFieldKind::Id).named("tuning_config_id"),
        ResearchField::new(ResearchFieldKind::Boolean).named("excluded"),
        ResearchField::new(ResearchFieldKind::Id)
            .named("exclusion_reason")
            .optional(),
        ResearchField::new(ResearchFieldKind::Id).named("agreement_version"),
        ResearchField::new(ResearchFieldKind::Boolean).named("model_use_covered"),
    ]
}

fn fold_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("fold_id"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("role")
            .values(fold_roles()),
        ResearchField::new(ResearchFieldKind::Array)
            .named("participant_ids")
            .element(
                ResearchField::new(ResearchFieldKind::Id)
                    .named("participant_id")
                    .min_length(16),
            ),
        ResearchField::new(ResearchFieldKind::Array)
            .named("build_ids")
            .element(ResearchField::new(ResearchFieldKind::Id).named("build_id")),
    ]
}

fn split_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("split_id"),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("grouping")
            .values(groupings()),
        ResearchField::new(ResearchFieldKind::Array)
            .named("folds")
            .min_length(2)
            .max_length(64)
            .element(
                ResearchField::new(ResearchFieldKind::Record)
                    .named("fold")
                    .fields(fold_fields()),
            ),
    ]
}

// Internal-analysis playtest data. The only permission question that
// survives is whether the accepted agreement versions covered training and
// shipping a model, and that cannot be granted after the fact.
fn usage_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Array)
            .named("agreement_versions")
            .min_length(1)
            .element(ResearchField::new(ResearchFieldKind::Id).named("agreement_version")),
        ResearchField::new(ResearchFieldKind::Boolean).named("model_use_covered"),
    ]
}

fn extraction_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Str).named("extraction_commit"),
        ResearchField::new(ResearchFieldKind::Id).named("extraction_config_id"),
        ResearchField::new(ResearchFieldKind::Hash).named("feature_registry_hash"),
    ]
}

fn feature_version_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("feature_id"),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("version")
            .min(1.0),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("aggregation_level")
            .values(research_features::grains()),
    ]
}

fn transformation_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("transformation_id"),
        ResearchField::new(ResearchFieldKind::Str)
            .named("description")
            .max_length(512),
        ResearchField::new(ResearchFieldKind::Hash).named("input_hash"),
        ResearchField::new(ResearchFieldKind::Hash).named("output_hash"),
    ]
}

/// The `research_dataset_manifest/v1` record shape.
#[must_use]
pub fn shape() -> ResearchShape {
    research_schema::record(
        "research_dataset_manifest/v1",
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
            ResearchField::new(ResearchFieldKind::Id).named("dataset_id"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("dataset_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Hash)
                .named("parent_dataset_hash")
                .optional(),
            ResearchField::new(ResearchFieldKind::Number)
                .named("created_wall_clock_ms")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("purpose")
                .values(research_schema::enum_values(&[
                    "analysis",
                    "model_training",
                    "qa",
                ])),
            ResearchField::new(ResearchFieldKind::Record)
                .named("usage")
                .fields(usage_fields()),
            ResearchField::new(ResearchFieldKind::Record)
                .named("extraction")
                .fields(extraction_fields()),
            ResearchField::new(ResearchFieldKind::Array)
                .named("feature_versions")
                .min_length(1)
                .max_length(512)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("feature_version")
                        .fields(feature_version_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Array)
                .named("sources")
                .min_length(1)
                .max_length(8192)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("source")
                        .fields(source_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Array)
                .named("transformations")
                .max_length(256)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("transformation")
                        .fields(transformation_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Array)
                .named("splits")
                .max_length(32)
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("split")
                        .fields(split_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Hash).named("dataset_hash"),
        ],
    )
}

fn text_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a str {
    Value::record_get(entries, name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("validated field {name} missing or not text"))
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

fn id_array(entries: &[(String, Value)], name: &str) -> Vec<String> {
    array_field(entries, name)
        .iter()
        .map(|v| v.as_str().expect("validated id").to_string())
        .collect()
}

/// Derive a dataset manifest's content-addressed `dataset_hash` from the rest
/// of its fields (with `dataset_hash` itself zeroed).
pub fn derive_hash(manifest: &Value) -> Result<String> {
    let entries = manifest
        .as_record()
        .ok_or_else(|| "research dataset manifest must be a table".to_string())?;
    let mut body_entries: Vec<(String, Value)> = entries.to_vec();
    zero_dataset_hash(&mut body_entries);
    let body = Value::Record(body_entries);
    let content = research_schema::content_hash(&shape(), &body)?;
    let dataset_id = text_field(entries, "dataset_id").to_string();
    let dataset_version = number_field(entries, "dataset_version");
    Ok(research_schema::tuple_hash(
        HASH_LABEL,
        &[
            TuplePart::Text(content),
            TuplePart::Text(dataset_id),
            TuplePart::Number(dataset_version),
        ],
    ))
}

fn zero_dataset_hash(entries: &mut Vec<(String, Value)>) {
    if let Some(entry) = entries.iter_mut().find(|(k, _)| k == "dataset_hash") {
        entry.1 = Value::str("0000000000000000");
    } else {
        entries.push(("dataset_hash".to_string(), Value::str("0000000000000000")));
    }
}

fn assert_group_disjoint(label: &str, groups: &[(String, Vec<String>)]) -> Result<()> {
    let refs: Vec<(&str, Vec<&str>)> = groups
        .iter()
        .map(|(id, members)| (id.as_str(), members.iter().map(String::as_str).collect()))
        .collect();
    let slices: Vec<(&str, &[&str])> = refs
        .iter()
        .map(|(id, members)| (*id, members.as_slice()))
        .collect();
    research_schema::assert_disjoint(label, &slices)
}

fn validate_split(
    split_entries: &[(String, Value)],
    path: &str,
    known_participants: &[String],
    known_builds: &[String],
) -> Result<()> {
    let grouping = text_field(split_entries, "grouping");
    let checks_participants = grouping != "build";
    let checks_builds = grouping != "participant";
    let mut participant_groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut build_groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut fold_ids: Vec<String> = Vec::new();
    let mut has_train = false;
    let mut has_test_or_holdout = false;

    let folds = array_field(split_entries, "folds");
    for (index, fold) in folds.iter().enumerate() {
        let fold_path = format!("{path}.folds.{}", index + 1);
        let fold_entries = fold.as_record().expect("validated fold record");
        let fold_id = text_field(fold_entries, "fold_id").to_string();
        if fold_ids.contains(&fold_id) {
            return Err(format!("{fold_path} duplicates fold id {fold_id}"));
        }
        fold_ids.push(fold_id.clone());
        let role = text_field(fold_entries, "role");
        if role == "train" {
            has_train = true;
        }
        if role == "test" || role == "holdout" {
            has_test_or_holdout = true;
        }
        let participant_ids = id_array(fold_entries, "participant_ids");
        let build_ids = id_array(fold_entries, "build_ids");

        if checks_participants {
            if participant_ids.is_empty() {
                return Err(format!("{fold_path} must hold at least one participant"));
            }
            let mut seen: Vec<&str> = Vec::new();
            for participant_id in &participant_ids {
                if seen.contains(&participant_id.as_str()) {
                    return Err(format!("{fold_path} repeats participant {participant_id}"));
                }
                seen.push(participant_id);
                if !known_participants.iter().any(|p| p == participant_id) {
                    return Err(format!(
                        "{fold_path} references participant {participant_id} with no source row"
                    ));
                }
            }
            participant_groups.push((fold_id.clone(), participant_ids));
        } else if !participant_ids.is_empty() {
            return Err(format!(
                "{fold_path} declares participants in a build-only split"
            ));
        }

        if checks_builds {
            if build_ids.is_empty() {
                return Err(format!("{fold_path} must hold at least one build"));
            }
            let mut seen: Vec<&str> = Vec::new();
            for build_id in &build_ids {
                if seen.contains(&build_id.as_str()) {
                    return Err(format!("{fold_path} repeats build {build_id}"));
                }
                seen.push(build_id);
                if !known_builds.iter().any(|b| b == build_id) {
                    return Err(format!(
                        "{fold_path} references build {build_id} with no source row"
                    ));
                }
            }
            build_groups.push((fold_id.clone(), build_ids));
        } else if !build_ids.is_empty() {
            return Err(format!(
                "{fold_path} declares builds in a participant-only split"
            ));
        }
    }

    if !has_train {
        return Err(format!("{path} needs a train fold"));
    }
    if !has_test_or_holdout {
        return Err(format!("{path} needs a test or holdout fold"));
    }

    if checks_participants {
        assert_group_disjoint(&format!("{path} participants"), &participant_groups)?;
        for participant_id in known_participants {
            let found = participant_groups
                .iter()
                .any(|(_, members)| members.iter().any(|m| m == participant_id));
            if !found {
                return Err(format!(
                    "{path} does not cover participant {participant_id}"
                ));
            }
        }
    }
    if checks_builds {
        assert_group_disjoint(&format!("{path} builds"), &build_groups)?;
        for build_id in known_builds {
            let found = build_groups
                .iter()
                .any(|(_, members)| members.iter().any(|m| m == build_id));
            if !found {
                return Err(format!("{path} does not cover build {build_id}"));
            }
        }
    }
    Ok(())
}

/// Validate a dataset manifest against [`shape`] plus every cross-field
/// invariant: feature register agreement, source-row exclusion bookkeeping,
/// split coverage/disjointness, model-use permission, transformation
/// lineage, and its own content hash.
pub fn validate(manifest: &Value) -> Result<()> {
    let entries_for_version = manifest
        .as_record()
        .ok_or_else(|| "research dataset manifest must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&shape(), manifest)?;
    let entries = manifest.as_record().expect("validated record");

    let extraction = record_field(entries, "extraction");
    if text_field(extraction, "feature_registry_hash") != research_features::registry_hash() {
        return Err(
            "research_dataset_manifest.extraction.feature_registry_hash does not match the active feature register"
                .to_string(),
        );
    }

    let purpose = text_field(entries, "purpose");
    let mut seen_features: Vec<&str> = Vec::new();
    let feature_versions = array_field(entries, "feature_versions");
    for (index, entry) in feature_versions.iter().enumerate() {
        let path = format!("research_dataset_manifest.feature_versions.{}", index + 1);
        let entry_fields = entry.as_record().expect("validated feature version record");
        let feature_id = text_field(entry_fields, "feature_id");
        if seen_features.contains(&feature_id) {
            return Err(format!("{path} duplicates feature {feature_id}"));
        }
        seen_features.push(feature_id);
        let feature = research_features::feature(feature_id).map_err(|e| format!("{path}: {e}"))?;
        let feature_fields = feature.as_record().expect("registered feature is a record");
        let declared_version = number_field(entry_fields, "version");
        let registered_version = number_field(feature_fields, "version");
        if declared_version != registered_version {
            return Err(format!(
                "{path} pins version {declared_version} but the register defines version {registered_version}"
            ));
        }
        let aggregation_level = text_field(entry_fields, "aggregation_level");
        research_features::aggregation_allowed(feature_id, aggregation_level)
            .map_err(|e| format!("{path}: {e}"))?;
        if purpose == "model_training" {
            research_features::use_allowed(feature_id, "model_input")
                .map_err(|e| format!("{path}: {e}"))?;
        }
    }

    let mut known_participants: Vec<String> = Vec::new();
    let mut known_builds: Vec<String> = Vec::new();
    let mut seen_traces: Vec<&str> = Vec::new();
    let sources = array_field(entries, "sources");
    for (index, row) in sources.iter().enumerate() {
        let path = format!("research_dataset_manifest.sources.{}", index + 1);
        let row_entries = row.as_record().expect("validated source row");
        let trace_id = text_field(row_entries, "trace_id");
        if seen_traces.contains(&trace_id) {
            return Err(format!("{path} duplicates trace {trace_id}"));
        }
        seen_traces.push(trace_id);
        let excluded = bool_field(row_entries, "excluded");
        let exclusion_reason = Value::record_get(row_entries, "exclusion_reason");
        if excluded {
            if exclusion_reason.is_none() {
                return Err(format!("{path} must record why it is excluded"));
            }
        } else if exclusion_reason.is_some() {
            return Err(format!(
                "{path} records an exclusion reason without being excluded"
            ));
        } else {
            let participant_id = text_field(row_entries, "participant_id").to_string();
            if !known_participants.contains(&participant_id) {
                known_participants.push(participant_id);
            }
            let build_id = text_field(row_entries, "build_id").to_string();
            if !known_builds.contains(&build_id) {
                known_builds.push(build_id);
            }
        }
    }
    if known_participants.is_empty() {
        return Err("research_dataset_manifest.sources has no included rows".to_string());
    }

    let mut excluded_participants: Vec<String> = Vec::new();
    for row in sources {
        let row_entries = row.as_record().expect("validated source row");
        if bool_field(row_entries, "excluded") {
            let participant_id = text_field(row_entries, "participant_id").to_string();
            if !known_participants.contains(&participant_id)
                && !excluded_participants.contains(&participant_id)
            {
                excluded_participants.push(participant_id);
            }
        }
    }

    let splits = array_field(entries, "splits");
    for (index, split) in splits.iter().enumerate() {
        let path = format!("research_dataset_manifest.splits.{}", index + 1);
        let split_entries = split.as_record().expect("validated split record");
        for fold in array_field(split_entries, "folds") {
            let fold_entries = fold.as_record().expect("validated fold record");
            for participant_id in array_field(fold_entries, "participant_ids") {
                let participant_id = participant_id.as_str().expect("validated id");
                if excluded_participants.iter().any(|p| p == participant_id) {
                    return Err(format!(
                        "{path} includes fully excluded participant {participant_id}"
                    ));
                }
            }
        }
        validate_split(split_entries, &path, &known_participants, &known_builds)?;
    }
    if purpose == "model_training" && splits.is_empty() {
        return Err(
            "research_dataset_manifest for model training requires a split manifest".to_string(),
        );
    }

    // Model-use permission is a property of the agreement each participant
    // accepted, so a dataset may only claim it when every included source row
    // carries it. It can never be granted retroactively at the dataset level.
    let usage = record_field(entries, "usage");
    let declared_versions = id_array(usage, "agreement_versions");
    let mut every_source_covers = true;
    for (index, row) in sources.iter().enumerate() {
        let path = format!("research_dataset_manifest.sources.{}", index + 1);
        let row_entries = row.as_record().expect("validated source row");
        let agreement_version = text_field(row_entries, "agreement_version");
        if !declared_versions.iter().any(|v| v == agreement_version) {
            return Err(format!(
                "{path} was recorded under agreement version {agreement_version} which the dataset does not declare"
            ));
        }
        if !bool_field(row_entries, "excluded") && !bool_field(row_entries, "model_use_covered") {
            every_source_covers = false;
        }
    }
    if bool_field(usage, "model_use_covered") && !every_source_covers {
        return Err(
            "research_dataset_manifest.usage.model_use_covered requires every included source to be covered by its accepted agreement"
                .to_string(),
        );
    }
    if purpose == "model_training" && !bool_field(usage, "model_use_covered") {
        return Err(
            "research_dataset_manifest for model training requires agreement-covered model use"
                .to_string(),
        );
    }

    let transformations = array_field(entries, "transformations");
    if !transformations.is_empty() && Value::record_get(entries, "parent_dataset_hash").is_none() {
        return Err(
            "research_dataset_manifest with transformations must name its parent dataset hash"
                .to_string(),
        );
    }
    let mut seen_transformations: Vec<&str> = Vec::new();
    for (index, transformation) in transformations.iter().enumerate() {
        let path = format!("research_dataset_manifest.transformations.{}", index + 1);
        let transformation_entries = transformation
            .as_record()
            .expect("validated transformation");
        let transformation_id = text_field(transformation_entries, "transformation_id");
        if seen_transformations.contains(&transformation_id) {
            return Err(format!("{path} is duplicated"));
        }
        seen_transformations.push(transformation_id);
        if text_field(transformation_entries, "input_hash")
            == text_field(transformation_entries, "output_hash")
        {
            return Err(format!("{path} must change its payload or not be recorded"));
        }
    }

    let expected = derive_hash(manifest)?;
    if text_field(entries, "dataset_hash") != expected {
        return Err(
            "research_dataset_manifest.dataset_hash does not cover its contents".to_string(),
        );
    }
    Ok(())
}

/// Seal a manifest by deriving its dataset hash, then validate it.
pub fn seal(manifest: &Value) -> Result<Value> {
    let entries = manifest
        .as_record()
        .ok_or_else(|| "research dataset manifest must be a table".to_string())?;
    let mut sealed_entries: Vec<(String, Value)> = entries.to_vec();
    zero_dataset_hash(&mut sealed_entries);
    let sealed = Value::Record(sealed_entries);
    research_schema::validate(&shape(), &sealed)?;
    let hash = derive_hash(&sealed)?;
    let mut final_entries = sealed.as_record().expect("just built").to_vec();
    if let Some(entry) = final_entries.iter_mut().find(|(k, _)| k == "dataset_hash") {
        entry.1 = Value::str(hash);
    }
    let sealed = Value::Record(final_entries);
    validate(&sealed)?;
    Ok(sealed)
}

/// Join every source row back to the session envelope it came from.
///
/// `agreement_version` and `model_use_covered` are *self-declared* on a
/// source row, which is only useful if it can be checked against the
/// accepted agreement it claims to reflect. This is that check: without it,
/// a dataset builder could assert model-use permission for a participant who
/// never granted it, and the non-retroactivity guarantee would be
/// unverifiable rather than merely unenforced.
pub fn validate_against_sessions(manifest: &Value, envelopes: &[Value]) -> Result<()> {
    validate(manifest)?;
    let entries = manifest.as_record().expect("validated record");

    let mut by_session: Vec<(String, &Value)> = Vec::new();
    for (index, envelope) in envelopes.iter().enumerate() {
        research_session::validate(envelope)
            .map_err(|e| format!("research session envelope {}: {e}", index + 1))?;
        let envelope_entries = envelope.as_record().expect("validated record");
        let session_id = text_field(envelope_entries, "session_id").to_string();
        if by_session.iter().any(|(id, _)| id == &session_id) {
            return Err(format!(
                "research session envelope {session_id} is supplied twice"
            ));
        }
        by_session.push((session_id, envelope));
    }

    let sources = array_field(entries, "sources");
    for (index, row) in sources.iter().enumerate() {
        let path = format!("research_dataset_manifest.sources.{}", index + 1);
        let row_entries = row.as_record().expect("validated source row");
        let session_id = text_field(row_entries, "session_id");
        let Some((_, envelope)) = by_session.iter().find(|(id, _)| id == session_id) else {
            return Err(format!("{path} is an orphan join"));
        };
        let envelope_entries = envelope.as_record().expect("validated record");
        let lifecycle = record_field(envelope_entries, "lifecycle");
        if text_field(lifecycle, "status") == "withdrawn" {
            return Err(format!("{path} retains a withdrawn session"));
        }
        if text_field(row_entries, "participant_id")
            != text_field(envelope_entries, "participant_id")
        {
            return Err(format!("{path} names another participant than its session"));
        }
        if text_field(row_entries, "condition_id") != text_field(envelope_entries, "condition_id") {
            return Err(format!("{path} names another condition than its session"));
        }
        let assignment = record_field(envelope_entries, "assignment");
        if text_field(row_entries, "build_id") != text_field(assignment, "build_id") {
            return Err(format!("{path} names another build than its session"));
        }
        if text_field(row_entries, "tuning_config_id") != text_field(assignment, "tuning_config_id")
        {
            return Err(format!(
                "{path} names another tuning configuration than its session"
            ));
        }
        let agreement = record_field(envelope_entries, "agreement");
        let row_agreement_version = text_field(row_entries, "agreement_version");
        let envelope_agreement_version = text_field(agreement, "agreement_version");
        if row_agreement_version != envelope_agreement_version {
            return Err(format!(
                "{path} claims agreement version {row_agreement_version} but the session accepted {envelope_agreement_version}"
            ));
        }
        let model_use_allowed = research_session::allows_model_use(envelope).is_ok();
        let row_model_use_covered = bool_field(row_entries, "model_use_covered");
        if row_model_use_covered != model_use_allowed {
            return Err(format!(
                "{path} claims model_use_covered={row_model_use_covered} but its accepted agreement says {model_use_allowed}"
            ));
        }
        let row_trace_id = text_field(row_entries, "trace_id");
        let linked = array_field(envelope_entries, "trace_links")
            .iter()
            .any(|link| {
                let link_entries = link.as_record().expect("validated link");
                text_field(link_entries, "trace_id") == row_trace_id
            });
        if !linked {
            return Err(format!(
                "{path} pins a trace its session does not reference"
            ));
        }
    }
    Ok(())
}

/// A dataset may not contain a payload a withdrawal tombstone revoked.
/// Derived datasets are rebuilt from the tombstoned manifest, never
/// hand-patched.
pub fn validate_against_tombstones(manifest: &Value, tombstones: &[Value]) -> Result<()> {
    validate(manifest)?;
    let entries = manifest.as_record().expect("validated record");

    let mut revoked_participants: Vec<String> = Vec::new();
    let mut revoked_sessions: Vec<String> = Vec::new();
    let mut revoked_hashes: Vec<String> = Vec::new();
    for (index, tombstone) in tombstones.iter().enumerate() {
        let tombstone_entries = tombstone.as_record().ok_or_else(|| {
            format!(
                "research withdrawal tombstone {} must be a table",
                index + 1
            )
        })?;
        // Withdrawal is always full: the session goes and anything derived
        // from it is regenerated. Revoking by participant and session as
        // well as by payload hash is what keeps this robust against a
        // source row that pinned a manifest hash at a different point in
        // time.
        let participant_id = text_field(tombstone_entries, "participant_id").to_string();
        if !revoked_participants.contains(&participant_id) {
            revoked_participants.push(participant_id);
        }
        let session_id = text_field(tombstone_entries, "session_id").to_string();
        if !revoked_sessions.contains(&session_id) {
            revoked_sessions.push(session_id);
        }
        if let Some(hashes) =
            Value::record_get(tombstone_entries, "revoked_payload_hashes").and_then(Value::as_array)
        {
            for hash in hashes {
                let hash_str = hash.as_str().expect("validated hash").to_string();
                if !revoked_hashes.contains(&hash_str) {
                    revoked_hashes.push(hash_str);
                }
            }
        }
    }

    let sources = array_field(entries, "sources");
    for (index, row) in sources.iter().enumerate() {
        let path = format!("research_dataset_manifest.sources.{}", index + 1);
        let row_entries = row.as_record().expect("validated source row");
        let participant_id = text_field(row_entries, "participant_id");
        if revoked_participants.iter().any(|p| p == participant_id) {
            return Err(format!(
                "{path} retains withdrawn participant {participant_id}"
            ));
        }
        let session_id = text_field(row_entries, "session_id");
        if revoked_sessions.iter().any(|s| s == session_id) {
            return Err(format!("{path} retains withdrawn session {session_id}"));
        }
        let trace_manifest_hash = text_field(row_entries, "trace_manifest_hash");
        if revoked_hashes.iter().any(|h| h == trace_manifest_hash) {
            return Err(format!("{path} retains a revoked payload hash"));
        }
    }
    Ok(())
}

/// Canonical bytes of a valid dataset manifest.
pub fn encode(manifest: &Value) -> Result<Vec<u8>> {
    validate(manifest)?;
    research_schema::encode(&shape(), manifest)
}

/// Decode and re-validate a dataset manifest.
pub fn decode(bytes: &[u8]) -> Result<Value> {
    let manifest = research_schema::decode(&shape(), bytes)?;
    validate(&manifest)?;
    Ok(manifest)
}
