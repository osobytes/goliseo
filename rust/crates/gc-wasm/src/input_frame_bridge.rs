//! `wasm-bindgen` control surface over `gc_sim::input_frame`'s per-slot
//! sample codec — `packages/online/src/net_diagnostics_fixture.ts`'s
//! `InputFramePort` (`neutralSample`/`newSample`/`EDGE_BITS`), and the
//! `inputProtocol` bridge's own row samples (see
//! [`crate::input_protocol_bridge`]'s doc for how the two connect).
//!
//! ## Why a sample crosses as its own canonical wire string, not a handle
//!
//! [`gc_sim::input_frame::InputSample`] already has a canonical, lossless,
//! ASCII text encoding (`encode_sample`/`decode_sample`: eight lowercase hex
//! characters). `crate::session::Session::step` and
//! `crate::match_driver_bridge::MatchDriverBridge::advance` already treat a
//! sample as exactly this wire string crossing the boundary — neither
//! invents an opaque handle or a JSON object for it. This module keeps that
//! one representation rather than adding a second: [`new_sample`]/
//! [`neutral_sample`] return the wire string, and
//! [`crate::input_protocol_bridge`]'s row rows carry the same string
//! verbatim as their `sample` field. `InputFramePort.newSample`/
//! `neutralSample` declare their return type `unknown` precisely because the
//! fixture that calls them never inspects it, only threads it through to
//! `inputProtocol` — a canonical hex string satisfies that with no
//! additional marshalling code anywhere.
//!
//! The wire is always seven-bit ASCII (hex digits), so no "binary string"
//! convention is needed here — contrast [`crate::input_protocol_bridge`]'s
//! `encode`, whose *packet* wire can carry the general case and therefore
//! crosses as bytes.

use gc_sim::input_frame::{self, EdgeAction, HeldAction, InputSampleOptions};
use wasm_bindgen::prelude::*;

use crate::json::Json;

fn js_err(message: impl Into<String>) -> JsValue {
    JsValue::from_str(&message.into())
}

/// Every [`EdgeAction`] wire name and bit, as JSON — `InputFramePort.EDGE_BITS`.
fn edge_bits_json() -> Json {
    Json::obj(vec![
        ("shoot", Json::int(EdgeAction::Shoot.bit())),
        ("pass", Json::int(EdgeAction::Pass.bit())),
        ("switch", Json::int(EdgeAction::Switch.bit())),
        ("dash", Json::int(EdgeAction::Dash.bit())),
        ("dodge", Json::int(EdgeAction::Dodge.bit())),
        (
            "equipment_pressed",
            Json::int(EdgeAction::EquipmentPressed.bit()),
        ),
        (
            "equipment_released",
            Json::int(EdgeAction::EquipmentReleased.bit()),
        ),
    ])
}

/// Every [`HeldAction`] wire name and bit, as JSON. Not part of
/// `InputFramePort`'s declared shape (only `EDGE_BITS` is), but the same
/// reproduction `gc_sim::input_frame` offers for held actions, at negligible
/// extra cost.
fn held_bits_json() -> Json {
    Json::obj(vec![
        ("shoot", Json::int(HeldAction::Shoot.bit())),
        ("pass", Json::int(HeldAction::Pass.bit())),
        ("sprint", Json::int(HeldAction::Sprint.bit())),
        ("jockey", Json::int(HeldAction::Jockey.bit())),
        ("lob", Json::int(HeldAction::Lob.bit())),
        ("aerial_strike", Json::int(HeldAction::AerialStrike.bit())),
        (
            "aerial_acrobatic",
            Json::int(HeldAction::AerialAcrobatic.bit()),
        ),
        ("equipment", Json::int(HeldAction::Equipment.bit())),
    ])
}

/// `InputFramePort.EDGE_BITS`: every canonical edge-action wire name mapped
/// to its bit, as JSON (`{"shoot": 1, "pass": 2, ...}`).
#[wasm_bindgen(js_name = inputFrameEdgeBitsJson)]
#[must_use]
pub fn input_frame_edge_bits_json() -> String {
    edge_bits_json().to_json_string()
}

/// Every canonical held-action wire name mapped to its bit, as JSON. See
/// [`held_bits_json`]'s doc for why this exists alongside the port's own
/// `EDGE_BITS`.
#[wasm_bindgen(js_name = inputFrameHeldBitsJson)]
#[must_use]
pub fn input_frame_held_bits_json() -> String {
    held_bits_json().to_json_string()
}

/// This build's `gc_sim::input_frame` bounds, as JSON: `version`,
/// `slot_count`, `home_slot_count`, `away_slot_count`, `move_scale`,
/// `max_tick`, `max_wire_bytes`.
#[wasm_bindgen(js_name = inputFrameConstantsJson)]
#[must_use]
pub fn input_frame_constants_json() -> String {
    Json::obj(vec![
        ("version", Json::int(input_frame::VERSION)),
        ("slot_count", Json::int(input_frame::SLOT_COUNT)),
        ("home_slot_count", Json::int(input_frame::HOME_SLOT_COUNT)),
        ("away_slot_count", Json::int(input_frame::AWAY_SLOT_COUNT)),
        ("move_scale", Json::int(input_frame::MOVE_SCALE)),
        ("max_tick", Json::int(input_frame::MAX_TICK)),
        (
            "max_wire_bytes",
            Json::int(input_frame::MAX_WIRE_BYTES as i64),
        ),
    ])
    .to_json_string()
}

/// `InputFramePort.neutralSample`: the all-zero sample's canonical wire —
/// `gc_sim::input_frame::neutral_sample`.
#[wasm_bindgen(js_name = inputFrameNeutralSample)]
#[must_use]
pub fn input_frame_neutral_sample() -> String {
    input_frame::encode_sample(&input_frame::neutral_sample())
        .expect("the neutral sample is always valid")
}

/// `InputFramePort.newSample`: builds a validated sample from optional
/// overrides (unset fields default to zero) and returns its canonical wire.
/// `InputFramePort` itself only ever calls this with `edges` set (see the
/// module doc), but every field `gc_sim::input_frame::InputSampleOptions`
/// accepts is exposed here rather than a narrower signature — a faithful
/// reproduction of `sim.input_frame.new_sample`, not just the one call shape
/// the fixture happens to use today.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if the resulting sample violates a
/// bound (an axis outside `[-127, 127]`, a mask outside its bit range, or an
/// invalid equipment held/edge combination — see
/// `gc_sim::input_frame::validate_sample`).
#[wasm_bindgen(js_name = inputFrameNewSample)]
pub fn input_frame_new_sample(
    move_x: Option<f64>,
    move_y: Option<f64>,
    held: Option<f64>,
    edges: Option<f64>,
) -> Result<String, JsValue> {
    let sample = input_frame::new_sample(InputSampleOptions {
        move_x: move_x.map(|v| v as i64),
        move_y: move_y.map(|v| v as i64),
        held: held.map(|v| v as i64),
        edges: edges.map(|v| v as i64),
    })
    .map_err(|err| js_err(err.to_string()))?;
    input_frame::encode_sample(&sample).map_err(|err| js_err(err.to_string()))
}

/// Decodes `wire` (a canonical `inputFrameNewSample`/`inputFrameNeutralSample`
/// wire) back into its fields, as JSON (`{"move_x", "move_y", "held",
/// "edges"}`). Rejects any wire that is not the canonical encoding of the
/// sample it decodes to — mirrors `gc_sim::input_frame::decode_sample`.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `wire` is not a canonical sample wire.
#[wasm_bindgen(js_name = inputFrameDecodeSampleJson)]
pub fn input_frame_decode_sample_json(wire: &str) -> Result<String, JsValue> {
    let sample = input_frame::decode_sample(wire).map_err(|err| js_err(err.to_string()))?;
    Ok(Json::obj(vec![
        ("move_x", Json::int(sample.move_x)),
        ("move_y", Json::int(sample.move_y)),
        ("held", Json::int(sample.held)),
        ("edges", Json::int(sample.edges)),
    ])
    .to_json_string())
}

/// Quantizes a raw `[-1, 1]` axis into a signed 8-bit value — mirrors
/// `gc_sim::input_frame::quantize_axis`. Kept alongside the sample codec
/// since it is how a real input capture layer would build the `move_x`/
/// `move_y` this module's `inputFrameNewSample` takes.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if `raw_axis` is not finite.
#[wasm_bindgen(js_name = inputFrameQuantizeAxis)]
pub fn input_frame_quantize_axis(raw_axis: f64) -> Result<f64, JsValue> {
    input_frame::quantize_axis(raw_axis)
        .map(|value| value as f64)
        .map_err(|err| js_err(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_sample_is_the_all_zero_wire() {
        assert_eq!(input_frame_neutral_sample(), "7f7f0000");
    }

    #[test]
    fn new_sample_round_trips_through_decode_json() {
        let wire = input_frame_new_sample(Some(10.0), Some(-10.0), Some(3.0), Some(1.0))
            .expect("valid sample");
        let json = Json::parse(&input_frame_decode_sample_json(&wire).unwrap()).unwrap();
        assert_eq!(json.field_i64("move_x"), Some(10));
        assert_eq!(json.field_i64("move_y"), Some(-10));
        assert_eq!(json.field_i64("held"), Some(3));
        assert_eq!(json.field_i64("edges"), Some(1));
    }

    // `new_sample`/`decode_sample_json`'s error paths return `JsValue`,
    // which `wasm_bindgen::JsValue::from_str` cannot construct off the
    // wasm32 target ("function not implemented on non-wasm32 targets",
    // aborting the whole native test process — a hard abort, not a normal
    // panic recoverable by `#[should_panic]`). This mirrors this crate's
    // own established split (`crate::coordinator_bridge`'s module doc,
    // `session.spec.ts`'s `expect(() => session.step("not a wire")).toThrow()`):
    // a `Result<_, JsValue>` error path is exercised from the TypeScript
    // suite, never from a native `#[test]`. `packages/wasm/src/input_frame.spec.ts`
    // covers `inputFrameNewSample` rejecting an out-of-range axis.

    #[test]
    fn edge_bits_json_matches_the_edge_action_enum() {
        let json = Json::parse(&input_frame_edge_bits_json()).unwrap();
        assert_eq!(json.field_i64("shoot"), Some(EdgeAction::Shoot.bit()));
        assert_eq!(
            json.field_i64("equipment_released"),
            Some(EdgeAction::EquipmentReleased.bit())
        );
    }

    #[test]
    fn held_bits_json_matches_the_held_action_enum() {
        let json = Json::parse(&input_frame_held_bits_json()).unwrap();
        assert_eq!(json.field_i64("sprint"), Some(HeldAction::Sprint.bit()));
    }

    #[test]
    fn constants_json_reports_the_canonical_slot_count() {
        let json = Json::parse(&input_frame_constants_json()).unwrap();
        assert_eq!(json.field_i64("slot_count"), Some(input_frame::SLOT_COUNT));
    }
}
