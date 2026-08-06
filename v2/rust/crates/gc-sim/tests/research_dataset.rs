//! Port of `spec/sim/research_dataset_spec.lua`.

mod research_fixtures;

use gc_sim::research_dataset::{self};
use gc_sim::research_features;
use gc_sim::research_schema::{self, Value};
use research_fixtures::{DatasetOverrides, DatasetSessionsOverrides};

fn record(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn record_entries(value: &Value) -> Vec<(String, Value)> {
    value.as_record().expect("value is a record").to_vec()
}

fn get<'a>(entries: &'a [(String, Value)], name: &str) -> &'a Value {
    Value::record_get(entries, name).unwrap_or_else(|| panic!("missing field {name}"))
}

fn set(entries: &mut Vec<(String, Value)>, name: &str, value: Value) {
    if let Some(entry) = entries.iter_mut().find(|(k, _)| k == name) {
        entry.1 = value;
    } else {
        entries.push((name.to_string(), value));
    }
}

fn with_top(manifest: &Value, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    let mut top = record_entries(manifest);
    f(&mut top);
    Value::Record(top)
}

fn with_source(manifest: &Value, index: usize, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    with_top(manifest, |top| {
        let mut sources: Vec<Value> = get(top, "sources").as_array().expect("array").to_vec();
        let mut source = record_entries(&sources[index]);
        f(&mut source);
        sources[index] = Value::Record(source);
        set(top, "sources", Value::Array(sources));
    })
}

fn with_fold(split: &Value, index: usize, f: impl FnOnce(&mut Vec<(String, Value)>)) -> Value {
    with_top(split, |top| {
        let mut folds: Vec<Value> = get(top, "folds").as_array().expect("array").to_vec();
        let mut fold = record_entries(&folds[index]);
        f(&mut fold);
        folds[index] = Value::Record(fold);
        set(top, "folds", Value::Array(folds));
    })
}

fn dataset() -> Value {
    let gameplay = research_fixtures::gameplay(None);
    research_fixtures::dataset(&gameplay.manifest, None).expect("dataset seals")
}

fn dataset_with_splits(splits: Vec<Value>) -> research_schema::Result<Value> {
    let gameplay = research_fixtures::gameplay(None);
    research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            splits: Some(splits),
            ..Default::default()
        }),
    )
}

fn participant_split() -> Value {
    record(vec![
        ("split_id", Value::str("split-participant-holdout")),
        ("grouping", Value::str("participant")),
        (
            "folds",
            Value::Array(vec![
                record(vec![
                    ("fold_id", Value::str("train")),
                    ("role", Value::str("train")),
                    (
                        "participant_ids",
                        Value::Array(vec![
                            Value::str(research_fixtures::PARTICIPANT_ID),
                            Value::str(research_fixtures::SECOND_PARTICIPANT_ID),
                        ]),
                    ),
                    ("build_ids", Value::Array(vec![])),
                ]),
                record(vec![
                    ("fold_id", Value::str("test")),
                    ("role", Value::str("test")),
                    (
                        "participant_ids",
                        Value::Array(vec![Value::str(research_fixtures::THIRD_PARTICIPANT_ID)]),
                    ),
                    ("build_ids", Value::Array(vec![])),
                ]),
            ]),
        ),
    ])
}

fn build_split() -> Value {
    record(vec![
        ("split_id", Value::str("split-build-holdout")),
        ("grouping", Value::str("build")),
        (
            "folds",
            Value::Array(vec![
                record(vec![
                    ("fold_id", Value::str("train")),
                    ("role", Value::str("train")),
                    ("participant_ids", Value::Array(vec![])),
                    ("build_ids", Value::Array(vec![Value::str("spec-build")])),
                ]),
                record(vec![
                    ("fold_id", Value::str("holdout")),
                    ("role", Value::str("holdout")),
                    ("participant_ids", Value::Array(vec![])),
                    (
                        "build_ids",
                        Value::Array(vec![Value::str("spec-build-next")]),
                    ),
                ]),
            ]),
        ),
    ])
}

#[test]
fn research_dataset_manifest_seals_validates_and_round_trips() {
    let manifest = dataset();
    research_dataset::validate(&manifest).expect("dataset validates");
    let top = record_entries(&manifest);
    let expected_hash = research_dataset::derive_hash(&manifest).expect("hash derives");
    assert_eq!(
        get(&top, "dataset_hash").as_str(),
        Some(expected_hash.as_str())
    );

    let bytes = research_dataset::encode(&manifest).expect("manifest encodes");
    let decoded = research_dataset::decode(&bytes).expect("manifest decodes");
    assert_eq!(
        research_dataset::encode(&decoded).expect("decoded manifest encodes"),
        bytes
    );
    let decoded_top = record_entries(&decoded);
    assert_eq!(
        get(&decoded_top, "dataset_id").as_str(),
        get(&top, "dataset_id").as_str()
    );
    assert_eq!(
        get(&decoded_top, "splits").as_array().expect("array").len(),
        2
    );
}

#[test]
fn research_dataset_manifest_detects_any_hand_edit_through_the_dataset_hash() {
    let manifest = dataset();
    let edited = with_source(&manifest, 0, |entries| {
        set(entries, "condition_id", Value::str("condition-a-combat-off"));
    });
    let err = research_dataset::validate(&edited).expect_err("edited dataset must fail");
    assert!(err.contains("dataset_hash"), "unexpected error: {err}");
}

#[test]
fn research_dataset_manifest_pins_the_feature_register_it_was_extracted_with() {
    let manifest = dataset();
    let drifted = with_top(&manifest, |top| {
        let mut extraction = record_entries(get(top, "extraction"));
        set(
            &mut extraction,
            "feature_registry_hash",
            Value::str("0000000000000000"),
        );
        set(top, "extraction", Value::Record(extraction));
    });
    let hash = research_dataset::derive_hash(&drifted).expect("hash derives");
    let drifted = with_top(&drifted, |top| set(top, "dataset_hash", Value::str(hash)));
    let err = research_dataset::validate(&drifted).expect_err("drifted registry must fail");
    assert!(err.contains("feature register"), "unexpected error: {err}");
}

#[test]
fn research_dataset_manifest_rejects_unknown_features_version_drift_and_undefined_aggregation_levels()
 {
    let gameplay = research_fixtures::gameplay(None);

    let unknown = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            feature_versions: Some(vec![record(vec![
                ("feature_id", Value::str("no-such-feature")),
                ("version", Value::Number(1.0)),
                ("aggregation_level", Value::str("match")),
            ])]),
            ..Default::default()
        }),
    );
    assert!(unknown.is_err());

    let drifted = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            feature_versions: Some(vec![record(vec![
                ("feature_id", Value::str("pxi_enjoyment_addon.enjoyment")),
                ("version", Value::Number(99.0)),
                ("aggregation_level", Value::str("condition_block")),
            ])]),
            ..Default::default()
        }),
    );
    assert!(drifted.is_err());

    let wrong_level = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            feature_versions: Some(vec![record(vec![
                ("feature_id", Value::str("pxi_enjoyment_addon.enjoyment")),
                ("version", Value::Number(1.0)),
                ("aggregation_level", Value::str("tick")),
            ])]),
            ..Default::default()
        }),
    );
    assert!(wrong_level.is_err());
}

#[test]
fn research_dataset_manifest_keeps_the_feature_register_hash_reachable_for_lineage() {
    let manifest = dataset();
    let top = record_entries(&manifest);
    let extraction = record_entries(get(&top, "extraction"));
    assert_eq!(
        get(&extraction, "feature_registry_hash").as_str(),
        Some(research_features::registry_hash().as_str())
    );
}

#[test]
fn research_dataset_splits_fails_closed_when_participants_overlap_across_folds() {
    let overlapping = with_fold(&participant_split(), 1, |fold| {
        set(
            fold,
            "participant_ids",
            Value::Array(vec![Value::str(research_fixtures::PARTICIPANT_ID)]),
        );
    });
    let err = dataset_with_splits(vec![overlapping]).expect_err("overlap must fail");
    assert!(err.contains("appears in both"), "unexpected error: {err}");
}

#[test]
fn research_dataset_splits_fails_closed_when_builds_overlap_across_folds() {
    let overlapping = with_fold(&build_split(), 1, |fold| {
        set(fold, "build_ids", Value::Array(vec![Value::str("spec-build")]));
    });
    let err = dataset_with_splits(vec![overlapping]).expect_err("overlap must fail");
    assert!(err.contains("appears in both"), "unexpected error: {err}");
}

#[test]
fn research_dataset_splits_supports_a_combined_participant_and_build_holdout() {
    let combined = record(vec![
        ("split_id", Value::str("split-participant-and-build")),
        ("grouping", Value::str("participant_and_build")),
        (
            "folds",
            Value::Array(vec![
                record(vec![
                    ("fold_id", Value::str("train")),
                    ("role", Value::str("train")),
                    (
                        "participant_ids",
                        Value::Array(vec![
                            Value::str(research_fixtures::PARTICIPANT_ID),
                            Value::str(research_fixtures::SECOND_PARTICIPANT_ID),
                        ]),
                    ),
                    ("build_ids", Value::Array(vec![Value::str("spec-build")])),
                ]),
                record(vec![
                    ("fold_id", Value::str("test")),
                    ("role", Value::str("test")),
                    (
                        "participant_ids",
                        Value::Array(vec![Value::str(research_fixtures::THIRD_PARTICIPANT_ID)]),
                    ),
                    (
                        "build_ids",
                        Value::Array(vec![Value::str("spec-build-next")]),
                    ),
                ]),
            ]),
        ),
    ]);
    assert!(dataset_with_splits(vec![combined.clone()]).is_ok());

    let leaky = with_fold(&combined, 1, |fold| {
        set(
            fold,
            "build_ids",
            Value::Array(vec![
                Value::str("spec-build"),
                Value::str("spec-build-next"),
            ]),
        );
    });
    assert!(dataset_with_splits(vec![leaky]).is_err());
}

#[test]
fn research_dataset_splits_requires_full_coverage_and_known_members() {
    let incomplete = with_fold(&participant_split(), 0, |fold| {
        set(
            fold,
            "participant_ids",
            Value::Array(vec![Value::str(research_fixtures::PARTICIPANT_ID)]),
        );
    });
    let err = dataset_with_splits(vec![incomplete]).expect_err("must fail");
    assert!(err.contains("does not cover"), "unexpected error: {err}");

    let unknown = with_fold(&participant_split(), 1, |fold| {
        set(
            fold,
            "participant_ids",
            Value::Array(vec![Value::str("p-ffffffffffffffff")]),
        );
    });
    assert!(dataset_with_splits(vec![unknown]).is_err());

    let empty_fold = with_fold(&participant_split(), 1, |fold| {
        set(fold, "participant_ids", Value::Array(vec![]));
    });
    assert!(dataset_with_splits(vec![empty_fold]).is_err());
}

#[test]
fn research_dataset_splits_rejects_a_split_with_no_train_or_no_held_out_fold() {
    let train_only = with_fold(&participant_split(), 1, |fold| {
        set(fold, "role", Value::str("train"));
    });
    assert!(dataset_with_splits(vec![train_only]).is_err());

    let test_only = with_fold(&participant_split(), 0, |fold| {
        set(fold, "role", Value::str("validation"));
    });
    assert!(dataset_with_splits(vec![test_only]).is_err());
}

#[test]
fn research_dataset_splits_keeps_participant_and_build_groupings_from_bleeding_into_each_other() {
    let participant_with_builds = with_fold(&participant_split(), 0, |fold| {
        set(fold, "build_ids", Value::Array(vec![Value::str("spec-build")]));
    });
    assert!(dataset_with_splits(vec![participant_with_builds]).is_err());

    let build_with_participants = with_fold(&build_split(), 0, |fold| {
        set(
            fold,
            "participant_ids",
            Value::Array(vec![Value::str(research_fixtures::PARTICIPANT_ID)]),
        );
    });
    assert!(dataset_with_splits(vec![build_with_participants]).is_err());
}

#[test]
fn research_dataset_splits_requires_a_split_manifest_for_model_training() {
    let gameplay = research_fixtures::gameplay(None);
    let without_splits = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            purpose: Some("model_training".to_string()),
            splits: Some(vec![]),
            ..Default::default()
        }),
    );
    assert!(without_splits.is_err());
    let with_default_splits = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            purpose: Some("model_training".to_string()),
            ..Default::default()
        }),
    );
    assert!(with_default_splits.is_ok());
}

#[test]
fn research_dataset_lineage_requires_a_parent_hash_whenever_a_transformation_is_recorded() {
    let gameplay = research_fixtures::gameplay(None);
    let transformation = record(vec![
        ("transformation_id", Value::str("drop-withdrawn")),
        (
            "description",
            Value::str("rebuild after a withdrawal tombstone"),
        ),
        ("input_hash", Value::str("1111111111111111")),
        ("output_hash", Value::str("2222222222222222")),
    ]);
    let orphan = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            transformations: Some(vec![transformation.clone()]),
            ..Default::default()
        }),
    );
    assert!(orphan.is_err());

    let child = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            dataset_id: Some("ds-m11-enjoyment-v2".to_string()),
            parent_dataset_hash: Some("3333333333333333".to_string()),
            transformations: Some(vec![transformation]),
            ..Default::default()
        }),
    )
    .expect("child dataset builds");
    let child_top = record_entries(&child);
    assert_eq!(
        get(&child_top, "parent_dataset_hash").as_str(),
        Some("3333333333333333")
    );

    let no_op_transform = record(vec![
        ("transformation_id", Value::str("no-op")),
        ("description", Value::str("changes nothing")),
        ("input_hash", Value::str("1111111111111111")),
        ("output_hash", Value::str("1111111111111111")),
    ]);
    let no_op = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            parent_dataset_hash: Some("3333333333333333".to_string()),
            transformations: Some(vec![no_op_transform]),
            ..Default::default()
        }),
    );
    assert!(no_op.is_err());
}

#[test]
fn research_dataset_lineage_ties_model_use_permission_to_every_sources_accepted_agreement() {
    let gameplay = research_fixtures::gameplay(None);
    assert!(
        research_fixtures::dataset(
            &gameplay.manifest,
            Some(&DatasetOverrides {
                purpose: Some("model_training".to_string()),
                ..Default::default()
            }),
        )
        .is_ok()
    );

    let uncovered_training = research_fixtures::dataset(
        &gameplay.manifest,
        Some(&DatasetOverrides {
            purpose: Some("model_training".to_string()),
            model_use_covered: Some(false),
            ..Default::default()
        }),
    );
    assert!(uncovered_training.is_err());

    let manifest = dataset();
    let retroactive = with_source(&manifest, 1, |entries| {
        set(entries, "model_use_covered", Value::Bool(false));
    });
    let hash = research_dataset::derive_hash(&retroactive).expect("hash derives");
    let retroactive = with_top(&retroactive, |top| set(top, "dataset_hash", Value::str(hash)));
    let err = research_dataset::validate(&retroactive).expect_err("must fail");
    assert!(err.contains("model_use_covered"), "unexpected error: {err}");

    let undeclared_version = with_source(&manifest, 0, |entries| {
        set(
            entries,
            "agreement_version",
            Value::str("playtest-agreement-v9"),
        );
    });
    let hash = research_dataset::derive_hash(&undeclared_version).expect("hash derives");
    let undeclared_version =
        with_top(&undeclared_version, |top| set(top, "dataset_hash", Value::str(hash)));
    let err = research_dataset::validate(&undeclared_version).expect_err("must fail");
    assert!(err.contains("does not declare"), "unexpected error: {err}");
}

#[test]
fn research_dataset_lineage_verifies_every_source_row_against_the_session_envelope_it_came_from() {
    let gameplay = research_fixtures::gameplay(None);
    let manifest =
        research_fixtures::dataset(&gameplay.manifest, None).expect("dataset seals");
    let envelopes = research_fixtures::dataset_sessions(&gameplay.manifest, None);
    research_dataset::validate_against_sessions(&manifest, &envelopes).expect("sessions validate");

    // The row says model use was covered; the accepted agreement says it was
    // not. Internal self-consistency cannot see this; the join can.
    let uncovered = research_fixtures::dataset_sessions(
        &gameplay.manifest,
        Some(&DatasetSessionsOverrides {
            third_model_use_covered: Some(false),
        }),
    );
    let err =
        research_dataset::validate_against_sessions(&manifest, &uncovered).expect_err("must fail");
    assert!(err.contains("model_use_covered"), "unexpected error: {err}");
    assert!(err.contains("accepted agreement"), "unexpected error: {err}");

    let mut drifted_version = research_fixtures::dataset_sessions(&gameplay.manifest, None);
    drifted_version[1] = with_top(&drifted_version[1], |top| {
        let mut agreement = record_entries(get(top, "agreement"));
        set(
            &mut agreement,
            "agreement_version",
            Value::str("playtest-agreement-v2"),
        );
        set(top, "agreement", Value::Record(agreement));
    });
    let err = research_dataset::validate_against_sessions(&manifest, &drifted_version)
        .expect_err("must fail");
    assert!(
        err.contains("but the session accepted"),
        "unexpected error: {err}"
    );

    let mut missing_session = research_fixtures::dataset_sessions(&gameplay.manifest, None);
    missing_session.remove(2);
    let err = research_dataset::validate_against_sessions(&manifest, &missing_session)
        .expect_err("must fail");
    assert!(err.contains("orphan join"), "unexpected error: {err}");

    let mut duplicated = research_fixtures::dataset_sessions(&gameplay.manifest, None);
    duplicated[2] = duplicated[1].clone();
    assert!(research_dataset::validate_against_sessions(&manifest, &duplicated).is_err());
}

#[test]
fn research_dataset_lineage_refuses_a_dataset_whose_source_session_was_withdrawn() {
    let gameplay = research_fixtures::gameplay(None);
    let manifest =
        research_fixtures::dataset(&gameplay.manifest, None).expect("dataset seals");
    let mut envelopes = research_fixtures::dataset_sessions(&gameplay.manifest, None);
    let second_entries = record_entries(&envelopes[1]);
    let second_session_id = get(&second_entries, "session_id")
        .as_str()
        .expect("id")
        .to_string();
    let second_participant_id = get(&second_entries, "participant_id")
        .as_str()
        .expect("id")
        .to_string();
    let withdrawn = with_top(&research_fixtures::withdrawn_session(), |top| {
        set(top, "session_id", Value::str(second_session_id));
        set(top, "participant_id", Value::str(second_participant_id));
    });
    envelopes[1] = withdrawn;
    let err = research_dataset::validate_against_sessions(&manifest, &envelopes)
        .expect_err("must fail");
    assert!(err.contains("withdrawn session"), "unexpected error: {err}");
}

#[test]
fn research_dataset_lineage_refuses_a_source_row_that_pins_a_trace_its_session_never_referenced() {
    let gameplay = research_fixtures::gameplay(None);
    let manifest =
        research_fixtures::dataset(&gameplay.manifest, None).expect("dataset seals");
    let mut envelopes = research_fixtures::dataset_sessions(&gameplay.manifest, None);
    envelopes[0] = with_top(&envelopes[0], |top| {
        let mut trace_links: Vec<Value> = get(top, "trace_links").as_array().expect("array").to_vec();
        let mut link0 = record_entries(&trace_links[0]);
        set(&mut link0, "trace_id", Value::str("abcdefabcdefabcd"));
        trace_links[0] = Value::Record(link0);
        set(top, "trace_links", Value::Array(trace_links));
    });
    let err = research_dataset::validate_against_sessions(&manifest, &envelopes)
        .expect_err("must fail");
    assert!(err.contains("does not reference"), "unexpected error: {err}");
}

#[test]
fn research_dataset_lineage_rejects_non_finite_numbers_in_this_family() {
    let manifest = dataset();
    let nan = with_top(&manifest, |top| {
        set(top, "created_wall_clock_ms", Value::Number(f64::NAN));
    });
    assert!(research_dataset::validate(&nan).is_err());
}

#[test]
fn research_dataset_lineage_requires_excluded_sources_to_record_a_reason_and_stay_out_of_folds() {
    let manifest = dataset();
    let unexplained = with_source(&manifest, 0, |entries| {
        set(entries, "excluded", Value::Bool(true));
    });
    let hash = research_dataset::derive_hash(&unexplained).expect("hash derives");
    let unexplained = with_top(&unexplained, |top| set(top, "dataset_hash", Value::str(hash)));
    assert!(research_dataset::validate(&unexplained).is_err());

    let excluded_in_fold = with_source(&manifest, 2, |entries| {
        set(entries, "excluded", Value::Bool(true));
        set(entries, "exclusion_reason", Value::str("technical_failure"));
    });
    let hash = research_dataset::derive_hash(&excluded_in_fold).expect("hash derives");
    let excluded_in_fold =
        with_top(&excluded_in_fold, |top| set(top, "dataset_hash", Value::str(hash)));
    let err = research_dataset::validate(&excluded_in_fold).expect_err("must fail");
    assert!(
        err.contains("excluded participant"),
        "unexpected error: {err}"
    );
}
