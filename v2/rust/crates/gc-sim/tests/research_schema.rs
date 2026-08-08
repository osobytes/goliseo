//! Port of `spec/sim/research_schema_spec.lua`.
//!
//! Two cases from the Lua spec have no Rust counterpart and are dropped
//! rather than stubbed `#[ignore]`: `sparse.tags = { "a", nil, "c" }` and
//! `keyed.tags = { a = 1 }` both exercise Lua's "is this table a dense
//! array" check, which exists only because Lua tables can have holes or
//! non-numeric keys. [`gc_sim::research_schema::Value::Array`] is a `Vec`,
//! always dense by construction, so there is no way to construct the
//! malformed input the Lua case is guarding against — see
//! `research_schema_validation_rejects_malformed_arrays_and_maps` below,
//! which keeps the one sub-case that does still apply (a map key outside
//! the `id` charset).

use gc_sim::research_schema::{
    HASH_LENGTH, ResearchField, ResearchFieldKind, ResearchShape, TuplePart, Value,
    accepts_version, assert_disjoint, content_hash, decode, encode, enum_values, record,
    tuple_hash, validate,
};

fn record_value(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn spec_shape() -> ResearchShape {
    record(
        "spec_shape",
        vec![
            ResearchField::new(ResearchFieldKind::Id).named("id"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("count")
                .min(0.0),
            ResearchField::new(ResearchFieldKind::Number)
                .named("share")
                .min(0.0)
                .max(1.0),
            ResearchField::new(ResearchFieldKind::Boolean).named("flag"),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("status")
                .values(enum_values(&["open", "closed"])),
            ResearchField::new(ResearchFieldKind::Hash).named("digest"),
            ResearchField::new(ResearchFieldKind::Text)
                .named("note")
                .optional(),
            ResearchField::new(ResearchFieldKind::Array)
                .named("tags")
                .element(ResearchField::new(ResearchFieldKind::Id).named("tag")),
            ResearchField::new(ResearchFieldKind::Map)
                .named("scores")
                .element(ResearchField::new(ResearchFieldKind::Integer).named("score")),
            ResearchField::new(ResearchFieldKind::Record)
                .named("nested")
                .optional()
                .fields(vec![
                    ResearchField::new(ResearchFieldKind::Str).named("label"),
                    ResearchField::new(ResearchFieldKind::Number).named("weight"),
                ]),
        ],
    )
}

fn spec_payload() -> Vec<(String, Value)> {
    vec![
        ("id".to_string(), Value::str("session.alpha-01")),
        ("count".to_string(), Value::Number(3.0)),
        ("share".to_string(), Value::Number(0.25)),
        ("flag".to_string(), Value::Bool(true)),
        ("status".to_string(), Value::str("open")),
        ("digest".to_string(), Value::str("0123456789abcdef")),
        (
            "tags".to_string(),
            Value::Array(vec![Value::str("a"), Value::str("b")]),
        ),
        (
            "scores".to_string(),
            Value::Map(vec![
                ("home".to_string(), Value::Number(1.0)),
                ("away".to_string(), Value::Number(2.0)),
            ]),
        ),
    ]
}

#[test]
fn research_schema_validation_accepts_a_complete_payload_and_reports_the_failing_path_otherwise() {
    let shape = spec_shape();
    assert!(validate(&shape, &Value::Record(spec_payload())).is_ok());

    let mut missing = spec_payload();
    missing.retain(|(k, _)| k != "count");
    let err = validate(&shape, &Value::Record(missing)).unwrap_err();
    assert_eq!(err, "spec_shape.count is required");
}

#[test]
fn research_schema_validation_rejects_unknown_fields_instead_of_ignoring_them() {
    let shape = spec_shape();
    let mut unknown = spec_payload();
    unknown.push(("future_field".to_string(), Value::Number(1.0)));
    let err = validate(&shape, &Value::Record(unknown)).unwrap_err();
    assert_eq!(err, "spec_shape has unknown field future_field");
}

fn with_field(mut payload: Vec<(String, Value)>, name: &str, value: Value) -> Vec<(String, Value)> {
    if let Some(entry) = payload.iter_mut().find(|(k, _)| k == name) {
        entry.1 = value;
    } else {
        payload.push((name.to_string(), value));
    }
    payload
}

#[test]
fn research_schema_validation_rejects_bad_enums_out_of_range_and_non_finite_numbers() {
    let shape = spec_shape();
    let bad_enum = with_field(spec_payload(), "status", Value::str("paused"));
    assert!(validate(&shape, &Value::Record(bad_enum)).is_err());

    let out_of_range = with_field(spec_payload(), "share", Value::Number(1.5));
    assert!(validate(&shape, &Value::Record(out_of_range)).is_err());

    let nan = with_field(spec_payload(), "share", Value::Number(f64::NAN));
    assert!(validate(&shape, &Value::Record(nan)).is_err());

    let infinite = with_field(spec_payload(), "count", Value::Number(f64::INFINITY));
    assert!(validate(&shape, &Value::Record(infinite)).is_err());

    let fractional = with_field(spec_payload(), "count", Value::Number(1.5));
    assert!(validate(&shape, &Value::Record(fractional)).is_err());
}

#[test]
fn research_schema_validation_rejects_malformed_arrays_and_maps() {
    let shape = spec_shape();
    // A Rust Vec cannot hold a hole, so the "sparse array" and "keyed
    // table masquerading as an array" cases from the Lua spec have no
    // Rust counterpart (Value::Array is always dense by construction).
    // The map-key charset case still applies:
    let bad_map_key = with_field(
        spec_payload(),
        "scores",
        Value::Map(vec![("Home Team".to_string(), Value::Number(1.0))]),
    );
    assert!(validate(&shape, &Value::Record(bad_map_key)).is_err());
}

#[test]
fn research_schema_validation_rejects_direct_identifiers_and_raw_paths_in_join_keys() {
    let shape = spec_shape();
    for value in [
        "participant@example.com",
        "https://example.com/p",
        "../secrets",
        "c:\\users\\p",
        "Participant01",
        "home/oscar/matches/save1.json",
        "c:/users/oscar/appdata/save.json",
        "/var/log/goliseo",
        "users:oscar",
    ] {
        let leaked = with_field(spec_payload(), "id", Value::str(value));
        let result = validate(&shape, &Value::Record(leaked));
        assert!(result.is_err(), "expected {value} to be rejected");
    }
}

#[test]
fn research_schema_validation_accepts_the_slug_grammar_the_contracts_actually_use() {
    let shape = spec_shape();
    for value in [
        "pxi_enjoyment_addon.enjoyment",
        "playtest-agreement-v3",
        "spec-build",
        "en-gb",
        "share_0_to_1",
    ] {
        let slug = with_field(spec_payload(), "id", Value::str(value));
        assert!(
            validate(&shape, &Value::Record(slug)).is_ok(),
            "{value} should be a legal id"
        );
    }
}

#[test]
fn research_schema_validation_rejects_malformed_digests_and_control_characters() {
    let shape = spec_shape();
    let short_digest = with_field(spec_payload(), "digest", Value::str("abc"));
    assert!(validate(&shape, &Value::Record(short_digest)).is_err());

    let upper_digest = with_field(spec_payload(), "digest", Value::str("0123456789ABCDEF"));
    assert!(validate(&shape, &Value::Record(upper_digest)).is_err());

    let control = with_field(
        spec_payload(),
        "nested",
        record_value(vec![
            ("label", Value::str("bad\nlabel")),
            ("weight", Value::Number(1.0)),
        ]),
    );
    assert!(validate(&shape, &Value::Record(control)).is_err());
}

#[test]
fn research_schema_canonical_serialization_round_trips_through_encode_decode_byte_for_byte() {
    let shape = spec_shape();
    let mut value = spec_payload();
    value = with_field(
        value,
        "note",
        Value::str("free text is bounded, never a join key"),
    );
    value = with_field(
        value,
        "nested",
        record_value(vec![
            ("label", Value::str("nested")),
            ("weight", Value::Number(-0.5)),
        ]),
    );
    let value = Value::Record(value);
    let bytes = encode(&shape, &value).unwrap();
    let decoded = decode(&shape, &bytes).unwrap();
    let re_encoded = encode(&shape, &decoded).unwrap();
    assert_eq!(re_encoded, bytes);

    let entries = decoded.as_record().unwrap();
    assert_eq!(
        Value::record_get(entries, "id").and_then(Value::as_str),
        Some("session.alpha-01")
    );
    assert_eq!(
        Value::record_get(entries, "note").and_then(Value::as_str),
        Some("free text is bounded, never a join key")
    );
    let nested = Value::record_get(entries, "nested")
        .unwrap()
        .as_record()
        .unwrap();
    assert_eq!(
        Value::record_get(nested, "weight").and_then(Value::as_number),
        Some(-0.5)
    );
    let scores = Value::record_get(entries, "scores")
        .unwrap()
        .as_map()
        .unwrap();
    assert_eq!(
        Value::record_get(scores, "home").and_then(Value::as_number),
        Some(1.0)
    );
    let tags = Value::record_get(entries, "tags")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(tags.len(), 2);
}

#[test]
fn research_schema_canonical_serialization_round_trips_awkward_finite_numbers_exactly() {
    let shape = spec_shape();
    for number in [
        0.1,
        -0.1,
        1.0 / 3.0,
        2f64.powi(-30),
        1e17,
        -1e-17,
        0.0,
        6.02e23,
    ] {
        let value = with_field(
            spec_payload(),
            "nested",
            record_value(vec![
                ("label", Value::str("n")),
                ("weight", Value::Number(number)),
            ]),
        );
        let value = Value::Record(value);
        let bytes = encode(&shape, &value).unwrap();
        let decoded = decode(&shape, &bytes).unwrap();
        let nested = decoded.as_record().unwrap();
        let nested = Value::record_get(nested, "nested")
            .unwrap()
            .as_record()
            .unwrap();
        let weight = Value::record_get(nested, "weight")
            .and_then(Value::as_number)
            .unwrap();
        assert_eq!(
            weight.to_bits(),
            number.to_bits(),
            "number round-trip for {number}"
        );
    }
}

#[test]
fn research_schema_canonical_serialization_hashes_independently_of_table_insertion_order() {
    let shape = spec_shape();
    let left = with_field(
        spec_payload(),
        "scores",
        Value::Map(vec![
            ("away".to_string(), Value::Number(2.0)),
            ("home".to_string(), Value::Number(1.0)),
        ]),
    );
    let right = with_field(
        spec_payload(),
        "scores",
        Value::Map(vec![
            ("home".to_string(), Value::Number(1.0)),
            ("away".to_string(), Value::Number(2.0)),
        ]),
    );
    assert_eq!(
        content_hash(&shape, &Value::Record(left)).unwrap(),
        content_hash(&shape, &Value::Record(right)).unwrap()
    );
}

#[test]
fn research_schema_canonical_serialization_changes_the_content_hash_when_any_field_changes() {
    let shape = spec_shape();
    let base = content_hash(&shape, &Value::Record(spec_payload())).unwrap();
    let changed = with_field(spec_payload(), "count", Value::Number(4.0));
    let changed = content_hash(&shape, &Value::Record(changed)).unwrap();
    assert_ne!(base, changed);
    assert_eq!(base.len(), HASH_LENGTH);
}

#[test]
fn research_schema_canonical_serialization_refuses_to_encode_an_invalid_payload() {
    let shape = spec_shape();
    let broken = with_field(spec_payload(), "status", Value::str("unknown"));
    assert!(encode(&shape, &Value::Record(broken)).is_err());
}

#[test]
fn research_schema_canonical_serialization_fails_closed_on_truncated_trailing_and_foreign_payloads()
{
    let shape = spec_shape();
    let bytes = encode(&shape, &Value::Record(spec_payload())).unwrap();
    assert!(decode(&shape, &bytes[..bytes.len() - 4]).is_err());
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(b"s1:x;");
    assert!(decode(&shape, &trailing).is_err());
    assert!(decode(&shape, b"not-a-research-payload").is_err());

    let other = record(
        "other_shape",
        vec![ResearchField::new(ResearchFieldKind::Id).named("id")],
    );
    let foreign = encode(
        &other,
        &Value::Record(vec![("id".to_string(), Value::str("x"))]),
    )
    .unwrap();
    assert!(decode(&shape, &foreign).is_err());
}

#[test]
fn research_schema_canonical_serialization_stops_with_a_migration_diagnostic_on_a_future_serialization_version()
 {
    let shape = spec_shape();
    let bytes = encode(&shape, &Value::Record(spec_payload())).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let future = text.replacen("GCRS1;", "GCRS9;", 1);
    let err = decode(&shape, future.as_bytes()).unwrap_err();
    assert!(err.contains("no migration"), "{err}");
}

#[test]
fn research_schema_helpers_gates_unsupported_schema_versions_with_a_migration_diagnostic() {
    let supported = [2i64, 3i64];
    assert!(accepts_version("trace", &supported, 3, Some(&Value::Number(2.0))).is_ok());
    let err = accepts_version("trace", &supported, 3, Some(&Value::Number(1.0))).unwrap_err();
    assert!(err.contains("no migration"), "{err}");
    assert!(accepts_version("trace", &supported, 3, Some(&Value::str("2"))).is_err());
}

#[test]
fn research_schema_helpers_hashes_ordered_tuples_unambiguously() {
    let left = tuple_hash(
        "run/v1",
        &[
            TuplePart::Text("a".to_string()),
            TuplePart::Text("bc".to_string()),
        ],
    );
    let right = tuple_hash(
        "run/v1",
        &[
            TuplePart::Text("ab".to_string()),
            TuplePart::Text("c".to_string()),
        ],
    );
    assert_ne!(left, right);
    assert_eq!(
        left,
        tuple_hash(
            "run/v1",
            &[
                TuplePart::Text("a".to_string()),
                TuplePart::Text("bc".to_string())
            ]
        )
    );
    assert_ne!(
        left,
        tuple_hash(
            "run/v2",
            &[
                TuplePart::Text("a".to_string()),
                TuplePart::Text("bc".to_string())
            ]
        )
    );
    assert_ne!(
        tuple_hash("run/v1", &[TuplePart::Number(1.0)]),
        tuple_hash("run/v1", &[TuplePart::Text("1".to_string())])
    );
}

#[test]
fn research_schema_helpers_detects_overlapping_membership_groups() {
    assert!(assert_disjoint("split", &[("train", &["p1", "p2"]), ("test", &["p3"])]).is_ok());
    let err = assert_disjoint("split", &[("train", &["p1", "p2"]), ("test", &["p2"])]).unwrap_err();
    assert!(err.contains("p2"), "{err}");
}
