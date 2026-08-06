//! Port of `spec/sim/outfield_ai_policy_spec.lua`.
//!
//! `outfield_ai_policy::id()` uses `gc_core::fnv1a64` over a canonical string
//! built from `match_snapshot::number_bytes`, and the resulting id is a
//! cross-process, cross-language contract: `data/outfield_ai_baseline.lua`
//! (ported to `gc_data::outfield_ai_baseline::RECORD`) freezes the id the real
//! Lua build computed, and #148/#149 cite it as a control value. That makes
//! this determinism-path, not merely a spec to satisfy (README rule 5.9 /
//! `v2/tools/lua_reference/README.md`), so this file differential-tests the
//! canonical byte string against a reference captured from the real Lua
//! (`tests/fixtures/outfield_ai_policy_canonical.txt`, captured with
//! `love .` per `v2/tools/lua_reference/README.md`), in addition to the
//! frozen-baseline id comparison the ported spec already provides.
//!
//! Several of the original spec's cases rely on Lua's live, mutable module
//! tables: they monkey-patch a "constant" at runtime (`ai.LANE_WIDTH = ...`,
//! `outfield_decision.BASE_TEMPERATURE = nil`, a `Knob.default` field, an
//! injected scratch field) and check that `outfield_ai_policy.id()` reacts.
//! `outfield_decision::BASE_TEMPERATURE` and friends are ported as `pub
//! const`s (and `tuning::KNOBS` as a `static` array of `Knob` value structs) —
//! genuinely immutable at runtime, not merely conventionally so — so those
//! specific mutations cannot be expressed in the port. Each is kept as an
//! `#[ignore]`d test naming the Lua case it replaces and why, per
//! `v2/README.md` §4 ("Every assertion must survive... port it as `#[ignore]`
//! ... and report it").

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
    // data/outfield_ai_baseline.lua (ported to gc_data::outfield_ai_baseline)
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
    // Differential test per v2/tools/lua_reference/README.md: captured with
    // `love .` over sim/outfield_ai_policy.lua's canonical()/id(), copied
    // into tests/fixtures/outfield_ai_policy_canonical.txt. A byte-exact
    // match here, not just an id match, pins the row order and every
    // `number_bytes` encoding along the way — including the
    // `offball_runs.VERSION` fallback this port had to supply (see
    // src/outfield_ai_policy.rs's module doc).
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
#[ignore = "retired: Lua case mutates a live module global at runtime (ai.LANE_WIDTH = ...) \
            to prove outfield_ai_policy.id() reacts; ai::LANE_WIDTH is a pub const in this \
            port and cannot be mutated, so the mutation itself is inexpressible. The \
            invariant it was probing -- that this field is live in the hashed id -- is \
            covered by covers_every_declared_surface_field_with_a_scalar (proves \
            ai.LANE_WIDTH is a descriptor() row) and \
            matches_the_canonical_bytes_captured_from_the_real_lua_build (proves \
            canonical()'s current LANE_WIDTH value round-trips byte-exact against the real \
            Lua build), both above in this file: descriptor() reads the constant directly, \
            so any change to it necessarily changes canonical()/id() by construction -- \
            there is no separate runtime reactivity path left to exercise."]
fn covers_the_off_ball_support_weights_sim_ai_supplies() {}

#[test]
#[ignore = "retired: Lua case mutates offball_runs.VERSION at runtime to prove \
            outfield_ai_policy.id() reacts. offball_runs::VERSION now exists in this port \
            (src/outfield_ai_policy.rs's OFFBALL_RUNS_VERSION comment: it was a dropped \
            constant when this file was first written, not a design choice, and has since \
            been added) as a pub const, so -- like every other constant in this file -- it \
            cannot be mutated at runtime. Covered the same way as LANE_WIDTH above: \
            covers_every_declared_surface_field_with_a_scalar and \
            matches_the_canonical_bytes_captured_from_the_real_lua_build together prove \
            offball_runs.VERSION is a live descriptor() row baked into canonical()/id()."]
fn detects_a_bumped_module_version() {}

#[test]
#[ignore = "retired: Lua case sets outfield_decision.BASE_TEMPERATURE = previous + 1 at \
            runtime to prove outfield_ai_policy.id() reacts; \
            outfield_decision::BASE_TEMPERATURE is a pub const in this port and cannot be \
            mutated. Covered the same way as LANE_WIDTH above: \
            covers_every_declared_surface_field_with_a_scalar and \
            matches_the_canonical_bytes_captured_from_the_real_lua_build together prove \
            outfield_decision.BASE_TEMPERATURE is a live descriptor() row baked into \
            canonical()/id()."]
fn detects_a_changed_decision_constant() {}

#[test]
#[ignore = "retired: Lua case mutates offball_runs.MAX_ACTIVE_PER_TEAM at runtime to prove \
            outfield_ai_policy.id() reacts; offball_runs::MAX_ACTIVE_PER_TEAM is a pub const \
            in this port and cannot be mutated. Covered the same way as LANE_WIDTH above: \
            covers_every_declared_surface_field_with_a_scalar and \
            matches_the_canonical_bytes_captured_from_the_real_lua_build together prove \
            offball_runs.MAX_ACTIVE_PER_TEAM is a live descriptor() row baked into \
            canonical()/id()."]
fn detects_a_changed_run_constant() {}

#[test]
#[ignore = "retired: Lua case mutates a Knob's .default field on the live tuning registry \
            at runtime to prove outfield_ai_policy.id() reacts; tuning::KNOBS is a static \
            &[Knob] of value structs in this port, immutable at runtime, so there is no live \
            default to nudge. Covered by matches_the_canonical_bytes_captured_from_the_real_lua_build \
            (descriptor()'s tuning.* rows, built by walking tuning::KNOBS' AI-category \
            defaults, are part of the byte-exact canonical() that test pins) together with \
            does_not_move_when_a_live_tuning_value_is_nudged_off_its_default below (proves \
            the complementary half: descriptor() never takes a live Tuning, so only the \
            static default -- never an in-session nudge -- can move the id)."]
fn detects_a_changed_ai_knob_default() {}

#[test]
#[ignore = "retired: Lua case injects a new field onto a live module table \
            (outfield_decision.SPEC_ONLY_SCRATCH_FIELD = 42) to prove undeclared fields do \
            not move the id; Rust structs/modules have a fixed, compile-time field set, so \
            there is no dynamic field-injection path to exercise. Covered structurally by \
            covers_every_declared_surface_field_with_a_scalar above: descriptor() only ever \
            reads the fields it is hand-written to read, which is the structural form of the \
            same guarantee (there is nowhere for an undeclared field to be read from even in \
            principle)."]
fn does_not_move_when_an_undeclared_module_field_is_added() {}

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
#[ignore = "retired: Lua case sets outfield_decision.BASE_TEMPERATURE = nil at runtime and \
            expects outfield_ai_policy.id() to error naming the missing field; a pub const \
            cannot be nil'd, and Rust's type system makes a declared-but-missing scalar \
            unrepresentable at this layer (a compile error, not a runtime one) -- the failure \
            mode this test pins does not exist in the port, and no runtime test can cover a \
            case that cannot occur at runtime."]
fn fails_loudly_when_a_declared_field_disappears() {}

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
