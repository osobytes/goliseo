#!/usr/bin/env node
// Verify the render frame payload crosses the wasm boundary correctly and
// cheaply (#332), under node.
//
// What this proves, in order of how much it would hurt to be wrong about:
//
//   1. ONE CROSSING PER FRAME. Every export is wrapped in a counter, so the
//      crossings-per-frame figure is measured rather than asserted by reading
//      the source. A renderer loop that called two exports per frame would
//      report 2.00 here and the check would fail.
//   2. A VERSION MISMATCH IS DETECTED. The harness corrupts the layout-version
//      word in linear memory and requires the reader to throw. A protocol whose
//      mismatch handling is never exercised is a protocol that does not have
//      any -- this is the gate's own way of going red.
//   3. ROLLBACK STAYS INSIDE. An eight-tick resim burst is one call, and the
//      state hash before and after a run of bursts must agree. No export hands
//      out a snapshot; the harness checks that too, by name.
//   4. COST, PER FRAME, SEPARATED. Payload extraction is timed apart from
//      simulation using the module's own breakdown of the same call.
//
// Node is not a browser and this does not pretend to be browser evidence:
// `wasm/sim-host/verify_payload_browser.py` runs the same sequence in Chrome
// and Firefox, in a worker, over HTTP. This runs first because it fails faster.
//
//   node wasm/sim-host/verify_payload.js [dist-dir]

"use strict";

const path = require("node:path");
const fs = require("node:fs");

const payload = require("./frame_payload.js");

// p95 update budget from docs/online/omp0_acceptance.md. It is a 60 Hz frame's
// share for simulation, chosen there to leave room for draw and for browser
// scheduling -- so it is the right budget for a rollback burst plus the one
// payload read that feeds the renderer.
const UPDATE_BUDGET_MS = 8.0;
// sim.fixed_clock.MAX_TICKS_PER_UPDATE: the deepest resim a single rendered
// frame can be asked for.
const MAX_RESIM_TICKS = 8;

const WARMUP_TICKS = 300;
const FRAME_SAMPLES = 600;
const BURST_SAMPLES = 200;

function fail(message) {
    console.error(`FAIL: ${message}`);
    process.exitCode = 1;
    throw new Error(message);
}

function percentile(samples, quantile) {
    const sorted = [...samples].sort((a, b) => a - b);
    // Nearest-rank, matching how the rollback campaigns report p95.
    const rank = Math.max(1, Math.ceil(sorted.length * quantile));
    return sorted[Math.min(rank, sorted.length) - 1];
}

function summarise(label, samples) {
    const mean = samples.reduce((a, b) => a + b, 0) / samples.length;
    return (
        `${label.padEnd(50)}${mean.toFixed(4)} ms mean   ` +
        `${percentile(samples, 0.95).toFixed(4)} ms p95   ` +
        `${percentile(samples, 1).toFixed(4)} ms max`
    );
}

// The emscripten glue declares its own `var Module`, which shadows anything a
// harness puts on globalThis -- the same trap `measure_cold_start.js` records.
// Arguments therefore go in through process.argv, which the node branch of the
// glue reads into `programArgs` at load time, and the Module object comes back
// out through `module.exports`. Without `--payload` the default entry point
// would run the 7201-tick fixture instead, which is minutes of work.
async function load(dist) {
    const simhost = path.join(dist, "simhost.js");
    if (!fs.existsSync(simhost)) {
        fail(`missing ${simhost}; run wasm/sim-host/build.sh first`);
    }
    process.argv = [process.argv[0], simhost, "--payload"];
    const Module = require(simhost);
    await new Promise((resolve, reject) => {
        const deadline = Date.now() + 120000;
        const poll = setInterval(() => {
            if (typeof Module._goliseo_payload_frame === "function") {
                clearInterval(poll);
                resolve();
            } else if (Date.now() > deadline) {
                clearInterval(poll);
                reject(new Error("module never exposed the payload exports"));
            }
        }, 5);
    });
    return Module;
}

// Count every crossing, so "one per frame" is a measurement.
function instrument(Module) {
    const counts = new Map();
    const wrapped = {};
    for (const name of Object.keys(Module)) {
        if (!name.startsWith("_goliseo_payload_")) {
            continue;
        }
        const original = Module[name];
        counts.set(name, 0);
        wrapped[name] = (...args) => {
            counts.set(name, counts.get(name) + 1);
            return original(...args);
        };
    }
    return { wrapped, counts };
}

function lastError(Module, api) {
    return Module.UTF8ToString(api._goliseo_payload_error());
}

function hashString(Module, api) {
    const pointer = api._goliseo_payload_hash();
    if (!pointer) {
        fail(`hash failed: ${lastError(Module, api)}`);
    }
    return Module.UTF8ToString(pointer);
}

async function main() {
    const dist = path.resolve(process.argv[2] || path.join(__dirname, "dist"));
    const Module = await load(dist);
    const { wrapped: api, counts } = instrument(Module);

    console.log("== render frame payload over wasm (#332), node");
    console.log(`dist            ${dist}`);

    // Rollback snapshot/restore staying inside the module is a property of the
    // SURFACE, not of a measurement: if no export can return a snapshot, JS
    // cannot be touching per-tick simulation state whatever it does.
    const surface = [...counts.keys()].sort();
    console.log(`exports         ${surface.join(", ")}`);
    const leaks = surface.filter((name) => /snapshot|restore|capture|state|tick/.test(name));
    if (leaks.length > 0) {
        fail(`these exports would let JS touch per-tick simulation state: ${leaks.join(", ")}`);
    }

    const count = api._goliseo_payload_boot();
    if (count < 0) {
        fail(`boot failed: ${lastError(Module, api)}`);
    }

    // The roster crosses ONCE per match. Its two calls are outside the
    // per-frame accounting below, which is the whole point of splitting it out
    // of the frame in #326.
    const roster = payload.readRoster(
        Module.HEAPF64,
        api._goliseo_payload_roster(),
        Module.UTF8ToString(api._goliseo_payload_roster_strings())
    );
    if (roster.count !== count) {
        fail(`roster block says ${roster.count} slots, boot said ${count}`);
    }
    console.log(`roster          ${roster.count} slots, once per match: ${roster.ids.join(", ")}`);

    api._goliseo_payload_frame(WARMUP_TICKS, 0);

    // --- one crossing per frame, and what it costs -------------------------
    //
    // Accounted per export rather than in aggregate, so the diagnostic timing
    // reads -- which a renderer never makes -- are visibly separate from the
    // one call that is the boundary.
    const before = new Map(counts);
    const frameMs = [];
    const extractionMs = [];
    const simMs = [];
    let words = 0;
    for (let index = 0; index < FRAME_SAMPLES; index += 1) {
        const started = performance.now();
        const pointer = api._goliseo_payload_frame(1, 0);
        const frame = payload.readFrame(Module.HEAPF64, pointer);
        frameMs.push(performance.now() - started);
        words = frame.totalWords;
        if (frame.playerCount !== roster.count) {
            fail(`frame carries ${frame.playerCount} players, roster has ${roster.count}`);
        }
        // Diagnostics, deliberately read AFTER the timed region so they are not
        // inside the number they describe.
        simMs.push(api._goliseo_payload_timing(0));
        extractionMs.push(api._goliseo_payload_timing(1) + api._goliseo_payload_timing(2));
    }
    const spent = new Map(
        [...counts].map(([name, value]) => [name, value - (before.get(name) ?? 0)])
    );
    const perFrame = spent.get("_goliseo_payload_frame") / FRAME_SAMPLES;
    const otherPerFrame =
        [...spent].filter(([name]) => name !== "_goliseo_payload_frame" && name !== "_goliseo_payload_timing")
            .reduce((sum, [, value]) => sum + value, 0) / FRAME_SAMPLES;
    console.log("");
    console.log(`frame block     ${words} words (${words * 8} bytes)`);
    console.log(
        `crossings/frame ${perFrame.toFixed(2)} payload` +
            ` + ${otherPerFrame.toFixed(2)} other` +
            ` + ${(spent.get("_goliseo_payload_timing") / FRAME_SAMPLES).toFixed(2)} diagnostic` +
            ` (a renderer makes only the first)`
    );
    if (perFrame !== 1 || otherPerFrame !== 0) {
        fail(`expected exactly one boundary crossing per frame, measured ${perFrame + otherPerFrame}`);
    }
    // PAYLOAD EXTRACTION, ALONE. `ticks = 0` simulates nothing, so the call is
    // exactly build + encode + copy, and the read after it is the JS side. This
    // is the per-frame number #332 is accountable for, measured from where a
    // renderer would measure it rather than read out of the module.
    const aloneMs = [];
    for (let index = 0; index < FRAME_SAMPLES; index += 1) {
        const started = performance.now();
        payload.readFrame(Module.HEAPF64, api._goliseo_payload_frame(0, 0));
        aloneMs.push(performance.now() - started);
    }

    console.log("");
    console.log(summarise("simulation  1 tick + snapshot capture", simMs));
    console.log(summarise("payload     EXTRACTION, 0 ticks, wall clock", aloneMs));
    console.log(summarise("payload     extraction, in-module breakdown", extractionMs));
    console.log(summarise("frame call  1 tick + payload, wall clock", frameMs));
    console.log(
        `payload share of the ${UPDATE_BUDGET_MS.toFixed(1)} ms budget: ` +
            `${((percentile(aloneMs, 0.95) / UPDATE_BUDGET_MS) * 100).toFixed(1)}% at p95`
    );

    // --- the eight-tick rollback burst -------------------------------------
    console.log("");
    console.log(`rollback bursts, one crossing each, against a ${UPDATE_BUDGET_MS.toFixed(1)} ms budget`);
    const hashBefore = hashString(Module, api);
    let worst = 0;
    for (const depth of [1, 2, 4, MAX_RESIM_TICKS]) {
        const burstMs = [];
        for (let index = 0; index < BURST_SAMPLES; index += 1) {
            const started = performance.now();
            const pointer = api._goliseo_payload_frame(depth, 1);
            payload.readFrame(Module.HEAPF64, pointer);
            burstMs.push(performance.now() - started);
        }
        const p95 = percentile(burstMs, 0.95);
        if (depth === MAX_RESIM_TICKS) {
            worst = p95;
        }
        console.log(
            `  ${summarise(`${depth}-tick resim + payload`, burstMs)}  -> ` +
                (p95 <= UPDATE_BUDGET_MS ? "WITHIN BUDGET" : "OVER BUDGET")
        );
    }
    const hashAfter = hashString(Module, api);
    console.log("");
    console.log(`state hash before bursts  ${hashBefore}`);
    console.log(`state hash after bursts   ${hashAfter}`);
    if (hashBefore !== hashAfter) {
        fail("a rollback burst did not reproduce the state it rolled back over");
    }
    console.log("rollback determinism      MATCH");

    // --- a mismatch must be detected, not misread --------------------------
    //
    // Written straight into linear memory rather than by building a fake block:
    // the reader has to reject a block that is real in every other respect,
    // which is exactly the shape a version skew would take.
    console.log("");
    const pointer = api._goliseo_payload_frame(1, 0);
    const base = pointer / 8;
    for (const [word, name] of [
        [0, "magic"],
        [1, "layout version"],
        [2, "render frame version"],
        [7, "player field count"],
    ]) {
        const original = Module.HEAPF64[base + word];
        Module.HEAPF64[base + word] = original + 1;
        let detected = false;
        try {
            payload.readFrame(Module.HEAPF64, pointer);
        } catch (error) {
            detected = error instanceof payload.PayloadVersionError;
        }
        Module.HEAPF64[base + word] = original;
        console.log(`mismatch detected: corrupted ${name.padEnd(22)}${detected ? "yes" : "NO"}`);
        if (!detected) {
            fail(`a corrupted ${name} was read as valid`);
        }
    }
    // ...and the unchanged block still reads, so the check above is not simply
    // rejecting everything.
    payload.readFrame(Module.HEAPF64, pointer);

    console.log("");
    console.log(`update budget             ${UPDATE_BUDGET_MS.toFixed(4)} ms p95 (omp0_acceptance)`);
    console.log(
        `${MAX_RESIM_TICKS}-tick worst case        ${worst.toFixed(4)} ms p95  -> ` +
            (worst <= UPDATE_BUDGET_MS ? "WITHIN BUDGET" : "OVER BUDGET")
    );
    console.log("");
    console.log("PAYLOAD OK: one crossing per frame, versioned, rollback inside the module.");
}

main().catch((error) => {
    if (process.exitCode !== 1) {
        console.error(error);
    }
    process.exitCode = 1;
});
