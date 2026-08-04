//! Rust host for the GOLISEO simulation (#327), with the Lua tree embedded (#334).
//!
//! Step one of proving the simulation can run in WebAssembly: host it in Rust
//! via mlua, natively, and confirm the frozen determinism contract still holds.
//! Native first is deliberate — it separates "does mlua host this codebase at
//! all" from "does Emscripten build it", so a failure points at one thing.
//!
//! The Lua sources are compiled into the binary by `build.rs`, which walks the
//! repository tree and emits an `include_str!` table. They are used UNMODIFIED.
//! If this ever needs a change inside `sim/`, `core/` or `data/` to work, that
//! is a finding about the layering and should be reported rather than patched
//! around.
//!
//! It deliberately runs the same `scripts/phase0_sim_host.lua` probe that the
//! bare interpreter and LÖVE runs use, so all four runtimes are measured doing
//! identical work and the numbers are directly comparable.

use mlua::{Lua, Value};
use std::time::Instant;

/// The generated `include_str!` table. Sorted by path; see `build.rs`.
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_lua.rs"));

    /// Look up a repository-relative path such as `sim/match.lua`.
    pub fn get(path: &str) -> Option<&'static str> {
        let index = SOURCES.binary_search_by_key(&path, |(name, _)| *name).ok()?;
        Some(SOURCES[index].1)
    }
}

const PROBE: &str = "scripts/phase0_sim_host.lua";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("== Rust host (mlua, Lua 5.1)");
    println!("embedded lua    {} files", embedded::SOURCES.len());

    let lua = Lua::new();
    println!("lua version     {}", lua.globals().get::<String>("_VERSION")?);

    install_embedded_searcher(&lua)?;

    let source = embedded::get(PROBE).ok_or("the probe was not embedded; check build.rs ROOTS")?;

    let started = Instant::now();
    lua.load(source).set_name(format!("@{PROBE}")).exec()?;
    let elapsed = started.elapsed();

    println!();
    println!("host wall time  {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    println!("HOST OK: the simulation ran under a Rust-hosted Lua 5.1 state.");
    Ok(())
}

/// Resolve `require` against the embedded table instead of the filesystem.
///
/// Lua 5.1 calls this list `package.loaders`. `package.searchers` is the 5.2+
/// spelling and does not exist here; writing to it would create a table nothing
/// ever reads, and every `require` would fall through to the filesystem — which
/// under Emscripten is empty, so the failure would arrive as a missing module
/// far from its cause.
///
/// The searcher is inserted at position 2: after the `package.preload` searcher,
/// whose precedence is part of Lua's contract, and ahead of the two filesystem
/// ones. That ordering is the reason there is no `cfg` split between native and
/// wasm any more. Both targets now execute exactly the bytes `build.rs` baked
/// in, so a native run genuinely rehearses the browser run instead of quietly
/// reading a different copy off disk.
fn install_embedded_searcher(lua: &Lua) -> mlua::Result<()> {
    let searcher = lua.create_function(|lua, name: String| {
        // The same dotted-name -> path translation the stock `?.lua` and
        // `?/init.lua` templates in `package.path` perform, so a module resolves
        // here to the file the bare interpreter would have opened.
        let base = name.replace('.', "/");
        for candidate in [format!("{base}.lua"), format!("{base}/init.lua")] {
            if let Some(source) = embedded::get(&candidate) {
                // `@` marks the chunk name as a file path, so tracebacks read
                // `sim/match.lua:12:` exactly as they do under `lua`.
                let chunk = lua.load(source).set_name(format!("@{candidate}")).into_function()?;
                return Ok(Value::Function(chunk));
            }
        }
        // Lua 5.1 appends a searcher's string return to `require`'s error, so a
        // genuine typo still reports every place that was looked.
        let miss = format!("\n\tno embedded module '{name}'");
        Ok(Value::String(lua.create_string(miss)?))
    })?;

    lua.load("local searcher = ...\ntable.insert(package.loaders, 2, searcher)")
        .set_name("=[embedded searcher]")
        .call::<()>(searcher)
}
