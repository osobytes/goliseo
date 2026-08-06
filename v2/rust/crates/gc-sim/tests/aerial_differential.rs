//! Differential test against the reference Lua implementation (README rule
//! 5.9, `v2/tools/lua_reference/README.md`), required for `aerial`: it
//! consumes `core.rng` directly (`aerial::resolve` draws four rolls) and its
//! probability/geometry arithmetic feeds ball trajectory that must be
//! bit-identical across two rollback clients.
//!
//! `tests/fixtures/aerial_lua_reference.txt` is the captured stdout of
//! running the real Lua `sim/aerial.lua` (plus its `core/vec2.lua`,
//! `core/rng.lua`, `sim/species.lua`, `sim/tuning.lua` dependencies) under
//! headless `love` (no display, no `xvfb`), via a scratch `conf.lua`/
//! `main.lua` harness built per that README (not committed — scratch dirs
//! are session-local). Every case below reconstructs the identical
//! `AerialContext`/contact/rng inputs in Rust and asserts the output matches
//! the captured Lua bit for bit via `.to_bits()` (parsing the reference's
//! `%.17g` text round-trips the exact binary64 value).
//!
//! Degenerate cases covered for the rng argument to `aerial::resolve`: the
//! minstd minimum state (`1`), the maximum (`MOD - 1 = 2147483646`, which
//! lands a `roll_contact` right at the boundary and produces a `miss` with
//! zero angle/weight error), and ordinary mid-range seeds. Contexts cover a
//! bicycle (behind-ball) contact, low/high skill, a "hard" contact
//! (stretch + pace + drop + instability + pressure all elevated), and a
//! jumping header.

use gc_core::vec2::Vec2;
use gc_sim::aerial::{self, AerialContext, AerialIntent, AerialOutcome};
use indexmap::IndexMap;

const FIXTURE: &str = include_str!("fixtures/aerial_lua_reference.txt");

fn reference() -> IndexMap<&'static str, &'static str> {
    FIXTURE
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("malformed fixture line: {line}"));
            (key, value)
        })
        .collect()
}

fn expect<'a>(reference: &IndexMap<&'a str, &'a str>, key: &str) -> &'a str {
    *reference
        .get(key)
        .unwrap_or_else(|| panic!("missing reference value for {key}"))
}

/// Parse the reference's `%.17g` text and compare bit patterns, not text —
/// this is what actually proves bit-exactness rather than merely "close".
fn assert_bits_eq(reference: &IndexMap<&str, &str>, key: &str, actual: f64) {
    let text = expect(reference, key);
    let expected: f64 = text
        .parse()
        .unwrap_or_else(|_| panic!("{key}={text} is not a valid f64"));
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "{key}: expected {expected} (bits {:x}), got {actual} (bits {:x})",
        expected.to_bits(),
        actual.to_bits()
    );
}

fn outcome_str(o: AerialOutcome) -> &'static str {
    match o {
        AerialOutcome::Clean => "clean",
        AerialOutcome::Heavy => "heavy",
        AerialOutcome::Miss => "miss",
    }
}

struct CtxOpts {
    ball_pos: Vec2,
    ball_vel: Vec2,
    ball_z: f64,
    ball_vz: f64,
    skill: f64,
    opponent_distance: f64,
    anticipated: bool,
    instability: f64,
}

impl Default for CtxOpts {
    fn default() -> Self {
        CtxOpts {
            ball_pos: Vec2::new(10.0, 0.0),
            ball_vel: Vec2::new(-120.0, 0.0),
            ball_z: 50.0,
            ball_vz: -120.0,
            skill: 0.6,
            opponent_distance: 100.0,
            anticipated: true,
            instability: 0.0,
        }
    }
}

fn context(o: CtxOpts) -> AerialContext {
    AerialContext {
        ball_pos: o.ball_pos,
        ball_vel: o.ball_vel,
        ball_z: o.ball_z,
        ball_vz: o.ball_vz,
        player_pos: Vec2::new(0.0, 0.0),
        player_vel: Vec2::new(0.0, 0.0),
        facing: Vec2::new(1.0, 0.0),
        move_speed: 200.0,
        skill: o.skill,
        strength: 0.5,
        opponent_distance: o.opponent_distance,
        anticipated: o.anticipated,
        instability: o.instability,
        extra_reach: 0.0,
        extra_lift: 0.0,
    }
}

/// Checks a `best_contact` result and every `resolve` seed against the
/// fixture for one labeled case.
fn check_case(
    reference: &IndexMap<&str, &str>,
    label: &str,
    ctx: &AerialContext,
    intent: AerialIntent,
    seeds: &[u32],
) {
    let contact =
        aerial::best_contact(ctx, intent).unwrap_or_else(|| panic!("{label}: expected a contact"));

    assert_bits_eq(reference, &format!("{label}.jump_lift"), contact.jump_lift);
    assert_bits_eq(
        reference,
        &format!("{label}.jump_ratio"),
        contact.jump_ratio,
    );
    assert_bits_eq(
        reference,
        &format!("{label}.difficulty"),
        contact.difficulty,
    );
    assert_bits_eq(reference, &format!("{label}.reach"), contact.reach);
    assert_bits_eq(
        reference,
        &format!("{label}.reach_ratio"),
        contact.reach_ratio,
    );

    for &seed in seeds {
        let resolution = aerial::resolve(ctx, &contact, seed);
        let tag = format!("{label}@{seed}");
        assert_eq!(
            outcome_str(resolution.outcome),
            expect(reference, &format!("{tag}.outcome")),
            "{tag}.outcome"
        );
        assert_eq!(
            resolution.rng.to_string(),
            expect(reference, &format!("{tag}.rng")),
            "{tag}.rng"
        );
        assert_bits_eq(
            reference,
            &format!("{tag}.angle_error"),
            resolution.angle_error,
        );
        assert_bits_eq(
            reference,
            &format!("{tag}.weight_error"),
            resolution.weight_error,
        );
        assert_bits_eq(
            reference,
            &format!("{tag}.contact_probability"),
            resolution.contact_probability,
        );
        assert_bits_eq(
            reference,
            &format!("{tag}.clean_probability"),
            resolution.clean_probability,
        );
    }
}

#[test]
fn best_contact_and_resolve_match_the_reference_lua_across_styles_seeds_and_degenerate_rng_states()
{
    let reference = reference();

    // Case A: acrobatic (bicycle) contact, ball behind the player.
    let ctx_a = context(CtxOpts {
        ball_z: 60.0,
        ball_pos: Vec2::new(-10.0, 0.0),
        ..Default::default()
    });
    check_case(
        &reference,
        "A",
        &ctx_a,
        AerialIntent::Acrobatic,
        &[4471, 1, 2147483646, 1000000000],
    );

    // Case B: low skill, receive.
    let ctx_b = context(CtxOpts {
        skill: 0.2,
        ..Default::default()
    });
    check_case(
        &reference,
        "B",
        &ctx_b,
        AerialIntent::Receive,
        &[91, 1, 2147483646],
    );

    // Case C: high skill, receive.
    let ctx_c = context(CtxOpts {
        skill: 0.9,
        ..Default::default()
    });
    check_case(&reference, "C", &ctx_c, AerialIntent::Receive, &[91, 12345]);

    // Case D: "hard" contact -- stretch, pace, drop, instability, pressure.
    let ctx_d = context(CtxOpts {
        ball_pos: Vec2::new(25.0, 0.0),
        ball_vel: Vec2::new(-560.0, 0.0),
        ball_z: 60.0,
        ball_vz: -480.0,
        opponent_distance: 8.0,
        anticipated: false,
        instability: 1.0,
        ..Default::default()
    });
    check_case(&reference, "D", &ctx_d, AerialIntent::Receive, &[777]);

    // Case E: header (strike), jumping.
    let ctx_e = context(CtxOpts {
        ball_z: 88.0,
        ..Default::default()
    });
    check_case(&reference, "E", &ctx_e, AerialIntent::Strike, &[555]);
}

#[test]
fn claim_score_matches_the_reference_lua() {
    let reference = reference();
    let good_ctx = context(CtxOpts {
        skill: 0.9,
        ball_pos: Vec2::new(4.0, 0.0),
        ..Default::default()
    });
    let bad_ctx = context(CtxOpts {
        skill: 0.2,
        ball_pos: Vec2::new(25.0, 0.0),
        ..Default::default()
    });
    let good_contact =
        aerial::best_contact(&good_ctx, AerialIntent::Receive).expect("a contact exists");
    let bad_contact =
        aerial::best_contact(&bad_ctx, AerialIntent::Receive).expect("a contact exists");

    assert_bits_eq(
        &reference,
        "claim_score.good",
        aerial::claim_score(&good_ctx, &good_contact, 0.0, 0.0, 0.0),
    );
    assert_bits_eq(
        &reference,
        "claim_score.bad",
        aerial::claim_score(&bad_ctx, &bad_contact, 0.0, 0.0, 0.0),
    );
    assert_bits_eq(
        &reference,
        "claim_score.with_bonuses",
        aerial::claim_score(&good_ctx, &good_contact, 0.12, 0.05, -0.7),
    );
}
