//! Pins [`gc_sim::ai_driven_evidence::run`]'s digests to the LUA fixture.
//!
//! This is the link that makes the wasm-side check meaningful. `run()` is a
//! self-contained replay with no fixture, so it can execute inside the
//! compiled wasm module where no file system exists — but a digest is only
//! worth as much as what it was pinned against. Here it is pinned against the
//! captured output of real Lua (`session_ai_driven_lua_reference.txt`), by
//! feeding the FIXTURE's parsed rows through the SAME `Digest` and `Row` the
//! live replay uses.
//!
//! So the chain reads:
//!
//!   Lua capture  --(this test)-->  the pinned constants
//!                                        |
//!   native run() --(this test)-----------+
//!                                        |
//!   wasm run()   --(packages/wasm/src/ai_driven.spec.ts and check_v2.sh)--+
//!
//! and "the browser agrees with Lua" is a chain of equalities, each of which
//! some gate actually checks, rather than an inference.
//!
//! `tests/session_ai_driven_differential.rs` remains the finer instrument:
//! it compares every field at every tick and names the tick and field where
//! two runs part. A digest only says THAT they parted. Both exist on purpose —
//! the differential cannot run inside wasm (it needs the fixture), and the
//! digest cannot tell you where to look.

use gc_sim::ai_driven_evidence::{self as evidence, Digest, Row};
use gc_sim::tuning::Tuning;

const FIXTURE: &str = include_str!("fixtures/session_ai_driven_lua_reference.txt");

const PLAYER_COUNT: usize = 10;
const FIELD_COUNT: usize = 11 + 2 * PLAYER_COUNT;

/// THE PINNED DIGESTS. Derived from the Lua fixture by
/// `digests_match_the_lua_fixture` below — not hand-written, and not copied
/// from a Rust run. If a change moves these, the question is whether Lua moved
/// too; if it did not, the port diverged.
///
/// Mirrored in `v2/ts/packages/wasm/src/ai_driven.spec.ts` and in
/// `scripts/check_v2.sh`, which assert the COMPILED WASM module reproduces
/// them.
/// FNV-1a-64 over the final row, derived from the Lua capture.
pub const EXPECTED_FINAL_HASH: &str = "628d7fc71238dec6";
/// FNV-1a-64 over every row in sequence, derived from the Lua capture.
pub const EXPECTED_SEQUENCE_DIGEST: &str = "29bbbc0f32b78dfa";

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

fn digests_of_fixture() -> (String, String) {
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
fn digests_match_the_lua_fixture() {
    let (fixture_final, fixture_sequence) = digests_of_fixture();
    let result = evidence::run(&Tuning::new());

    assert_eq!(
        result.final_hash, fixture_final,
        "the native replay's final row does not digest to the Lua fixture's final row -- \
         run tests/session_ai_driven_differential.rs, which reports the exact tick and field"
    );
    assert_eq!(
        result.sequence_digest, fixture_sequence,
        "the native replay's tick sequence does not digest to the Lua fixture's -- \
         some tick diverged and may have self-corrected; the differential names it"
    );

    // The constants above are what the wasm side asserts. Keeping them in the
    // same test that derives them means they cannot be updated to whatever a
    // Rust run happens to produce without that run first agreeing with Lua.
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
