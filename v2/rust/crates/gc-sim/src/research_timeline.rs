//! Port of `sim/research_timeline.lua`.
//!
//! Canonical confirmed-event stream and annotation timeline for research
//! exports.
//!
//! Rollback makes "the events that happened" a function of confirmation,
//! not of presentation. `sim/rollback_events.lua` already separates a
//! speculative window from confirmed steps and reports revocations; this
//! module is the research-facing projection of that fact:
//!
//!   * only events delivered by `rollback_events.confirm` become export
//!     rows;
//!   * an event id that a correction revoked can never appear in an export,
//!     and a revocation of an already-confirmed id fails closed; and
//!   * render-time and wall-clock cues are mapped onto the nearest
//!     canonical boundary with the mapping error retained, never silently
//!     snapped.
//!
//! ## The `rollback_events` view types (README §5.1)
//!
//! This module reads `RollbackWrappedEvent`/`RollbackEventDiff`/
//! `RollbackEventStep`-shaped data (only their field *shapes*, via
//! `event.id`/`event.domain`/`event.tick`/`event.ordinal`, `diff.added`/
//! `.replaced`/`.revoked`, `step.tick`/`.start_boundary`/`.end_boundary`/
//! `.match_events`/`.combat_events`/`.lifecycle_events` in the Lua
//! original), never anything from `rollback_events`'s own resim/replay
//! logic. `sim::rollback_events` (`sim/rollback_events.lua`) is a separate,
//! concurrently-ported module this crate does not depend on here, so
//! [`RollbackEvent`], [`RollbackEventReplacement`], [`RollbackEventDiff`],
//! and [`RollbackEventStep`] below are narrow local views, exactly the
//! precedent the README documents for `metrics`/`bot`/`aerial`'s
//! `MatchStateView` structs. Whoever stabilizes `sim::rollback_events`
//! resolves this the same way §5.1 asks for `sim::match`: either this
//! module switches to the real types, or the views survive as deliberately
//! narrow read interfaces declared once in a shared module.

use crate::fixed_clock;
use crate::input_frame;
use crate::research_schema::{
    self, ResearchField, ResearchFieldKind, ResearchShape, Result, TuplePart, Value,
};

/// Reader version for the event-stream/annotation-set wire shapes.
pub const VERSION: i64 = 1;
/// Serialization versions this reader accepts.
pub const SUPPORTED_VERSIONS: &[i64] = &[1];
/// `manifest_kind` for a confirmed event stream.
pub const STREAM_KIND: &str = "research_event_stream";
/// `manifest_kind` for an annotation set.
pub const ANNOTATION_KIND: &str = "research_annotation_set";
/// Tuple-hash label for the rows-only content hash.
pub const STREAM_LABEL: &str = "confirmed-event-stream/v1";
/// Tuple-hash label for one event id.
pub const EVENT_LABEL: &str = "research-event/v1";

/// Closed schema enum. Ranks are part of the canonical sort key, so they
/// may never be renumbered without a serialization version bump.
#[must_use]
pub fn domain_rank(domain: &str) -> Option<i64> {
    match domain {
        "input" => Some(1),
        "soccer" => Some(2),
        "combat" => Some(3),
        "lifecycle" => Some(4),
        _ => None,
    }
}

/// Declared event domains.
#[must_use]
pub fn domains() -> Vec<String> {
    research_schema::enum_values(&["input", "soccer", "combat", "lifecycle"])
}

// ---------------------------------------------------------------------
// `rollback_events` view types (module doc comment).
// ---------------------------------------------------------------------

/// A local view of `rollback_events.lua`'s `RollbackWrappedEvent`. See the
/// module doc comment.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEvent {
    /// Rollback-assigned event id.
    pub id: String,
    /// `"<group>/<kind>"` or `"<group>/<kind>/<sequence>"`.
    pub domain: String,
    /// Canonical tick the event occurred on.
    pub tick: i64,
    /// Ordinal among same-kind events at this tick.
    pub ordinal: i64,
}

/// A local view of `rollback_events.lua`'s replacement entry (`{ before,
/// after }`).
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventReplacement {
    /// The event being replaced.
    pub before: RollbackEvent,
    /// The event replacing it.
    pub after: RollbackEvent,
}

/// A local view of `rollback_events.lua`'s `RollbackEventDiff`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RollbackEventDiff {
    /// Newly speculative events.
    pub added: Vec<RollbackEvent>,
    /// Speculative events replaced by a correction.
    pub replaced: Vec<RollbackEventReplacement>,
    /// Events a correction revoked.
    pub revoked: Vec<RollbackEvent>,
}

/// A local view of `rollback_events.lua`'s `RollbackEventStep`.
#[derive(Clone, Debug, PartialEq)]
pub struct RollbackEventStep {
    /// The confirmed tick this step covers.
    pub tick: i64,
    /// Rollback boundary this step starts at.
    pub start_boundary: i64,
    /// Rollback boundary this step ends at.
    pub end_boundary: i64,
    /// Soccer-domain events confirmed this tick.
    pub match_events: Vec<RollbackEvent>,
    /// Combat-domain events confirmed this tick (empty when combat is
    /// inactive — mirrors the Lua original's `step.combat_events or {}`).
    pub combat_events: Vec<RollbackEvent>,
    /// Lifecycle-domain events confirmed this tick.
    pub lifecycle_events: Vec<RollbackEvent>,
}

// ---------------------------------------------------------------------
// Shapes
// ---------------------------------------------------------------------

fn row_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Integer)
            .named("canonical_tick")
            .min(0.0)
            .max(input_frame::MAX_TICK as f64),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("domain")
            .values(domains()),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("domain_rank")
            .min(1.0)
            .max(4.0),
        ResearchField::new(ResearchFieldKind::Id).named("event_kind"),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("source_sequence")
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("same_kind_ordinal")
            .min(1.0),
        ResearchField::new(ResearchFieldKind::Hash).named("event_id"),
        ResearchField::new(ResearchFieldKind::Str).named("rollback_event_id"),
        ResearchField::new(ResearchFieldKind::Hash).named("payload_hash"),
    ]
}

/// The `research_event_rows/v1` shape: the subset of [`stream_shape`]
/// covered by [`stream_hash`].
#[must_use]
pub fn rows_shape() -> ResearchShape {
    research_schema::record(
        "research_event_rows/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Hash).named("run_scope_id"),
            ResearchField::new(ResearchFieldKind::Id).named("game_instance_id"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("confirmed_through_tick")
                .min(-1.0)
                .max(input_frame::MAX_TICK as f64),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("confirmed_boundary")
                .min(0.0)
                .max(input_frame::MAX_TICK as f64),
            ResearchField::new(ResearchFieldKind::Array)
                .named("rows")
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("row")
                        .fields(row_fields()),
                ),
        ],
    )
}

/// The `research_event_stream/v1` record shape.
#[must_use]
pub fn stream_shape() -> ResearchShape {
    research_schema::record(
        "research_event_stream/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Integer)
                .named("schema_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("manifest_kind")
                .values(research_schema::enum_values(&[STREAM_KIND])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("digest")
                .values(research_schema::enum_values(&[research_schema::DIGEST])),
            ResearchField::new(ResearchFieldKind::Hash).named("run_scope_id"),
            ResearchField::new(ResearchFieldKind::Id).named("game_instance_id"),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("confirmed_through_tick")
                .min(-1.0)
                .max(input_frame::MAX_TICK as f64),
            ResearchField::new(ResearchFieldKind::Integer)
                .named("confirmed_boundary")
                .min(0.0)
                .max(input_frame::MAX_TICK as f64),
            ResearchField::new(ResearchFieldKind::Array)
                .named("rows")
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("row")
                        .fields(row_fields()),
                ),
            ResearchField::new(ResearchFieldKind::Hash).named("stream_hash"),
        ],
    )
}

fn annotation_fields() -> Vec<ResearchField> {
    vec![
        ResearchField::new(ResearchFieldKind::Id).named("annotation_id"),
        ResearchField::new(ResearchFieldKind::Integer)
            .named("canonical_tick")
            .min(0.0)
            .max(input_frame::MAX_TICK as f64),
        ResearchField::new(ResearchFieldKind::Enum)
            .named("boundary_source")
            .values(research_schema::enum_values(&[
                "canonical_tick",
                "wall_clock_mapped",
                "render_frame_mapped",
            ])),
        ResearchField::new(ResearchFieldKind::Number)
            .named("mapping_error_ms")
            .min(-1000.0)
            .max(1000.0),
        ResearchField::new(ResearchFieldKind::Number)
            .named("wall_clock_ms")
            .optional()
            .min(0.0),
        ResearchField::new(ResearchFieldKind::Hash)
            .named("event_id")
            .optional(),
        ResearchField::new(ResearchFieldKind::Id).named("code_id"),
        ResearchField::new(ResearchFieldKind::Number)
            .named("confidence")
            .optional()
            .min(0.0)
            .max(1.0),
        ResearchField::new(ResearchFieldKind::Id)
            .named("disagreement_group")
            .optional(),
        ResearchField::new(ResearchFieldKind::Text)
            .named("free_text")
            .optional(),
    ]
}

/// The `research_annotation_set/v1` record shape.
#[must_use]
pub fn annotation_set_shape() -> ResearchShape {
    research_schema::record(
        "research_annotation_set/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Integer)
                .named("schema_version")
                .min(1.0),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("manifest_kind")
                .values(research_schema::enum_values(&[ANNOTATION_KIND])),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("digest")
                .values(research_schema::enum_values(&[research_schema::DIGEST])),
            ResearchField::new(ResearchFieldKind::Id).named("annotation_set_id"),
            ResearchField::new(ResearchFieldKind::Hash).named("run_scope_id"),
            ResearchField::new(ResearchFieldKind::Id).named("session_id"),
            ResearchField::new(ResearchFieldKind::Enum)
                .named("author_role")
                .values(research_schema::enum_values(&[
                    "participant",
                    "researcher",
                    "tool",
                ])),
            // Which in-game agreement version the participant accepted for
            // this session.
            ResearchField::new(ResearchFieldKind::Id).named("agreement_version"),
            ResearchField::new(ResearchFieldKind::Id).named("coding_scheme_version"),
            ResearchField::new(ResearchFieldKind::Array)
                .named("annotations")
                .element(
                    ResearchField::new(ResearchFieldKind::Record)
                        .named("annotation")
                        .fields(annotation_fields()),
                ),
        ],
    )
}

fn identity_shape() -> ResearchShape {
    research_schema::record(
        "research_timeline_identity/v1",
        vec![
            ResearchField::new(ResearchFieldKind::Hash).named("run_scope_id"),
            ResearchField::new(ResearchFieldKind::Id).named("game_instance_id"),
        ],
    )
}

fn is_lower_underscore(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn is_signed_int(s: &str) -> bool {
    let core = s.strip_prefix('-').unwrap_or(s);
    !core.is_empty() && core.bytes().all(|b| b.is_ascii_digit())
}

/// Parse a rollback event `domain` string (`"<group>/<kind>"` or
/// `"<group>/<kind>/<sequence>"`) into `(research domain, event kind, source
/// sequence)`, folding the legacy `"match"` group name onto `"soccer"`.
fn split_domain(domain: &str) -> Result<(String, String, i64)> {
    let Some(slash) = domain.find('/') else {
        return Err(format!(
            "rollback event domain {domain} is not a research domain"
        ));
    };
    let group_raw = &domain[..slash];
    let remainder = &domain[slash + 1..];
    if !is_lower_underscore(group_raw) || remainder.is_empty() {
        return Err(format!(
            "rollback event domain {domain} is not a research domain"
        ));
    }
    let group = if group_raw == "match" {
        "soccer".to_string()
    } else {
        group_raw.to_string()
    };
    if domain_rank(&group).is_none() {
        return Err(format!(
            "rollback event domain group {group} is unknown to this reader"
        ));
    }
    if let Some(inner_slash) = remainder.find('/') {
        let kind_part = &remainder[..inner_slash];
        let sequence_part = &remainder[inner_slash + 1..];
        if is_lower_underscore(kind_part) && is_signed_int(sequence_part) {
            let parsed: f64 = sequence_part.parse().map_err(|_| {
                format!("rollback event domain {domain} has a malformed source sequence")
            })?;
            return Ok((group, kind_part.to_string(), parsed.floor() as i64));
        }
    }
    if !is_lower_underscore(remainder) {
        return Err(format!(
            "rollback event domain {domain} has a malformed kind"
        ));
    }
    Ok((group, remainder.to_string(), 0))
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

fn array_field<'a>(entries: &'a [(String, Value)], name: &str) -> &'a [Value] {
    Value::record_get(entries, name)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("validated field {name} missing or not an array"))
}

fn row_precedes(left: &Value, right: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let left = left.as_record().expect("row is a record");
    let right = right.as_record().expect("row is a record");
    let left_tick = number_field(left, "canonical_tick");
    let right_tick = number_field(right, "canonical_tick");
    if left_tick != right_tick {
        return left_tick
            .partial_cmp(&right_tick)
            .unwrap_or(Ordering::Equal);
    }
    let left_rank = number_field(left, "domain_rank");
    let right_rank = number_field(right, "domain_rank");
    if left_rank != right_rank {
        return left_rank
            .partial_cmp(&right_rank)
            .unwrap_or(Ordering::Equal);
    }
    let left_kind = text_field(left, "event_kind");
    let right_kind = text_field(right, "event_kind");
    if left_kind != right_kind {
        return left_kind.cmp(right_kind);
    }
    let left_seq = number_field(left, "source_sequence");
    let right_seq = number_field(right, "source_sequence");
    if left_seq != right_seq {
        return left_seq.partial_cmp(&right_seq).unwrap_or(Ordering::Equal);
    }
    let left_ord = number_field(left, "same_kind_ordinal");
    let right_ord = number_field(right, "same_kind_ordinal");
    if left_ord != right_ord {
        return left_ord.partial_cmp(&right_ord).unwrap_or(Ordering::Equal);
    }
    text_field(left, "event_id").cmp(text_field(right, "event_id"))
}

// ---------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------

/// Confirmed-event accumulator: the speculative/confirmed/revoked
/// bookkeeping [`observe_diff`] and [`confirm`] maintain between rollback
/// corrections, and [`export`] turns into a canonical [`stream_shape`]
/// payload.
#[derive(Clone, Debug)]
pub struct ResearchTimeline {
    /// Content-addressed id of the input tape this timeline covers.
    pub run_scope_id: String,
    /// Which game instance (of a possibly-multi-instance run) this is.
    pub game_instance_id: String,
    /// Last confirmed tick (`-1` before anything is confirmed).
    pub confirmed_tick: i64,
    /// Rollback boundary the next confirmed step must start at.
    pub confirmed_boundary: i64,
    /// Accumulated confirmed rows, unsorted and unvalidated — [`export`]
    /// produces the canonical, validated, sorted view. `pub` so a test can
    /// exercise the "duplicate total key" failure path directly, mirroring
    /// the Lua spec's `timeline._rows[#timeline._rows + 1] = ...`.
    pub rows: Vec<Value>,
    /// Ids of events already promoted to confirmed rows.
    pub confirmed: Vec<String>,
    /// Ids of events currently in the speculative window.
    pub speculative: Vec<String>,
    /// Ids of events a rollback correction revoked.
    pub revoked: Vec<String>,
}

/// Construct a new timeline, validating the identity fields.
pub fn new(
    run_scope_id: impl Into<String>,
    game_instance_id: impl Into<String>,
) -> Result<ResearchTimeline> {
    let run_scope_id = run_scope_id.into();
    let game_instance_id = game_instance_id.into();
    research_schema::validate(
        &identity_shape(),
        &Value::Record(vec![
            ("run_scope_id".to_string(), Value::str(run_scope_id.clone())),
            (
                "game_instance_id".to_string(),
                Value::str(game_instance_id.clone()),
            ),
        ]),
    )?;
    Ok(ResearchTimeline {
        run_scope_id,
        game_instance_id,
        confirmed_tick: -1,
        confirmed_boundary: 0,
        rows: Vec::new(),
        confirmed: Vec::new(),
        speculative: Vec::new(),
        revoked: Vec::new(),
    })
}

/// Record what a rollback correction did to the speculative window.
/// Revoked events are remembered so they can never be exported, and
/// revoking an already-confirmed event id is a contract violation rather
/// than a correction.
pub fn observe_diff(timeline: &mut ResearchTimeline, diff: &RollbackEventDiff) -> Result<()> {
    for event in &diff.added {
        if timeline.confirmed.contains(&event.id) {
            return Err(format!(
                "rollback re-added already-confirmed event {}",
                event.id
            ));
        }
        if !timeline.speculative.contains(&event.id) {
            timeline.speculative.push(event.id.clone());
        }
        timeline.revoked.retain(|id| id != &event.id);
    }
    for replacement in &diff.replaced {
        let id = &replacement.after.id;
        if timeline.confirmed.contains(id) {
            return Err(format!("rollback replaced already-confirmed event {id}"));
        }
        if !timeline.speculative.contains(id) {
            timeline.speculative.push(id.clone());
        }
        timeline.revoked.retain(|existing| existing != id);
    }
    for event in &diff.revoked {
        if timeline.confirmed.contains(&event.id) {
            return Err(format!(
                "rollback revoked already-confirmed event {}",
                event.id
            ));
        }
        timeline.speculative.retain(|id| id != &event.id);
        if !timeline.revoked.contains(&event.id) {
            timeline.revoked.push(event.id.clone());
        }
    }
    Ok(())
}

fn make_row(timeline: &ResearchTimeline, event: &RollbackEvent) -> Result<Value> {
    let (group, kind, sequence) = split_domain(&event.domain)?;
    if sequence < 0 {
        return Err(format!(
            "rollback event {} has a negative source sequence",
            event.id
        ));
    }
    let payload_shape_hash = research_schema::tuple_hash(
        "research-event-payload/v1",
        &[
            TuplePart::Text(event.id.clone()),
            TuplePart::Text(event.domain.clone()),
            TuplePart::Number(event.tick as f64),
            TuplePart::Number(event.ordinal as f64),
        ],
    );
    let event_id = research_schema::tuple_hash(
        EVENT_LABEL,
        &[
            TuplePart::Text(timeline.run_scope_id.clone()),
            TuplePart::Text(timeline.game_instance_id.clone()),
            TuplePart::Number(event.tick as f64),
            TuplePart::Text(group.clone()),
            TuplePart::Text(kind.clone()),
            TuplePart::Number(sequence as f64),
            TuplePart::Number(event.ordinal as f64),
        ],
    );
    Ok(Value::Record(vec![
        (
            "canonical_tick".to_string(),
            Value::Number(event.tick as f64),
        ),
        ("domain".to_string(), Value::str(group.clone())),
        (
            "domain_rank".to_string(),
            Value::Number(domain_rank(&group).expect("group already validated") as f64),
        ),
        ("event_kind".to_string(), Value::str(kind)),
        (
            "source_sequence".to_string(),
            Value::Number(sequence as f64),
        ),
        (
            "same_kind_ordinal".to_string(),
            Value::Number(event.ordinal as f64),
        ),
        ("event_id".to_string(), Value::str(event_id)),
        (
            "rollback_event_id".to_string(),
            Value::str(event.id.clone()),
        ),
        ("payload_hash".to_string(), Value::str(payload_shape_hash)),
    ]))
}

/// Promote confirmed rollback steps into export rows. Returns the number of
/// rows added.
pub fn confirm(timeline: &mut ResearchTimeline, steps: &[RollbackEventStep]) -> Result<i64> {
    let mut added = 0i64;
    for (index, step) in steps.iter().enumerate() {
        if step.tick != timeline.confirmed_tick + 1 {
            return Err(format!(
                "confirmed step {} is noncontiguous with the research timeline",
                index + 1
            ));
        }
        if step.start_boundary != timeline.confirmed_boundary {
            return Err(format!(
                "confirmed step {} does not start at the confirmed boundary",
                index + 1
            ));
        }
        let groups: [&[RollbackEvent]; 3] = [
            &step.match_events,
            &step.combat_events,
            &step.lifecycle_events,
        ];
        for events in groups {
            for event in events {
                if timeline.revoked.contains(&event.id) {
                    return Err(format!(
                        "confirmed step contains revoked event {}",
                        event.id
                    ));
                }
                if timeline.confirmed.contains(&event.id) {
                    return Err(format!("confirmed step repeats event {}", event.id));
                }
                let row = make_row(timeline, event)?;
                timeline.confirmed.push(event.id.clone());
                timeline.speculative.retain(|id| id != &event.id);
                timeline.rows.push(row);
                added += 1;
            }
        }
        timeline.confirmed_tick = step.tick;
        timeline.confirmed_boundary = step.end_boundary;
    }
    Ok(added)
}

fn rows_value(stream_entries: &[(String, Value)]) -> Value {
    Value::Record(vec![
        (
            "run_scope_id".to_string(),
            Value::str(text_field(stream_entries, "run_scope_id")),
        ),
        (
            "game_instance_id".to_string(),
            Value::str(text_field(stream_entries, "game_instance_id")),
        ),
        (
            "confirmed_through_tick".to_string(),
            Value::Number(number_field(stream_entries, "confirmed_through_tick")),
        ),
        (
            "confirmed_boundary".to_string(),
            Value::Number(number_field(stream_entries, "confirmed_boundary")),
        ),
        (
            "rows".to_string(),
            Value::Array(array_field(stream_entries, "rows").to_vec()),
        ),
    ])
}

/// Content hash of a stream's rows-only projection.
pub fn stream_hash(stream: &Value) -> Result<String> {
    let entries = stream.as_record().expect("stream is a record");
    research_schema::content_hash(&rows_shape(), &rows_value(entries))
}

/// Materialize the canonical confirmed stream. Fails closed if any revoked
/// or still-speculative event reached the rows, or if two rows share a
/// total key.
pub fn export(timeline: &ResearchTimeline) -> Result<Value> {
    let mut rows: Vec<Value> = Vec::with_capacity(timeline.rows.len());
    for row in &timeline.rows {
        let row_entries = row.as_record().expect("row is a record");
        let rollback_event_id = text_field(row_entries, "rollback_event_id");
        if timeline.revoked.iter().any(|id| id == rollback_event_id) {
            return Err(format!(
                "research export contains revoked event {rollback_event_id}"
            ));
        }
        if timeline
            .speculative
            .iter()
            .any(|id| id == rollback_event_id)
        {
            return Err(format!(
                "research export contains speculative event {rollback_event_id}"
            ));
        }
        rows.push(row.clone());
    }
    rows.sort_by(row_precedes);
    let mut seen: Vec<String> = Vec::new();
    for row in &rows {
        let entries = row.as_record().unwrap();
        let key = format!(
            "{}/{}/{}/{}/{}",
            number_field(entries, "canonical_tick"),
            number_field(entries, "domain_rank"),
            text_field(entries, "event_kind"),
            number_field(entries, "source_sequence"),
            number_field(entries, "same_kind_ordinal"),
        );
        if seen.contains(&key) {
            return Err(format!("research export has a duplicate total key {key}"));
        }
        seen.push(key);
    }
    let mut stream_entries: Vec<(String, Value)> = vec![
        ("schema_version".to_string(), Value::Number(VERSION as f64)),
        ("manifest_kind".to_string(), Value::str(STREAM_KIND)),
        ("digest".to_string(), Value::str(research_schema::DIGEST)),
        (
            "run_scope_id".to_string(),
            Value::str(timeline.run_scope_id.clone()),
        ),
        (
            "game_instance_id".to_string(),
            Value::str(timeline.game_instance_id.clone()),
        ),
        (
            "confirmed_through_tick".to_string(),
            Value::Number(timeline.confirmed_tick as f64),
        ),
        (
            "confirmed_boundary".to_string(),
            Value::Number(timeline.confirmed_boundary as f64),
        ),
        ("rows".to_string(), Value::Array(rows)),
        ("stream_hash".to_string(), Value::str("0000000000000000")),
    ];
    let stream = Value::Record(stream_entries.clone());
    let hash = stream_hash(&stream)?;
    if let Some(entry) = stream_entries.iter_mut().find(|(k, _)| k == "stream_hash") {
        entry.1 = Value::str(hash);
    }
    let stream = Value::Record(stream_entries);
    validate_stream(&stream)?;
    Ok(stream)
}

/// Validate a confirmed event stream against [`stream_shape`] plus its
/// ordering and hash invariants.
pub fn validate_stream(stream: &Value) -> Result<()> {
    let entries_for_version = stream
        .as_record()
        .ok_or_else(|| "research event stream must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(STREAM_KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&stream_shape(), stream)?;
    let entries = stream.as_record().expect("validated record");

    let confirmed_boundary = number_field(entries, "confirmed_boundary");
    let confirmed_through_tick = number_field(entries, "confirmed_through_tick");
    if confirmed_boundary != confirmed_through_tick + 1.0 {
        return Err(
            "research_event_stream confirmed boundary must follow its confirmed tick".to_string(),
        );
    }
    let rows = array_field(entries, "rows");
    let mut previous: Option<&Value> = None;
    for (index, row) in rows.iter().enumerate() {
        let row_entries = row.as_record().unwrap();
        let domain = text_field(row_entries, "domain");
        let domain_rank_value = number_field(row_entries, "domain_rank");
        if Some(domain_rank_value as i64) != domain_rank(domain) {
            return Err(format!(
                "research_event_stream.rows.{} has a wrong domain rank",
                index + 1
            ));
        }
        if number_field(row_entries, "canonical_tick") > confirmed_through_tick {
            return Err(format!(
                "research_event_stream.rows.{} is not confirmed",
                index + 1
            ));
        }
        if let Some(previous) = previous
            && row_precedes(previous, row) != std::cmp::Ordering::Less
        {
            return Err(format!(
                "research_event_stream.rows.{} breaks the canonical order",
                index + 1
            ));
        }
        previous = Some(row);
    }
    let expected = stream_hash(stream)?;
    if expected != text_field(entries, "stream_hash") {
        return Err("research_event_stream.stream_hash does not cover its rows".to_string());
    }
    Ok(())
}

/// A wall clock's mapping onto the canonical tick space, replicated from
/// `ResearchBoundaryClock`'s LuaCATS annotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResearchBoundaryClock {
    /// Wall-clock time of `first_boundary_tick`, in milliseconds.
    pub origin_wall_clock_ms: f64,
    /// Simulation tick rate this clock was recorded at.
    pub tick_rate: f64,
    /// First boundary this clock's window covers.
    pub first_boundary_tick: i64,
    /// Last boundary this clock's window covers.
    pub last_boundary_tick: i64,
    /// Pause, goal replay, and menu wall time before the cue.
    pub non_simulated_ms: f64,
}

/// One cue mapped onto the canonical boundary space.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchBoundaryMapping {
    /// The nearest canonical tick.
    pub canonical_tick: i64,
    /// Signed cue minus boundary, in wall-clock milliseconds.
    pub mapping_error_ms: f64,
    /// How this mapping was produced.
    pub boundary_source: String,
}

/// Map a wall-clock or render-time cue to the nearest canonical boundary
/// and keep the signed mapping error. A cue outside the trace window fails
/// closed rather than clamping silently.
pub fn map_to_boundary(
    clock: &ResearchBoundaryClock,
    wall_clock_ms: f64,
    boundary_source: Option<&str>,
) -> Result<ResearchBoundaryMapping> {
    if clock.origin_wall_clock_ms < 0.0 {
        return Err(
            "research boundary clock needs a non-negative origin_wall_clock_ms".to_string(),
        );
    }
    if clock.tick_rate < 1.0 {
        return Err("research boundary clock needs a tick_rate of at least 1".to_string());
    }
    if clock.first_boundary_tick < 0 {
        return Err("research boundary clock needs a non-negative first_boundary_tick".to_string());
    }
    if clock.last_boundary_tick < 0 {
        return Err("research boundary clock needs a non-negative last_boundary_tick".to_string());
    }
    if clock.non_simulated_ms < 0.0 {
        return Err("research boundary clock needs a non-negative non_simulated_ms".to_string());
    }
    if !wall_clock_ms.is_finite() {
        return Err(
            "research boundary cue must be a finite wall-clock millisecond value".to_string(),
        );
    }
    if clock.last_boundary_tick < clock.first_boundary_tick {
        return Err("research boundary clock window is empty".to_string());
    }
    if clock.tick_rate != fixed_clock::TICK_RATE {
        return Err("research boundary clock tick rate is unsupported".to_string());
    }
    let ms_per_tick = 1000.0 / clock.tick_rate;
    let simulated_ms = wall_clock_ms - clock.origin_wall_clock_ms - clock.non_simulated_ms;
    let raw_tick = clock.first_boundary_tick as f64 + simulated_ms / ms_per_tick;
    if raw_tick < clock.first_boundary_tick as f64 - 0.5
        || raw_tick > clock.last_boundary_tick as f64 + 0.5
    {
        return Err("research boundary cue lies outside the trace window".to_string());
    }
    let mut nearest = (raw_tick + 0.5).floor() as i64;
    if nearest < clock.first_boundary_tick {
        nearest = clock.first_boundary_tick;
    } else if nearest > clock.last_boundary_tick {
        nearest = clock.last_boundary_tick;
    }
    Ok(ResearchBoundaryMapping {
        canonical_tick: nearest,
        mapping_error_ms: (raw_tick - nearest as f64) * ms_per_tick,
        boundary_source: boundary_source.unwrap_or("wall_clock_mapped").to_string(),
    })
}

/// Validate an annotation set against [`annotation_set_shape`] plus its
/// cross-field invariants.
pub fn validate_annotation_set(set: &Value) -> Result<()> {
    let entries_for_version = set
        .as_record()
        .ok_or_else(|| "research annotation set must be a table".to_string())?;
    let schema_version = Value::record_get(entries_for_version, "schema_version");
    research_schema::accepts_version(ANNOTATION_KIND, SUPPORTED_VERSIONS, VERSION, schema_version)?;
    research_schema::validate(&annotation_set_shape(), set)?;
    let entries = set.as_record().expect("validated record");
    let author_role = text_field(entries, "author_role");

    let mut seen: Vec<&str> = Vec::new();
    let annotations = array_field(entries, "annotations");
    for (index, annotation) in annotations.iter().enumerate() {
        let path = format!("research_annotation_set.annotations.{}", index + 1);
        let annotation_entries = annotation.as_record().unwrap();
        let annotation_id = text_field(annotation_entries, "annotation_id");
        if seen.contains(&annotation_id) {
            return Err(format!("{path} duplicates an annotation id"));
        }
        seen.push(annotation_id);
        let boundary_source = text_field(annotation_entries, "boundary_source");
        if boundary_source == "canonical_tick" {
            if number_field(annotation_entries, "mapping_error_ms") != 0.0 {
                return Err(format!(
                    "{path} canonical cues cannot carry a mapping error"
                ));
            }
        } else if Value::record_get(annotation_entries, "wall_clock_ms").is_none() {
            return Err(format!(
                "{path} mapped cues must retain their wall-clock source"
            ));
        }
        if Value::record_get(annotation_entries, "free_text").is_some() && author_role == "tool" {
            return Err(format!(
                "{path} free text must come from a participant or a researcher"
            ));
        }
    }
    Ok(())
}

/// Orphan-join and withdrawal guard for the annotation side, mirroring
/// `research_response::validate_against_session`. `envelope` is a
/// [`crate::research_session`]-shaped [`Value`].
pub fn validate_against_session(set: &Value, envelope: &Value) -> Result<()> {
    validate_annotation_set(set)?;
    let entries = set.as_record().expect("validated record");
    let envelope_entries = envelope
        .as_record()
        .ok_or_else(|| "research session envelope is required".to_string())?;
    let envelope_session_id = opt_text_field(envelope_entries, "session_id")
        .ok_or_else(|| "research session envelope is required".to_string())?;
    if text_field(entries, "session_id") != envelope_session_id {
        return Err("research_annotation_set.session_id is an orphan join".to_string());
    }
    let lifecycle = Value::record_get(envelope_entries, "lifecycle")
        .and_then(Value::as_record)
        .expect("validated envelope lifecycle");
    if text_field(lifecycle, "status") == "withdrawn" {
        return Err("research_annotation_set belongs to a withdrawn session".to_string());
    }
    let agreement = Value::record_get(envelope_entries, "agreement")
        .and_then(Value::as_record)
        .expect("validated envelope agreement");
    if text_field(entries, "agreement_version") != text_field(agreement, "agreement_version") {
        return Err(
            "research_annotation_set.agreement_version disagrees with the accepted agreement"
                .to_string(),
        );
    }
    Ok(())
}

/// Join annotation sets onto one confirmed stream. Many sets may reference
/// the same trace; none of them may reference an event or boundary the
/// stream does not contain.
pub fn join_annotations(stream: &Value, sets: &[Value]) -> Result<()> {
    validate_stream(stream)?;
    let stream_entries = stream.as_record().expect("validated record");
    let stream_run_scope_id = text_field(stream_entries, "run_scope_id");
    let confirmed_through_tick = number_field(stream_entries, "confirmed_through_tick");
    let mut event_ids: Vec<&str> = Vec::new();
    for row in array_field(stream_entries, "rows") {
        event_ids.push(text_field(row.as_record().unwrap(), "event_id"));
    }

    let mut set_ids: Vec<&str> = Vec::new();
    let mut annotation_keys: Vec<String> = Vec::new();
    for (index, set) in sets.iter().enumerate() {
        validate_annotation_set(set)?;
        let entries = set.as_record().expect("validated record");
        if text_field(entries, "run_scope_id") != stream_run_scope_id {
            return Err(format!(
                "research annotation set {} references another trace",
                index + 1
            ));
        }
        let annotation_set_id = text_field(entries, "annotation_set_id");
        if set_ids.contains(&annotation_set_id) {
            return Err(format!(
                "research annotation set {annotation_set_id} is duplicated"
            ));
        }
        set_ids.push(annotation_set_id);
        for (position, annotation) in array_field(entries, "annotations").iter().enumerate() {
            let annotation_entries = annotation.as_record().unwrap();
            let key = format!(
                "{annotation_set_id}/{}",
                text_field(annotation_entries, "annotation_id")
            );
            if annotation_keys.contains(&key) {
                return Err(format!("research annotation {key} is duplicated"));
            }
            annotation_keys.push(key.clone());
            if number_field(annotation_entries, "canonical_tick") > confirmed_through_tick {
                return Err(format!(
                    "research annotation {key} points past the confirmed boundary at position {}",
                    position + 1
                ));
            }
            if let Some(event_id) = opt_text_field(annotation_entries, "event_id")
                && !event_ids.contains(&event_id)
            {
                return Err(format!("research annotation {key} is an orphan join"));
            }
        }
    }
    Ok(())
}
