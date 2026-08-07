//! A minimal, dependency-free JSON reader/writer for this crate's control
//! surface bindings ([`crate::coordinator_bridge`], [`crate::match_driver_bridge`],
//! [`crate::rollback_events_bridge`]).
//!
//! ## Why hand-rolled, not `serde_json`
//!
//! This crate's own doc explains the two binding strategies it uses
//! (`wasm-bindgen` structs for ergonomic getters, raw exports for the
//! per-frame hot path). The coordinator/match-driver/rollback-events
//! reducers this module serves carry deep, closed-set-enum-shaped state
//! (`gc_netcode::coordinator::CoordinatorState`, `Event`, `Outcome`, ...)
//! that has no `serde` derive — and per this wave's brief, adding one would
//! either touch `gc-netcode` types that stay clean on purpose or require a
//! JSON crate dependency purely to shuttle presentation/control data that
//! never touches the determinism path. A ~250-line hand-rolled encoder/
//! decoder, scoped to exactly the JSON this crate emits and accepts, avoids
//! both: no new dependency, and no temptation to reuse a "just add
//! `#[derive(Serialize)]`" shortcut on a type where that would matter.
//!
//! This is presentation/control glue only. Nothing in [`crate::session`] or
//! [`crate::determinism`] — the modules on the determinism path — touches
//! this file.

use std::fmt::Write as _;

/// A JSON value. `Object` is an ordered `Vec` of pairs, never a map — this
/// crate's own workspace lint denies hash-map types, and object key order is
/// preserved on the wire the same way `gc_netcode::protocol::Value::Table`
/// preserves table order (README rule 5.4).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// JSON `null`.
    Null,
    /// JSON `true`/`false`.
    Bool(bool),
    /// A JSON number, always represented as `f64`.
    Number(f64),
    /// A JSON string.
    String(String),
    /// A JSON array.
    Array(Vec<Json>),
    /// A JSON object, key order preserved.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// A string value.
    #[must_use]
    pub fn str(value: impl Into<String>) -> Json {
        Json::String(value.into())
    }

    /// `null` if `value` is `None`, a string otherwise.
    #[must_use]
    pub fn opt_str(value: Option<&str>) -> Json {
        match value {
            Some(value) => Json::String(value.to_string()),
            None => Json::Null,
        }
    }

    /// An integer value, represented as `f64` (JSON has one number type;
    /// every integer this crate emits is well inside `f64`'s exact range).
    #[must_use]
    pub fn int(value: i64) -> Json {
        Json::Number(value as f64)
    }

    /// `null` if `value` is `None`, an integer otherwise.
    #[must_use]
    pub fn opt_int(value: Option<i64>) -> Json {
        match value {
            Some(value) => Json::int(value),
            None => Json::Null,
        }
    }

    /// A boolean value.
    #[must_use]
    pub fn bool(value: bool) -> Json {
        Json::Bool(value)
    }

    /// An object built from `(key, value)` pairs, in the order given.
    #[must_use]
    pub fn obj(fields: Vec<(&str, Json)>) -> Json {
        Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// [`Json::obj`], but a pair whose value is [`Json::Null`] is dropped
    /// from the object entirely rather than written as `"key":null`.
    ///
    /// Use this for a field that mirrors an `Option<T>`/`t[k] = nil` on the
    /// Lua side, where "absent" is the actual wire contract: Lua's
    /// `t[k] = nil` deletes the key outright, so a Lua caller reading that
    /// table sees the key missing, never present with a `nil` value. This
    /// crate's `Json::opt_int`/`Json::opt_str` (and any `Option`-shaped field
    /// built by hand with `.map_or(Json::Null, ...)`) previously encoded that
    /// same "absent" case as an explicit JSON `null`, which round-trips
    /// through `JSON.parse` as JS `null` — a value `!== undefined`, so a
    /// caller checking `field !== undefined` (the natural TypeScript
    /// spelling of "this optional field is present") sees a stray `null` and
    /// crashes dereferencing it. That is the same class of bug this port
    /// already fixed once for `gc_netcode::protocol::Value::Nil` stored as a
    /// present table field — see `coordinator_bridge.rs`'s `value_to_json`.
    /// Use [`Json::obj`] instead when `null` genuinely is a meaningful,
    /// present value on the wire (e.g. round-tripping a real
    /// `protocol::Value::Nil` payload) rather than a stand-in for "this
    /// optional field has no value."
    #[must_use]
    pub fn obj_omit_null(fields: Vec<(&str, Json)>) -> Json {
        Json::Object(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    /// An array.
    #[must_use]
    pub fn arr(items: Vec<Json>) -> Json {
        Json::Array(items)
    }

    /// Looks up a string-keyed field on an object; `None` if this is not an
    /// object or the key is absent.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Borrows this value as a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    /// Reads this value as `f64`.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// Reads this value as `i64` (truncating; every caller of this bridge
    /// passes integral numbers through the fields this is used on).
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().map(|value| value as i64)
    }

    /// Reads this value as `bool`.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// Borrows this value as an array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// True for [`Json::Null`].
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// A field lookup that also treats a missing key or an explicit `null`
    /// as absent — the common case for an optional field crossing the wasm
    /// boundary as JSON.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Json> {
        self.get(key).filter(|value| !value.is_null())
    }

    /// [`Json::field`] plus [`Json::as_str`].
    #[must_use]
    pub fn field_str(&self, key: &str) -> Option<&str> {
        self.field(key).and_then(Json::as_str)
    }

    /// [`Json::field`] plus [`Json::as_i64`].
    #[must_use]
    pub fn field_i64(&self, key: &str) -> Option<i64> {
        self.field(key).and_then(Json::as_i64)
    }

    /// [`Json::field`] plus [`Json::as_bool`].
    #[must_use]
    pub fn field_bool(&self, key: &str) -> Option<bool> {
        self.field(key).and_then(Json::as_bool)
    }

    /// Serializes this value to canonical (compact, no insignificant
    /// whitespace) JSON text.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out);
        out
    }

    /// Parses `text` as JSON. Returns `Err` for anything malformed — this is
    /// external input crossing the wasm boundary, so a parse failure is an
    /// expected, recoverable error (README rule 5.5), never a panic.
    pub fn parse(text: &str) -> Result<Json, String> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            index: 0,
        };
        parser.skip_whitespace();
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.index != parser.bytes.len() {
            return Err("trailing bytes after JSON value".to_string());
        }
        Ok(value)
    }
}

/// Converts a PascalCase `Debug` tag (a Rust unit-only enum variant name) to
/// `snake_case`. Shared by every bridge module in this crate that reads a
/// coordinator-local/match-driver-local/rollback-event-local enum with no
/// hand-written `wire_str` of its own (`gc_netcode::coordinator`'s
/// `Disposition`/`RejectCode`/`TerminalReason`/..., `gc_sim::rollback_events`'s
/// payload-kind enums, ...) — see `coordinator_bridge`'s module doc for why
/// `Debug` plus this conversion, rather than a hand-written `wire_str` for
/// every one of them, is the right amount of code for presentation JSON.
#[must_use]
pub(crate) fn snake_case(pascal: &str) -> String {
    let mut out = String::with_capacity(pascal.len() + 4);
    for (index, ch) in pascal.chars().enumerate() {
        if ch.is_uppercase() {
            if index != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// [`snake_case`] applied to `value`'s `Debug` output. Only meaningful for a
/// unit-only enum (a struct-variant's `Debug` output includes its fields,
/// which this does not attempt to parse back out) — every call site in this
/// crate applies it to exactly that shape.
#[must_use]
pub(crate) fn debug_tag<T: std::fmt::Debug>(value: &T) -> String {
    snake_case(&format!("{value:?}"))
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn write_number(value: f64, out: &mut String) {
    if value.is_finite() {
        if value == value.trunc() && value.abs() < 1e15 {
            let _ = write!(out, "{}", value as i64);
        } else {
            let _ = write!(out, "{value}");
        }
    } else {
        // JSON has no NaN/Infinity literal; this crate never intentionally
        // emits one, but a malformed upstream f64 must still serialize to
        // something a JSON parser accepts rather than corrupt the document.
        out.push_str("null");
    }
}

fn write_value(value: &Json, out: &mut String) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Json::Number(number) => write_number(*number, out),
        Json::String(text) => write_string(text, out),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Json::Object(fields) => {
            out.push('{');
            for (index, (key, item)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(item, out);
            }
            out.push('}');
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.index += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.index += 1;
            Ok(())
        } else {
            Err(format!(
                "expected '{}' at byte {}",
                byte as char, self.index
            ))
        }
    }

    fn expect_literal(&mut self, literal: &str) -> Result<(), String> {
        let bytes = literal.as_bytes();
        if self.bytes[self.index..].starts_with(bytes) {
            self.index += bytes.len();
            Ok(())
        } else {
            Err(format!("expected `{literal}` at byte {}", self.index))
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => self.parse_string().map(Json::String),
            Some(b't') => {
                self.expect_literal("true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.expect_literal("false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.expect_literal("null")?;
                Ok(Json::Null)
            }
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.parse_number(),
            Some(byte) => Err(format!(
                "unexpected byte '{}' at {}",
                byte as char, self.index
            )),
            None => Err("unexpected end of JSON input".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.index += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b'}') => {
                    self.index += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.index)),
            }
        }
        Ok(Json::Object(fields))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.index += 1;
            return Ok(Json::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b']') => {
                    self.index += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.index)),
            }
        }
        Ok(Json::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| "unterminated JSON string".to_string())?;
            match byte {
                b'"' => {
                    self.index += 1;
                    break;
                }
                b'\\' => {
                    self.index += 1;
                    let escape = self
                        .peek()
                        .ok_or_else(|| "unterminated JSON string escape".to_string())?;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            self.index += 1;
                            let code = self.parse_hex4()?;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                            continue;
                        }
                        other => {
                            return Err(format!(
                                "invalid JSON string escape '\\{}'",
                                other as char
                            ));
                        }
                    }
                    self.index += 1;
                }
                _ => {
                    // Bytes are re-scanned as UTF-8 below the ASCII fast
                    // path; this crate only ever sees well-formed UTF-8 JSON
                    // text (it comes from `JSON.stringify` on the JS side),
                    // so decoding one full `char` at a time here is enough.
                    let rest = std::str::from_utf8(&self.bytes[self.index..])
                        .map_err(|_| "invalid UTF-8 in JSON string".to_string())?;
                    let ch = rest
                        .chars()
                        .next()
                        .ok_or_else(|| "unterminated JSON string".to_string())?;
                    out.push(ch);
                    self.index += ch.len_utf8();
                }
            }
        }
        Ok(out)
    }

    fn parse_hex4(&mut self) -> Result<u32, String> {
        if self.index + 4 > self.bytes.len() {
            return Err("truncated \\u escape in JSON string".to_string());
        }
        let text = std::str::from_utf8(&self.bytes[self.index..self.index + 4])
            .map_err(|_| "invalid \\u escape in JSON string".to_string())?;
        let code = u32::from_str_radix(text, 16)
            .map_err(|_| "invalid \\u escape in JSON string".to_string())?;
        self.index += 4;
        Ok(code)
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
            self.index += 1;
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.index += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.index += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index])
            .expect("number bytes are ASCII by construction");
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|_| format!("invalid JSON number '{text}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_scalar_kind() {
        let value = Json::obj(vec![
            ("a", Json::Null),
            ("b", Json::Bool(true)),
            ("c", Json::int(-42)),
            ("d", Json::Number(1.5)),
            ("e", Json::str("hello \"world\"\n")),
            (
                "f",
                Json::arr(vec![Json::int(1), Json::int(2), Json::int(3)]),
            ),
        ]);
        let text = value.to_json_string();
        let parsed = Json::parse(&text).expect("round-trip parses");
        assert_eq!(parsed, value);
    }

    #[test]
    fn parses_nested_objects_and_arrays() {
        let text = r#"{"peers":[{"id":"a","ready":true},{"id":"b","ready":false}],"count":2}"#;
        let parsed = Json::parse(text).expect("parses");
        let peers = parsed.get("peers").unwrap().as_array().unwrap();
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].get("id").unwrap().as_str(), Some("a"));
        assert_eq!(peers[1].field_bool("ready"), Some(false));
        assert_eq!(parsed.field_i64("count"), Some(2));
    }

    #[test]
    fn obj_omit_null_drops_null_valued_pairs_but_keeps_everything_else() {
        let value = Json::obj_omit_null(vec![
            ("present", Json::int(1)),
            ("absent", Json::Null),
            ("also_present", Json::bool(false)),
        ]);
        let text = value.to_json_string();
        assert_eq!(text, r#"{"present":1,"also_present":false}"#);
        let parsed = Json::parse(&text).unwrap();
        // The whole point: `field()`/a JS `!== undefined` check sees the key
        // itself missing, not present with a `null` value.
        assert!(parsed.get("absent").is_none());
        assert_eq!(parsed.field_i64("present"), Some(1));
    }

    #[test]
    fn field_treats_missing_and_null_alike() {
        let value = Json::parse(r#"{"a":null}"#).unwrap();
        assert!(value.field("a").is_none());
        assert!(value.field("b").is_none());
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(Json::parse("{}garbage").is_err());
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(Json::parse("{").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse("\"unterminated").is_err());
    }

    #[test]
    fn decodes_unicode_escapes() {
        let parsed = Json::parse(r#""café""#).unwrap();
        assert_eq!(parsed.as_str(), Some("café"));
    }
}
