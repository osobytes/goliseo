//! Port of `sim/research_schema.lua`.
//!
//! Declarative shapes, strict validation, canonical serialization, and
//! content hashing for the versioned human-playtest research contracts.
//!
//! Research payloads arrive from tooling, operators, and participants, so
//! every public reader here is an external-input validator: it returns
//! [`Result::Err`] and never guesses. Programmer invariants (a malformed
//! *shape*, not malformed data) still `assert!`, because an unusable shape
//! is a code bug (AGENTS.md §7).
//!
//! Serialization follows the length-prefixed `lp(value)` discipline recorded
//! in `docs/design/combat_fun_evidence_contract.md` section 4.0: every
//! scalar emits its byte length before its bytes, records emit declared
//! field order, and maps emit sorted keys. Delimiter-joined or table-order
//! hashing is forbidden because it is ambiguous.
//!
//! This is the sibling of `gc_netcode::diagnostics_schema` — same
//! `fnv1a64/v1` digest family, same "shape errors panic, value errors
//! return `Err`" split, same dynamically-typed [`Value`] (`Text` holds
//! `Vec<u8>` rather than `String` so a byte that is not valid UTF-8 can
//! still round-trip through validation instead of being rejected before it
//! ever reaches a validator). The two wire formats are *not* the same byte
//! format, though: this module's tag characters (`n`/`b0`/`b1`/`i`/`d`/`e`/
//! `s`/`a`/`m`/`r`/`k`) and its `GCRS` header follow `sim/research_schema.lua`
//! exactly, not `diagnostics_schema`'s `GCND` format.
//!
//! Numbers route through [`crate::match_snapshot::number_bytes`] for the
//! `number` field kind, exactly as the Lua original calls
//! `match_snapshot.number_bytes` — never a reimplementation, and never a
//! `%.17g`-style text format, which was the source of a real cross-language
//! digest defect earlier in this port.
//!
//! ## Cross-language digest agreement
//!
//! Per `v2/README.md` §2.2, this module's canonical encoding and digest are
//! pinned by a shared vector file generated from the real Lua implementation:
//! `v2/tools/lua_reference/research_schema_vectors.txt`, asserted by
//! `tests::shared_vectors_agree_with_lua` below.

use crate::match_snapshot;
use gc_core::fnv1a64;

// ---------------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------------

/// Which kind of value a [`ResearchField`] holds. Mirrors the Lua
/// `ResearchFieldKind` alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchFieldKind {
    /// `true`/`false`.
    Boolean,
    /// A finite whole number, `abs(n) <= MAX_SAFE_INTEGER`.
    Integer,
    /// Any finite number.
    Number,
    /// An opaque bounded machine string (hashes excluded from digests — see
    /// the Lua original's `"string"` doc comment).
    Str,
    /// A join key: bounded charset, no direct identifiers.
    Id,
    /// Bounded human free text, never a join key.
    Text,
    /// A lowercase `fnv1a64` hex digest produced by this module.
    Hash,
    /// A string drawn from a fixed, declared member set.
    Enum,
    /// An ordered, length-bounded list of one element shape.
    Array,
    /// An unordered `id -> element shape` map, sorted at encode time.
    Map,
    /// A fixed set of named, individually (optional-or-not) typed fields.
    Record,
}

/// One field in a research shape tree. Build with [`ResearchField::new`]
/// plus the builder methods, mirroring the Lua original's table literals.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchField {
    /// The field's name. Meaningful for a shape's own root record and for a
    /// record's immediate children (used in error paths and as the wire
    /// key); unused (but always present) on array/map element shapes.
    pub name: String,
    /// This field's value kind.
    pub kind: ResearchFieldKind,
    /// Whether a `record` may omit this field entirely.
    pub optional: bool,
    /// Declared members, for `Enum` fields.
    pub values: Option<Vec<String>>,
    /// Element shape, for `Array`/`Map` fields.
    pub element: Option<Box<ResearchField>>,
    /// Field list, for `Record` fields, in canonical (encoding) order.
    pub fields: Option<Vec<ResearchField>>,
    /// Inclusive lower bound, for `Integer`/`Number` fields.
    pub min: Option<f64>,
    /// Inclusive upper bound, for `Integer`/`Number` fields.
    pub max: Option<f64>,
    /// Byte/entry-count lower bound, for `Id`/`Str`/`Text`/`Hash`/`Array`/`Map` fields.
    pub min_length: Option<usize>,
    /// Byte/entry-count upper bound, for `Id`/`Str`/`Text`/`Hash`/`Array` fields.
    /// `Map` has no upper bound in the Lua original — see `validate_value`'s
    /// `"map"` branch, which checks `min_length` only.
    pub max_length: Option<usize>,
}

impl ResearchField {
    /// A field of `kind`, otherwise unnamed, required, and unbounded.
    #[must_use]
    pub fn new(kind: ResearchFieldKind) -> Self {
        ResearchField {
            name: String::new(),
            kind,
            optional: false,
            values: None,
            element: None,
            fields: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
        }
    }

    /// Set this field's name.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Mark this field optional (a `record` may omit it).
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Set this `Enum` field's declared members.
    #[must_use]
    pub fn values(mut self, values: Vec<String>) -> Self {
        self.values = Some(values);
        self
    }

    /// Set this `Array`/`Map` field's element shape.
    #[must_use]
    pub fn element(mut self, element: ResearchField) -> Self {
        self.element = Some(Box::new(element));
        self
    }

    /// Set this `Record` field's field list.
    #[must_use]
    pub fn fields(mut self, fields: Vec<ResearchField>) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Set the inclusive numeric lower bound of an `Integer`/`Number` field.
    #[must_use]
    pub fn min(mut self, min: f64) -> Self {
        self.min = Some(min);
        self
    }

    /// Set the inclusive numeric upper bound of an `Integer`/`Number` field.
    #[must_use]
    pub fn max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }

    /// Set the byte/entry-count lower bound of an
    /// `Id`/`Str`/`Text`/`Hash`/`Array`/`Map` field.
    #[must_use]
    pub fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    /// Set the byte/entry-count upper bound of an
    /// `Id`/`Str`/`Text`/`Hash`/`Array` field.
    #[must_use]
    pub fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }
}

/// A [`ResearchField`] of kind `Record`: the root of a shape tree.
pub type ResearchShape = ResearchField;

/// A dynamically-typed value validated, encoded, and decoded against a
/// [`ResearchField`] shape. Lua strings are byte arrays, so text-carrying
/// variants hold `Vec<u8>` rather than `String`: a research payload arriving
/// from tooling, an operator, or a participant must be able to carry an
/// invalid-UTF-8 byte through to a graceful validation error rather than
/// being rejected (or panicking) before validation even runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A `Boolean` field's value.
    Bool(bool),
    /// An `Integer` or `Number` field's value. Every Lua number is an `f64`
    /// (AGENTS.md §5 rule 1); which of the two kinds is intended is a
    /// property of the [`ResearchField`], not the [`Value`].
    Number(f64),
    /// An `Id`/`Str`/`Text`/`Hash`/`Enum` field's value.
    Text(Vec<u8>),
    /// An `Array` field's value.
    Array(Vec<Value>),
    /// A `Map` field's value: unordered `(id, value)` entries, sorted by key
    /// at encode time.
    Map(Vec<(String, Value)>),
    /// A `Record` field's value: present fields only. A field absent from
    /// this list is "not provided" (must be `optional` on the shape).
    Record(Vec<(String, Value)>),
}

impl Value {
    /// Build a [`Value::Text`] from anything byte-convertible.
    #[must_use]
    pub fn bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Value::Text(bytes.into())
    }

    /// Build a [`Value::Text`] from a UTF-8 string, for the overwhelmingly
    /// common case where the text is not attacker-controlled bytes.
    #[must_use]
    pub fn str(text: impl Into<String>) -> Self {
        Value::Text(text.into().into_bytes())
    }

    /// Look up a named entry in a `Record`/`Map`-shaped entry list.
    #[must_use]
    pub fn record_get<'a>(entries: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
        entries.iter().find(|(key, _)| key == name).map(|(_, v)| v)
    }

    /// This value as `f64`, if it is a [`Value::Number`].
    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// This value's bytes, if it is a [`Value::Text`].
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Text(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// This value as a UTF-8 `&str`, if it is a [`Value::Text`] holding
    /// valid UTF-8. Most research join keys and enum members are ASCII by
    /// construction, so this is the common accessor; callers on the
    /// arbitrary-bytes path should use [`Value::as_bytes`] instead.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.as_bytes().and_then(|b| std::str::from_utf8(b).ok())
    }

    /// This value as `bool`, if it is a [`Value::Bool`].
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// This value's elements, if it is a [`Value::Array`].
    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    /// This value's entries, if it is a [`Value::Map`].
    #[must_use]
    pub fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Map(entries) => Some(entries),
            _ => None,
        }
    }

    /// This value's entries, if it is a [`Value::Record`].
    #[must_use]
    pub fn as_record(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Record(entries) => Some(entries),
            _ => None,
        }
    }
}

/// Bumping this changes every canonical preimage in the research contracts
/// and therefore every content hash. It is a coordinated breaking change;
/// see `docs/design/player_evidence_contracts.md` for the bump rules.
pub const SERIALIZATION_VERSION: i64 = 1;
/// Named so a future SHA-256 tooling digest can be introduced as a new value
/// instead of silently reinterpreting stored hashes.
pub const DIGEST: &str = "fnv1a64/v1";
/// Exact byte length of a canonical digest's hex encoding.
pub const HASH_LENGTH: usize = 16;
/// Default byte-length bound for `Str` fields.
pub const MAX_STRING_LENGTH: usize = 512;
/// Default byte-length bound for `Text` fields.
pub const MAX_TEXT_LENGTH: usize = 4096;
/// Default byte-length bound for `Id` fields.
pub const MAX_ID_LENGTH: usize = 96;
/// Default entry-count bound for `Array` fields.
pub const MAX_ARRAY_LENGTH: usize = 100_000;
/// The largest integer magnitude an `Integer`/`Number` field may exactly
/// represent (`2^53`).
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_992.0;

const HEADER: &[u8] = b"GCRS";

fn is_finite(value: f64) -> bool {
    value.is_finite()
}

fn is_integer(value: f64) -> bool {
    is_finite(value) && value == value.floor() && value.abs() <= MAX_SAFE_INTEGER
}

fn is_id_start_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

fn is_id_byte(byte: u8) -> bool {
    is_id_start_byte(byte) || matches!(byte, b'_' | b'-' | b'.')
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn is_control_byte(byte: u8) -> bool {
    byte < 0x20 || byte == 0x7f
}

fn assert_field_shape(field: &ResearchField, path: &str) {
    match field.kind {
        ResearchFieldKind::Enum => {
            let values = field
                .values
                .as_ref()
                .unwrap_or_else(|| panic!("{path} enum shape needs values"));
            assert!(
                !values.is_empty(),
                "{path} enum shape needs at least one member"
            );
        }
        ResearchFieldKind::Array | ResearchFieldKind::Map => {
            let element = field
                .element
                .as_deref()
                .unwrap_or_else(|| panic!("{path} needs an element shape"));
            assert_field_shape(element, &format!("{path}[]"));
        }
        ResearchFieldKind::Record => {
            let fields = field
                .fields
                .as_ref()
                .unwrap_or_else(|| panic!("{path} record shape needs fields"));
            let mut seen: Vec<&str> = Vec::with_capacity(fields.len());
            for child in fields {
                assert!(!child.name.is_empty(), "{path} field needs a name");
                assert!(
                    !seen.contains(&child.name.as_str()),
                    "{path} field {} is declared twice",
                    child.name
                );
                seen.push(&child.name);
                assert_field_shape(child, &format!("{path}.{}", child.name));
            }
        }
        _ => {}
    }
}

/// Build a validated record shape. A broken shape is a programmer error, so
/// this panics rather than returning a diagnostic.
///
/// # Panics
///
/// Panics if `name` is empty, or if the shape violates a structural
/// invariant (missing element/fields, duplicate field name, empty enum, ...).
#[must_use]
pub fn record(name: impl Into<String>, fields: Vec<ResearchField>) -> ResearchShape {
    let name = name.into();
    assert!(!name.is_empty(), "research shape needs a name");
    let shape = ResearchField {
        name: name.clone(),
        kind: ResearchFieldKind::Record,
        optional: false,
        values: None,
        element: None,
        fields: Some(fields),
        min: None,
        max: None,
        min_length: None,
        max_length: None,
    };
    assert_field_shape(&shape, &name);
    shape
}

/// Declare a closed enum member set.
///
/// # Panics
///
/// Panics on an empty member, or a duplicate member.
#[must_use]
pub fn enum_values(members: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = Vec::with_capacity(members.len());
    for &member in members {
        assert!(!member.is_empty(), "enum member must be a non-empty string");
        assert!(
            !values.iter().any(|v| v == member),
            "enum member {member} is declared twice"
        );
        values.push(member.to_string());
    }
    values
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Result alias for fallible research-schema value operations (AGENTS.md
/// §7: expected, recoverable failures from external input).
pub type Result<T> = std::result::Result<T, String>;

fn validate_string(field: &ResearchField, bytes: &[u8], path: &str) -> Result<()> {
    let max = field.max_length.unwrap_or(match field.kind {
        ResearchFieldKind::Text => MAX_TEXT_LENGTH,
        ResearchFieldKind::Id => MAX_ID_LENGTH,
        _ => MAX_STRING_LENGTH,
    });
    let min = field
        .min_length
        .unwrap_or(if field.kind == ResearchFieldKind::Text {
            0
        } else {
            1
        });
    if bytes.len() < min {
        return Err(format!("{path} must be at least {min} bytes"));
    }
    if bytes.len() > max {
        return Err(format!("{path} must be at most {max} bytes"));
    }
    if field.kind == ResearchFieldKind::Hash {
        let canonical = bytes.len() == HASH_LENGTH
            && bytes
                .iter()
                .all(|&b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !canonical {
            return Err(format!("{path} must be a {DIGEST} hex digest"));
        }
        return Ok(());
    }
    if field.kind == ResearchFieldKind::Id {
        let canonical =
            !bytes.is_empty() && is_id_start_byte(bytes[0]) && bytes.iter().all(|&b| is_id_byte(b));
        if !canonical {
            return Err(format!("{path} must be a lowercase bounded id"));
        }
        if contains_subslice(bytes, b"@") || contains_subslice(bytes, b"..") {
            return Err(format!(
                "{path} must not contain a direct identifier or raw path"
            ));
        }
        return Ok(());
    }
    if bytes.iter().any(|&b| is_control_byte(b)) {
        return Err(format!("{path} must not contain control characters"));
    }
    Ok(())
}

fn validate_numeric(field: &ResearchField, value: f64, path: &str) -> Result<()> {
    if field.kind == ResearchFieldKind::Integer {
        if !is_integer(value) {
            return Err(format!("{path} must be a finite integer"));
        }
    } else if !is_finite(value) {
        return Err(format!("{path} must be a finite number"));
    }
    if let Some(min) = field.min
        && value < min
    {
        return Err(format!("{path} must be at least {min}"));
    }
    if let Some(max) = field.max
        && value > max
    {
        return Err(format!("{path} must be at most {max}"));
    }
    Ok(())
}

fn validate_value(field: &ResearchField, value: &Value, path: &str) -> Result<()> {
    match field.kind {
        ResearchFieldKind::Boolean => match value {
            Value::Bool(_) => Ok(()),
            _ => Err(format!("{path} must be a boolean")),
        },
        ResearchFieldKind::Integer | ResearchFieldKind::Number => match value {
            Value::Number(n) => validate_numeric(field, *n, path),
            _ => Err(format!(
                "{path} must be a finite {}",
                if field.kind == ResearchFieldKind::Integer {
                    "integer"
                } else {
                    "number"
                }
            )),
        },
        ResearchFieldKind::Str
        | ResearchFieldKind::Id
        | ResearchFieldKind::Text
        | ResearchFieldKind::Hash => match value {
            Value::Text(bytes) => validate_string(field, bytes, path),
            _ => Err(format!("{path} must be a string")),
        },
        ResearchFieldKind::Enum => {
            let Value::Text(bytes) = value else {
                return Err(format!("{path} must be an enum string"));
            };
            let members = field.values.as_ref().expect("enum shape always has values");
            if !members.iter().any(|m| m.as_bytes() == bytes.as_slice()) {
                return Err(format!("{path} must be one of {}", members.join("|")));
            }
            Ok(())
        }
        ResearchFieldKind::Array => {
            let Value::Array(items) = value else {
                return Err(format!("{path} must be an array"));
            };
            if let Some(min_length) = field.min_length
                && items.len() < min_length
            {
                return Err(format!("{path} needs at least {min_length} entries"));
            }
            let max = field.max_length.unwrap_or(MAX_ARRAY_LENGTH);
            if items.len() > max {
                return Err(format!("{path} allows at most {max} entries"));
            }
            let element = field
                .element
                .as_deref()
                .expect("array shape always has an element");
            for (index, item) in items.iter().enumerate() {
                validate_value(element, item, &format!("{path}.{}", index + 1))?;
            }
            Ok(())
        }
        ResearchFieldKind::Map => {
            let Value::Map(entries) = value else {
                return Err(format!("{path} must be a map"));
            };
            let element = field
                .element
                .as_deref()
                .expect("map shape always has an element");
            let key_field = ResearchField::new(ResearchFieldKind::Id).named("key");
            for (key, child) in entries {
                validate_string(&key_field, key.as_bytes(), &format!("{path} key"))?;
                validate_value(element, child, &format!("{path}.{key}"))?;
            }
            if let Some(min_length) = field.min_length
                && entries.len() < min_length
            {
                return Err(format!("{path} needs at least {min_length} entries"));
            }
            Ok(())
        }
        ResearchFieldKind::Record => {
            let Value::Record(entries) = value else {
                return Err(format!("{path} must be a table"));
            };
            let declared = field
                .fields
                .as_ref()
                .expect("record shape always has fields");
            // Strict readers reject unknown fields instead of ignoring them:
            // an unknown field means the writer knows something this reader
            // does not.
            for (key, _) in entries {
                if !declared.iter().any(|f| f.name == *key) {
                    return Err(format!("{path} has unknown field {key}"));
                }
            }
            for child in declared {
                let child_path = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}.{}", child.name)
                };
                match Value::record_get(entries, &child.name) {
                    None => {
                        if !child.optional {
                            return Err(format!("{child_path} is required"));
                        }
                    }
                    Some(child_value) => validate_value(child, child_value, &child_path)?,
                }
            }
            Ok(())
        }
    }
}

/// Validate `value` against `shape`.
pub fn validate(shape: &ResearchShape, value: &Value) -> Result<()> {
    assert!(
        shape.kind == ResearchFieldKind::Record,
        "research shape is required"
    );
    validate_value(shape, value, &shape.name)
}

// ---------------------------------------------------------------------------
// Canonical serialization
// ---------------------------------------------------------------------------

fn emit_lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(bytes);
    out.push(b';');
}

fn encode_value(out: &mut Vec<u8>, field: &ResearchField, value: Option<&Value>) {
    let Some(value) = value else {
        out.extend_from_slice(b"n;");
        return;
    };
    match field.kind {
        ResearchFieldKind::Boolean => {
            let Value::Bool(flag) = value else {
                panic!("encode_value called with a value that does not match its shape's kind");
            };
            out.extend_from_slice(if *flag { b"b1;" } else { b"b0;" });
        }
        ResearchFieldKind::Integer => {
            let n = value.as_number().expect("validated integer value");
            emit_lp_tagged(out, b'i', (n as i64).to_string().as_bytes());
        }
        ResearchFieldKind::Number => {
            let n = value.as_number().expect("validated number value");
            let payload = match_snapshot::number_bytes(n);
            emit_lp_tagged(out, b'd', payload.as_bytes());
        }
        ResearchFieldKind::Enum => {
            let bytes = value.as_bytes().expect("validated enum value");
            emit_lp_tagged(out, b'e', bytes);
        }
        ResearchFieldKind::Str
        | ResearchFieldKind::Id
        | ResearchFieldKind::Text
        | ResearchFieldKind::Hash => {
            let bytes = value.as_bytes().expect("validated string value");
            emit_lp_tagged(out, b's', bytes);
        }
        ResearchFieldKind::Array => {
            let items = value.as_array().expect("validated array value");
            out.push(b'a');
            emit_lp(out, items.len().to_string().as_bytes());
            let element = field
                .element
                .as_deref()
                .expect("array shape always has an element");
            for item in items {
                encode_value(out, element, Some(item));
            }
        }
        ResearchFieldKind::Map => {
            let entries = value.as_map().expect("validated map value");
            let mut sorted: Vec<&(String, Value)> = entries.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            out.push(b'm');
            emit_lp(out, sorted.len().to_string().as_bytes());
            let element = field
                .element
                .as_deref()
                .expect("map shape always has an element");
            for (key, child) in sorted {
                out.push(b'k');
                emit_lp(out, key.as_bytes());
                encode_value(out, element, Some(child));
            }
        }
        ResearchFieldKind::Record => {
            let entries = value.as_record().expect("validated record value");
            let declared = field
                .fields
                .as_ref()
                .expect("record shape always has fields");
            out.push(b'r');
            emit_lp(out, declared.len().to_string().as_bytes());
            for child in declared {
                out.push(b'k');
                emit_lp(out, child.name.as_bytes());
                encode_value(out, child, Value::record_get(entries, &child.name));
            }
        }
    }
}

fn emit_lp_tagged(out: &mut Vec<u8>, tag: u8, payload: &[u8]) {
    out.push(tag);
    emit_lp(out, payload);
}

/// Canonical bytes for a payload that already satisfies `shape`.
pub fn encode(shape: &ResearchShape, value: &Value) -> Result<Vec<u8>> {
    validate(shape, value)?;
    let mut out = Vec::new();
    out.extend_from_slice(HEADER);
    out.extend_from_slice(SERIALIZATION_VERSION.to_string().as_bytes());
    out.push(b';');
    emit_lp(&mut out, shape.name.as_bytes());
    encode_value(&mut out, shape, Some(value));
    Ok(out)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn read_lp(&mut self, path: &str) -> Result<&'a [u8]> {
        let rest = &self.bytes[self.at..];
        let Some(colon) = rest.iter().position(|&b| b == b':') else {
            return Err(format!("{path} is missing a length prefix"));
        };
        let digits = &rest[..colon];
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err(format!("{path} has an invalid length prefix"));
        }
        let text = std::str::from_utf8(digits).expect("ascii digits are valid utf-8");
        let size: usize = text
            .parse()
            .map_err(|_| format!("{path} has an invalid length prefix"))?;
        let first = self.at + colon + 1;
        let last = first + size;
        if last >= self.bytes.len() || self.bytes[last] != b';' {
            return Err(format!("{path} is truncated"));
        }
        self.at = last + 1;
        Ok(&self.bytes[first..last])
    }
}

/// Inverse of `match_snapshot::number_bytes`'s `frexp`-based encoding:
/// `mantissa * 2^exponent`, computed via IEEE 754 bit manipulation rather
/// than a chain of floating-point multiplications, whose intermediate steps
/// are not guaranteed exact at extreme exponents (denormal reconstruction,
/// in particular). `mantissa` here is always the normal, positive value
/// [`decode_number_bytes`] reconstructs from its two integer limbs (always
/// in `[0.5, 1.0]`), so only a normal input needs handling; the *result* may
/// legitimately land in the subnormal range or overflow, both handled
/// explicitly.
fn ldexp(mantissa: f64, exponent: i64) -> f64 {
    debug_assert!(mantissa.is_finite() && mantissa > 0.0);
    let bits = mantissa.to_bits();
    #[allow(clippy::cast_possible_wrap)]
    let raw_exponent = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    let unbiased = raw_exponent - 1023;
    let new_biased = unbiased + exponent + 1023;
    if new_biased >= 0x7ff {
        return f64::INFINITY;
    }
    if new_biased <= 0 {
        let shift = 1 - new_biased;
        if shift > 52 {
            return 0.0;
        }
        let full = 0x0010_0000_0000_0000u64 | fraction;
        #[allow(clippy::cast_sign_loss)]
        return f64::from_bits(full >> shift);
    }
    #[allow(clippy::cast_sign_loss)]
    f64::from_bits(((new_biased as u64) << 52) | fraction)
}

fn decode_number_bytes(payload: &[u8]) -> Option<f64> {
    if payload == b"z" {
        return Some(0.0);
    }
    if payload == b"Z" {
        return Some(-0.0);
    }
    let text = std::str::from_utf8(payload).ok()?;
    let mut parts = text.splitn(4, ':');
    let sign = parts.next()?;
    let exponent = parts.next()?;
    let high = parts.next()?;
    let low = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if sign != "p" && sign != "m" {
        return None;
    }
    let is_valid_int = |s: &str| {
        let s = s.strip_prefix('-').unwrap_or(s);
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
    };
    let is_valid_uint = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !is_valid_int(exponent) || !is_valid_uint(high) || !is_valid_uint(low) {
        return None;
    }
    let exponent: i64 = exponent.parse().ok()?;
    let high: f64 = high.parse().ok()?;
    let low: f64 = low.parse().ok()?;
    let mantissa = (high + low / 134_217_728.0) / 67_108_864.0;
    let value = ldexp(mantissa, exponent);
    Some(if sign == "m" { -value } else { value })
}

fn decode_value(cursor: &mut Cursor, field: &ResearchField, path: &str) -> Result<Option<Value>> {
    if cursor.at >= cursor.bytes.len() {
        return Err(format!("{path} is truncated"));
    }
    let tag = cursor.bytes[cursor.at];
    if tag == b'n' {
        if cursor.at + 1 >= cursor.bytes.len() || cursor.bytes[cursor.at + 1] != b';' {
            return Err(format!("{path} has a malformed absent marker"));
        }
        cursor.at += 2;
        return Ok(None);
    }
    if field.kind == ResearchFieldKind::Boolean {
        if cursor.at + 3 > cursor.bytes.len() {
            return Err(format!("{path} has a malformed boolean"));
        }
        let wire = &cursor.bytes[cursor.at..cursor.at + 3];
        let result = if wire == b"b1;" {
            true
        } else if wire == b"b0;" {
            false
        } else {
            return Err(format!("{path} has a malformed boolean"));
        };
        cursor.at += 3;
        return Ok(Some(Value::Bool(result)));
    }
    cursor.at += 1;
    match field.kind {
        ResearchFieldKind::Integer => {
            if tag != b'i' {
                return Err(format!("{path} expected an integer"));
            }
            let payload = cursor.read_lp(path)?;
            let text = std::str::from_utf8(payload)
                .map_err(|_| format!("{path} has a non-canonical integer"))?;
            let core = text.strip_prefix('-').unwrap_or(text);
            if core.is_empty() || !core.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("{path} has a non-canonical integer"));
            }
            let n: i64 = text
                .parse()
                .map_err(|_| format!("{path} has a non-canonical integer"))?;
            Ok(Some(Value::Number(n as f64)))
        }
        ResearchFieldKind::Number => {
            if tag != b'd' {
                return Err(format!("{path} expected a number"));
            }
            let payload = cursor.read_lp(path)?;
            let number = decode_number_bytes(payload)
                .ok_or_else(|| format!("{path} has a malformed number"))?;
            Ok(Some(Value::Number(number)))
        }
        ResearchFieldKind::Enum
        | ResearchFieldKind::Str
        | ResearchFieldKind::Id
        | ResearchFieldKind::Text
        | ResearchFieldKind::Hash => {
            let expected_tag = if field.kind == ResearchFieldKind::Enum {
                b'e'
            } else {
                b's'
            };
            if tag != expected_tag {
                return Err(format!("{path} expected a string"));
            }
            let payload = cursor.read_lp(path)?;
            Ok(Some(Value::Text(payload.to_vec())))
        }
        ResearchFieldKind::Array => {
            if tag != b'a' {
                return Err(format!("{path} expected an array"));
            }
            let payload = cursor.read_lp(path)?;
            let count = parse_count(payload)
                .ok_or_else(|| format!("{path} has an invalid array length"))?;
            let element = field
                .element
                .as_deref()
                .expect("array shape always has an element");
            let mut result = Vec::with_capacity(count);
            for index in 1..=count {
                let child_path = format!("{path}.{index}");
                match decode_value(cursor, element, &child_path)? {
                    Some(child) => result.push(child),
                    None => return Err(format!("{child_path} is missing")),
                }
            }
            Ok(Some(Value::Array(result)))
        }
        ResearchFieldKind::Map => {
            if tag != b'm' {
                return Err(format!("{path} expected a map"));
            }
            let payload = cursor.read_lp(path)?;
            let count =
                parse_count(payload).ok_or_else(|| format!("{path} has an invalid map length"))?;
            let element = field
                .element
                .as_deref()
                .expect("map shape always has an element");
            let mut result: Vec<(String, Value)> = Vec::with_capacity(count);
            let mut previous: Option<String> = None;
            for _ in 0..count {
                if cursor.at >= cursor.bytes.len() || cursor.bytes[cursor.at] != b'k' {
                    return Err(format!("{path} map key is malformed"));
                }
                cursor.at += 1;
                let key_bytes = cursor.read_lp(&format!("{path} key"))?;
                let key = std::str::from_utf8(key_bytes)
                    .map_err(|_| format!("{path} key is not valid utf-8"))?
                    .to_string();
                if let Some(prev) = &previous
                    && key.as_str() <= prev.as_str()
                {
                    return Err(format!("{path} map keys are not canonically sorted"));
                }
                previous = Some(key.clone());
                let child_path = format!("{path}.{key}");
                match decode_value(cursor, element, &child_path)? {
                    Some(child) => result.push((key, child)),
                    None => return Err(format!("{child_path} is missing")),
                }
            }
            Ok(Some(Value::Map(result)))
        }
        ResearchFieldKind::Record => {
            if tag != b'r' {
                return Err(format!("{path} expected a record"));
            }
            let payload = cursor.read_lp(path)?;
            let declared = field
                .fields
                .as_ref()
                .expect("record shape always has fields");
            let count =
                parse_count(payload).ok_or_else(|| format!("{path} has an invalid field count"))?;
            if count != declared.len() {
                return Err(format!(
                    "{path} declares a different field count than this reader knows"
                ));
            }
            let mut result: Vec<(String, Value)> = Vec::with_capacity(declared.len());
            for child in declared {
                if cursor.at >= cursor.bytes.len() || cursor.bytes[cursor.at] != b'k' {
                    return Err(format!("{path} field key is malformed"));
                }
                cursor.at += 1;
                let name_bytes = cursor.read_lp(&format!("{path} field name"))?;
                let name = std::str::from_utf8(name_bytes)
                    .map_err(|_| format!("{path} field name is not valid utf-8"))?;
                if name != child.name {
                    return Err(format!(
                        "{path} expected field {} but found {name}",
                        child.name
                    ));
                }
                let child_path = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}.{}", child.name)
                };
                if let Some(value) = decode_value(cursor, child, &child_path)? {
                    result.push((child.name.clone(), value));
                }
            }
            Ok(Some(Value::Record(result)))
        }
        ResearchFieldKind::Boolean => unreachable!("handled above"),
    }
}

fn parse_count(payload: &[u8]) -> Option<usize> {
    if payload.is_empty() || !payload.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(payload).ok()?.parse().ok()
}

/// Parse canonical bytes back into a payload and re-validate it.
/// Round-tripping is a contract requirement: `encode(decode(bytes))` must
/// reproduce `bytes`.
pub fn decode(shape: &ResearchShape, bytes: &[u8]) -> Result<Value> {
    assert!(
        shape.kind == ResearchFieldKind::Record,
        "research shape is required"
    );
    let mut prefix = Vec::with_capacity(HEADER.len() + 8);
    prefix.extend_from_slice(HEADER);
    prefix.extend_from_slice(SERIALIZATION_VERSION.to_string().as_bytes());
    prefix.push(b';');
    if !bytes.starts_with(&prefix) {
        if bytes.starts_with(HEADER) {
            let rest = &bytes[HEADER.len()..];
            let digits_len = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits_len > 0 && rest.get(digits_len) == Some(&b';') {
                let found = std::str::from_utf8(&rest[..digits_len]).unwrap_or("?");
                return Err(format!(
                    "{} was written by serialization version {found} and no migration to version {} is registered",
                    shape.name, SERIALIZATION_VERSION
                ));
            }
        }
        return Err(format!(
            "{} wire payload is not a research contract",
            shape.name
        ));
    }
    let mut cursor = Cursor {
        bytes,
        at: prefix.len(),
    };
    let name_bytes = cursor.read_lp(&format!("{} shape name", shape.name))?;
    let name = std::str::from_utf8(name_bytes)
        .map_err(|_| format!("{} shape name is not valid utf-8", shape.name))?;
    if name != shape.name {
        return Err(format!("{} wire payload declares shape {name}", shape.name));
    }
    let value = decode_value(&mut cursor, shape, &shape.name)?
        .ok_or_else(|| format!("{} wire payload is empty", shape.name))?;
    if cursor.at < bytes.len() {
        return Err(format!("{} wire payload has trailing bytes", shape.name));
    }
    validate(shape, &value)?;
    Ok(value)
}

/// Content hash of a payload that already satisfies `shape`.
pub fn content_hash(shape: &ResearchShape, value: &Value) -> Result<String> {
    let bytes = encode(shape, value)?;
    Ok(fnv1a64::hash(&bytes))
}

/// One part of an ordered tuple hashed by [`tuple_hash`].
#[derive(Clone, Debug, PartialEq)]
pub enum TuplePart {
    /// A numeric part.
    Number(f64),
    /// A string part.
    Text(String),
}

/// Hash an ordered tuple of already-canonical scalars. Used for compound ids
/// (`run_id`-style tuples) where there is no record shape to lean on.
///
/// # Panics
///
/// Panics if `label` is empty, or a numeric part is non-finite: `parts` here
/// are always already-known-valid internal values (ids, ticks, counts), not
/// raw external input, so a broken call is a programmer error (AGENTS.md
/// §7) rather than something to report as `Err`.
#[must_use]
pub fn tuple_hash(label: &str, parts: &[TuplePart]) -> String {
    assert!(!label.is_empty(), "tuple hash needs a label");
    let mut buffer = Vec::new();
    buffer.extend_from_slice(HEADER);
    buffer.extend_from_slice(SERIALIZATION_VERSION.to_string().as_bytes());
    buffer.push(b';');
    emit_lp(&mut buffer, label.as_bytes());
    for part in parts {
        match part {
            TuplePart::Number(n) => {
                assert!(is_finite(*n), "tuple hash parts must be finite");
                if is_integer(*n) {
                    emit_lp_tagged(&mut buffer, b'i', (*n as i64).to_string().as_bytes());
                } else {
                    emit_lp_tagged(
                        &mut buffer,
                        b'd',
                        match_snapshot::number_bytes(*n).as_bytes(),
                    );
                }
            }
            TuplePart::Text(s) => emit_lp_tagged(&mut buffer, b's', s.as_bytes()),
        }
    }
    fnv1a64::hash(&buffer)
}

/// Version gate for a stored payload. Unsupported versions stop with a
/// diagnostic that names the missing migration instead of reinterpreting
/// fields.
pub fn accepts_version(
    label: &str,
    supported: &[i64],
    current: i64,
    version: Option<&Value>,
) -> Result<()> {
    let Some(n) = version.and_then(Value::as_number) else {
        return Err(format!("{label} schema_version must be an integer"));
    };
    if !is_integer(n) {
        return Err(format!("{label} schema_version must be an integer"));
    }
    let version_i64 = n as i64;
    if !supported.contains(&version_i64) {
        return Err(format!(
            "{label} schema_version {version_i64} is unsupported by reader version {current} and no migration is registered"
        ));
    }
    Ok(())
}

/// Reject a payload whose declared field sets overlap. Split manifests and
/// the simulation/research field partition both need this fail-closed
/// check. `groups` is `(group name, members)`, checked in the given order.
pub fn assert_disjoint(label: &str, groups: &[(&str, &[&str])]) -> Result<()> {
    let mut owner: Vec<(&str, &str)> = Vec::new();
    for &(name, members) in groups {
        for &member in members {
            if let Some((_, previous)) = owner.iter().find(|(m, _)| *m == member) {
                return Err(format!(
                    "{label} member {member} appears in both {previous} and {name}"
                ));
            }
            owner.push((member, name));
        }
    }
    Ok(())
}
