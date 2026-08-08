//! Port of `sim/combat_identity.lua`.
//!
//! A stable identity string for the combat build/content a match is running:
//! the shared combat rule constants plus every catalogued action family's
//! full definition, plus each fixture player's current loadout/family pair.
//! Two peers with the same identity string are running byte-identical
//! combat content; a divergence here is a content/build skew, not a
//! gameplay desync.

use crate::combat_rules;
use crate::combat_snapshot::{CombatMatchState, action_family_wire_id};
use crate::match_snapshot::number_bytes;
use gc_data::action_families::{self, ActionActivation, ActionContactKind, ActionFamilyId};

const FAMILY_IDS: [ActionFamilyId; 4] = [
    ActionFamilyId::Unarmed,
    ActionFamilyId::Guard,
    ActionFamilyId::LightMelee,
    ActionFamilyId::Ranged,
];

enum Part {
    Nil,
    Bool(bool),
    Number(f64),
    Text(String),
}

fn append(parts: &mut String, value: Part) {
    match value {
        Part::Nil => parts.push_str("z;"),
        Part::Bool(b) => parts.push_str(if b { "b1;" } else { "b0;" }),
        Part::Number(n) => {
            parts.push('n');
            parts.push_str(&number_bytes(n));
            parts.push(';');
        }
        Part::Text(s) => {
            parts.push('s');
            parts.push_str(&s.len().to_string());
            parts.push(':');
            parts.push_str(&s);
            parts.push(';');
        }
    }
}

fn append_str(parts: &mut String, s: &str) {
    append(parts, Part::Text(s.to_string()));
}
fn append_num(parts: &mut String, n: f64) {
    append(parts, Part::Number(n));
}
fn append_opt_num(parts: &mut String, n: Option<f64>) {
    append(parts, n.map_or(Part::Nil, Part::Number));
}
fn append_opt_i(parts: &mut String, n: Option<i64>) {
    append(parts, n.map_or(Part::Nil, |v| Part::Number(v as f64)));
}
fn append_bool(parts: &mut String, b: bool) {
    append(parts, Part::Bool(b));
}
fn append_opt_str(parts: &mut String, s: Option<&str>) {
    append(parts, s.map_or(Part::Nil, |v| Part::Text(v.to_string())));
}

fn activation_wire(v: ActionActivation) -> &'static str {
    match v {
        ActionActivation::Press => "press",
        ActionActivation::Held => "held",
        ActionActivation::HeldRelease => "held_release",
    }
}

fn contact_kind_wire(v: ActionContactKind) -> &'static str {
    match v {
        ActionContactKind::Melee => "melee",
        ActionContactKind::Guard => "guard",
        ActionContactKind::Projectile => "projectile",
    }
}

/// A stable identity string for `combat_state`'s ruleset and content, or
/// `"none"` when there is no combat companion. Same shared combat rules,
/// same catalogued family definitions, same per-player loadout/family
/// pairs — same identity string.
#[must_use]
pub fn for_state(combat_state: Option<&CombatMatchState>) -> String {
    let Some(combat_state) = combat_state else {
        return "none".to_string();
    };
    let mut parts = String::from("GCCI;1;");

    append_str(&mut parts, "MAX_DISABLE_TICKS");
    append_num(&mut parts, combat_rules::MAX_DISABLE_TICKS as f64);
    append_str(&mut parts, "IMMUNITY_TICKS");
    append_num(&mut parts, combat_rules::IMMUNITY_TICKS as f64);
    append_str(&mut parts, "KNOCKBACK_THRESHOLD_PX");
    append_num(&mut parts, combat_rules::KNOCKBACK_THRESHOLD_PX);

    for family_id in FAMILY_IDS {
        let family = action_families::get(family_id);
        append_str(&mut parts, action_family_wire_id(family_id));

        append_str(&mut parts, "id");
        append_str(&mut parts, action_family_wire_id(family.id));
        append_str(&mut parts, "activation");
        append_str(&mut parts, activation_wire(family.activation));
        append_str(&mut parts, "contact_kind");
        append_str(&mut parts, contact_kind_wire(family.contact_kind));
        append_str(&mut parts, "windup_ticks");
        append_num(&mut parts, family.windup_ticks as f64);
        append_str(&mut parts, "active_ticks");
        append_opt_i(&mut parts, family.active_ticks);
        append_str(&mut parts, "held_active");
        append_bool(&mut parts, family.held_active);
        append_str(&mut parts, "recovery_ticks");
        append_num(&mut parts, family.recovery_ticks as f64);
        append_str(&mut parts, "cooldown_ticks");
        append_num(&mut parts, family.cooldown_ticks as f64);
        append_str(&mut parts, "reach_px");
        append_opt_num(&mut parts, family.reach_px);
        append_str(&mut parts, "projectile_speed_px_per_second");
        append_opt_num(&mut parts, family.projectile_speed_px_per_second);
        append_str(&mut parts, "projectile_lifetime_ticks");
        append_opt_i(&mut parts, family.projectile_lifetime_ticks);
        append_str(&mut parts, "front_arc_degrees");
        append_num(&mut parts, family.front_arc_degrees);
        append_str(&mut parts, "movement_multiplier");
        append_num(&mut parts, family.movement_multiplier);
        append_str(&mut parts, "guarded_recoil_px");
        append_num(&mut parts, family.guarded_recoil_px);

        append_str(&mut parts, "unguarded_outcome");
        if let Some(outcome) = &family.unguarded_outcome {
            append_str(&mut parts, "present");
            append_str(&mut parts, "interruption_ticks");
            append_num(&mut parts, outcome.interruption_ticks as f64);
            append_str(&mut parts, "displacement_px");
            append_num(&mut parts, outcome.displacement_px);
            append_str(&mut parts, "ball_spill");
            append_bool(&mut parts, outcome.ball_spill);
        } else {
            append(&mut parts, Part::Nil);
        }
    }

    append_num(&mut parts, combat_state.player_ids.len() as f64);
    for (index, player_id) in combat_state.player_ids.iter().enumerate() {
        let runtime = &combat_state.players[index];
        append_str(&mut parts, player_id);
        append_opt_str(&mut parts, runtime.loadout_id.as_deref());
        append_opt_str(&mut parts, runtime.family_id.map(action_family_wire_id));
    }
    parts
}
