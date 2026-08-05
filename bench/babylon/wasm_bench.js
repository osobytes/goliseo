/*
 * GOLISEO — Babylon.js driven by the LIVE wasm simulation (#328).
 *
 * WHAT THIS IS, AND WHAT #341 LEFT
 *
 * #341 measured Babylon's skinned-character cost from a FILE of captured
 * `RenderFrame` payloads, and said so: live wasm integration was explicitly out
 * of its scope. #332 landed the boundary — one crossing per rendered frame,
 * structure-of-arrays in shared linear memory read through a typed-array view.
 * This page is the join. The scene, the character build, the clip table and the
 * measurement loop are `scene.js`, byte for byte the ones #341 measured; the
 * only thing that changed is where a pose comes from.
 *
 * THE ONE CROSSING, AND WHY IT IS MEASURED RATHER THAN ASSERTED
 *
 * Per rendered frame this page calls `_goliseo_payload_frame(1, 0)` exactly
 * once. That call advances the match one fixed tick, builds the `RenderFrame`,
 * serialises it into linear memory and returns a pointer;
 * `GoliseoFramePayload.readFrame` then hands back `Float64Array` subarrays over
 * the module's own memory, so nothing is copied and nothing is marshalled per
 * entity. Every `_goliseo_payload_*` export is wrapped in a counter — not just
 * the frame call — so "one crossing per frame" is a number in the result marker
 * that the Python runner FAILS on, rather than a claim about the source.
 *
 * `Module.HEAPF64` is re-read every frame on purpose: `ALLOW_MEMORY_GROWTH` is
 * on, and growing the heap detaches every existing view. The wasm address is
 * stable across growth; the JavaScript view over it is not.
 *
 * THE SIMULATION RUNS ON THE RENDER THREAD HERE, AND THAT IS A CHOICE
 *
 * A shipped build would put the match in a worker so an eight-tick rollback
 * burst cannot stall a frame — that is what `verify_payload_browser.py`
 * measures. This page deliberately does not, for two reasons. It is the
 * arrangement in which the payload is genuinely zero-copy (a worker has to
 * clone the block through postMessage), so it measures the boundary #332
 * designed rather than a transport on top of it; and it puts the simulation's
 * cost inside `update`, next to the draw cost, where a reader can see both.
 * Frame time here is therefore a PESSIMISTIC single-thread figure; the draw
 * numbers, which are what #328 compares, are unaffected because `draw` times
 * only `scene.render()`.
 *
 * DOES RENDERING PERTURB THE SIMULATION?
 *
 * That is the one genuinely new correctness property in #328, and it is checked
 * three ways when `?determinism=1`:
 *
 *   1. The same match, same tick count, run WITH rendering and then WITHOUT it,
 *      in the same page, in the same module instance. The two state hashes must
 *      agree. This is the direct test, and it is the one that can fail.
 *   2. A second module instance in a Web Worker runs the frozen 7201-tick
 *      determinism contract WHILE this thread renders, and must still produce
 *      `bfbb106aea5480f8` / `a190b60058a64e63`.
 *   3. Both hashes are compared across Chrome and Firefox, and against a node
 *      control, by the Python runner.
 */

(function () {
    "use strict";

    const S = globalThis.GoliseoBenchScene;
    const P = globalThis.GoliseoFramePayload;

    const params = new URLSearchParams(window.location.search);
    const config = {
        model: params.get("model") || "vendor/character.glb",
        count: Math.max(1, parseInt(params.get("count") || "10", 10)),
        frames: Math.max(1, parseInt(params.get("frames") || "600", 10)),
        warmup: Math.max(0, parseInt(params.get("warmup") || "300", 10)),
        variant: S.VARIANTS.has(params.get("variant")) ? params.get("variant") : "merged",
        width: Math.max(320, parseInt(params.get("width") || "960", 10)),
        height: Math.max(240, parseInt(params.get("height") || "540", 10)),
        shadows: params.get("shadows") !== "off",
        determinism: params.get("determinism") === "1",
        // Ticks each hash pass advances. Long enough that a perturbation has
        // somewhere to show up, short enough that the two passes plus the
        // measured run fit in one page load.
        hashTicks: Math.max(1, parseInt(params.get("hash_ticks") || "600", 10)),
        // How long the page keeps rendering while waiting for the frozen
        // fixture worker. Overridable because the wait is a race against a
        // second wasm module on a contended machine, not a fixed cost.
        fixtureDeadlineMs: Math.max(1000, parseInt(params.get("fixture_deadline_ms") || "300000", 10)),
    };

    // Every `_goliseo_payload_*` export, wrapped. Discovered dynamically rather
    // than listed, exactly as `verify_payload.js` does it: hand-wrapping only
    // the frame call would make "one crossing per frame" true by construction
    // instead of by measurement.
    const crossings = new Map();
    let api = null;

    // Running totals as well as the per-name map. The map is the surface report;
    // these two are what the PER-FRAME counter reads, and they are scalars so
    // reading them inside the timed `update` region costs two loads rather than
    // a walk over the map.
    let totalCalls = 0;
    let frameCalls = 0;

    function instrument() {
        const wrapped = {};
        for (const name of Object.keys(Module)) {
            if (!name.startsWith("_goliseo_payload_")) {
                continue;
            }
            const original = Module[name];
            const isFrame = name === "_goliseo_payload_frame";
            crossings.set(name, 0);
            wrapped[name] = (...args) => {
                crossings.set(name, crossings.get(name) + 1);
                totalCalls += 1;
                if (isFrame) {
                    frameCalls += 1;
                }
                return original(...args);
            };
        }
        return wrapped;
    }

    /*
     * PER-FRAME crossing counts, not an average of them.
     *
     * The first version of this reported `frameCalls / renderedFrames` and
     * nothing else. That mean is 1.00 for a loop that crosses exactly once every
     * frame -- and equally 1.00 for one that crosses twice on one frame and not
     * at all on the next. The rule this boundary exists to enforce is "one
     * crossing per rendered frame", so a counter that cannot tell "always 1"
     * from "averages 1" is weaker than the claim it backs. AGENTS.md section 9:
     * never trust one signal.
     *
     * So every frame's delta is recorded, and min and max travel with the mean
     * into the result marker. The Python runner gates on `min == max == 1`; the
     * mean is kept beside it as the second signal rather than the only one.
     */
    const perFrame = {
        frames: 0,
        payloadMin: Infinity,
        payloadMax: 0,
        payloadSum: 0,
        otherMin: Infinity,
        otherMax: 0,
        otherSum: 0,
    };

    function recordFrameCrossings(payload, other) {
        perFrame.frames += 1;
        perFrame.payloadSum += payload;
        perFrame.otherSum += other;
        if (payload < perFrame.payloadMin) {
            perFrame.payloadMin = payload;
        }
        if (payload > perFrame.payloadMax) {
            perFrame.payloadMax = payload;
        }
        if (other < perFrame.otherMin) {
            perFrame.otherMin = other;
        }
        if (other > perFrame.otherMax) {
            perFrame.otherMax = other;
        }
    }

    function crossingTotal(exclude) {
        let total = 0;
        for (const [name, value] of crossings) {
            if (name !== exclude) {
                total += value;
            }
        }
        return total;
    }

    function waitForExports() {
        return new Promise((resolve, reject) => {
            const started = Date.now();
            const poll = setInterval(() => {
                if (typeof Module._goliseo_payload_frame === "function") {
                    clearInterval(poll);
                    resolve();
                } else if (Date.now() - started > 120000) {
                    clearInterval(poll);
                    reject(new Error("the wasm module never exposed the payload exports"));
                }
            }, 10);
        });
    }

    // Through `api`, not through `Module`. This is only ever reached on a call
    // that already failed and is about to throw, so routing it around the
    // wrapper would cost nothing measurable -- but "every `_goliseo_payload_*`
    // export is wrapped" is the sentence that makes the crossing count
    // trustworthy, and one export quietly exempt from it is exactly the kind of
    // hole that makes the whole claim unverifiable. No export escapes the
    // counter, including the ones that cannot matter.
    function lastError() {
        return Module.UTF8ToString(api._goliseo_payload_error());
    }

    function hashNow() {
        const pointer = api._goliseo_payload_hash();
        if (!pointer) {
            throw new Error(`hash failed: ${lastError()}`);
        }
        return Module.UTF8ToString(pointer);
    }

    // ONE crossing. Everything after it is ordinary memory reads on this side.
    function nextFrame(ticks) {
        const pointer = api._goliseo_payload_frame(ticks, 0);
        if (!pointer) {
            throw new Error(`frame failed: ${lastError()}`);
        }
        return P.readFrame(Module.HEAPF64, pointer);
    }

    async function main() {
        S.setStatus("waiting for the wasm simulation");
        await waitForExports();
        api = instrument();

        // The roster crosses ONCE per match and is not part of the per-frame
        // accounting. It carries the only strings that ever cross at all.
        const roster = P.readRoster(
            Module.HEAPF64,
            api._goliseo_payload_roster(),
            Module.UTF8ToString(api._goliseo_payload_roster_strings()),
        );
        const first = nextFrame(0);
        if (first.playerCount !== roster.count) {
            throw new Error(
                `frame carries ${first.playerCount} players, roster has ${roster.count}`,
            );
        }
        const rosterCrossings = crossingTotal(null);

        const field = {
            w: first.scalars.field_w,
            h: first.scalars.field_h,
            penalty_box_depth: first.scalars.field_penalty_box_depth,
            penalty_box_h: first.scalars.field_penalty_box_h,
            goal_h: first.scalars.goal_home_h,
        };

        const { engine, scene, geometry, shadowGenerator, ball } = S.createScene(config, field);
        const gpu = S.gpuIdentity(engine);

        // A live match can select any pose family, so every clip the mapping can
        // reach is kept rather than only those a particular capture happened to
        // contain. Unplayed groups are stopped and cost nothing per frame; what
        // this changes against #341 is load time and memory, and `asset_clips`
        // in the result marker records it so the difference is visible.
        S.setStatus("loading character");
        const container = await S.loadContainer(scene, config.model);
        const assetClips = S.pruneClips(container, S.requiredClips(Object.keys(S.POSE_CLIPS)));

        const blending = config.variant !== "merged_static";

        S.setStatus(`building ${config.count} characters`);
        const characters = [];
        for (let i = 0; i < config.count; i += 1) {
            const character = S.buildCharacter(
                container,
                i,
                scene,
                shadowGenerator,
                config.variant,
                blending,
            );
            character.slot = i % roster.count;
            // Past the roster's own ten, copies read a DIFFERENT slot of the
            // SAME live frame, rotated into a different part of the pitch. That
            // differs from #341, whose copies could read a different point in
            // time because the whole capture was on disk: a live simulation has
            // exactly one current frame, and buffering past frames to imitate
            // that would put a copy back in a path built to have none. Every
            // skeleton is still independently posed and independently
            // evaluated; the set of poses on screen repeats every ten
            // characters. Ten -- the count #328 is accountable for -- is
            // unaffected, because there every character is its own player.
            character.copy = Math.floor(i / roster.count);
            character.rotation = character.copy * 2.399;
            characters.push(character);
        }
        const drawnMeshes = characters.reduce((n, c) => n + c.meshes.length, 0);
        const mergedCount = characters.filter((c) => c.merged).length;
        S.normaliseHeights(characters);

        const scale = geometry.scale;
        const halfW = field.w / 2;
        const halfH = field.h / 2;
        // A live match cannot be scanned ahead for its fastest player the way a
        // capture can, so the reference is a running maximum. It settles within
        // the first few dozen ticks and the warm-up is three hundred.
        let topSpeed = 1e-6;

        function place(character, frame) {
            const p = frame.players;
            const idx = character.slot;
            let dx = (p.x[idx] - halfW) / halfW;
            let dz = (p.y[idx] - halfH) / halfH;
            let fx = p.facing_x[idx];
            let fz = p.facing_y[idx];
            const a = character.rotation;
            if (a !== 0) {
                // Normalised pitch space, not world space: rotating a metric
                // offset throws half a copy off a 105x60 rectangle, and
                // characters standing over the void are not a football frame.
                // Rotation preserves the magnitude of the offset, so a point
                // inside the unit disc stays inside it; a corner of the
                // normalised rectangle reaches sqrt(2), and the ground mesh is
                // 1.15x1.3 the pitch so even that stands on drawn geometry.
                const ca = Math.cos(a);
                const sa = Math.sin(a);
                const rx = dx * ca - dz * sa;
                const rz = dx * sa + dz * ca;
                dx = rx;
                dz = rz;
                const rfx = fx * ca - fz * sa;
                const rfz = fx * sa + fz * ca;
                fx = rfx;
                fz = rfz;
            }
            character.root.position.set(dx * halfW * scale, 0, dz * halfH * scale);
            character.root.rotation.y = Math.atan2(fx, fz);

            if (!blending) {
                return;
            }

            const poseId = P.POSE_ID[p.pose_id[idx]];
            // Pose timers say how far into the action the player is; a pose with
            // no timer (contain, fatigue, a keeper's ready stance) is a held
            // stance and reads at full weight.
            const timer = Math.max(
                p.dive[idx],
                p.grab[idx],
                p.throw[idx],
                Math.min(1, p.windup[idx]),
                p.aerial[idx],
            );
            const poseWeight = poseId === "locomotion" ? 0 : Math.max(0.35, Math.min(1, timer || 1));
            const applied = S.applyPose(character, poseId, poseWeight);
            S.applyLocomotion(character, p.speed[idx] / topSpeed, 1 - applied);
        }

        function placeBall(frame) {
            const s = frame.scalars;
            ball.position.set((s.ball_x - halfW) * scale, 0.22 + s.ball_z * scale, (s.ball_y - halfH) * scale);
            ball.setEnabled(s.ball_visible === 1);
        }

        function applyFrame(frame) {
            const speeds = frame.players.speed;
            for (let i = 0; i < speeds.length; i += 1) {
                if (speeds[i] > topSpeed) {
                    topSpeed = speeds[i];
                }
            }
            for (const character of characters) {
                place(character, frame);
            }
            placeBall(frame);
        }

        S.marker(
            S.envMarker(
                engine,
                gpu,
                config,
                `source=wasm|roster_slots=${roster.count}` +
                    `|roster_ids=${roster.ids.join(" ")}` +
                    `|determinism=${config.determinism ? "on" : "off"}`,
            ),
        );

        // --- does rendering perturb the simulation? ---------------------------
        //
        // Same module, same match parameters, same tick count, run twice: once
        // with every frame drawn, once with nothing drawn at all. A renderer
        // that reached back into simulation state — through an aliased view
        // written instead of read, say — would show up here as two different
        // hashes. `_goliseo_payload_boot` restarts the match, so pass two starts
        // from the same place pass one did.
        let simVerdict = "";
        if (config.determinism) {
            S.setStatus(`hash pass 1/2: ${config.hashTicks} ticks WITH rendering`);
            if (api._goliseo_payload_boot() < 0) {
                throw new Error(`re-boot failed: ${lastError()}`);
            }
            const renderedStart = performance.now();
            for (let tick = 0; tick < config.hashTicks; tick += 1) {
                applyFrame(nextFrame(1));
                scene.render();
            }
            const renderedMs = performance.now() - renderedStart;
            const rendered = hashNow();

            S.setStatus(`hash pass 2/2: ${config.hashTicks} ticks with NO rendering`);
            if (api._goliseo_payload_boot() < 0) {
                throw new Error(`re-boot failed: ${lastError()}`);
            }
            const controlStart = performance.now();
            for (let tick = 0; tick < config.hashTicks; tick += 1) {
                nextFrame(1);
            }
            const controlMs = performance.now() - controlStart;
            const control = hashNow();

            simVerdict =
                `|hash_ticks=${config.hashTicks}` +
                `|hash_rendered=${rendered}|hash_control=${control}` +
                `|hash_agrees=${rendered === control ? "yes" : "NO"}` +
                `|hash_rendered_ms=${renderedMs.toFixed(1)}` +
                `|hash_control_ms=${controlMs.toFixed(1)}`;
            S.marker(`GC_BENCH_SIMHASH|source=wasm${simVerdict}`);

            // Back to a clean match for the measured run, so the frames it
            // measures are the same part of the match every configuration sees.
            if (api._goliseo_payload_boot() < 0) {
                throw new Error(`re-boot failed: ${lastError()}`);
            }
        }

        S.setStatus(`warming up (${config.warmup} frames)`);
        S.state.status = "running";
        const beforeLoop = crossingTotal(null);
        const framesBefore = crossings.get("_goliseo_payload_frame");
        // Whether the frozen fixture was still running when measurement began.
        // "Under a live renderer" is a claim about overlap, so the overlap is
        // recorded rather than assumed from the fact that both were started.
        const fixtureLiveAtStart = config.determinism && !fixtureFinished();

        S.driveLoop({
            engine,
            scene,
            warmup: config.warmup,
            frames: config.frames,
            update() {
                // Bracketing the frame's work with two scalar reads is what
                // turns the crossing count from an average into a per-frame
                // measurement. Warm-up frames are counted too: a loop that
                // crossed twice during warm-up and once afterwards would be a
                // violation of the same rule.
                const payloadBefore = frameCalls;
                const totalBefore = totalCalls;
                applyFrame(nextFrame(1));
                const payloadDelta = frameCalls - payloadBefore;
                recordFrameCrossings(payloadDelta, totalCalls - totalBefore - payloadDelta);
            },
            finish(stats) {
                // Counted before anything else in this callback crosses the
                // boundary, so the diagnostic hash below cannot inflate the
                // number that describes the render loop.
                const rendered = config.warmup + stats.measured;
                const loopFrameCalls = crossings.get("_goliseo_payload_frame") - framesBefore;
                const otherCalls = crossingTotal(null) - beforeLoop - loopFrameCalls;
                const frameCrossings = loopFrameCalls / rendered;
                const otherCrossings = otherCalls / rendered;
                const renderer = `babylon-wasm-${config.variant}`;
                const result = S.resultMarker(
                    stats,
                    `renderer=${renderer}` +
                        `|source=wasm|variant=${config.variant}|characters=${config.count}` +
                        `|merged_characters=${mergedCount}|drawn_meshes=${drawnMeshes}` +
                        `|asset_clips=${assetClips}|roster_slots=${roster.count}` +
                        `|warmup_frames=${config.warmup}` +
                        `|payload_words=${first.totalWords}` +
                        `|payload_bytes=${first.totalWords * 8}` +
                        `|combat_present=${first.combatPresent ? 1 : 0}` +
                        `|crossings_per_frame=${frameCrossings.toFixed(2)}` +
                        `|other_crossings_per_frame=${otherCrossings.toFixed(2)}` +
                        // The per-frame extremes. These are what the runner
                        // gates on; the two means above are the second signal,
                        // not the only one.
                        `|crossings_frames=${perFrame.frames}` +
                        `|crossings_min=${perFrame.payloadMin === Infinity ? -1 : perFrame.payloadMin}` +
                        `|crossings_max=${perFrame.payloadMax}` +
                        `|other_crossings_min=${perFrame.otherMin === Infinity ? -1 : perFrame.otherMin}` +
                        `|other_crossings_max=${perFrame.otherMax}` +
                        `|roster_crossings=${rosterCrossings}` +
                        `|fixture_live_at_start=${fixtureLiveAtStart ? "yes" : "no"}` +
                        `|active_meshes=${scene.getActiveMeshes().length}` +
                        `|total_vertices=${scene.getTotalVertices()}` +
                        `|shadows=${config.shadows ? "on" : "off"}` +
                        `|blended_clips=${blending ? S.LOCOMOTION_CLIPS.length + 1 : 1}` +
                        `|state_hash=${hashNow()}${simVerdict}`,
                );
                S.marker(result.line);
                for (const line of S.sampleMarkers(renderer, stats)) {
                    S.marker(line);
                }
                S.setStatus(
                    `done: ${config.count} characters, ${result.meanCalls.toFixed(1)} draw calls, ` +
                        `draw p95 ${S.percentile(
                            stats.drawMs.slice().sort((a, b) => a - b),
                            0.95,
                        ).toFixed(2)} ms`,
                );
                if (config.determinism) {
                    keepRenderingUntilFixtureFinishes(scene, config.fixtureDeadlineMs);
                } else {
                    S.state.status = "done";
                }
            },
        });
    }

    /*
     * The frozen determinism contract, in a worker, WHILE this thread renders.
     *
     * `worker.js` is `verify_common.WORKER` verbatim — the same page-and-worker
     * pair #327 and #342 use to assert `bfbb106aea5480f8` / `a190b60058a64e63`.
     * It is not re-implemented here, because a fixture that differed by so much
     * as a flag could not be compared with the runs already recorded.
     *
     * Rendering does not stop while it runs. That is the point: the claim is
     * that the hashes hold under a live renderer, and a fixture that finished
     * during an idle main thread would not be evidence for it.
     */
    const fixture = { out: [], marks: [], done: false, started: false, startedAt: 0 };

    function startFixtureWorker() {
        fixture.started = true;
        fixture.startedAt = performance.now();
        const worker = new Worker("worker.js");
        worker.onmessage = (event) => {
            const data = event.data || {};
            if (data.line !== undefined) {
                fixture.out.push(String(data.line));
            }
            if (data.mark !== undefined) {
                fixture.marks.push(data);
            }
            if (data.done) {
                fixture.done = true;
            }
        };
        worker.onerror = (event) => {
            fixture.out.push(`WORKER_ERROR: ${event && event.message}`);
            fixture.done = true;
        };
    }

    function fixtureFinished() {
        return (
            fixture.done ||
            fixture.out.some((line) => line.includes("PHASE0 OK") || line.includes("FAILED"))
        );
    }

    function keepRenderingUntilFixtureFinishes(scene, deadlineMs) {
        // Deadline, not a hang: a worker that never finishes must produce a
        // reported failure rather than a timed-out browser session.
        const deadline = performance.now() + deadlineMs;
        let frames = 0;
        const channel = new MessageChannel();
        channel.port1.onmessage = () => {
            try {
                scene.render();
                frames += 1;
                if (fixtureFinished() || performance.now() > deadline) {
                    S.marker(
                        `GC_BENCH_FIXTURE|finished=${fixtureFinished() ? "yes" : "NO"}` +
                            `|lines=${fixture.out.length}` +
                            `|elapsed_ms=${(performance.now() - fixture.startedAt).toFixed(0)}` +
                            `|deadline_ms=${deadlineMs}` +
                            `|extra_rendered_frames=${frames}`,
                    );
                    for (const mark of fixture.marks) {
                        S.marker(
                            `GC_BENCH_FIXTURE_MARK|name=${String(mark.mark).replace(/[|\n]/g, " ")}` +
                                `|ms=${Number(mark.ms).toFixed(1)}`,
                        );
                    }
                    // Verbatim, so `verify_common.check` judges the same text it
                    // judges for every other runtime.
                    S.state.fixture = fixture.out;
                    for (const line of fixture.out) {
                        S.marker(`GC_BENCH_FIXTURE_LINE|text=${String(line).replace(/\n/g, " ")}`);
                    }
                    S.state.status = "done";
                    return;
                }
                channel.port2.postMessage(0);
            } catch (error) {
                S.fail("fixture-render", error);
            }
        };
        channel.port2.postMessage(0);
    }

    if (config.determinism) {
        startFixtureWorker();
    }

    main().catch((error) => S.fail("main", error));
})();
