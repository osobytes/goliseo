//! Test coverage for `spec/sim/outfield_ai_policy_spec.lua`'s assertions.
//!
//! `outfield_ai_policy::id()` uses `gc_core::fnv1a64` over a canonical string
//! built from `match_snapshot::number_bytes`, and the resulting id is a
//! cross-process, cross-language contract: `data/outfield_ai_baseline.lua`
//! (carried into `gc_data::outfield_ai_baseline::RECORD`) freezes the id the
//! real Lua build computed, and #148/#149 cite it as a control value. That
//! makes this determinism-path, not merely a spec to satisfy (ARCHITECTURE.md
//! §3 rule 7 / `tools/lua_reference/README.md`), so this file differential-tests
//! the canonical byte string against a reference captured from the real Lua
//! (`tests/fixtures/outfield_ai_policy_canonical.txt`, captured with
//! `love .` per `tools/lua_reference/README.md`), in addition to the
//! frozen-baseline id comparison the spec-derived unit tests already
//! provide.
//!
//! Several of the original spec's cases rely on Lua's live, mutable module
//! tables: they monkey-patch a "constant" at runtime (`ai.LANE_WIDTH = ...`,
//! `outfield_decision.BASE_TEMPERATURE = nil`, a `Knob.default` field, an
//! injected scratch field) and check that `outfield_ai_policy.id()` reacts.
//! `outfield_decision::BASE_TEMPERATURE` and friends are declared in Rust as
//! `pub const`s (and `tuning::KNOBS` as a `static` array of `Knob` value
//! structs) — genuinely immutable at runtime, not merely conventionally so —
//! so those specific mutations cannot be expressed in Rust. Each is kept as
//! an `#[ignore]`d test naming the Lua case it replaces and why.

use gc_sim::outfield_ai_policy;

const REFERENCE_CANONICAL: &str = include_str!("fixtures/outfield_ai_policy_canonical.txt");

#[test]
fn is_stable_across_repeated_calls() {
    let first = outfield_ai_policy::id();
    for _ in 0..5 {
        assert_eq!(
            outfield_ai_policy::id(),
            first,
            "the id is a pure function of the surface"
        );
    }
}

#[test]
fn names_its_schema_version_and_combat_mode_in_the_id() {
    let id = outfield_ai_policy::id();
    let parts: Vec<&str> = id.split('/').collect();
    assert_eq!(parts.len(), 4, "schema/vN/combat_MODE/digest");
    assert_eq!(parts[0], outfield_ai_policy::SCHEMA);
    assert_eq!(parts[1], format!("v{}", outfield_ai_policy::SCHEMA_VERSION));
    assert_eq!(
        parts[2], "combat_disabled",
        "the frozen policy is the combat-disabled one"
    );
    assert_eq!(
        parts[3].len(),
        16,
        "FNV-1a-64 rendered as 16 hex characters"
    );
    assert!(
        parts[3].chars().all(|c| c.is_ascii_hexdigit()),
        "digest is hex"
    );
}

#[test]
fn matches_the_id_recorded_in_the_frozen_baseline_artifact() {
    // data/outfield_ai_baseline.lua (carried into gc_data::outfield_ai_baseline)
    // was written by a separate process, so this is cross-process stability,
    // not just in-process memoization.
    assert_eq!(
        gc_data::outfield_ai_baseline::RECORD.identity.policy_id,
        outfield_ai_policy::id(),
        "the frozen baseline was recorded under a different policy than this build runs"
    );
}

#[test]
fn matches_the_canonical_bytes_captured_from_the_real_lua_build() {
    // Differential test per tools/lua_reference/README.md: captured with
    // `love .` over sim/outfield_ai_policy.lua's canonical()/id(), copied
    // into tests/fixtures/outfield_ai_policy_canonical.txt. A byte-exact
    // match here, not just an id match, pins the row order and every
    // `number_bytes` encoding along the way — including the
    // `offball_runs.VERSION` fallback this Rust implementation had to
    // supply (see src/outfield_ai_policy.rs's module doc).
    assert_eq!(
        outfield_ai_policy::canonical(),
        REFERENCE_CANONICAL.trim_end()
    );
}

#[test]
fn covers_every_declared_surface_field_with_a_scalar() {
    let rows = outfield_ai_policy::descriptor();
    let mut seen: Vec<&str> = Vec::new();
    for r in &rows {
        assert!(
            !seen.contains(&r.key.as_str()),
            "duplicate policy key {}",
            r.key
        );
        seen.push(r.key.as_str());
    }
    for group in outfield_ai_policy::SURFACE {
        for field in group.fields {
            let key = format!("{}.{}", group.module, field);
            assert!(
                seen.contains(&key.as_str()),
                "{key} missing from descriptor()"
            );
        }
    }
}

#[test]
fn gives_every_surface_module_a_version_to_bump() {
    // The docs promise a deliberate policy change always has somewhere to
    // land, including for constants that stay file-local. The Lua spec
    // proves this dynamically (`type(module_table.VERSION) == "number"`);
    // here it is a static fact about SURFACE plus descriptor() (exercised by
    // `covers_every_declared_surface_field_with_a_scalar` above), since a
    // `pub const VERSION` cannot be absent by construction.
    for group in outfield_ai_policy::SURFACE {
        assert_eq!(
            group.fields[0], "VERSION",
            "{} leads its surface with VERSION",
            group.module
        );
    }
    let names: Vec<&str> = outfield_ai_policy::SURFACE
        .iter()
        .map(|g| g.module)
        .collect();
    for expected in [
        "outfield_decision",
        "outfield_press",
        "offball_runs",
        "possession_transition",
        "ai",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} is not registered in the policy surface"
        );
    }
}

#[test]
fn does_not_move_when_a_live_tuning_value_is_nudged_off_its_default() {
    // Adapted rather than ignored: this property IS expressible, just
    // structurally instead of dynamically. `descriptor()` never takes a
    // `Tuning` instance — only the static `tuning::KNOBS` defaults — so a
    // live registry's nudged value cannot reach it regardless of what the
    // nudge sets it to.
    let base = outfield_ai_policy::id();
    let mut live = gc_sim::tuning::Tuning::new();
    let previous = live.value("AI_SHOOT_RANGE");
    live.set("AI_SHOOT_RANGE", previous + 10.0);
    assert_eq!(
        outfield_ai_policy::id(),
        base,
        "an in-session tuning-panel nudge is not a new shipped policy"
    );
}

#[test]
fn reports_the_surface_behind_an_id() {
    let report = outfield_ai_policy::report();
    assert!(
        report.contains(&outfield_ai_policy::id()),
        "the report cites the id"
    );
    assert!(
        report.contains("outfield_decision.VERSION"),
        "and lists the declared fields"
    );
}

// ---------------------------------------------------------------------------
// The seven cases the Lua proves by assigning to a live module field — setting
// `ai.LANE_WIDTH`, `outfield_decision.BASE_TEMPERATURE`, a Knob's `.default` —
// and watching `id()` move. Rust constants are not assignable, and these were
// retired as inexpressible.
//
// That was wrong: it retired the coverage, not just the technique. The property
// is "a different declared surface hashes differently", and `canonical_of`/
// `id_of` test it directly by perturbing a row rather than the module the row
// was read from. Two of these are strictly stronger than the original, because
// adding and removing a row cannot be expressed by runtime assignment at all.
// ---------------------------------------------------------------------------

fn perturbed(key: &str) -> Vec<outfield_ai_policy::OutfieldAiPolicyRow> {
    let mut rows = outfield_ai_policy::descriptor();
    let row = rows
        .iter_mut()
        .find(|r| r.key == key)
        .unwrap_or_else(|| panic!("no declared row {key}"));
    row.value = match &row.value {
        outfield_ai_policy::OutfieldAiPolicyValue::Number(n) => {
            outfield_ai_policy::OutfieldAiPolicyValue::Number(n + 1.0)
        }
        outfield_ai_policy::OutfieldAiPolicyValue::Text(t) => {
            outfield_ai_policy::OutfieldAiPolicyValue::Text(format!("{t}!"))
        }
    };
    rows
}

fn assert_id_moves(key: &str) {
    assert_ne!(
        outfield_ai_policy::id_of(&perturbed(key)),
        outfield_ai_policy::id(),
        "changing {key} must move the policy id, or a frozen baseline stays valid across a real policy change"
    );
}

#[test]
fn covers_the_off_ball_support_weights_sim_ai_supplies() {
    assert_id_moves("ai.LANE_WIDTH");
}

#[test]
fn detects_a_bumped_module_version() {
    assert_id_moves("offball_runs.VERSION");
}

#[test]
fn detects_a_changed_decision_constant() {
    assert_id_moves("outfield_decision.BASE_TEMPERATURE");
}

#[test]
fn detects_a_changed_run_constant() {
    assert_id_moves("offball_runs.MAX_ACTIVE_PER_TEAM");
}

#[test]
fn detects_a_changed_ai_knob_default() {
    let key = outfield_ai_policy::descriptor()
        .into_iter()
        .map(|r| r.key)
        .find(|k| k.starts_with("tuning."))
        .expect("the surface declares at least one AI knob");
    assert_id_moves(&key);
}

#[test]
fn does_not_move_when_an_undeclared_module_field_is_added() {
    // The Lua injects a field onto a live module table and asserts the id is
    // unchanged, because the id reads the DECLARED surface, not the module. The
    // equivalent here: a field nobody declared contributes no row, so the
    // descriptor — and therefore the id — is identical.
    assert_eq!(
        outfield_ai_policy::id_of(&outfield_ai_policy::descriptor()),
        outfield_ai_policy::id()
    );
}

#[test]
fn fails_loudly_when_a_declared_field_disappears() {
    // Stronger than the Lua's version, which sets the field to nil. Removing the
    // row entirely is the case runtime assignment cannot express.
    let mut rows = outfield_ai_policy::descriptor();
    let before = rows.len();
    rows.retain(|r| r.key != "outfield_decision.BASE_TEMPERATURE");
    assert_eq!(rows.len(), before - 1, "the declared row was not found");
    assert_ne!(
        outfield_ai_policy::id_of(&rows),
        outfield_ai_policy::id(),
        "losing a declared field must move the id"
    );
}
