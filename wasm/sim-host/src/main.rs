//! Rust host for the GOLISEO simulation (#327).
//!
//! Step one of proving the simulation can run in WebAssembly: host it in Rust
//! via mlua, natively, and confirm the frozen determinism contract still holds.
//! Native first is deliberate — it separates "does mlua host this codebase at
//! all" from "does Emscripten build it", so a failure points at one thing.
//!
//! The Lua sources are loaded from the repository tree UNMODIFIED. If this ever
//! needs a change inside `sim/`, `core/` or `data/` to work, that is a finding
//! about the layering and should be reported rather than patched around.
//!
//! It deliberately runs the same `scripts/phase0_sim_host.lua` probe that the
//! bare interpreter and LÖVE runs use, so all four runtimes are measured doing
//! identical work and the numbers are directly comparable.

use mlua::Lua;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Repository root, so the probe's relative `package.path` and `require` calls
/// resolve exactly as they do under the bare interpreter.
fn repo_root() -> PathBuf {
    if let Ok(explicit) = std::env::var("GOLISEO_ROOT") {
        return PathBuf::from(explicit);
    }
    // The crate lives at <root>/wasm/sim-host.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate should sit two levels below the repository root")
        .to_path_buf()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();
    println!("== Rust host (mlua, Lua 5.1)");
    println!("repo root       {}", root.display());

    // The probe resolves modules relative to the working directory, matching how
    // `love .` and the bare interpreter are invoked.
    std::env::set_current_dir(&root)?;

    let lua = Lua::new();
    println!("lua version     {}", lua.globals().get::<String>("_VERSION")?);

    let probe = root.join("scripts/phase0_sim_host.lua");
    let source = std::fs::read_to_string(&probe)?;

    let started = Instant::now();
    lua.load(&source).set_name("phase0_sim_host.lua").exec()?;
    let elapsed = started.elapsed();

    println!();
    println!("host wall time  {:.1} ms", elapsed.as_secs_f64() * 1000.0);
    println!("HOST OK: the simulation ran under a Rust-hosted Lua 5.1 state.");
    Ok(())
}
