/*
 * GOLISEO — Babylon.js skinned-character benchmark, CAPTURED-PAYLOAD driver (#341).
 *
 * WHAT THIS ANSWERS
 *
 * #330 has to choose between migrating presentation to Babylon and optimising
 * the LÖVE renderer, and it rests on an assumption: that a real animation system
 * handles skeletons cheaply enough that character count stops being the binding
 * constraint. That assumption is testable without any of the migration. Babylon
 * does not care where a pose came from, so a file of captured `RenderFrame`
 * payloads (scripts/capture_render_frames.lua) drives this page exactly as a
 * live simulation would.
 *
 * The number under test is DRAW CALLS, not frame time. Frame time on an RTX
 * 2070 SUPER will look fine at every count we can build; what decides the
 * argument is the shape of the cost curve as characters are added. So this page
 * reports both, at 10, 20 and 40 characters, and reports them per configuration
 * rather than as endpoints.
 *
 * WHAT IS AND IS NOT IN THIS FILE
 *
 * Everything that is not the data source moved to `scene.js` in #328, so that
 * this page and `wasm_bench.js` — which drives the SAME scene from the live
 * simulation across the wasm boundary — cannot drift apart. What is left here
 * is the captured-file adapter: fetch the JSON, index its flat frame-major
 * streams, and hand poses to the shared loop. The definitions of `update`,
 * `draw` and `frame`, the reason the loop is not requestAnimationFrame, and the
 * variant table all live in `scene.js`.
 */

(function () {
    "use strict";

    const S = globalThis.GoliseoBenchScene;

    const params = new URLSearchParams(window.location.search);
    const config = {
        payload: params.get("payload") || "render_frames.json",
        model: params.get("model") || "vendor/character.glb",
        count: Math.max(1, parseInt(params.get("count") || "10", 10)),
        frames: Math.max(1, parseInt(params.get("frames") || "1200", 10)),
        warmup: Math.max(0, parseInt(params.get("warmup") || "300", 10)),
        variant: S.VARIANTS.has(params.get("variant")) ? params.get("variant") : "authored",
        width: Math.max(320, parseInt(params.get("width") || "960", 10)),
        height: Math.max(240, parseInt(params.get("height") || "540", 10)),
        shadows: params.get("shadows") !== "off",
    };

    async function main() {
        S.setStatus("fetching payload");
        const response = await fetch(config.payload, { cache: "no-store" });
        if (!response.ok) {
            throw new Error(`payload fetch failed: ${response.status}`);
        }
        const payload = await response.json();
        if (payload.schema !== 1) {
            throw new Error(`unsupported capture schema ${payload.schema}`);
        }

        const { engine, scene, geometry, shadowGenerator, ball } = S.createScene(config, {
            w: payload.field.w,
            h: payload.field.h,
            penalty_box_depth: payload.field.penalty_box_depth,
            penalty_box_h: payload.field.penalty_box_h,
            goal_h: (payload.field.goal_home || {}).h || 90,
        });
        const gpu = S.gpuIdentity(engine);

        S.setStatus("loading character");
        const container = await S.loadContainer(scene, config.model);
        const assetClips = S.pruneClips(container, S.requiredClips(payload.pose_ids));

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
            character.slot = i % payload.count;
            // Copies past the captured roster are the same ten streams read from
            // a different point in time and rotated to a different part of the
            // pitch. That keeps every skeleton independently posed — which is
            // what the scaling curve is about — without inventing simulation
            // data that no match produced.
            character.copy = Math.floor(i / payload.count);
            character.frameOffset = character.copy * 137;
            character.rotation = character.copy * 2.399;
            characters.push(character);
        }
        const drawnMeshes = characters.reduce((n, c) => n + c.meshes.length, 0);
        const mergedCount = characters.filter((c) => c.merged).length;
        S.normaliseHeights(characters);

        // Normalise locomotion against what this capture actually contains
        // rather than a guessed top speed: the blend then spans the real range.
        let topSpeed = 0;
        for (const v of payload.players.speed) {
            if (v > topSpeed) {
                topSpeed = v;
            }
        }
        topSpeed = Math.max(1e-6, topSpeed);

        const scale = geometry.scale;
        const halfW = payload.field.w / 2;
        const halfH = payload.field.h / 2;

        function place(character, frameIndex) {
            const f = (frameIndex + character.frameOffset) % payload.frames;
            const idx = f * payload.count + character.slot;
            const p = payload.players;
            // Normalised pitch space, not world space: rotating a metric offset
            // by 137 degrees throws half a copy off a 105x60 rectangle, and
            // characters standing over the void are not a football frame.
            //
            // This is a strong mitigation, NOT a proven invariant, and the
            // difference matters to anyone reusing it. Rotation preserves the
            // magnitude of the offset, so a point inside the unit disc stays
            // inside it and therefore on the pitch — but a corner of the
            // normalised rectangle reaches sqrt(2), and 25 of the 18000
            // player-frames in the shipped capture do sit outside the disc.
            // What has been VERIFIED is empirical, not algebraic: across those
            // 18000 frames and the three copy rotations actually used, the worst
            // rotated component is 0.985, so nothing left the pitch in the
            // measured runs. A different capture or a different rotation could
            // exceed 1; the ground mesh is 1.15x1.3 the pitch, so such a
            // character would still stand on drawn geometry rather than in the
            // void, but it would be outside the markings.
            let dx = (p.x[idx] - halfW) / halfW;
            let dz = (p.y[idx] - halfH) / halfH;
            let fx = p.facing_x[idx];
            let fz = p.facing_y[idx];
            const a = character.rotation;
            if (a !== 0) {
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

            // The control plays one clip at full weight and never reweights, so
            // it must not pay for the pose arithmetic either -- otherwise the
            // gap it is meant to isolate would carry some of its own cost.
            if (!blending) {
                return;
            }

            const poseId = payload.pose_ids[p.pose[idx] - 1];
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

        function placeBall(frameIndex) {
            const b = payload.ball;
            const f = frameIndex % payload.frames;
            ball.position.set(
                (b.x[f] - halfW) * scale,
                0.22 + b.z[f] * scale,
                (b.y[f] - halfH) * scale,
            );
            ball.setEnabled(b.visible[f] === 1);
        }

        S.marker(S.envMarker(engine, gpu, config, "source=captured"));

        S.setStatus(`warming up (${config.warmup} frames)`);
        S.state.status = "running";

        S.driveLoop({
            engine,
            scene,
            warmup: config.warmup,
            frames: config.frames,
            update(frameIndex) {
                for (const character of characters) {
                    place(character, frameIndex);
                }
                placeBall(frameIndex);
            },
            finish(stats) {
                const renderer = `babylon-${config.variant}`;
                const result = S.resultMarker(
                    stats,
                    `renderer=${renderer}` +
                        `|source=captured|variant=${config.variant}|characters=${config.count}` +
                        `|merged_characters=${mergedCount}|drawn_meshes=${drawnMeshes}` +
                        `|asset_clips=${assetClips}|roster_slots=${payload.count}` +
                        `|seed=${payload.seed}|warmup_frames=${config.warmup}` +
                        `|active_meshes=${scene.getActiveMeshes().length}` +
                        `|total_vertices=${scene.getTotalVertices()}` +
                        `|shadows=${config.shadows ? "on" : "off"}` +
                        `|blended_clips=${blending ? S.LOCOMOTION_CLIPS.length + 1 : 1}` +
                        `|state_hash=${payload.final_state_hash}`,
                );
                S.marker(result.line);
                for (const line of S.sampleMarkers(renderer, stats)) {
                    S.marker(line);
                }
                S.state.status = "done";
                S.setStatus(
                    `done: ${config.count} characters, ${result.meanCalls.toFixed(1)} draw calls, ` +
                        `draw p95 ${S.percentile(
                            stats.drawMs.slice().sort((a, b) => a - b),
                            0.95,
                        ).toFixed(2)} ms`,
                );
            },
        });
    }

    main().catch((error) => S.fail("main", error));
})();
