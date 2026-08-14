//! Structural regression for #531 acceptance criterion 1 (as amended in the
//! issue body): "No `release_pass` call site exists outside the
//! `MatchInput` path or the documented forced-release rule."
//!
//! Phase 1 of #531 landed the seam so that the ONLY two things left calling
//! `release_pass` are `try_pass` and `keeper_throw` -- both reached
//! exclusively from `outfield_actions`/`keeper_actions`, which now run
//! identically for a human's real `MatchInput` and a charging AI's
//! synthesized one. The six-second forced-release rule
//! (`keeper_actions`'s trailing block) does not need a carve-out here: it
//! was rewritten to call `keeper_throw` directly rather than re-running
//! `keeper_distribute`'s scoring, so it IS one of the two blessed callers
//! rather than a third case.
//!
//! `keeper_throw` itself calls `release_pass` from two branches (bowled
//! flat when `plan_throw` finds the lane genuinely clear, lofted
//! otherwise -- see that function's own doc for why both exist), so the
//! total is four: the `fn release_pass(` definition, `try_pass`'s one
//! call, and `keeper_throw`'s two.
//!
//! This is a plain source-text count rather than an AST walk, so it is a
//! narrower guarantee than the doc comment above might suggest: it catches
//! an ordinary reintroduced call to `release_pass(` -- the shape every
//! call site in this file's own history has taken, including the five
//! #531 was written to remove -- because that is the shape `cargo fmt`
//! normalizes call sites into. It would not catch a bypass routed through
//! a function alias, a stored function pointer, or some other indirection
//! that never spells the literal substring `release_pass(` in this file;
//! nothing here parses the source as Rust. Given how this codebase's past
//! bypasses actually looked (five ordinary direct calls), that is judged
//! an acceptable gap for what this test is cheap enough to be worth: a
//! reintroduced call site anywhere in the file raises the count and fails
//! this test without needing to know in advance which function it would
//! live in.
use std::fs;
use std::path::Path;

#[test]
fn release_pass_has_no_call_site_outside_try_pass_and_keeper_throw() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/match.rs");
    let source = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("could not read {}: {e}", path.display());
    });

    let occurrences = source.matches("release_pass(").count();
    assert_eq!(
        occurrences, 4,
        "expected exactly 4 occurrences of `release_pass(` in match.rs -- the function's own \
         `fn release_pass(` definition, the call inside `try_pass`, and `keeper_throw`'s two \
         (bowled-flat and lofted branches) -- found {occurrences}. A new occurrence means \
         either a genuine new caller (which must be one of `try_pass`/`keeper_throw`/the \
         documented forced-release rule -- see this test's module doc) or a reintroduced \
         direct-mutation bypass of the kind #531 removed."
    );

    // Directly confirm the survivors are where they are expected, so a
    // pathological rename (e.g. every call still inside `try_pass`, none
    // inside `keeper_throw`) cannot slip the count check by coincidence.
    let try_pass_start = source
        .find("fn try_pass(")
        .expect("try_pass must still exist");
    let keeper_throw_start = source
        .find("fn keeper_throw(")
        .expect("keeper_throw must still exist");
    let next_top_level_fn_after = |start: usize| {
        source[start + 1..]
            .match_indices("\nfn ")
            .map(|(offset, _)| start + 1 + offset)
            .find(|&idx| {
                // A top-level fn starts at column 0; `\nfn ` already
                // guarantees that for everything after the first line.
                idx > start
            })
            .unwrap_or(source.len())
    };
    let try_pass_body = &source[try_pass_start..next_top_level_fn_after(try_pass_start)];
    let keeper_throw_body =
        &source[keeper_throw_start..next_top_level_fn_after(keeper_throw_start)];

    assert_eq!(
        try_pass_body.matches("release_pass(").count(),
        1,
        "try_pass must call release_pass exactly once"
    );
    assert_eq!(
        keeper_throw_body.matches("release_pass(").count(),
        2,
        "keeper_throw must call release_pass exactly twice (bowled-flat and lofted branches)"
    );
}
