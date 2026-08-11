//! Cross-language differential test for `sim/research_schema.lua`'s
//! canonical serializer and fnv1a64 content digest (ARCHITECTURE.md §3
//! rule 7, `tools/lua_reference/README.md`).
//!
//! `tools/lua_reference/research_schema_vectors.txt` is the captured
//! canonical wire encoding and fnv1a64 digest of `sim.research_schema.encode`
//! / `.content_hash` / `.tuple_hash`, run under headless `love` (no display,
//! no `xvfb`) against a hand-built shape and payloads chosen to cover the
//! module's own stated risk surface: an ordinary record, a nested map whose
//! keys arrive out of sorted order (proving the encoder sorts rather than
//! trusting Lua's unspecified `pairs()` order), empty values, negative and
//! fractional numbers at extreme exponents, and a `string`-kind field
//! carrying a raw non-UTF-8 byte and an invalid UTF-8 continuation sequence.
//! See the vectors file's own header comment for the exact cases and shape.

use gc_sim::research_schema::{
    ResearchField, ResearchFieldKind, ResearchShape, TuplePart, Value, content_hash, decode,
    encode, enum_values, record, tuple_hash,
};
use std::path::Path;

fn test_shape() -> ResearchShape {
    record(
        "research_vector_record/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Id).named("id"),
            ResearchField::new(ResearchFieldKind::Integer).named("count"),
            ResearchField::new(ResearchFieldKind::Boolean).named("flag"),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("tag")
                .values(enum_values(&["alpha", "beta"])),
            ResearchField::new(ResearchFieldKind::Hash).named("digest"),
            ResearchField::new(ResearchFieldKind::Text)
                .named("note")
                .optional(),
            ResearchField::new(ResearchFieldKind::Str).named("payload"),
            ResearchField::new(ResearchFieldKind::Array)
                .named("tags")
                .element(ResearchField::new(ResearchFieldKind::Integer).named("item")),
            ResearchField::new(ResearchFieldKind::Map)
                .named("scores")
                .element(ResearchField::new(ResearchFieldKind::Number).named("score")),
        ],
    )
}

fn record_value(entries: Vec<(&str, Value)>) -> Value {
    Value::Record(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn ordinary_record_value() -> Value {
    record_value(vec![
        ("id", Value::str("sample.alpha-01")),
        ("count", Value::Number(7.0)),
        ("flag", Value::Bool(true)),
        ("tag", Value::str("alpha")),
        ("digest", Value::str("0123456789abcdef")),
        ("note", Value::str("hello world")),
        ("payload", Value::str("raw payload")),
        (
            "tags",
            Value::Array(vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::Number(3.0),
            ]),
        ),
        (
            "scores",
            Value::Map(vec![
                ("zebra".to_string(), Value::Number(1.0)),
                ("apple".to_string(), Value::Number(2.0)),
                ("mango".to_string(), Value::Number(3.0)),
            ]),
        ),
    ])
}

fn empty_values_value() -> Value {
    record_value(vec![
        ("id", Value::str("sample.beta")),
        ("count", Value::Number(0.0)),
        ("flag", Value::Bool(false)),
        ("tag", Value::str("beta")),
        ("digest", Value::str("fedcba9876543210")),
        ("payload", Value::str("x")),
        ("tags", Value::Array(vec![])),
        ("scores", Value::Map(vec![])),
    ])
}

fn negative_and_fractional_numbers_value() -> Value {
    record_value(vec![
        ("id", Value::str("sample.gamma")),
        ("count", Value::Number(-5.0)),
        ("flag", Value::Bool(true)),
        ("tag", Value::str("alpha")),
        ("digest", Value::str("1111222233334444")),
        ("payload", Value::str("delta")),
        (
            "tags",
            Value::Array(vec![
                Value::Number(-1.0),
                Value::Number(0.0),
                Value::Number(1.0),
            ]),
        ),
        (
            "scores",
            Value::Map(vec![
                ("alpha".to_string(), Value::Number(-0.5)),
                ("beta".to_string(), Value::Number(1.0 / 3.0)),
                ("gamma".to_string(), Value::Number(1e17)),
                ("delta".to_string(), Value::Number(-1e-17)),
                ("epsilon".to_string(), Value::Number(0.0)),
                ("zeta".to_string(), Value::Number(6.02e23)),
            ]),
        ),
    ])
}

fn non_utf8_byte_value() -> Value {
    let mut payload = b"abc".to_vec();
    payload.push(0xff);
    payload.extend_from_slice(b"def");
    payload.push(0xc3);
    payload.push(0x28);
    record_value(vec![
        ("id", Value::str("sample.delta")),
        ("count", Value::Number(42.0)),
        ("flag", Value::Bool(false)),
        ("tag", Value::str("beta")),
        ("digest", Value::str("abcdefabcdefabcd")),
        ("payload", Value::Text(payload)),
        ("tags", Value::Array(vec![Value::Number(255.0)])),
        (
            "scores",
            Value::Map(vec![("only".to_string(), Value::Number(0.1))]),
        ),
    ])
}

fn load_vectors() -> Vec<(String, String, String)> {
    let vectors_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tools/lua_reference/research_schema_vectors.txt");
    let contents = std::fs::read_to_string(&vectors_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", vectors_path.display()));
    let mut result = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let label = parts.next().unwrap_or_default().to_string();
        let encoded_hex = parts.next().unwrap_or_default().to_string();
        let digest = parts.next().unwrap_or_default().to_string();
        result.push((label, encoded_hex, digest));
    }
    assert!(
        !result.is_empty(),
        "no vectors were found in {}",
        vectors_path.display()
    );
    result
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() {
        return Some(Vec::new());
    }
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn find_digest<'a>(vectors: &'a [(String, String, String)], label: &str) -> &'a str {
    vectors
        .iter()
        .find(|(l, _, _)| l == label)
        .map(|(_, _, digest)| digest.as_str())
        .unwrap_or_else(|| panic!("vector {label:?} not found"))
}

#[test]
fn shared_vectors_agree_with_lua() {
    let shape = test_shape();
    let vectors = load_vectors();
    let mut checked = 0;
    for (label, encoded_hex, expected_digest) in &vectors {
        let value = match label.as_str() {
            "ordinary_record" => Some(ordinary_record_value()),
            "empty_values" => Some(empty_values_value()),
            "negative_and_fractional_numbers" => Some(negative_and_fractional_numbers_value()),
            "non_utf8_byte_in_payload" => Some(non_utf8_byte_value()),
            "tuple_hash_two_parts" | "tuple_hash_numeric" => None,
            other => panic!("unrecognized vector label {other:?}; add a case for it"),
        };
        if let Some(value) = value {
            let expected_bytes = decode_hex(encoded_hex)
                .unwrap_or_else(|| panic!("vector {label:?} has invalid encoded_hex"));
            let actual_bytes = encode(&shape, &value)
                .unwrap_or_else(|err| panic!("vector {label:?} failed to encode: {err}"));
            assert_eq!(
                &actual_bytes, &expected_bytes,
                "vector {label:?}: encoded bytes mismatch"
            );
            let actual_digest = content_hash(&shape, &value)
                .unwrap_or_else(|err| panic!("vector {label:?} failed to hash: {err}"));
            assert_eq!(
                &actual_digest, expected_digest,
                "vector {label:?}: fnv1a64 digest mismatch"
            );

            // Round-trip: decode(encode(value)) re-encodes byte-for-byte.
            let decoded = decode(&shape, &actual_bytes)
                .unwrap_or_else(|err| panic!("vector {label:?} failed to decode: {err}"));
            let re_encoded = encode(&shape, &decoded)
                .unwrap_or_else(|err| panic!("vector {label:?} failed to re-encode: {err}"));
            assert_eq!(
                re_encoded, actual_bytes,
                "vector {label:?}: round-trip mismatch"
            );

            checked += 1;
        }
    }
    assert_eq!(
        checked, 4,
        "expected exactly the four record-shaped vectors to be exercised"
    );

    let tuple_two_parts = tuple_hash(
        "research-vector/v1",
        &[
            TuplePart::Text("a".to_string()),
            TuplePart::Text("bc".to_string()),
        ],
    );
    assert_eq!(
        tuple_two_parts,
        find_digest(&vectors, "tuple_hash_two_parts"),
        "tuple_hash_two_parts mismatch"
    );
    let tuple_numeric = tuple_hash(
        "research-vector/v1",
        &[TuplePart::Number(1.0), TuplePart::Number(-2.5)],
    );
    assert_eq!(
        tuple_numeric,
        find_digest(&vectors, "tuple_hash_numeric"),
        "tuple_hash_numeric mismatch"
    );
}
