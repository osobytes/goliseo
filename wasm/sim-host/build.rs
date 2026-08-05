//! Embed the Lua tree into the binary (#334).
//!
//! #327 shipped the Lua sources as an Emscripten `--preload-file` package: a
//! second 3.2 MB artifact to deploy beside the .wasm, kept in sync by hand, and
//! mounted into a virtual filesystem before the first tick could run. This
//! build script removes both costs by turning the tree into an `include_str!`
//! table that the host resolves `require` against directly.
//!
//! The tree WALKS; it is never listed. A hand-maintained manifest would make
//! adding a `sim/` module a two-file change and would go stale silently -- the
//! failure mode being a module that exists natively and is missing in the
//! browser. `cargo:rerun-if-changed` is emitted for every directory and every
//! file *under `ROOTS`*, so within those roots an edit, an addition or a
//! deletion all force a rebuild.
//!
//! `ROOTS` was the caveat, and it cost us exactly what it predicted. #336
//! taught `scripts/phase0_sim_host.lua` to require `render.identity`,
//! `render.player_pose` and `render.frame` without adding `render` here.
//! Nothing failed at build time. The artifact linked clean and then died at run
//! time on the first `render.` require, BEFORE reaching the determinism section
//! it exists to prove -- a module that is present natively and missing in the
//! browser, which is the precise failure this file was written to prevent.
//!
//! So the roots are no longer trusted on their own. `assert_closed` walks every
//! embedded source, extracts its literal `require` targets, and fails the build
//! if any of them is not itself embedded. Forgetting a root is now a build
//! error naming the missing module, not a run-time surprise in a browser. It
//! only sees literal string requires, which is all this tree has; a computed
//! require would slip past, and if one ever appears the guard should grow.
//!
//! Nothing under `core/`, `data/`, `render/` or `sim/` is read for anything but
//! its bytes. Those layers stay unmodified and remain the single source of truth.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory roots walked for `.lua` files, relative to the repository root.
/// These are the layers the probe requires: the pure quartet plus the probe's
/// own home. Everything else in the repository needs LOVE and cannot run here.
const ROOTS: [&str; 5] = ["core", "data", "render", "scripts", "sim"];

/// Top-level directories that contain Lua and are DELIBERATELY not embedded.
///
/// This list exists so that `assert_roots_cover_the_tree` can tell "we decided
/// not to embed this" from "nobody has looked at this yet". Both of those are
/// silent today; only the second is a bug, and they are indistinguishable
/// unless the first one is written down.
const EXCLUDED_ROOTS: [&str; 2] = [
    // LOVE-dependent by definition: the engine binding.
    "game",
    // Tests. They run under LOVE's headless runner, not here.
    "spec",
];

/// Lua files sitting at the repository ROOT that are deliberately not embedded.
/// `ROOTS` cannot express "this one file", so root-level Lua needs its own
/// list -- without it a `.lua` dropped beside `main.lua` is in no root, is
/// therefore unembedded, and nothing would say so.
const EXCLUDED_ROOT_FILES: [&str; 2] = [
    // The LOVE entry point.
    "main.lua",
    // LOVE's window/config callback.
    "conf.lua",
];

fn main() {
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );
    let repo_root = manifest
        .ancestors()
        .nth(2)
        .expect("crate should sit two levels below the repository root");

    let mut sources: Vec<(String, PathBuf)> = Vec::new();
    for root in ROOTS {
        let directory = repo_root.join(root);
        // A root that has been renamed away, or that contributes nothing, is a
        // stale manifest entry. Neither is fatal on its own -- but both mean
        // this list no longer describes the tree, and the next edit to it will
        // be made against a wrong picture.
        assert!(
            directory.is_dir(),
            "ROOTS names '{root}', which is not a directory under {}",
            repo_root.display()
        );
        let before = sources.len();
        collect(&directory, repo_root, &mut sources);
        assert!(
            sources.len() > before,
            "ROOTS names '{root}', which contains no .lua files; remove it or fix the path"
        );
    }
    assert!(
        !sources.is_empty(),
        "no Lua sources found under {}; is the repository root wrong?",
        repo_root.display()
    );
    // Sorted so the generated table -- and with it the binary -- does not
    // depend on directory iteration order, and so the host can binary search.
    //
    // This is a tuple sort over (String, PathBuf) while the host binary-searches
    // on the &str key alone. The two orderings agree because every key is a
    // distinct repo-relative path: ROOTS do not overlap, and a walk yields each
    // file once, so no two entries share a key and the PathBuf tie-breaker never
    // decides anything. Break that -- overlapping roots, or a symlink that walks
    // one file in under two names -- and the sort could order equal keys by a
    // field the search cannot see, making a lookup miss a module that is present.
    sources.sort();
    assert_roots_cover_the_tree(repo_root);
    assert_closed(&sources);

    let mut generated = String::from("// @generated by build.rs (#334) -- do not edit.\n");
    generated.push_str("/// Repository-relative path -> source text, sorted by path.\n");
    generated.push_str("pub static SOURCES: &[(&str, &str)] = &[\n");
    for (path, absolute) in &sources {
        let absolute = absolute.to_str().expect("source path should be UTF-8");
        // `{:?}` on a &str emits a correctly escaped Rust string literal.
        writeln!(generated, "    ({path:?}, include_str!({absolute:?})),")
            .expect("writing to a String cannot fail");
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR"));
    let destination = out_dir.join("embedded_lua.rs");
    fs::write(&destination, generated)
        .unwrap_or_else(|error| panic!("write {}: {error}", destination.display()));

    println!("cargo:rerun-if-changed=build.rs");
}

/// Fail the build if top-level Lua -- a directory or a root-level file -- is
/// classified by none of `ROOTS`, `EXCLUDED_ROOTS` or `EXCLUDED_ROOT_FILES`.
/// THIS is the guard that would have caught #343.
///
/// `assert_closed` below is the other half, and on its own it was not enough:
/// it only sees LITERAL `require("a.b")` calls, and the probe that #336 broke
/// requires through `pcall(require, name)` over a table of names. No literal,
/// no detection. Adding `render/` to the repository, on the other hand, is
/// impossible to miss from here -- so the trip wire is placed on the act that
/// actually causes the problem: a new top-level Lua directory appearing.
///
/// The failure is deliberately not "add it to ROOTS". Some directories must
/// not be embedded. It is "decide", and record the decision in one of the
/// lists, so the next reader can tell a choice from an oversight.
///
/// THE RESIDUAL RISK, NAMED: an exclusion silences this check exactly as
/// cleanly as a correct classification does. Someone who hits the failure and
/// reaches for `EXCLUDED_ROOTS` to make the build pass, on a directory the host
/// genuinely needs, recreates #343 precisely -- and this guard will agree with
/// them. Nothing here can distinguish a considered exclusion from a careless
/// one; only review can. What the guard buys is that the decision now has to
/// be made, written down, and shown in a diff.
fn assert_roots_cover_the_tree(repo_root: &Path) {
    // Watched, so adding a top-level directory re-runs this check rather than
    // passing on a cached build script result.
    println!("cargo:rerun-if-changed={}", repo_root.display());

    let known: BTreeSet<&str> = ROOTS.into_iter().chain(EXCLUDED_ROOTS).collect();
    let excluded_files: BTreeSet<&str> = EXCLUDED_ROOT_FILES.into_iter().collect();
    let mut undecided: BTreeSet<String> = BTreeSet::new();

    let entries = fs::read_dir(repo_root)
        .unwrap_or_else(|error| panic!("read {}: {error}", repo_root.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("read entry in {}: {error}", repo_root.display()))
            .path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // Dotfiles are tooling (.git, .github); they are not source layers.
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if !known.contains(name) && contains_lua(&path) {
                undecided.insert(format!("{name}/  (directory)"));
            }
        } else if path.extension().is_some_and(|extension| extension == "lua")
            && !excluded_files.contains(name)
        {
            // A .lua dropped at the repository root is in no root at all, so it
            // is unembedded and, before this arm existed, unflagged. `ROOTS`
            // cannot express "this one file", which is why it needs its own
            // list rather than a directory entry.
            undecided.insert(format!("{name}   (file at the repository root)"));
        }
    }

    assert!(
        undecided.is_empty(),
        "these top-level Lua sources appear in none of ROOTS, EXCLUDED_ROOTS or \
         EXCLUDED_ROOT_FILES -- decide whether the wasm host needs them and say so in \
         one of the three lists:\n  {}",
        undecided.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

/// Whether a directory tree holds any `.lua` file at all.
fn contains_lua(directory: &Path) -> bool {
    let Ok(entries) = fs::read_dir(directory) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_lua = path.extension().is_some_and(|extension| extension == "lua");
        if is_lua || (path.is_dir() && contains_lua(&path)) {
            return true;
        }
    }
    false
}

/// Fail the build if any embedded module requires a module that is not itself
/// embedded. The complement of the check above: that one catches a directory
/// nobody classified, this one catches a module nobody embedded.
///
/// It resolves a dotted name the same two ways the host's searcher does --
/// `a/b.lua` then `a/b/init.lua` -- so agreement between them is structural
/// rather than coincidental.
fn assert_closed(sources: &[(String, PathBuf)]) {
    let embedded: BTreeSet<&str> = sources.iter().map(|(path, _)| path.as_str()).collect();
    let mut missing: BTreeSet<String> = BTreeSet::new();

    for (path, absolute) in sources {
        let text = fs::read_to_string(absolute)
            .unwrap_or_else(|error| panic!("read {}: {error}", absolute.display()));
        for name in required_modules(&text) {
            let base = name.replace('.', "/");
            let resolves = embedded.contains(format!("{base}.lua").as_str())
                || embedded.contains(format!("{base}/init.lua").as_str());
            if !resolves {
                missing.insert(format!("{name} (required by {path})"));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "these modules are required by an embedded source but are not embedded; \
         add their top-level directory to ROOTS:\n  {}",
        missing.into_iter().collect::<Vec<_>>().join("\n  ")
    );
}

/// Literal `require` targets in one Lua source: `require("a.b")`,
/// `require "a.b"` and `require'a.b'` all count. Comments are not stripped,
/// which can only ever produce a false POSITIVE -- a module named in a comment
/// that does not exist -- and that fails loudly rather than passing silently.
fn required_modules(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (index, _) in text.match_indices("require") {
        let rest = text[index + "require".len()..].trim_start();
        let rest = rest.strip_prefix('(').map_or(rest, str::trim_start);
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let body = &rest[quote.len_utf8()..];
        let Some(end) = body.find(quote) else {
            continue;
        };
        let name = &body[..end];
        // A module name is dotted identifiers; anything else is not a literal
        // require target and is left to the reader.
        if !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            found.insert(name.to_string());
        }
    }
    found
}

/// Recursively gather `.lua` files, recording each as (repo-relative path,
/// absolute path) and registering both directories and files for rebuild.
fn collect(directory: &Path, repo_root: &Path, out: &mut Vec<(String, PathBuf)>) {
    // A directory's own mtime changes when a file is added or removed, which is
    // how a deleted module stops being embedded without a `cargo clean`.
    println!("cargo:rerun-if-changed={}", directory.display());

    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("read entry in {}: {error}", directory.display()))
            .path();
        if path.is_dir() {
            collect(&path, repo_root, out);
        } else if path.extension().is_some_and(|extension| extension == "lua") {
            println!("cargo:rerun-if-changed={}", path.display());
            let relative = path
                .strip_prefix(repo_root)
                .expect("walked paths live under the repository root")
                .to_str()
                .expect("source path should be UTF-8")
                .replace(std::path::MAIN_SEPARATOR, "/");
            out.push((relative, path));
        }
    }
}
