//! Pins [`gc_sim::ai_driven_evidence::run`]'s digests, and makes the
//! wasm-side check mean something.
//!
//! `run()` is a self-contained replay with no fixture, so it can execute
//! inside the compiled wasm module where no file system exists — but a digest
//! is only worth as much as what it was pinned against.
//!
//! # What the chain proves now, and what it used to
//!
//! It used to be pinned against the captured output of real Lua, and the
//! chain read "the browser agrees with **Lua**" as a series of equalities
//! each of which some gate checked. **That link is retired (#520).** The
//! fixture half is now a baseline recorded from this build by
//! `session_ai_driven_differential`'s `record_session_ai_driven_baseline`, so
//! the chain reads:
//!
//! ```text
//!   recorded baseline --(this test)--> the pinned constants
//!                                            |
//!   native run()      --(this test)----------+
//!                                            |
//!   wasm run()        --(packages/wasm/src/ai_driven.spec.ts)--+
//! ```
//!
//! What survives is the part that never depended on Lua and is the more
//! urgent claim today: **native and wasm produce the same bits**. Those are
//! two different compilations of the same source, against two different
//! libms, and #517 exists because they are known to disagree — the same file's
//! `it.fails` pins record #405's divergence at tick 96 of this very scenario.
//! So the constants below are not decoration around a retired Lua claim; they
//! are the only thing in the workspace that would catch this scenario's native
//! and wasm builds parting company, and they keep gating through every
//! gameplay rework because both sides move together.
//!
//! What is LOST is real and worth stating plainly: nothing here can any longer
//! catch the simulation being *wrong* rather than merely *consistent across
//! two targets*. Both sides could agree perfectly on a wrong answer. The Lua
//! capture could have caught that; there is no second implementation left.
//!
//! `tests/session_ai_driven_differential.rs` remains the finer instrument: it
//! compares every field at every tick and names where two runs part. A digest
//! only says THAT they parted. Both exist on purpose — the differential
//! cannot run inside wasm (it needs the fixture), and the digest cannot tell
//! you where to look.

use gc_sim::ai_driven_evidence::{self as evidence, Digest, Row};
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/session_ai_driven_baseline.txt");

const PLAYER_COUNT: usize = 10;
const FIELD_COUNT: usize = 11 + 2 * PLAYER_COUNT;

/// THE PINNED DIGESTS. Derived from the Lua fixture by
/// `digests_match_the_lua_fixture` below — not hand-written, and not copied
/// from a Rust run. If a change moves these, the question is whether the
/// pinned fixture moved too; if it did not, this simulation diverged from the
/// reference it was validated against.
///
/// Mirrored in `ts/packages/wasm/src/ai_driven.spec.ts` and in
/// `scripts/check.sh`, which assert the COMPILED WASM module reproduces
/// them.
/// FNV-1a-64 over the final row, derived from the recorded baseline (#520).
pub const EXPECTED_FINAL_HASH: &str = "ef0d733d30f615f8";
/// FNV-1a-64 over every row in sequence, derived from the recorded baseline (#520).
pub const EXPECTED_SEQUENCE_DIGEST: &str = "2abf5a39a8c0351a";

fn parse_row(line: &str) -> Row {
    let f: Vec<&str> = line.split('\t').collect();
    assert_eq!(f.len(), FIELD_COUNT, "fixture row field count");
    let num = |i: usize| -> f64 { f[i].parse().expect("fixture float parses") };
    let int = |i: usize| -> i64 { f[i].parse().expect("fixture integer parses") };
    let mut players = [(0.0_f64, 0.0_f64); PLAYER_COUNT];
    for (i, slot) in players.iter_mut().enumerate() {
        *slot = (num(11 + 2 * i), num(12 + 2 * i));
    }
    Row {
        tick: int(0),
        // Seventeen significant digits round-trip an f64 exactly, so these
        // bits ARE Lua's bits -- see ai_driven_evidence.rs's header on why
        // the digest is over bit patterns rather than printed text.
        ball: [num(1), num(2), num(3), num(4), num(5), num(6)],
        owner: int(7),
        score_home: int(8),
        score_away: int(9),
        rng: f[10].parse().expect("fixture rng parses"),
        players,
    }
}

fn digests_of_baseline() -> (String, String) {
    let rows: Vec<Row> = FIXTURE.lines().map(parse_row).collect();
    assert_eq!(rows.len(), (evidence::TICKS + 1) as usize);
    let mut sequence = Digest::new();
    for row in &rows {
        row.absorb(&mut sequence);
    }
    let mut final_digest = Digest::new();
    rows.last().expect("non-empty").absorb(&mut final_digest);
    (final_digest.hex(), sequence.hex())
}

#[test]
fn digests_match_the_recorded_baseline_and_the_constants_wasm_mirrors() {
    let (fixture_final, fixture_sequence) = digests_of_baseline();
    let result = evidence::run(&Tuning::new());

    assert_eq!(
        result.final_hash, fixture_final,
        "the native replay's final row does not digest to the recorded baseline's final row \
         -- run tests/session_ai_driven_differential.rs, which reports the exact tick and field"
    );
    assert_eq!(
        result.sequence_digest, fixture_sequence,
        "the native replay's tick sequence does not digest to the recorded baseline's -- \
         some tick diverged and may have self-corrected; the differential names it"
    );

    // The constants above are what `packages/wasm/src/ai_driven.spec.ts`
    // asserts against the wasm build. Keeping them in the same test that
    // derives them from the baseline means they cannot be quietly updated to
    // whatever a native run happens to produce: a re-record has to move the
    // baseline, these constants and the TypeScript mirror together, and if
    // wasm then disagrees, that is #517 and not a stale constant.
    assert_eq!(
        result.final_hash, EXPECTED_FINAL_HASH,
        "pinned EXPECTED_FINAL_HASH is stale -- update it, and every mirror listed in \
         ai_driven_evidence.rs, to {}",
        result.final_hash
    );
    assert_eq!(
        result.sequence_digest, EXPECTED_SEQUENCE_DIGEST,
        "pinned EXPECTED_SEQUENCE_DIGEST is stale -- update it, and every mirror, to {}",
        result.sequence_digest
    );
}

#[test]
fn the_reference_match_is_actually_played() {
    let result = evidence::run(&Tuning::new());
    assert_eq!(result.ticks, 7_200);
    assert_eq!(result.rows, 7_201);
    assert_eq!(result.fixture_id, "session_ai_driven/v1");
    // A regression that reduced the bot to an idle player would still produce
    // stable digests -- and would silently turn this back into the AFK
    // scenario it exists to replace.
    assert!(
        result.score_home + result.score_away > 0,
        "the AI-driven reference match records no goals; the shooting path is unexercised"
    );
}
