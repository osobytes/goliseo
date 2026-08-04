#!/usr/bin/env node
// Measure the wasm sim host's cold start (#334).
//
// "Cold start" here is the time the module spends getting ready BEFORE the
// simulation can be stepped. That is the cost the preload approach added and
// the number this issue exists to move, so it has to be measured rather than
// asserted.
//
// Two marks, both timed from the moment `node simhost.js` is spawned:
//
//   runtime ready  the first line of host output. main() has been entered, so
//                  the wasm module is instantiated and -- under the preload
//                  build -- the .data package has been fetched and mounted
//                  into the virtual filesystem. Everything before this mark is
//                  loader work, not simulation work.
//   sim ready      the "frozen determinism contract" heading. All 13 sim/,
//                  core/ and data/ modules have been resolved and executed, so
//                  the first tick can now be taken.
//
// Timing a child process rather than hooking Module means node's own startup is
// inside both marks. It is not assumed: nodeFloor() measures it on every run and
// prints it (~18 ms on this machine) so it can be subtracted. That
// is deliberate: the emscripten glue declares its own `var Module`, which
// shadows any globalThis.Module a harness sets, so in-process hooks would be
// measuring a module we had quietly failed to instrument. An honest number
// with a stated constant in it beats a precise number that is wrong.
//
//   node wasm/sim-host/measure_cold_start.js [dist-dir] [--runs N]

"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");
const fs = require("node:fs");

const RUNTIME_READY = /^== Rust host/;
const SIM_READY = /^== frozen determinism contract/;

function parseArgs(argv) {
    let dist = path.join(__dirname, "dist");
    let runs = 3;
    for (let i = 0; i < argv.length; i += 1) {
        if (argv[i] === "--runs") {
            runs = Number(argv[i + 1]);
            i += 1;
        } else {
            dist = path.resolve(argv[i]);
        }
    }
    return { dist, runs };
}

// The glue resolves simhost.data relative to the working directory, so the
// child has to run inside dist/ for the preload build to find its package.
function measure(dist) {
    return new Promise((resolve, reject) => {
        const started = process.hrtime.bigint();
        const marks = { runtimeReady: null, simReady: null };
        const child = spawn(process.execPath, ["simhost.js"], { cwd: dist });
        let pending = "";

        const note = (line) => {
            const ms = Number(process.hrtime.bigint() - started) / 1e6;
            if (marks.runtimeReady === null && RUNTIME_READY.test(line)) {
                marks.runtimeReady = ms;
            }
            if (marks.simReady === null && SIM_READY.test(line)) {
                marks.simReady = ms;
                // Both marks are in hand; the 7201-tick fixture that follows is
                // measured elsewhere and would add minutes per run here.
                child.kill("SIGKILL");
            }
        };

        child.stdout.on("data", (chunk) => {
            pending += chunk.toString();
            const lines = pending.split("\n");
            pending = lines.pop();
            for (const line of lines) {
                note(line);
            }
        });
        child.stderr.on("data", (chunk) => process.stderr.write(chunk));
        child.on("error", reject);
        child.on("close", () => {
            if (marks.simReady === null) {
                reject(new Error("host exited before the sim was ready; see stderr above"));
                return;
            }
            resolve(marks);
        });
    });
}

function nodeFloor() {
    return new Promise((resolve) => {
        const started = process.hrtime.bigint();
        const child = spawn(process.execPath, ["-e", ""]);
        child.on("close", () => resolve(Number(process.hrtime.bigint() - started) / 1e6));
    });
}

function summarise(label, values) {
    const sorted = [...values].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)];
    return `${label.padEnd(16)}${median.toFixed(1)} ms   (runs: ${sorted
        .map((v) => v.toFixed(1))
        .join(", ")})`;
}

async function main() {
    const { dist, runs } = parseArgs(process.argv.slice(2));
    if (!fs.existsSync(path.join(dist, "simhost.js"))) {
        console.error(`missing ${path.join(dist, "simhost.js")}; run wasm/sim-host/build.sh first`);
        process.exitCode = 1;
        return;
    }

    const artifacts = fs
        .readdirSync(dist)
        .filter((name) => fs.statSync(path.join(dist, name)).isFile())
        .sort();
    console.log(`dist            ${dist}`);
    for (const name of artifacts) {
        console.log(`  ${name.padEnd(14)}${fs.statSync(path.join(dist, name)).size} bytes`);
    }
    console.log(`node floor      ${(await nodeFloor()).toFixed(1)} ms   (included in both marks)`);
    console.log("");

    const runtime = [];
    const sim = [];
    for (let i = 0; i < runs; i += 1) {
        const marks = await measure(dist);
        runtime.push(marks.runtimeReady);
        sim.push(marks.simReady);
    }
    console.log(summarise("runtime ready", runtime));
    console.log(summarise("sim ready", sim));
}

main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
});
