//! Port of `game/online/diagnostics_schema.lua`.
//!
//! Declarative shapes, strict validation, canonical serialization, and
//! content hashing for the OMP-3 network diagnostic contracts.
//!
//! Serialization follows a length-prefixed discipline: every scalar emits its
//! byte length before its bytes, records emit declared field order, and maps
//! emit sorted keys. Delimiter-joined or table-order hashing is ambiguous and
//! is **forbidden**.
//!
//! ## Domains: the separation this schema exists to enforce
//!
//! Every field belongs to exactly one [`Domain`], and the domain decides
//! which vocabulary the field name is allowed to use:
//!
//! | Domain      | Means                                   | Forbidden vocabulary |
//! | ----------- | ---------------------------------------- | --------------------- |
//! | `Identity`  | Build/protocol/manifest identity.        | wall clock            |
//! | `Canonical` | Simulation and tick-space evidence.      | wall clock             |
//! | `Runtime`   | Wall-clock and transport observation.    | simulation             |
//! | `Anchor`    | The one binding between the two clocks.  | neither, both needed   |
//!
//! So a reader cannot mistake an RTT sample for a tick, or a boundary hash
//! for a frame time, by reading the shape alone: the names that would allow
//! the confusion are rejected at shape-construction time. `Anchor` is the
//! sole exception and it earns it — an anchor exists only to state "input
//! tick X was observed at monotonic time Y, +/- this mapping error", so it
//! must name both and must declare its own error term.
//!
//! Shape errors panic (`assert!`): a malformed *shape* is a code bug (AGENTS.md
//! §7). Value errors return `Err` (a `Result` alias): diagnostic values arrive
//! from transports, drivers, and operators and are external input.
//!
//! ## Cross-language digest agreement
//!
//! [`DIGEST`] (`"fnv1a64/v1"`) names a versioned content digest that
//! `desync_package` (this crate) and `net_diagnostics` (TypeScript,
//! `v2/ts/packages/online/src/diagnostics_schema.ts`) both produce and must
//! agree on bit-for-bit — a desync package is evidence peers exchange. Per
//! `v2/README.md` §2.2 this crate does not merely trust the TypeScript port
//! to match: see `v2/tools/lua_reference/diagnostics_schema_vectors.txt`,
//! generated from the real Lua, and asserted by `tests::shared_vectors_agree_with_lua`
//! below.

use gc_core::fnv1a64;

/// Which vocabulary a field's name is allowed to use. See the module doc
/// comment's domain table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Build/protocol/manifest identity. May not use wall-clock vocabulary.
    Identity,
    /// Simulation and tick-space evidence. May not use wall-clock vocabulary.
    Canonical,
    /// Wall-clock and transport observation. May not use simulation vocabulary.
    Runtime,
    /// The one binding between the wall clock and simulation ticks. Must name
    /// both vocabularies and declare `mapping_error_ms`.
    Anchor,
}

/// A field's value kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// `true`/`false`.
    Boolean,
    /// A finite whole number, `abs(n) <= MAX_SAFE_INTEGER`.
    Integer,
    /// Any finite number.
    Number,
    /// A bounded join key: no direct identifiers, no paths, no addresses.
    Id,
    /// A bounded opaque machine string.
    Str,
    /// Bounded human-readable text, never a join key.
    Text,
    /// A lowercase `fnv1a64` hex digest.
    Hash,
    /// A string drawn from a fixed, declared member set.
    Enum,
    /// An ordered, length-bounded list of one element shape.
    Array,
    /// An unordered, length-bounded `id -> element shape` map, sorted at
    /// encode time.
    Map,
    /// A fixed set of named, individually (op­tional-or-not) typed fields.
    Record,
}

/// One field in a diagnostics shape tree. Build with [`Field::new`] plus the
/// builder methods, mirroring the Lua original's option-table literals.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The field's name. `None` only for a shape's own root record and for
    /// array/map element shapes, which have no name of their own.
    pub name: Option<String>,
    /// This field's value kind.
    pub kind: FieldKind,
    /// This field's domain. `None` means "inherit the enclosing record's
    /// domain"; only a `record`/`array`/`map` container may leave this unset
    /// with no domain to inherit (i.e. at a shape's root).
    pub domain: Option<Domain>,
    /// Whether a `record` may omit this field entirely.
    pub optional: bool,
    /// Declared members, for `Enum` fields.
    pub values: Option<Vec<String>>,
    /// Element shape, for `Array`/`Map` fields.
    pub element: Option<Box<Field>>,
    /// Field list, for `Record` fields, in canonical (encoding) order.
    pub fields: Option<Vec<Field>>,
    /// Inclusive lower bound, for `Integer`/`Number` fields.
    pub min: Option<f64>,
    /// Inclusive upper bound, for `Integer`/`Number` fields.
    pub max: Option<f64>,
    /// Byte-length lower bound, for `Id`/`Str`/`Text`/`Hash` fields.
    pub min_length: Option<usize>,
    /// Byte-length upper bound, for `Id`/`Str`/`Text`/`Hash`/`Array`/`Map` fields.
    pub max_length: Option<usize>,
}

impl Field {
    /// A field of `kind`, otherwise unnamed, domainless, required, and unbounded.
    #[must_use]
    pub fn new(kind: FieldKind) -> Self {
        Field {
            name: None,
            kind,
            domain: None,
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
        self.name = Some(name.into());
        self
    }

    /// Set this field's domain.
    #[must_use]
    pub fn domain(mut self, domain: Domain) -> Self {
        self.domain = Some(domain);
        self
    }

    /// Mark this record field as omittable.
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Set the declared members of an `Enum` field.
    #[must_use]
    pub fn values(mut self, values: Vec<String>) -> Self {
        self.values = Some(values);
        self
    }

    /// Set the element shape of an `Array`/`Map` field.
    #[must_use]
    pub fn element(mut self, element: Field) -> Self {
        self.element = Some(Box::new(element));
        self
    }

    /// Set the field list of a `Record` field.
    #[must_use]
    pub fn fields(mut self, fields: Vec<Field>) -> Self {
        self.fields = Some(fields);
        self
    }

    /// Set the inclusive lower bound of an `Integer`/`Number` field.
    #[must_use]
    pub fn min(mut self, min: f64) -> Self {
        self.min = Some(min);
        self
    }

    /// Set the inclusive upper bound of an `Integer`/`Number` field.
    #[must_use]
    pub fn max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }

    /// Set the byte-length lower bound of an `Id`/`Str`/`Text`/`Hash` field.
    #[must_use]
    pub fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    /// Set the byte-length/entry-count upper bound of an
    /// `Id`/`Str`/`Text`/`Hash`/`Array`/`Map` field.
    #[must_use]
    pub fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }
}

/// A [`Field`] of kind `Record`: the root of a shape tree.
pub type Shape = Field;

/// A dynamically-typed value validated and encoded against a [`Field`]
/// shape. Lua strings are byte arrays, so text-carrying variants hold
/// `Vec<u8>` rather than `String`: a diagnostic value arriving from a
/// transport, a driver, or an operator must be able to carry an
/// invalid-UTF-8 byte through to a graceful validation error rather than
/// being rejected (or panicking) before validation even runs.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A `Boolean` field's value.
    Boolean(bool),
    /// An `Integer` or `Number` field's value. Every Lua number is an f64
    /// (AGENTS.md §5 rule 1); which of the two kinds is intended is a
    /// property of the [`Field`], not the [`Value`].
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
    pub fn text(bytes: impl Into<Vec<u8>>) -> Self {
        Value::Text(bytes.into())
    }

    /// Build a [`Value::Text`] from a UTF-8 string, for the overwhelmingly
    /// common case where the text is not attacker-controlled bytes.
    #[must_use]
    pub fn str(text: impl Into<String>) -> Self {
        Value::Text(text.into().into_bytes())
    }

    fn record_get<'a>(entries: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
        entries.iter().find(|(key, _)| key == name).map(|(_, v)| v)
    }
}

/// Bumping this changes every canonical preimage and therefore every export
/// digest. It is a coordinated breaking change.
pub const SERIALIZATION_VERSION: i64 = 1;
/// Named so a future digest algorithm can be introduced as a new value
/// instead of silently reinterpreting stored digests.
pub const DIGEST: &str = "fnv1a64/v1";
/// Exact byte length of a canonical digest's hex encoding.
pub const HASH_LENGTH: usize = 16;
/// Default byte-length bound for `Id` fields.
pub const MAX_ID_LENGTH: usize = 128;
/// Default byte-length bound for `Str` fields.
pub const MAX_STRING_LENGTH: usize = 512;
/// Default byte-length bound for `Text` fields.
pub const MAX_TEXT_LENGTH: usize = 2048;
/// Default entry-count bound for `Array`/`Map` fields.
pub const MAX_ARRAY_LENGTH: usize = 4096;
/// The largest integer magnitude an `Integer`/`Number` field may exactly
/// represent (`2^53`).
pub const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_992.0;
/// The marker a bounded collection carries once it has dropped anything. It
/// is a field, never a silently shorter array: a truncated export that looks
/// complete is worse than no export.
pub const TRUNCATED: &str = "truncated";
/// The marker a bounded collection carries when it dropped nothing.
pub const COMPLETE: &str = "complete";
/// What a redacted field reads as. It is a constant so a spec can assert the
/// absence of the original *and* the presence of the marker: a field that
/// simply vanished cannot be told apart from a field nobody recorded.
pub const REDACTED: &str = "[redacted]";

const HEADER: &[u8] = b"GCND";

const SENSITIVE_TEXT_SUBSTRINGS: [&str; 9] = [
    "ice-",
    "candidate",
    "fingerprint",
    "sdp",
    "stun:",
    "turn:",
    "turns:",
    "://",
    "@",
];

const SDP_LINE_KEYS: [&str; 13] = [
    "v", "o", "s", "c", "t", "m", "a", "b", "i", "u", "k", "r", "z",
];

const WALL_CLOCK_WORDS: [&str; 14] = [
    "_ms$",
    "^ms_",
    "_ms_",
    "rtt",
    "jitter",
    "wall",
    "monotonic",
    "elapsed",
    "timestamp",
    "_at$",
    "second",
    "millis",
    "realtime",
    "frame_time",
];

const SIMULATION_WORDS: [&str; 8] = [
    "tick",
    "boundary",
    "hash",
    "checkpoint",
    "confirmed",
    "rollback",
    "resimulat",
    "snapshot",
];

fn is_finite(value: f64) -> bool {
    value.is_finite()
}

fn is_integer(value: f64) -> bool {
    is_finite(value) && value == value.floor() && value.abs() <= MAX_SAFE_INTEGER
}

/// Match one of this module's simplified Lua patterns: a plain substring,
/// optionally anchored at the start (`^prefix`), the end (`suffix$`), or
/// both (`^exact$`). Every pattern in [`WALL_CLOCK_WORDS`]/[`SIMULATION_WORDS`]
/// is one of these four shapes — no other Lua pattern magic character is
/// used by this schema, so a general Lua-pattern engine would be solving a
/// problem this module does not have.
fn pattern_matches(lowered: &str, pattern: &str) -> bool {
    let (anchored_start, rest) = match pattern.strip_prefix('^') {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let (anchored_end, core) = match rest.strip_suffix('$') {
        Some(core) => (true, core),
        None => (false, rest),
    };
    match (anchored_start, anchored_end) {
        (true, true) => lowered == core,
        (true, false) => lowered.starts_with(core),
        (false, true) => lowered.ends_with(core),
        (false, false) => lowered.contains(core),
    }
}

fn matches_vocabulary(name: &str, words: &[&'static str]) -> Option<&'static str> {
    let lowered = name.to_ascii_lowercase();
    words
        .iter()
        .find(|&&word| pattern_matches(&lowered, word))
        .copied()
}

/// Does this free text look like it carries network or identity material?
/// Deliberately over-rejects: two or more colons is treated as
/// address-shaped even though a wall-clock time like `12:34:56` also
/// matches, because losing one timestamp from a diagnostic detail is cheap
/// and leaking an IPv6 address is not.
#[must_use]
pub fn is_sensitive_text(text: &[u8]) -> bool {
    let lowered: Vec<u8> = text.iter().map(u8::to_ascii_lowercase).collect();
    for needle in SENSITIVE_TEXT_SUBSTRINGS {
        if contains_subslice(&lowered, needle.as_bytes()) {
            return true;
        }
    }
    // A dotted quad, anywhere in the string.
    if has_dotted_quad(&lowered) {
        return true;
    }
    // Address-shaped: two or more colons covers every IPv6 form, compressed
    // or not, without needing to parse one.
    if lowered.iter().filter(|&&b| b == b':').count() >= 2 {
        return true;
    }
    // An SDP body line at a line start.
    let mut probe = vec![b'\n'];
    for &byte in &lowered {
        probe.push(if byte == b'\r' { b'\n' } else { byte });
    }
    for key in SDP_LINE_KEYS {
        let mut needle = vec![b'\n'];
        needle.extend_from_slice(key.as_bytes());
        needle.push(b'=');
        if contains_subslice(&probe, &needle) {
            return true;
        }
    }
    false
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn has_dotted_quad(bytes: &[u8]) -> bool {
    let len = bytes.len();
    for start in 0..len {
        let mut index = start;
        let mut matched = true;
        for part in 0..4 {
            let group_start = index;
            while index < len && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index == group_start {
                matched = false;
                break;
            }
            if part < 3 {
                if index < len && bytes[index] == b'.' {
                    index += 1;
                } else {
                    matched = false;
                    break;
                }
            }
        }
        if matched {
            return true;
        }
    }
    false
}

/// Bound and redact one free-text field: the redaction marker when the text
/// looks sensitive ([`is_sensitive_text`]), the truncated text when it is
/// merely long, and the text unchanged otherwise.
#[must_use]
pub fn redact_free_text(text: &[u8], max_bytes: usize) -> Vec<u8> {
    if is_sensitive_text(text) {
        return REDACTED.as_bytes().to_vec();
    }
    if text.len() <= max_bytes {
        return text.to_vec();
    }
    let keep = max_bytes.saturating_sub(TRUNCATED.len() + 1);
    let mut result = text[..keep.min(text.len())].to_vec();
    result.push(b' ');
    result.extend_from_slice(TRUNCATED.as_bytes());
    result
}

/// Does `name` use wall-clock vocabulary (`rtt`, `_ms`, `timestamp`, ...)?
#[must_use]
pub fn is_wall_clock_name(name: &str) -> bool {
    matches_vocabulary(name, &WALL_CLOCK_WORDS).is_some()
}

/// Does `name` use simulation vocabulary (`tick`, `hash`, `checkpoint`, ...)?
#[must_use]
pub fn is_simulation_name(name: &str) -> bool {
    matches_vocabulary(name, &SIMULATION_WORDS).is_some()
}

#[derive(Default)]
struct VocabularyScope {
    wall: bool,
    simulation: bool,
    mapping_error: bool,
}

fn assert_domain(field: &Field, domain: Domain, path: &str, scope: &mut VocabularyScope) {
    let Some(name) = &field.name else { return };
    let wall = matches_vocabulary(name, &WALL_CLOCK_WORDS);
    let simulation = matches_vocabulary(name, &SIMULATION_WORDS);
    if wall.is_some() {
        scope.wall = true;
    }
    if simulation.is_some() {
        scope.simulation = true;
    }
    if name == "mapping_error_ms" {
        scope.mapping_error = true;
    }
    match domain {
        Domain::Identity | Domain::Canonical => {
            assert!(
                wall.is_none(),
                "{path} is identity/canonical and may not use wall-clock vocabulary ({wall:?})"
            );
        }
        Domain::Runtime => {
            assert!(
                simulation.is_none(),
                "{path} is runtime observation and may not use simulation vocabulary ({simulation:?})"
            );
        }
        Domain::Anchor => {}
    }
}

fn assert_field_shape(
    field: &Field,
    inherited: Option<Domain>,
    path: &str,
    scope: Option<&mut VocabularyScope>,
) {
    let domain = field.domain.or(inherited);
    if let Some(declared) = field.domain {
        assert!(
            inherited.is_none() || inherited == Some(declared),
            "{path} may not change domain from {inherited:?}"
        );
    }
    // One scope per anchor subtree, keyed on the *domain transition* into
    // `Anchor` rather than on being the outermost call — see the Lua
    // original's comment on why keying this on "no scope yet" was wrong.
    let entering_anchor = domain == Some(Domain::Anchor) && inherited != Some(Domain::Anchor);
    let mut owned_scope = VocabularyScope::default();
    let scope: &mut VocabularyScope = if entering_anchor {
        &mut owned_scope
    } else {
        match scope {
            Some(scope) => scope,
            None => &mut owned_scope,
        }
    };
    match domain {
        Some(domain) => assert_domain(field, domain, path, scope),
        None => {
            // Only a container may be domainless, and only so its children
            // can each declare one. A leaf with no domain would escape the
            // vocabulary guard.
            assert!(
                matches!(
                    field.kind,
                    FieldKind::Record | FieldKind::Array | FieldKind::Map
                ),
                "{path} has no domain and none is inherited"
            );
        }
    }
    match field.kind {
        FieldKind::Enum => {
            let values = field
                .values
                .as_ref()
                .unwrap_or_else(|| panic!("{path} enum shape needs values"));
            assert!(
                !values.is_empty(),
                "{path} enum shape needs at least one member"
            );
        }
        FieldKind::Array | FieldKind::Map => {
            let element = field
                .element
                .as_deref()
                .unwrap_or_else(|| panic!("{path} needs an element shape"));
            assert_field_shape(element, domain, &format!("{path}[]"), Some(scope));
        }
        FieldKind::Record => {
            let fields = field
                .fields
                .as_ref()
                .unwrap_or_else(|| panic!("{path} record shape needs fields"));
            let mut seen_names: Vec<&str> = Vec::with_capacity(fields.len());
            for child in fields {
                let name = child
                    .name
                    .as_deref()
                    .unwrap_or_else(|| panic!("{path} field needs a name"));
                assert!(!name.is_empty(), "{path} field needs a name");
                assert!(
                    !seen_names.contains(&name),
                    "{path} field {name} is declared twice"
                );
                seen_names.push(name);
                assert_field_shape(child, domain, &format!("{path}.{name}"), Some(scope));
            }
        }
        _ => {}
    }
    // An anchor is the only place both vocabularies may meet, and it only
    // earns that by actually binding them and by naming its own error term.
    if entering_anchor {
        assert!(
            scope.wall,
            "{path} anchor must name a wall-clock observation"
        );
        assert!(
            scope.simulation,
            "{path} anchor must name a simulation tick"
        );
        assert!(
            scope.mapping_error,
            "{path} anchor must declare mapping_error_ms"
        );
    }
}

/// Build a validated record shape.
///
/// # Panics
///
/// Panics if the shape violates a structural invariant (missing name,
/// duplicate field, domain that contradicts its parent, wrong vocabulary for
/// its domain, ...): a broken shape is a programmer error (AGENTS.md §7).
#[must_use]
pub fn record(name: impl Into<String>, domain: Option<Domain>, fields: Vec<Field>) -> Shape {
    let name = name.into();
    assert!(!name.is_empty(), "diagnostic shape needs a name");
    let shape = Field {
        name: Some(name.clone()),
        kind: FieldKind::Record,
        domain,
        optional: false,
        values: None,
        element: None,
        fields: Some(fields),
        min: None,
        max: None,
        min_length: None,
        max_length: None,
    };
    assert_field_shape(&shape, None, &name, None);
    shape
}

/// A record field nested inside another: it inherits its parent's domain, so
/// it is checked when the parent is built rather than standalone.
#[must_use]
pub fn nested(name: impl Into<String>, fields: Vec<Field>) -> Field {
    Field::new(FieldKind::Record).named(name).fields(fields)
}

/// Declare a closed enum member set.
///
/// # Panics
///
/// Panics on an empty or duplicate member (AGENTS.md §7: a malformed shape is
/// a programmer error).
#[must_use]
pub fn enum_values(members: &[&str]) -> Vec<String> {
    let mut values = Vec::with_capacity(members.len());
    for &member in members {
        assert!(!member.is_empty(), "enum member must be a non-empty string");
        assert!(
            !values.contains(&member.to_string()),
            "enum member {member} is declared twice"
        );
        values.push(member.to_string());
    }
    values
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate_text(field: &Field, bytes: &[u8], path: &str) -> std::result::Result<(), String> {
    let max = field.max_length.unwrap_or(match field.kind {
        FieldKind::Text => MAX_TEXT_LENGTH,
        FieldKind::Id => MAX_ID_LENGTH,
        _ => MAX_STRING_LENGTH,
    });
    let min = field
        .min_length
        .unwrap_or(if field.kind == FieldKind::Text { 0 } else { 1 });
    if bytes.len() < min {
        return Err(format!("{path} must be at least {min} bytes"));
    }
    if bytes.len() > max {
        return Err(format!("{path} must be at most {max} bytes"));
    }
    if field.kind == FieldKind::Hash {
        let canonical = bytes.len() == HASH_LENGTH
            && bytes
                .iter()
                .all(|&b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !canonical {
            return Err(format!("{path} must be a {DIGEST} hex digest"));
        }
        return Ok(());
    }
    if field.kind == FieldKind::Id {
        let canonical = !bytes.is_empty()
            && (bytes[0].is_ascii_alphanumeric())
            && bytes
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
        if !canonical {
            return Err(format!("{path} is not a bounded pseudonymous id"));
        }
        if contains_subslice(bytes, b"..") {
            return Err(format!("{path} may not contain .."));
        }
        if contains_subslice(bytes, b"@") {
            return Err(format!("{path} may not contain @"));
        }
        if is_ipv4_shaped(bytes) {
            return Err(format!("{path} looks like a raw address"));
        }
    }
    Ok(())
}

fn is_ipv4_shaped(bytes: &[u8]) -> bool {
    let groups: Vec<&[u8]> = bytes.split(|&b| b == b'.').collect();
    groups.len() == 4
        && groups
            .iter()
            .all(|group| !group.is_empty() && group.iter().all(u8::is_ascii_digit))
}

/// Validate `value` against `field`, returning the first violation found
/// (deepest-first within a container, matching the Lua original's
/// short-circuit order).
fn validate_field(field: &Field, value: &Value, path: &str) -> std::result::Result<(), String> {
    match field.kind {
        FieldKind::Boolean => match value {
            Value::Boolean(_) => Ok(()),
            _ => Err(format!("{path} must be a boolean")),
        },
        FieldKind::Integer | FieldKind::Number => {
            let Value::Number(number) = value else {
                let noun = if field.kind == FieldKind::Integer {
                    "integer"
                } else {
                    "number"
                };
                return Err(format!("{path} must be a finite {noun}"));
            };
            if field.kind == FieldKind::Integer {
                if !is_integer(*number) {
                    return Err(format!("{path} must be a finite integer"));
                }
            } else if !is_finite(*number) {
                return Err(format!("{path} must be a finite number"));
            }
            if let Some(min) = field.min
                && *number < min
            {
                return Err(format!("{path} must be at least {min}"));
            }
            if let Some(max) = field.max
                && *number > max
            {
                return Err(format!("{path} must be at most {max}"));
            }
            Ok(())
        }
        FieldKind::Enum => {
            let Value::Text(bytes) = value else {
                return Err(format!("{path} must be a string"));
            };
            let members = field.values.as_ref().expect("enum shape always has values");
            if !members
                .iter()
                .any(|member| member.as_bytes() == bytes.as_slice())
            {
                return Err(format!("{path} is not a declared member"));
            }
            Ok(())
        }
        FieldKind::Id | FieldKind::Str | FieldKind::Text | FieldKind::Hash => match value {
            Value::Text(bytes) => validate_text(field, bytes, path),
            _ => Err(format!("{path} must be a string")),
        },
        FieldKind::Array => {
            let Value::Array(items) = value else {
                return Err(format!("{path} must be an array"));
            };
            let max = field.max_length.unwrap_or(MAX_ARRAY_LENGTH);
            if items.len() > max {
                return Err(format!("{path} holds more than {max} entries"));
            }
            let element = field
                .element
                .as_deref()
                .expect("array shape always has an element");
            for (index, item) in items.iter().enumerate() {
                validate_field(element, item, &format!("{path}[{}]", index + 1))?;
            }
            Ok(())
        }
        FieldKind::Map => {
            let Value::Map(entries) = value else {
                return Err(format!("{path} must be a map"));
            };
            let element = field
                .element
                .as_deref()
                .expect("map shape always has an element");
            for (key, child) in entries {
                validate_text(
                    &Field::new(FieldKind::Id),
                    key.as_bytes(),
                    &format!("{path} key"),
                )?;
                validate_field(element, child, &format!("{path}.{key}"))?;
            }
            let max = field.max_length.unwrap_or(MAX_ARRAY_LENGTH);
            if entries.len() > max {
                return Err(format!("{path} holds more than {max} entries"));
            }
            Ok(())
        }
        FieldKind::Record => {
            let Value::Record(entries) = value else {
                return Err(format!("{path} must be a record"));
            };
            let declared_fields = field
                .fields
                .as_ref()
                .expect("record shape always has fields");
            for child in declared_fields {
                let name = child
                    .name
                    .as_deref()
                    .expect("record field always has a name");
                match Value::record_get(entries, name) {
                    None => {
                        if !child.optional {
                            return Err(format!("{path}.{name} is required"));
                        }
                    }
                    Some(child_value) => {
                        validate_field(child, child_value, &format!("{path}.{name}"))?;
                    }
                }
            }
            for (key, _) in entries {
                if !declared_fields
                    .iter()
                    .any(|f| f.name.as_deref() == Some(key.as_str()))
                {
                    return Err(format!("{path}.{key} is not declared by this schema"));
                }
            }
            Ok(())
        }
    }
}

/// Result alias for fallible diagnostics-schema value operations (AGENTS.md
/// §7: expected, recoverable failures from external input).
pub type Result<T> = std::result::Result<T, String>;

/// Validate `value` against `shape`. Returns `Err` rather than panicking:
/// diagnostic values come from transports, drivers, and operators.
pub fn validate(shape: &Shape, value: &Value) -> Result<()> {
    let path = shape.name.as_deref().expect("a shape always has a name");
    validate_field(shape, value, path)
}

// ---------------------------------------------------------------------------
// Canonical serialization
// ---------------------------------------------------------------------------

fn lp(bytes: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(bytes.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(bytes);
}

/// Integers print exactly; other numbers print with a fixed 17-significant-digit
/// format so the preimage cannot depend on locale or platform `f64`-to-text
/// precision. Mirrors C's (and Lua's `string.format`'s) `%.17g`: see
/// [`format_g17`].
fn number_bytes(value: f64) -> Vec<u8> {
    if value == value.floor() && value.abs() <= MAX_SAFE_INTEGER {
        format!("{value:.0}").into_bytes()
    } else {
        format_g17(value).into_bytes()
    }
}

/// A Rust implementation of C's `printf("%.17g", value)` (equivalently,
/// Lua's `("%.17g"):format(value)`), used wherever this codebase needs an
/// exact, cross-language-stable text rendering of a non-integer `f64` —
/// [`number_bytes`] here, and `desync_package::bounded_text`'s numeric branch.
///
/// `%g` with precision 17 means: 17 significant decimal digits, rendered in
/// fixed-point notation when the decimal exponent is in `[-4, 17)` and in
/// scientific notation otherwise, with trailing fractional zeros (and a
/// trailing bare decimal point) stripped either way. This is *not* the same
/// text `f64::to_string`/`{:e}` would print: Rust's default float formatting
/// prints the *shortest* string that round-trips, while `%g` always renders
/// exactly `precision` significant digits before stripping trailing zeros —
/// the two coincide only when the exact value happens to need all 17 digits.
///
/// Differential-tested against real Lua output; see
/// `v2/tools/lua_reference/diagnostics_schema_vectors.txt` and this module's
/// `tests::shared_vectors_agree_with_lua`.
///
/// # Panics
///
/// Never called with a non-finite value: callers only reach this after
/// [`is_finite`]/[`is_integer`] validation already ran.
pub(crate) fn format_g17(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let negative = value < 0.0;
    let magnitude = value.abs();
    // 17 significant digits: one before the point plus sixteen after, in
    // scientific form. Rust's float formatting is correctly rounded, exactly
    // like the C library formatter `%.17g` is built on, so the two agree
    // digit-for-digit.
    let scientific = format!("{magnitude:.16e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("scientific formatting always includes an exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("Rust's scientific exponent is always a plain decimal integer");
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if !(-4..17).contains(&exponent) {
        let mut fraction = digits[1..].to_string();
        while fraction.ends_with('0') {
            fraction.pop();
        }
        out.push_str(&digits[..1]);
        if !fraction.is_empty() {
            out.push('.');
            out.push_str(&fraction);
        }
        out.push('e');
        out.push(if exponent >= 0 { '+' } else { '-' });
        out.push_str(&format!("{:02}", exponent.unsigned_abs()));
    } else if exponent >= 0 {
        let split = exponent as usize + 1;
        let mut fraction = digits[split..].to_string();
        while fraction.ends_with('0') {
            fraction.pop();
        }
        out.push_str(&digits[..split]);
        if !fraction.is_empty() {
            out.push('.');
            out.push_str(&fraction);
        }
    } else {
        out.push_str("0.");
        out.push_str(&"0".repeat((-exponent - 1) as usize));
        let mut fraction = digits.clone();
        while fraction.ends_with('0') {
            fraction.pop();
        }
        out.push_str(&fraction);
    }
    out
}

fn encode_field(field: &Field, value: &Value, out: &mut Vec<u8>) {
    match (field.kind, value) {
        (FieldKind::Boolean, Value::Boolean(flag)) => out.push(if *flag { b'T' } else { b'F' }),
        (FieldKind::Integer | FieldKind::Number, Value::Number(number)) => {
            lp(&number_bytes(*number), out);
        }
        (FieldKind::Array, Value::Array(items)) => {
            lp(items.len().to_string().as_bytes(), out);
            let element = field
                .element
                .as_deref()
                .expect("array shape always has an element");
            for item in items {
                encode_field(element, item, out);
            }
        }
        (FieldKind::Map, Value::Map(entries)) => {
            let mut sorted: Vec<&(String, Value)> = entries.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            lp(sorted.len().to_string().as_bytes(), out);
            let element = field
                .element
                .as_deref()
                .expect("map shape always has an element");
            for (key, child) in sorted {
                lp(key.as_bytes(), out);
                encode_field(element, child, out);
            }
        }
        (FieldKind::Record, Value::Record(entries)) => {
            let declared_fields = field
                .fields
                .as_ref()
                .expect("record shape always has fields");
            for child in declared_fields {
                let name = child
                    .name
                    .as_deref()
                    .expect("record field always has a name");
                match Value::record_get(entries, name) {
                    None => out.push(b'-'),
                    Some(child_value) => {
                        out.push(b'+');
                        encode_field(child, child_value, out);
                    }
                }
            }
        }
        (_, Value::Text(bytes)) => lp(bytes, out),
        _ => panic!("encode_field called with a value that does not match its shape's kind"),
    }
}

/// Canonical bytes for a validated value: byte-identical for byte-identical
/// input, on every platform, independent of any collection's construction
/// order.
pub fn encode(shape: &Shape, value: &Value) -> Result<Vec<u8>> {
    validate(shape, value)?;
    let mut out = Vec::new();
    out.extend_from_slice(HEADER);
    out.push(b';');
    out.extend_from_slice(SERIALIZATION_VERSION.to_string().as_bytes());
    out.push(b';');
    lp(
        shape
            .name
            .as_deref()
            .expect("a shape always has a name")
            .as_bytes(),
        &mut out,
    );
    encode_field(shape, value, &mut out);
    Ok(out)
}

/// `fnv1a64` digest of [`encode`]'s output.
pub fn digest(shape: &Shape, value: &Value) -> Result<String> {
    let bytes = encode(shape, value)?;
    Ok(fnv1a64::hash(&bytes))
}

/// A digest over an ordered tuple of opaque byte strings, independent of
/// [`Shape`]/[`Value`] — used where the "record" being hashed is not itself a
/// diagnostics value (e.g. a set of wire-format strings).
#[must_use]
pub fn tuple_digest(label: &str, parts: &[&[u8]]) -> String {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(HEADER);
    buffer.push(b';');
    buffer.extend_from_slice(SERIALIZATION_VERSION.to_string().as_bytes());
    buffer.push(b';');
    lp(label.as_bytes(), &mut buffer);
    lp(parts.len().to_string().as_bytes(), &mut buffer);
    for part in parts {
        lp(part, &mut buffer);
    }
    fnv1a64::hash(&buffer)
}

/// A sub-shape holding only the top-level fields whose domain is in
/// `allowed`. The point is a digest over *just* the deterministic half of an
/// artifact: canonical evidence can honestly be claimed byte-stable across
/// runs, and runtime observation cannot, so the two need separate preimages
/// rather than one preimage and an optimistic assertion.
///
/// # Panics
///
/// Panics if no field's domain is in `allowed` (a projection must keep at
/// least one section).
#[must_use]
pub fn project(shape: &Shape, allowed: &[Domain]) -> Shape {
    let declared_fields = shape.fields.as_ref().expect("a shape is always a record");
    let mut kept = Vec::new();
    let mut names = Vec::new();
    for field in declared_fields {
        if let Some(domain) = field.domain
            && allowed.contains(&domain)
        {
            kept.push(field.clone());
            names.push(domain_label(domain));
        }
    }
    assert!(
        !kept.is_empty(),
        "a projection must keep at least one section"
    );
    names.sort_unstable();
    let base_name = shape.name.as_deref().expect("a shape always has a name");
    record(format!("{base_name}/{}", names.join("+")), None, kept)
}

fn domain_label(domain: Domain) -> &'static str {
    match domain {
        Domain::Identity => "identity",
        Domain::Canonical => "canonical",
        Domain::Runtime => "runtime",
        Domain::Anchor => "anchor",
    }
}

/// Walk a shape and report every declared field path alongside its domain.
/// Used to assert the identity/canonical/runtime separation holds across a
/// *whole* schema rather than at a handful of hand-picked fields.
#[must_use]
pub fn domains(shape: &Shape) -> Vec<(String, Option<Domain>)> {
    let mut result = Vec::new();
    fn walk(
        field: &Field,
        inherited: Option<Domain>,
        path: &str,
        result: &mut Vec<(String, Option<Domain>)>,
    ) {
        let domain = field.domain.or(inherited);
        if field.name.is_some() {
            result.push((path.to_string(), domain));
        }
        match field.kind {
            FieldKind::Record => {
                if let Some(fields) = &field.fields {
                    for child in fields {
                        let name = child.name.as_deref().unwrap_or("");
                        walk(child, domain, &format!("{path}.{name}"), result);
                    }
                }
            }
            FieldKind::Array | FieldKind::Map => {
                if let Some(element) = &field.element {
                    walk(element, domain, &format!("{path}[]"), result);
                }
            }
            _ => {}
        }
    }
    let root_name = shape.name.as_deref().expect("a shape always has a name");
    walk(shape, None, root_name, &mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The shape every record-shaped vector in
    /// `diagnostics_schema_vectors.txt` validates against — reproduced here
    /// field-by-field to match the Lua script that generated the vectors
    /// (see that file's header comment).
    fn test_shape() -> Shape {
        record(
            "test_record",
            None,
            vec![
                nested(
                    "identity",
                    vec![
                        Field::new(FieldKind::Id).named("id_field"),
                        Field::new(FieldKind::Integer).named("count").min(0.0),
                        Field::new(FieldKind::Boolean).named("flag"),
                        Field::new(FieldKind::Enum)
                            .named("label")
                            .values(enum_values(&["alpha", "beta", "gamma"])),
                        Field::new(FieldKind::Hash).named("digest"),
                        Field::new(FieldKind::Text).named("note").optional(),
                    ],
                )
                .domain(Domain::Identity),
                nested(
                    "canonical",
                    vec![
                        Field::new(FieldKind::Map)
                            .named("tags")
                            .element(Field::new(FieldKind::Id)),
                        Field::new(FieldKind::Array)
                            .named("items")
                            .element(Field::new(FieldKind::Integer)),
                        Field::new(FieldKind::Number).named("rating").optional(),
                    ],
                )
                .domain(Domain::Canonical),
            ],
        )
    }

    fn identity(
        id_field: &str,
        count: f64,
        flag: bool,
        label: &str,
        digest: &str,
        note: Option<&str>,
    ) -> Value {
        let mut fields = vec![
            ("id_field".to_string(), Value::str(id_field)),
            ("count".to_string(), Value::Number(count)),
            ("flag".to_string(), Value::Boolean(flag)),
            ("label".to_string(), Value::str(label)),
            ("digest".to_string(), Value::str(digest)),
        ];
        if let Some(note) = note {
            fields.push(("note".to_string(), Value::str(note)));
        }
        Value::Record(fields)
    }

    fn ordinary_record_value() -> Value {
        Value::Record(vec![
            (
                "identity".to_string(),
                identity(
                    "peer_alpha",
                    42.0,
                    true,
                    "beta",
                    "0123456789abcdef",
                    Some("hello world"),
                ),
            ),
            (
                "canonical".to_string(),
                Value::Record(vec![
                    (
                        "tags".to_string(),
                        // Deliberately built out of alphabetical order; the
                        // schema must sort keys at encode time regardless.
                        Value::Map(vec![
                            ("z".to_string(), Value::str("last")),
                            ("a".to_string(), Value::str("first")),
                            ("m".to_string(), Value::str("middle")),
                        ]),
                    ),
                    (
                        "items".to_string(),
                        Value::Array(vec![
                            Value::Number(1.0),
                            Value::Number(2.0),
                            Value::Number(3.0),
                        ]),
                    ),
                ]),
            ),
        ])
    }

    fn empty_values_value() -> Value {
        Value::Record(vec![
            (
                "identity".to_string(),
                identity("peer_beta", 0.0, false, "alpha", "fedcba9876543210", None),
            ),
            (
                "canonical".to_string(),
                Value::Record(vec![
                    ("tags".to_string(), Value::Map(vec![])),
                    ("items".to_string(), Value::Array(vec![])),
                ]),
            ),
        ])
    }

    fn non_utf8_byte_value() -> Value {
        let mut note = b"abc".to_vec();
        note.push(0xff);
        note.extend_from_slice(b"def");
        note.push(0x00);
        note.extend_from_slice(b"ghi");
        Value::Record(vec![
            (
                "identity".to_string(),
                Value::Record(vec![
                    ("id_field".to_string(), Value::str("peer_gamma")),
                    ("count".to_string(), Value::Number(7.0)),
                    ("flag".to_string(), Value::Boolean(true)),
                    ("label".to_string(), Value::str("gamma")),
                    ("digest".to_string(), Value::str("00112233445566aa")),
                    ("note".to_string(), Value::text(note)),
                ]),
            ),
            (
                "canonical".to_string(),
                Value::Record(vec![
                    (
                        "tags".to_string(),
                        Value::Map(vec![("solo".to_string(), Value::str("one"))]),
                    ),
                    ("items".to_string(), Value::Array(vec![Value::Number(9.0)])),
                ]),
            ),
        ])
    }

    fn negative_and_fractional_numbers_value() -> Value {
        Value::Record(vec![
            (
                "identity".to_string(),
                identity(
                    "peer_delta",
                    5.0,
                    false,
                    "alpha",
                    "1111222233334444",
                    Some(""),
                ),
            ),
            (
                "canonical".to_string(),
                Value::Record(vec![
                    ("tags".to_string(), Value::Map(vec![])),
                    (
                        "items".to_string(),
                        Value::Array(vec![
                            Value::Number(-1.0),
                            Value::Number(0.0),
                            Value::Number(-9_007_199_254_740_992.0),
                        ]),
                    ),
                    ("rating".to_string(), Value::Number(480.75)),
                ]),
            ),
        ])
    }

    // Cross-language digest agreement (v2/README.md §2.2): a shared vector
    // file generated from the real Lua `diagnostics_schema.lua`, checked into
    // `v2/tools/lua_reference/diagnostics_schema_vectors.txt`. This does not
    // merely re-hash the pinned bytes (that would only prove `fnv1a64`
    // agrees, which is already covered by `gc-core`'s own tests): it rebuilds
    // each case's `Value` tree in Rust and asserts this crate's `encode`
    // produces byte-identical output to the real Lua's, then that `digest`
    // matches too.
    #[test]
    fn shared_vectors_agree_with_lua() {
        let shape = test_shape();
        let vectors = load_vectors();
        let mut checked = 0;
        for (label, encoded_hex, expected_digest) in &vectors {
            let value = match label.as_str() {
                "ordinary_record" => Some(ordinary_record_value()),
                "empty_values" => Some(empty_values_value()),
                "non_utf8_byte_in_text_field" => Some(non_utf8_byte_value()),
                "negative_and_fractional_numbers" => Some(negative_and_fractional_numbers_value()),
                "tuple_digest_two_parts" | "tuple_digest_empty" => None,
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
                let actual_digest = digest(&shape, &value)
                    .unwrap_or_else(|err| panic!("vector {label:?} failed to digest: {err}"));
                assert_eq!(
                    &actual_digest, expected_digest,
                    "vector {label:?}: fnv1a64 digest mismatch"
                );
                checked += 1;
            }
        }
        assert_eq!(
            checked, 4,
            "expected exactly the four record-shaped vectors to be exercised"
        );

        let tuple_two_parts = tuple_digest(
            "desync_input_wires",
            &[b"GCIP;1;G;wire-one", b"GCIP;1;H;wire-two"],
        );
        assert_eq!(
            tuple_two_parts,
            find_digest(&vectors, "tuple_digest_two_parts"),
            "tuple_digest_two_parts mismatch"
        );
        let tuple_empty = tuple_digest("desync_input_wires", &[]);
        assert_eq!(
            tuple_empty,
            find_digest(&vectors, "tuple_digest_empty"),
            "tuple_digest_empty mismatch"
        );
    }

    // A map's encoded bytes must not depend on the order its entries were
    // constructed in — that is the entire point of sorting keys at encode
    // time. `ordinary_record_value` already declares `tags` out of
    // alphabetical order; this additionally checks a *different* insertion
    // order (and a different in-memory `Vec` order) against the same pinned
    // golden bytes.
    #[test]
    fn map_encoding_is_independent_of_entry_order() {
        let shape = test_shape();
        let vectors = load_vectors();
        let (_, expected_hex, _) = vectors
            .iter()
            .find(|(label, _, _)| label == "ordinary_record")
            .expect("ordinary_record vector exists");
        let expected_bytes = decode_hex(expected_hex).unwrap();

        let mut value = ordinary_record_value();
        if let Value::Record(sections) = &mut value {
            let canonical = sections
                .iter_mut()
                .find(|(name, _)| name == "canonical")
                .map(|(_, v)| v)
                .unwrap();
            if let Value::Record(fields) = canonical {
                let tags = fields
                    .iter_mut()
                    .find(|(name, _)| name == "tags")
                    .map(|(_, v)| v)
                    .unwrap();
                if let Value::Map(entries) = tags {
                    entries.reverse();
                    let moved = entries.remove(0);
                    entries.push(moved);
                }
            }
        }
        let actual_bytes = encode(&shape, &value).unwrap();
        assert_eq!(actual_bytes, expected_bytes);
    }

    fn find_digest<'a>(vectors: &'a [(String, String, String)], label: &str) -> &'a str {
        vectors
            .iter()
            .find(|(l, _, _)| l == label)
            .map(|(_, _, digest)| digest.as_str())
            .unwrap_or_else(|| panic!("vector {label:?} not found"))
    }

    fn load_vectors() -> Vec<(String, String, String)> {
        let vectors_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tools/lua_reference/diagnostics_schema_vectors.txt");
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
}
