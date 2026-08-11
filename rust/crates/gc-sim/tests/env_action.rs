//! Port of `spec/sim/env_action_spec.lua`.
//!
//! The Lua spec builds its `mask`-family fixtures via `sim.env.reset` /
//! `sim.env.observe`, which need `sim.match` and `sim.combat` — neither
//! ported yet (v2/README §1, task scope for this module explicitly excludes
//! `env.lua`/`env_observation.lua`). `env_action.mask`/`check_mask` only ever
//! read `view.own`/`view.ball`, so every assertion below is preserved by
//! constructing an [`EnvActionView`] with exactly the fields the Lua fixture
//! would have produced, instead of driving a match to produce one. This is
//! on the wire path (`to_sample`/`from_sample` feed the same `InputSample`
//! that crosses rollback resim); see `differential.rs` for the required
//! bit-for-bit comparison against the reference Lua implementation for the
//! `input_frame` primitives this module builds on.

use gc_sim::env_action::{
    self, EnvActionErrorCode, EnvActionView, EnvActionViewBall, EnvActionViewEquipment,
    EnvActionViewSelf, EnvObservationProfile, RawAction, RawTable, RawValue,
};
use gc_sim::input_frame;

fn table(pairs: Vec<(&str, RawValue)>) -> RawTable {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn num(value: f64) -> RawValue {
    RawValue::Number(value)
}

fn boolean(value: bool) -> RawValue {
    RawValue::Bool(value)
}

fn nested(pairs: Vec<(&str, RawValue)>) -> RawValue {
    RawValue::Table(table(pairs))
}

#[test]
fn env_action_validate_normalizes_a_sparse_action_into_the_full_contract() {
    let raw = RawAction::Table(table(vec![
        ("move", nested(vec![("x", num(0.5))])),
        ("held", nested(vec![("sprint", boolean(true))])),
    ]));
    let action = env_action::validate(&raw).unwrap();
    assert_eq!(action.version, env_action::VERSION);
    assert_eq!(action.movement.x, 0.5);
    assert_eq!(action.movement.y, 0.0);
    assert!(action.held.sprint);
    assert!(!action.held.lob);
    assert_eq!(action.edges, Default::default());
}

#[test]
fn env_action_validate_treats_a_neutral_action_as_a_real_no_op() {
    let sample = env_action::to_sample(&env_action::neutral()).unwrap();
    let neutral = input_frame::neutral_sample();
    assert_eq!(sample.move_x, neutral.move_x);
    assert_eq!(sample.move_y, neutral.move_y);
    assert_eq!(sample.held, neutral.held);
    assert_eq!(sample.edges, neutral.edges);
}

#[test]
fn env_action_validate_rejects_malformed_actions_with_machine_readable_reasons() {
    let cases: Vec<(RawAction, EnvActionErrorCode)> = vec![
        (RawAction::Other, EnvActionErrorCode::Malformed),
        (
            RawAction::Table(table(vec![("throttle", num(1.0))])),
            EnvActionErrorCode::Malformed,
        ),
        (
            RawAction::Table(table(vec![("version", num(99.0))])),
            EnvActionErrorCode::Malformed,
        ),
        (
            RawAction::Table(table(vec![(
                "move",
                nested(vec![("x", num(0.0)), ("y", num(0.0)), ("z", num(1.0))]),
            )])),
            EnvActionErrorCode::Malformed,
        ),
        (
            RawAction::Table(table(vec![(
                "move",
                nested(vec![("x", num(f64::INFINITY))]),
            )])),
            EnvActionErrorCode::Malformed,
        ),
        (
            RawAction::Table(table(vec![(
                "move",
                nested(vec![("x", num(1.0)), ("y", num(1.0))]),
            )])),
            EnvActionErrorCode::MoveOutOfRange,
        ),
        (
            RawAction::Table(table(vec![(
                "held",
                nested(vec![("teleport", boolean(true))]),
            )])),
            EnvActionErrorCode::UnknownHeldAction,
        ),
        (
            RawAction::Table(table(vec![("held", nested(vec![("sprint", num(1.0))]))])),
            EnvActionErrorCode::Malformed,
        ),
        (
            RawAction::Table(table(vec![(
                "edges",
                nested(vec![("set_owner", boolean(true))]),
            )])),
            EnvActionErrorCode::UnknownEdgeAction,
        ),
        (
            RawAction::Table(table(vec![(
                "edges",
                nested(vec![("dash", RawValue::Table(table(vec![])))]),
            )])),
            EnvActionErrorCode::Malformed,
        ),
    ];
    for (raw, expected_code) in cases {
        let err = env_action::validate(&raw).expect_err("the action must be rejected");
        assert_eq!(err.code, expected_code);
        assert!(!err.message.is_empty(), "a reason is always supplied");
    }
}

#[test]
fn env_action_validate_accepts_the_unit_disc_boundary() {
    assert!(
        env_action::validate(&RawAction::Table(table(vec![(
            "move",
            nested(vec![("x", num(1.0)), ("y", num(0.0))]),
        )])))
        .is_ok()
    );
    assert!(
        env_action::validate(&RawAction::Table(table(vec![(
            "move",
            nested(vec![("x", num(0.6)), ("y", num(0.8))]),
        )])))
        .is_ok()
    );
}

#[test]
fn env_action_sample_conversion_round_trips_through_the_canonical_quantization() {
    let raw = RawAction::Table(table(vec![
        ("move", nested(vec![("x", num(1.0)), ("y", num(0.0))])),
        (
            "held",
            nested(vec![
                ("sprint", boolean(true)),
                ("lob", boolean(true)),
                ("equipment", boolean(true)),
            ]),
        ),
        (
            "edges",
            nested(vec![
                ("dash", boolean(true)),
                ("equipment_pressed", boolean(true)),
            ]),
        ),
    ]));
    let action = env_action::validate(&raw).unwrap();
    let sample = env_action::to_sample(&action).unwrap();
    assert!(input_frame::is_held(&sample, input_frame::HeldAction::Sprint).unwrap());
    assert!(input_frame::is_held(&sample, input_frame::HeldAction::Lob).unwrap());
    assert!(input_frame::is_held(&sample, input_frame::HeldAction::Equipment).unwrap());
    assert!(!input_frame::is_held(&sample, input_frame::HeldAction::Jockey).unwrap());
    assert!(input_frame::has_edge(&sample, input_frame::EdgeAction::Dash).unwrap());
    assert!(input_frame::has_edge(&sample, input_frame::EdgeAction::EquipmentPressed).unwrap());
    assert!(!input_frame::has_edge(&sample, input_frame::EdgeAction::Switch).unwrap());

    let decoded = env_action::from_sample(&sample).unwrap();
    assert!(decoded.held.sprint);
    assert!(!decoded.held.jockey);
    assert!(decoded.edges.dash);
    let re_sample = env_action::to_sample(&decoded).unwrap();
    assert_eq!(re_sample.held, sample.held);
    assert_eq!(re_sample.move_x, sample.move_x);
}

#[test]
fn env_action_sample_conversion_drops_one_shot_edges_when_an_action_is_held_across_ticks() {
    let raw = RawAction::Table(table(vec![
        ("move", nested(vec![("x", num(-1.0)), ("y", num(0.0))])),
        ("held", nested(vec![("jockey", boolean(true))])),
        ("edges", nested(vec![("dodge", boolean(true))])),
    ]));
    let action = env_action::validate(&raw).unwrap();
    let held = env_action::without_edges(&action);
    assert!(held.held.jockey);
    assert_eq!(held.edges, Default::default());
    assert_eq!(held.movement.x, -1.0);
}

/// A fresh, unconstrained slot: nothing on cooldown, nothing equipped, ball
/// grounded. Mirrors the fixture the Lua spec gets from `env.reset` before
/// any mutation.
fn fresh_view() -> EnvActionView {
    EnvActionView {
        profile: EnvObservationProfile::Representative,
        slot: 1,
        own: EnvActionViewSelf {
            stunned: false,
            header_ready: true,
            tackle_ready: true,
            dodge_ready: true,
            equipment: None,
        },
        ball: EnvActionViewBall { airborne: false },
    }
}

#[test]
fn env_action_mask_derives_legality_from_the_view_alone() {
    let mask = env_action::mask(&fresh_view());
    assert_eq!(mask.version, env_action::VERSION);
    assert_eq!(mask.slot, 1);
    assert!(mask.movement);
    assert!(!mask.privileged);
    assert!(mask.held.shoot);
    assert!(mask.held.pass);
    assert!(mask.held.sprint);
    assert!(mask.edges.dash, "a fresh player may challenge");
    assert!(mask.edges.dodge);
    assert!(
        !mask.edges.switch,
        "fixed-slot routing has no player switch"
    );
    assert!(!mask.held.equipment, "no loadout, no equipment intent");
    assert!(!mask.edges.equipment_pressed);
}

#[test]
fn env_action_mask_closes_gated_intents_while_the_observable_cooldown_runs() {
    let mut view = fresh_view();
    view.own.tackle_ready = false;
    view.own.dodge_ready = false;
    let gated = env_action::mask(&view);
    assert!(!gated.edges.dash);
    assert!(!gated.edges.dodge);

    view.own.tackle_ready = true;
    view.own.dodge_ready = true;
    view.own.stunned = true;
    let stunned = env_action::mask(&view);
    assert!(!stunned.edges.dash);
    assert!(!stunned.edges.dodge);
}

#[test]
fn env_action_mask_opens_aerial_intents_only_while_the_ball_is_airborne() {
    let mut view = fresh_view();
    assert!(!env_action::mask(&view).held.aerial_strike);
    view.ball.airborne = true;
    let airborne = env_action::mask(&view);
    assert!(airborne.held.aerial_strike);
    assert!(airborne.held.aerial_acrobatic);
}

#[test]
fn env_action_mask_gates_equipment_on_the_observable_combat_readiness() {
    let mut view = fresh_view();
    view.own.equipment = Some(EnvActionViewEquipment { ready: true });
    let ready = env_action::mask(&view);
    assert!(ready.held.equipment);
    assert!(ready.edges.equipment_pressed);

    view.own.equipment = Some(EnvActionViewEquipment { ready: false });
    let cooling = env_action::mask(&view);
    assert!(!cooling.edges.equipment_pressed);
    assert!(
        cooling.held.equipment,
        "the equipment still exists, it is just not ready"
    );
}

#[test]
fn env_action_mask_rejects_a_masked_out_intent_with_a_reason_naming_the_slot() {
    let mask = env_action::mask(&fresh_view());

    let switch = RawAction::Table(table(vec![(
        "edges",
        nested(vec![("switch", boolean(true))]),
    )]));
    let switch_action = env_action::validate(&switch).unwrap();
    let err = env_action::check_mask(&switch_action, &mask).expect_err("switch is never legal");
    assert_eq!(err.code, EnvActionErrorCode::UnavailableAction);
    assert!(err.message.contains("fixed-slot routing"));

    let allowed_raw = RawAction::Table(table(vec![
        ("move", nested(vec![("x", num(1.0))])),
        ("edges", nested(vec![("dash", boolean(true))])),
    ]));
    let allowed_action = env_action::validate(&allowed_raw).unwrap();
    assert!(env_action::check_mask(&allowed_action, &mask).unwrap());

    let aerial_raw = RawAction::Table(table(vec![(
        "held",
        nested(vec![("aerial_strike", boolean(true))]),
    )]));
    let aerial_action = env_action::validate(&aerial_raw).unwrap();
    let aerial_err =
        env_action::check_mask(&aerial_action, &mask).expect_err("aerial is not open here");
    assert_eq!(aerial_err.code, EnvActionErrorCode::UnavailableAction);
}
