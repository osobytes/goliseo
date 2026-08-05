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
//!
//! # Two entry points, and why the default one did not move (#332)
//!
//! With no arguments this is still exactly the #327 fixture runner: it executes
//! the probe and exits. That path is load-bearing — `verify_browser.py` drives
//! it and asserts the frozen hashes — so it is untouched, and the payload work
//! below is reached only by passing `--payload`.
//!
//! `--payload` turns the binary from a program that runs once into a MODULE
//! with a callable surface, because a renderer does not want a fixture result,
//! it wants one render frame per rendered frame. The surface is deliberately
//! tiny and appears in `payload::` below. Under Emscripten it is reached from
//! JavaScript through the exported `_goliseo_payload_*` symbols; natively it
//! runs a benchmark in-process, which is how the per-frame numbers get measured
//! on a clock that is not rounded to a browser's timer resolution.

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
    if std::env::args().any(|argument| argument == "--payload") {
        return payload::main();
    }
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
/// whose precedence is part of Lua's contract, and ahead of the filesystem one.
/// Singular — mlua's safe `Lua::new()` applies `disable_c_modules()`, which
/// replaces loader 3 (the C-path searcher) with a permanently-failing stub and
/// drops loader 4 (the all-in-one loader). The live table is therefore
/// `[preload, Lua path, disabled C stub]`, not stock Lua 5.1's four.
///
/// That ordering is the reason there is no `cfg` split between native and wasm
/// any more. Both targets now execute exactly the bytes `build.rs` baked in, so
/// a native run genuinely rehearses the browser run instead of quietly reading a
/// different copy off disk.
///
/// A module reading `...` at file scope behaves identically here and under the
/// stock loader, which is worth recording because it is not obvious and 5.2+
/// would answer differently. Lua 5.1's `require` calls the winning loader with
/// exactly one argument, the module name (`lua_call(L, 1, 1)`, loadlib.c:473);
/// the extra loader-data value that 5.2 added, and which would show up as a
/// second vararg, does not exist in this version. No embedded module reads
/// chunk-scope `...` today, and if one starts to, it sees the same thing it
/// would under `lua`.
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

/// The render frame payload boundary (#332).
///
/// # The one rule this module exists to keep
///
/// ONE CROSSING PER RENDERED FRAME, IN BATCH. `goliseo_payload_frame` is that
/// crossing: it advances (or rolls back and re-simulates) the match, builds the
/// `RenderFrame`, serialises it into one flat block of doubles and returns a
/// pointer into linear memory. JavaScript then reads the whole frame through a
/// single `Float64Array` view. Nothing is marshalled per field, per entity or
/// per tick, and there is no export through which a caller could do so.
///
/// The block's own header carries `total_words`, so sizing the view costs no
/// second call — the reader takes it out of memory it is already looking at.
/// That is why there is no `goliseo_payload_frame_len`: adding one would make
/// the honest answer to "how many crossings per frame" two.
///
/// # Rollback stays inside
///
/// `goliseo_payload_frame(ticks, rollback = 1)` restores a snapshot from
/// `ticks` ticks ago and replays the recorded inputs, up to the fixed clock's
/// eight-tick maximum, entirely inside this module. Snapshot bytes are never
/// exported and there is no way to ask for one. An eight-tick resim burst
/// therefore costs the boundary exactly what a quiet frame costs it: one call.
///
/// # Pointer lifetime
///
/// The returned pointer is owned by this module and is valid until the next
/// `goliseo_payload_frame` call. A caller re-reads it every frame anyway, since
/// that call is what produces the frame. Separately: `ALLOW_MEMORY_GROWTH` is
/// on, so a JavaScript caller must re-read `Module.HEAPF64` each frame — the
/// wasm address is stable, the JS view over it is not.
mod payload {
    use super::install_embedded_searcher;
    use mlua::{Function, Lua, Table};
    use std::cell::RefCell;
    use std::ffi::{c_char, CString};
    use std::time::Instant;

    /// The simulation side of the boundary; see the module's own header.
    const HOST_MODULE: &str = "scripts.wasm_frame_payload_host";

    /// Word index (0-based) of `total_words` in both block headers. Mirrors
    /// `render.frame_buffer`; a reader uses it to size its view.
    const TOTAL_WORDS_INDEX: usize = 3;

    struct Host {
        /// Kept alive because every cached handle below borrows from it.
        _lua: Lua,
        advance: Function,
        rollback: Function,
        frame: Function,
        hash: Function,
        /// Reused per frame, so a steady-state frame allocates nothing.
        frame_words: Vec<f64>,
        roster_words: Vec<f64>,
        roster_strings: CString,
        last_hash: CString,
        /// Diagnostics for the measurement harnesses only. NOT part of the
        /// per-frame path: reading them is an extra crossing, which is exactly
        /// why the harness times the frame call itself and treats these as a
        /// breakdown of a number it already has.
        lua_ms: f64,
        copy_ms: f64,
        sim_ms: f64,
    }

    thread_local! {
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
        static ERROR: RefCell<CString> = RefCell::new(CString::default());
    }

    /// Copy a 1-based Lua array of numbers into a flat `Vec<f64>`.
    ///
    /// This is the only place the payload is touched element by element, and it
    /// is inside the module, not across the boundary — which is the whole
    /// point. Its cost is reported separately (`copy_ms`) so the claim can be
    /// checked rather than believed.
    fn copy_words(table: &Table, out: &mut Vec<f64>) -> mlua::Result<()> {
        let len = table.raw_len();
        out.clear();
        out.reserve(len);
        for index in 1..=len {
            out.push(table.raw_get::<f64>(index)?);
        }
        Ok(())
    }

    fn boot() -> mlua::Result<Host> {
        let lua = Lua::new();
        install_embedded_searcher(&lua)?;
        let require: Function = lua.globals().get("require")?;
        let module: Table = require.call(HOST_MODULE)?;
        module.get::<Function>("boot")?.call::<i64>(())?;

        let (roster_table, roster_strings): (Table, String) =
            module.get::<Function>("roster")?.call(())?;
        let mut roster_words = Vec::new();
        copy_words(&roster_table, &mut roster_words)?;

        Ok(Host {
            advance: module.get("advance")?,
            rollback: module.get("rollback")?,
            frame: module.get("frame")?,
            hash: module.get("hash")?,
            _lua: lua,
            frame_words: Vec::new(),
            roster_words,
            // A NUL in a roster id would truncate the blob silently, so it is
            // rejected here rather than trusted. `frame_buffer` already refuses
            // newlines for the same reason.
            roster_strings: CString::new(roster_strings)
                .map_err(|error| mlua::Error::runtime(format!("roster blob: {error}")))?,
            last_hash: CString::default(),
            lua_ms: 0.0,
            copy_ms: 0.0,
            sim_ms: 0.0,
        })
    }

    impl Host {
        fn produce(&mut self, ticks: i32, rollback: bool) -> mlua::Result<()> {
            // Nothing in here hashes. `sim_ms` is meant to be the simulation
            // cost a rendered frame actually pays, and a canonical-encode hash
            // costs several times an eight-tick resim; see `host.rollback`.
            let simulated = Instant::now();
            if rollback {
                self.rollback.call::<i64>(ticks)?;
            } else if ticks > 0 {
                self.advance.call::<i64>(ticks)?;
            }
            self.sim_ms = millis(simulated);

            let built = Instant::now();
            let words: Table = self.frame.call(())?;
            self.lua_ms = millis(built);

            let copied = Instant::now();
            copy_words(&words, &mut self.frame_words)?;
            self.copy_ms = millis(copied);
            Ok(())
        }

        fn refresh_hash(&mut self) -> mlua::Result<()> {
            let hash: String = self.hash.call(())?;
            self.last_hash = CString::new(hash)
                .map_err(|error| mlua::Error::runtime(format!("hash: {error}")))?;
            Ok(())
        }
    }

    fn millis(since: Instant) -> f64 {
        since.elapsed().as_secs_f64() * 1000.0
    }

    fn record(error: &str) {
        ERROR.with(|slot| {
            *slot.borrow_mut() = CString::new(error).unwrap_or_default();
        });
    }

    /// Run `body` against the live host, turning any Lua error into a recorded
    /// message and a sentinel return. `extern "C"` functions must not unwind.
    fn with_host<T>(sentinel: T, body: impl FnOnce(&mut Host) -> mlua::Result<T>) -> T {
        HOST.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(host) = slot.as_mut() else {
                record("goliseo_payload_boot has not been called");
                return sentinel;
            };
            match body(host) {
                Ok(value) => value,
                Err(error) => {
                    record(&error.to_string());
                    sentinel
                }
            }
        })
    }

    /// Create the match and encode the match-constant roster. Returns the
    /// roster slot count, or -1 with `goliseo_payload_error` set.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_boot() -> i32 {
        match boot() {
            Ok(host) => {
                let count = host.roster_words[5] as i32;
                HOST.with(|slot| *slot.borrow_mut() = Some(host));
                count
            }
            Err(error) => {
                record(&error.to_string());
                -1
            }
        }
    }

    /// Pointer to the roster block. Crosses ONCE per match, not per frame:
    /// nothing in it can change while a match is running. Its header carries
    /// `total_words` at word 3, like the frame block's does.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_roster() -> *const f64 {
        with_host(std::ptr::null(), |host| Ok(host.roster_words.as_ptr()))
    }

    /// The roster's id/name blob: newline-joined `id`, `name`, ... in slot
    /// order. Also once per match — no string crosses this boundary per frame.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_roster_strings() -> *const c_char {
        with_host(std::ptr::null(), |host| Ok(host.roster_strings.as_ptr()))
    }

    /// THE per-frame crossing. Advances `ticks` ticks (or, with `rollback`
    /// non-zero, restores a snapshot from `ticks` ticks ago and re-simulates
    /// them), then returns a pointer to one complete serialised render frame.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_frame(ticks: i32, rollback: i32) -> *const f64 {
        with_host(std::ptr::null(), |host| {
            host.produce(ticks, rollback != 0)?;
            Ok(host.frame_words.as_ptr())
        })
    }

    /// `match_snapshot.hash` of the current state. Diagnostic: it is what
    /// proves a rollback burst reproduced the state it rolled back over, and it
    /// is a hash, never the snapshot itself.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_hash() -> *const c_char {
        with_host(std::ptr::null(), |host| {
            host.refresh_hash()?;
            Ok(host.last_hash.as_ptr())
        })
    }

    /// In-module breakdown of the last frame, in milliseconds. Diagnostic only.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_timing(which: i32) -> f64 {
        with_host(-1.0, |host| {
            Ok(match which {
                0 => host.sim_ms,
                1 => host.lua_ms,
                2 => host.copy_ms,
                _ => -1.0,
            })
        })
    }

    /// The last recorded failure, or an empty string.
    #[no_mangle]
    pub extern "C" fn goliseo_payload_error() -> *const c_char {
        ERROR.with(|slot| slot.borrow().as_ptr())
    }

    #[cfg(target_os = "emscripten")]
    extern "C" {
        /// Return from `main` WITHOUT tearing the runtime down, so the exports
        /// above stay callable. Built with `EXIT_RUNTIME=1` — which the default
        /// fixture path depends on to exit — so this is the supported way to
        /// opt one entry point out of it without changing the other's
        /// behaviour.
        fn emscripten_exit_with_live_runtime() -> !;
    }

    #[cfg(target_os = "emscripten")]
    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        // Boot eagerly so a failure is reported on stdout, where the harness is
        // already listening, rather than as a null pointer on the first call.
        let count = goliseo_payload_boot();
        if count < 0 {
            println!("PAYLOAD HOST FAILED: {}", last_error());
            return Err(last_error().into());
        }
        println!("== payload host (#332)");
        println!("roster slots    {count}");
        println!("PAYLOAD HOST READY");
        unsafe { emscripten_exit_with_live_runtime() }
    }

    fn last_error() -> String {
        ERROR.with(|slot| slot.borrow().to_string_lossy().into_owned())
    }

    /// Natively there is no JavaScript to call the exports, so `--payload`
    /// measures them here instead. Two reasons this is the primary measurement
    /// rather than a convenience:
    ///
    /// 1. `Instant` on a native clock has nanosecond resolution. A browser's
    ///    `performance.now()` is coarsened against timing attacks — to 0.1 ms
    ///    or worse in a worker — which is the same order as the number being
    ///    measured, so a browser can only honestly time a BATCH of frames.
    /// 2. It runs without Emscripten or Docker, so the payload cost is
    ///    measurable on any checkout.
    ///
    /// The browser harnesses (`verify_payload.js`, `verify_payload_browser.py`)
    /// then confirm the same work fits the same budget where it has to.
    #[cfg(not(target_os = "emscripten"))]
    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        const WARMUP_TICKS: i32 = 300;
        const SAMPLES: usize = 600;
        const BURSTS: usize = 200;
        /// `sim.fixed_clock.MAX_TICKS_PER_UPDATE`; the deepest resim one
        /// rendered frame can be asked for.
        const BURST_TICKS: i32 = 8;
        /// p95 update budget from `docs/online/omp0_acceptance.md`.
        const UPDATE_BUDGET_MS: f64 = 8.0;

        println!("== payload host (#332), native");
        report_conditions();
        let count = goliseo_payload_boot();
        if count < 0 {
            return Err(last_error().into());
        }
        println!("roster slots    {count}");

        // Warm up into a state with the ball in play, so the measured frames
        // are representative rather than kickoff-quiet.
        if goliseo_payload_frame(WARMUP_TICKS, 0).is_null() {
            return Err(last_error().into());
        }
        let words = frame_len();
        println!("frame block     {words} words ({} bytes)", words * 8);
        println!();

        // The quiet frame: one tick (with the snapshot capture rollback needs
        // to be possible at all) plus one payload read. `sim_ms` / `lua_ms` /
        // `copy_ms` come from the module's own breakdown of the very call the
        // outer clock is timing, so the parts and the whole are the same event.
        let mut sim = Vec::with_capacity(SAMPLES);
        let mut lua = Vec::with_capacity(SAMPLES);
        let mut copy = Vec::with_capacity(SAMPLES);
        let mut total = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            if goliseo_payload_frame(1, 0).is_null() {
                return Err(last_error().into());
            }
            total.push(millis(started));
            sim.push(goliseo_payload_timing(0));
            lua.push(goliseo_payload_timing(1));
            copy.push(goliseo_payload_timing(2));
        }
        // Payload extraction alone: `ticks = 0` simulates nothing, so the whole
        // call is build + encode + copy. Timed from outside, so it includes the
        // call overhead a caller actually pays rather than only the parts the
        // module can see.
        let mut alone = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            if goliseo_payload_frame(0, 0).is_null() {
                return Err(last_error().into());
            }
            alone.push(millis(started));
        }

        report("simulation  1 tick + snapshot capture", &sim);
        report("payload     build + encode (lua)", &lua);
        report("payload     copy into linear memory", &copy);
        report("payload     extraction, in-module breakdown", &sum(&lua, &copy));
        report("payload     EXTRACTION, 0 ticks, one crossing", &alone);
        report("frame call  1 tick + payload, one crossing", &total);
        println!(
            "payload share of the {UPDATE_BUDGET_MS:.1} ms budget: {:.1}% at p95",
            percentile(&alone, 0.95) / UPDATE_BUDGET_MS * 100.0
        );
        println!();

        // Rollback depth sweep, because "does it fit" has a different answer at
        // each depth and only reporting the worst one hides where the cliff is.
        // Every row is one crossing: restore, resim, build, encode, copy.
        println!("rollback bursts (one crossing each), against a {UPDATE_BUDGET_MS:.1} ms budget");
        let before = hash_string();
        let mut deepest_fitting = 0;
        let mut worst_case = 0.0;
        for depth in [1, 2, 4, BURST_TICKS] {
            let mut burst = Vec::with_capacity(BURSTS);
            let mut payload = Vec::with_capacity(BURSTS);
            for _ in 0..BURSTS {
                let started = Instant::now();
                if goliseo_payload_frame(depth, 1).is_null() {
                    return Err(last_error().into());
                }
                burst.push(millis(started));
                payload.push(goliseo_payload_timing(1) + goliseo_payload_timing(2));
            }
            let p95 = percentile(&burst, 0.95);
            let fits = p95 <= UPDATE_BUDGET_MS;
            if fits {
                deepest_fitting = depth;
            }
            if depth == BURST_TICKS {
                worst_case = p95;
            }
            println!(
                "  {depth}-tick resim + payload   {:.4} ms mean   {p95:.4} ms p95   \
                 {:.4} ms max   (payload {:.4} ms p95, {:.1}% of budget)  -> {}",
                burst.iter().sum::<f64>() / burst.len() as f64,
                percentile(&burst, 1.0),
                percentile(&payload, 0.95),
                percentile(&payload, 0.95) / UPDATE_BUDGET_MS * 100.0,
                if fits { "WITHIN BUDGET" } else { "OVER BUDGET" }
            );
        }
        let after = hash_string();
        println!();
        println!("state hash before bursts  {before}");
        println!("state hash after bursts   {after}");
        println!(
            "rollback determinism      {}",
            if before == after { "MATCH" } else { "DIVERGED" }
        );

        println!();
        println!("update budget             {UPDATE_BUDGET_MS:.4} ms p95 (omp0_acceptance)");
        println!("deepest fitting resim     {deepest_fitting} ticks");
        println!(
            "{BURST_TICKS}-tick worst case        {worst_case:.4} ms p95  -> {}",
            if worst_case <= UPDATE_BUDGET_MS { "WITHIN BUDGET" } else { "OVER BUDGET" }
        );

        // Correctness is what fails this program. The budget verdict is a
        // MEASUREMENT, and turning a measurement into an exit code is how a
        // number stops being reported once it becomes inconvenient. It is
        // printed above, in full, at every depth, and read by a human.
        if before != after {
            return Err("rollback did not reproduce the state it rolled back over".into());
        }
        report_conditions();
        println!();
        println!("PAYLOAD OK: one crossing per frame, rollback reproduced its own state.");
        Ok(())
    }

    /// The conditions the numbers were produced under.
    ///
    /// Not decoration: the difference between a 5.1 ms and a 13.1 ms p95 for
    /// the same artifact on the same machine was load average. A measurement
    /// without its conditions is not reproducible, only re-runnable -- so this
    /// prints before and after every run, and says plainly when the machine was
    /// too busy for the tail numbers to mean anything.
    #[cfg(not(target_os = "emscripten"))]
    fn report_conditions() {
        let cores = std::thread::available_parallelism().map_or(0, |count| count.get());
        let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let fields: Vec<&str> = load.split_whitespace().take(3).collect();
        let load1: f64 = fields.first().and_then(|v| v.parse().ok()).unwrap_or(f64::NAN);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        println!(
            "conditions      unix {stamp}   {cores} cores   load {}{}",
            if fields.is_empty() { "unavailable".into() } else { fields.join("/") },
            if load1 > cores as f64 * 0.5 {
                "   CONTENDED - tail numbers below are inflated"
            } else {
                ""
            }
        );
    }

    #[cfg(not(target_os = "emscripten"))]
    fn frame_len() -> usize {
        HOST.with(|slot| {
            slot.borrow().as_ref().map_or(0, |host| host.frame_words[TOTAL_WORDS_INDEX] as usize)
        })
    }

    #[cfg(not(target_os = "emscripten"))]
    fn hash_string() -> String {
        let pointer = goliseo_payload_hash();
        if pointer.is_null() {
            return last_error();
        }
        unsafe { std::ffi::CStr::from_ptr(pointer) }.to_string_lossy().into_owned()
    }

    #[cfg(not(target_os = "emscripten"))]
    fn sum(left: &[f64], right: &[f64]) -> Vec<f64> {
        left.iter().zip(right).map(|(a, b)| a + b).collect()
    }

    #[cfg(not(target_os = "emscripten"))]
    fn percentile(samples: &[f64], quantile: f64) -> f64 {
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("timings are never NaN"));
        // Nearest-rank, matching how the rollback campaigns report p95.
        let rank = ((sorted.len() as f64) * quantile).ceil().max(1.0) as usize;
        sorted[rank.min(sorted.len()) - 1]
    }

    #[cfg(not(target_os = "emscripten"))]
    fn report(label: &str, samples: &[f64]) {
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        println!(
            "{label:<50}{:.4} ms mean   {:.4} ms p95   {:.4} ms max",
            mean,
            percentile(samples, 0.95),
            percentile(samples, 1.0)
        );
    }
}
