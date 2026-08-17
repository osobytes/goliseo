//! Structural proof that #489's possession invariant has no verb-level
//! bypass: `r#match::set_owner` is the ONLY thing in this crate that
//! changes ball ownership, so the committed-action clear it performs
//! cannot be skipped by writing `MatchState::owner` directly.
//!
//! ## Why this is a source scan and not another scripted match
//!
//! `tests/action_slot_integration.rs` already drives a real
//! `sim_match::step` through one possession change (a keeper smother) and
//! asserts the carrier's slot clears from every phase. That is the
//! behavioural half, and it is worth having — but it can only ever prove
//! the invariant at the entry points it happens to script. It stayed green
//! for the whole of PR #548 while seven `s.owner = …` writes in `match.rs`
//! and one in `combat.rs` bypassed the choke point entirely, because
//! Tackle — the only verb wired to the slot so far — never holds the ball
//! while it charges or executes, so nothing observable was clearing
//! wrongly yet. The bypass only becomes a player-visible bug when the Shot
//! or Pass verb moves onto the slot, at which point a shot that is
//! supposed to break when you are dispossessed simply would not.
//!
//! A test that cannot go red for the defect it is named after is not
//! evidence (AGENTS.md §9). So this file asserts the property the doc
//! comment on `set_owner` actually claims — "every assignment to a
//! `MatchState::owner` field anywhere in this crate goes through this
//! function" — against the crate's own source text, where a future bypass
//! shows up the moment it is written rather than the moment somebody
//! thinks to script the entry point that exercises it.
//! `scanner_reports_a_planted_bypass` is this file's own red
//! demonstration: it feeds the scanner a synthetic bypass and requires it
//! to be found, so a scanner that silently stopped matching anything would
//! fail here rather than pass everything.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the choke point itself is authored.
const CHOKE_POINT_FILE: &str = "match.rs";

/// The exact signature line this scan anchors the choke point's body to.
const CHOKE_POINT_SIGNATURE: &str = "pub(crate) fn set_owner(";

/// Mutating `Option` methods that would change ownership without ever
/// writing `= …`, and so would slip past a write-only scan.
const MUTATORS: [&str; 5] = ["take(", "replace(", "insert(", "get_or_insert", "as_mut("];

/// One flagged access to a `.owner` field, with enough context to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerAccess {
    /// Path relative to the crate root, e.g. `src/match.rs`.
    file: String,
    /// One-based line number in that file.
    line: usize,
    /// The trimmed source line, for the failure message.
    text: String,
    /// Byte offset of the `.owner` token within the file.
    offset: usize,
}

/// Every `.rs` file under this crate's `src/`, in a deterministic order.
///
/// `CARGO_MANIFEST_DIR` is resolved at compile time, so this does not
/// depend on the working directory a test runner happens to pick.
fn source_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"));
        let mut here: Vec<PathBuf> = entries
            .map(|e| {
                e.unwrap_or_else(|err| panic!("dir entry in {dir:?}: {err}"))
                    .path()
            })
            .collect();
        here.sort();
        for path in here {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    assert!(
        !out.is_empty(),
        "no Rust sources found under this crate's src/ -- the scan would vacuously pass"
    );
    out
}

/// Blank out comments and string literals, preserving byte offsets and
/// line breaks so a match's position still maps back to the real file.
///
/// Byte-wise on purpose: every byte of a multi-byte UTF-8 character is
/// `>= 0x80`, so it can never be mistaken for one of the ASCII delimiters
/// below, and blanking a whole region never splits a character.
fn blank_noise(src: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Code,
        Line,
        Block(u32),
        Str,
        Raw(usize),
    }

    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut mode = Mode::Code;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let next = bytes.get(i + 1).copied();
        match mode {
            Mode::Code => {
                if b == b'/' && next == Some(b'/') {
                    mode = Mode::Line;
                } else if b == b'/' && next == Some(b'*') {
                    mode = Mode::Block(1);
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    continue;
                } else if b == b'"' {
                    // A raw string opens as `r"` or `r#*"`; count the hashes
                    // so the matching close is unambiguous.
                    let mut hashes = 0usize;
                    let mut back = i;
                    while back > 0 && bytes[back - 1] == b'#' {
                        hashes += 1;
                        back -= 1;
                    }
                    let raw = back > 0 && bytes[back - 1] == b'r';
                    mode = if raw { Mode::Raw(hashes) } else { Mode::Str };
                    i += 1;
                    continue;
                }
            }
            Mode::Line => {
                if b == b'\n' {
                    mode = Mode::Code;
                }
            }
            Mode::Block(depth) => {
                if b == b'/' && next == Some(b'*') {
                    mode = Mode::Block(depth + 1);
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    continue;
                }
                if b == b'*' && next == Some(b'/') {
                    mode = if depth == 1 {
                        Mode::Code
                    } else {
                        Mode::Block(depth - 1)
                    };
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    continue;
                }
            }
            Mode::Str => {
                if b == b'\\' {
                    out[i] = b' ';
                    if i + 1 < out.len() && bytes[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    i += 2;
                    continue;
                }
                if b == b'"' {
                    mode = Mode::Code;
                    i += 1;
                    continue;
                }
            }
            Mode::Raw(hashes) => {
                if b == b'"' && bytes[i + 1..].iter().take(hashes).all(|&c| c == b'#') {
                    mode = Mode::Code;
                    i += 1 + hashes;
                    continue;
                }
            }
        }
        if mode != Mode::Code && b != b'\n' {
            out[i] = b' ';
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking only ever writes ASCII spaces over whole regions")
}

/// True if `byte` can appear inside a Rust identifier.
fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Every access to a `.owner` field in `src` that could CHANGE it: a
/// direct assignment, a mutating `Option` method, or a `&mut` borrow of
/// the field. Reads (`s.owner == …`, `s.owner.map(…)`) are not flagged.
fn ownership_writes(src: &str, file: &str) -> Vec<OwnerAccess> {
    let masked = blank_noise(src);
    let bytes = masked.as_bytes();
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = masked[from..].find(".owner") {
        let at = from + rel;
        from = at + ".owner".len();
        // `.ownership`, `.owner_team`, `.owned_verb` are different fields.
        if bytes.get(from).copied().is_some_and(is_ident) {
            continue;
        }
        let mut after = from;
        while bytes.get(after).is_some_and(u8::is_ascii_whitespace) {
            after += 1;
        }
        let assigns = bytes.get(after) == Some(&b'=') && bytes.get(after + 1) != Some(&b'=');
        let mutates = bytes.get(after) == Some(&b'.')
            && MUTATORS.iter().any(|m| masked[after + 1..].starts_with(m));
        // Walk back over the whole path expression (`self.state.owner`),
        // then over any whitespace, and see whether it was borrowed
        // mutably.
        let mut start = at;
        while start > 0 && (is_ident(bytes[start - 1]) || bytes[start - 1] == b'.') {
            start -= 1;
        }
        while start > 0 && bytes[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        let borrowed = masked[..start].ends_with("&mut");
        if !(assigns || mutates || borrowed) {
            continue;
        }
        let line = masked[..at].bytes().filter(|&c| c == b'\n').count() + 1;
        let text = src
            .lines()
            .nth(line - 1)
            .unwrap_or_default()
            .trim()
            .to_string();
        found.push(OwnerAccess {
            file: file.to_string(),
            line,
            text,
            offset: at,
        });
    }
    found
}

/// The byte range of `set_owner`'s body (the braces included), located by
/// its signature rather than a line number so ordinary edits above it do
/// not silently move the window.
fn choke_point_body(masked: &str) -> (usize, usize) {
    let sig = masked.find(CHOKE_POINT_SIGNATURE).unwrap_or_else(|| {
        panic!(
            "`{CHOKE_POINT_SIGNATURE}` not found in {CHOKE_POINT_FILE} -- \
             the ownership choke point was renamed or its visibility changed; update this \
             test rather than deleting it"
        )
    });
    assert!(
        !masked[sig + CHOKE_POINT_SIGNATURE.len()..].contains(CHOKE_POINT_SIGNATURE),
        "more than one `set_owner` definition: the invariant needs exactly one choke point"
    );
    let open = sig + masked[sig..].find('{').expect("set_owner must have a body");
    let bytes = masked.as_bytes();
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return (open, i + 1);
            }
        }
    }
    panic!("unbalanced braces while walking set_owner's body");
}

#[test]
fn set_owner_is_the_only_writer_of_ball_ownership_in_this_crate() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let choke_source = fs::read_to_string(root.join("src").join(CHOKE_POINT_FILE))
        .unwrap_or_else(|e| panic!("read src/{CHOKE_POINT_FILE}: {e}"));
    let (body_start, body_end) = choke_point_body(&blank_noise(&choke_source));

    let mut outside: Vec<OwnerAccess> = Vec::new();
    let mut inside = 0usize;
    for path in source_files() {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"));
        for access in ownership_writes(&src, &rel) {
            let in_choke =
                rel.ends_with(CHOKE_POINT_FILE) && (body_start..body_end).contains(&access.offset);
            if in_choke {
                inside += 1;
            } else {
                outside.push(access);
            }
        }
    }

    assert_eq!(
        inside, 1,
        "expected exactly one ownership write, inside set_owner's body"
    );
    assert!(
        outside.is_empty(),
        "#489's possession invariant is applied once, in `r#match::set_owner`, which clears \
         the outgoing owner's committed action slot. These {} write(s) change ball ownership \
         without going through it, so a committed Shot/Pass/Tackle would survive losing the \
         ball there. Route them through `set_owner` (it is `pub(crate)`), or -- if a site \
         genuinely must not clear the slot -- change the invariant deliberately and say so \
         here:\n{}",
        outside.len(),
        outside
            .iter()
            .map(|a| format!("  {}:{}  {}", a.file, a.line, a.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn the_choke_point_still_clears_the_outgoing_owners_action_slot() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(root.join("src").join(CHOKE_POINT_FILE))
        .unwrap_or_else(|e| panic!("read src/{CHOKE_POINT_FILE}: {e}"));
    let masked = blank_noise(&src);
    let (start, end) = choke_point_body(&masked);
    let body = &masked[start..end];
    assert!(
        body.contains("action_slot::clear("),
        "set_owner is the only place the possession invariant is applied, so gutting its \
         `action_slot::clear` would silently disable #489's clearing everywhere at once. \
         Body was:\n{}",
        &src[start..end]
    );
}

#[test]
fn scanner_reports_a_planted_bypass() {
    // The red demonstration for the two tests above: a scanner that
    // stopped matching would let every bypass through while still passing,
    // so it has to prove it can find one.
    let planted = r#"
fn hand_the_ball_over(s: &mut MatchState) {
    s.owner = Some(3);
}
"#;
    let hits = ownership_writes(planted, "planted.rs");
    assert_eq!(
        hits.len(),
        1,
        "the scanner must flag a direct `s.owner = …` write: {hits:?}"
    );
    assert_eq!(hits[0].line, 3, "and report where it is: {hits:?}");

    for form in [
        "fn f(s: &mut MatchState) { s.owner.take(); }",
        "fn f(s: &mut MatchState) { s.owner.replace(2); }",
        "fn f(s: &mut MatchState) { let r = &mut s.owner; }",
        "fn f(s: &mut MatchState) { self.state.owner = None; }",
    ] {
        assert_eq!(
            ownership_writes(form, "planted.rs").len(),
            1,
            "a bypass that avoids `=` must still be flagged: {form}"
        );
    }
}

#[test]
fn scanner_ignores_reads_comments_strings_and_neighbouring_fields() {
    let benign = concat!(
        "fn f(s: &MatchState) -> bool {\n",
        "    // s.owner = None; in a comment is not a write\n",
        "    let _ = \"s.owner = None; in a string is not a write\";\n",
        "    /* s.owner = None; in a block comment is not a write */\n",
        "    let _ = s.ownership;\n",
        "    let _ = s.owner_team;\n",
        "    let _ = s.owner.map(|o| o + 1);\n",
        "    s.owner == Some(1)\n",
        "}\n",
    );
    assert_eq!(
        ownership_writes(benign, "benign.rs"),
        Vec::new(),
        "reads and look-alike fields must not be reported, or the real signal drowns"
    );
}
