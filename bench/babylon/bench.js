/*
 * GOLISEO — Babylon.js skinned-character benchmark (#341).
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
 * WHY THE RENDER LOOP IS NOT requestAnimationFrame
 *
 * rAF is vsync-locked. Under it every configuration reports 16.67 ms whether it
 * has 90% headroom or none, which is the exact trap game/render/benchmark.lua
 * disables vsync to avoid. The loop below is driven by a MessageChannel port
 * (a 0 ms task that browsers do not clamp), so frame time is free to fall below
 * the refresh interval and free to rise above it.
 *
 * WHAT `draw` AND `frame` MEAN, precisely, because the comparison depends on it
 *
 *   draw  = CPU time inside scene.render(). This is submission cost: culling,
 *           material binds, bone-matrix uploads, draw-call issue. It is the
 *           direct counterpart of the native baseline's `draw` sample, which
 *           also times only the CPU side of one LÖVE draw.
 *   frame = wall time for one loop iteration INCLUDING a gl.finish(), so the
 *           GPU has actually retired the frame. This is the number a player
 *           feels. It has no counterpart in the native baseline and must not be
 *           compared against it.
 *
 * VARIANTS
 *
 * `authored` draws the asset as its author shipped it: six skinned meshes per
 * character (body, head, two arms, two legs) plus bone-parented headgear.
 * `merged` collapses the six into one skinned mesh — legal here only because
 * the whole pack shares a single material — which is the Babylon-side
 * equivalent of the rigid-GPU-skinning optimisation #330 costs for LÖVE. Both
 * are measured, because reporting only one of them would answer a different
 * question than the one asked. `merged_static` is a control rather than a
 * candidate — see the comment on VARIANTS below.
 */

(function () {
    "use strict";

    const state = (window.__GC_BENCH__ = {
        status: "boot",
        markers: [],
        errors: [],
    });

    function marker(line) {
        state.markers.push(line);
        // Console too: it is the channel a human reading the page uses, and it
        // costs nothing that the runner reads the array instead.
        console.log(line);
    }

    function fail(where, error) {
        const text = `${where}: ${error && error.stack ? error.stack : error}`;
        state.errors.push(text);
        state.status = "error";
        marker(`GC_BENCH_ERROR|where=${where}|message=${String(error).replace(/[|\n]/g, " ")}`);
        setStatus(text);
    }

    function setStatus(text) {
        const node = document.getElementById("status");
        if (node) {
            node.textContent = text;
        }
    }

    window.addEventListener("error", (event) => fail("window", event.message));
    window.addEventListener("unhandledrejection", (event) => fail("promise", event.reason));

    /*
     * `merged_static` is not a candidate configuration -- it is a control. It
     * draws the same merged meshes but plays ONE clip at full weight instead of
     * three blended locomotion clips plus a pose clip, so the difference between
     * it and `merged` is the cost of skeletal animation and nothing else. It
     * exists because the headline curve is linear, and "why" decides what #330
     * does about it: a curve dominated by draw submission is fixed by batching,
     * a curve dominated by CPU animation evaluation is not.
     */
    const VARIANTS = new Set(["authored", "merged", "merged_static"]);

    const params = new URLSearchParams(window.location.search);
    const config = {
        payload: params.get("payload") || "render_frames.json",
        model: params.get("model") || "vendor/character.glb",
        count: Math.max(1, parseInt(params.get("count") || "10", 10)),
        frames: Math.max(1, parseInt(params.get("frames") || "1200", 10)),
        warmup: Math.max(0, parseInt(params.get("warmup") || "300", 10)),
        variant: VARIANTS.has(params.get("variant")) ? params.get("variant") : "authored",
        width: Math.max(320, parseInt(params.get("width") || "960", 10)),
        height: Math.max(240, parseInt(params.get("height") || "540", 10)),
        shadows: params.get("shadows") !== "off",
    };

    // A regulation pitch is about 105 m long; the simulation's is 960 units.
    // Everything else (character height, ball radius, camera distance) then
    // reads in metres, which is the space the glTF was authored in.
    const PITCH_METRES_X = 105;
    const CHARACTER_HEIGHT_METRES = 1.8;

    /*
     * Pose family -> clip. The left-hand side is `render/player_pose.lua`'s
     * closed set; the right-hand side is KayKit's clip list. The mapping is
     * deliberately literal and deliberately incomplete in expressiveness: a
     * knight's PickUp is not a keeper's smother, and nobody should read these
     * as authored animation. What must be right for a BENCHMARK is that each
     * family costs a real skeletal clip evaluated on a real skeleton, blended
     * over locomotion, and that is exactly what this buys.
     */
    const POSE_CLIPS = {
        keeper_grab: "PickUp",
        keeper_throw: "Throw",
        keeper_punt: "Unarmed_Melee_Attack_Kick",
        keeper_tip: "Dodge_Right",
        keeper_spread: "Lie_Down",
        keeper_central: "Dodge_Forward",
        keeper_stretch: "Dodge_Left",
        keeper_dive: "Dodge_Right",
        keeper_get_up: "Lie_StandUp",
        keeper_set: "Blocking",
        keeper_ready_low: "Blocking",
        keeper_shuffle: "Running_Strafe_Left",
        keeper_ready_tall: "Unarmed_Idle",
        aerial_bicycle: "2H_Melee_Attack_Spin",
        aerial_action: "Jump_Full_Short",
        combat_knockback: "Hit_B",
        combat_stagger: "Hit_A",
        combat_guard: "Blocking",
        combat_active: "Unarmed_Melee_Attack_Punch_A",
        combat_windup: "Unarmed_Melee_Attack_Punch_B",
        combat_aim: "1H_Ranged_Aiming",
        combat_recovery: "Interact",
        soccer_windup: "Unarmed_Melee_Attack_Kick",
        slide: "Dodge_Forward",
        tackle: "Unarmed_Melee_Attack_Kick",
        stumble: "Hit_A",
        kick_follow: "Unarmed_Melee_Attack_Kick",
        settle: "Interact",
        run_telegraph: "Jump_Start",
        contain: "Blocking",
        fatigue: "Unarmed_Idle",
    };
    const LOCOMOTION_CLIPS = ["Idle", "Walking_A", "Running_A"];

    // Which clips survive the prune. Cloning all 76 groups for 40 characters is
    // minutes of load time and hundreds of megabytes for animation nobody plays.
    function requiredClips(poseIds) {
        const wanted = new Set(LOCOMOTION_CLIPS);
        for (const id of poseIds) {
            const clip = POSE_CLIPS[id];
            if (clip) {
                wanted.add(clip);
            }
        }
        return wanted;
    }

    function percentile(sorted, q) {
        if (sorted.length === 0) {
            return 0;
        }
        const rank = Math.max(1, Math.ceil(q * sorted.length));
        return sorted[Math.min(rank, sorted.length) - 1];
    }

    function countAbove(values, threshold) {
        let n = 0;
        for (const v of values) {
            if (v > threshold) {
                n += 1;
            }
        }
        return n;
    }

    // Same field names and same order as game/render/benchmark.lua's
    // summary_fields, so a Babylon row and a LÖVE row parse with one reader.
    function summaryFields(samplesMs, prefix) {
        const sorted = samplesMs.slice().sort((a, b) => a - b);
        const total = samplesMs.reduce((a, b) => a + b, 0);
        const f = (v) => v.toFixed(4);
        return [
            `${prefix}_p50=${f(percentile(sorted, 0.5))}`,
            `${prefix}_p95=${f(percentile(sorted, 0.95))}`,
            `${prefix}_p99=${f(percentile(sorted, 0.99))}`,
            `${prefix}_max=${f(sorted.length ? sorted[sorted.length - 1] : 0)}`,
            `${prefix}_mean=${f(samplesMs.length ? total / samplesMs.length : 0)}`,
            `${prefix}_over16=${countAbove(samplesMs, 1000 / 60)}`,
            `${prefix}_over33=${countAbove(samplesMs, 33)}`,
            `${prefix}_over250=${countAbove(samplesMs, 250)}`,
            `${prefix}_n=${samplesMs.length}`,
        ].join("|");
    }

    /*
     * The GPU identity, read the hard way and reported verbatim.
     *
     * Headless Chrome silently falls back to SwiftShader, a software
     * rasteriser, and #100 already published one false negative from exactly
     * that. This page does not decide whether the string is acceptable — it
     * reports what it found and lets scripts/babylon_bench.py refuse, so the
     * refusal lives in one place and cannot be talked out of by a page.
     */
    function gpuIdentity(engine) {
        const gl = engine._gl || (engine.getRenderingCanvas() || {}).__gl;
        const out = {
            renderer: "?",
            unmasked_renderer: "?",
            vendor: "?",
            unmasked_vendor: "?",
            api: engine.webGLVersion === 2 ? "webgl2" : "webgl1",
            description: (engine.description || "?").replace(/[|]/g, " "),
        };
        if (!gl) {
            return out;
        }
        try {
            out.renderer = String(gl.getParameter(gl.RENDERER) || "?");
            out.vendor = String(gl.getParameter(gl.VENDOR) || "?");
            const debug = gl.getExtension("WEBGL_debug_renderer_info");
            if (debug) {
                out.unmasked_renderer = String(
                    gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) || "?",
                );
                out.unmasked_vendor = String(gl.getParameter(debug.UNMASKED_VENDOR_WEBGL) || "?");
            }
        } catch (error) {
            out.renderer = `<unavailable: ${error}>`;
        }
        for (const key of Object.keys(out)) {
            out[key] = String(out[key]).replace(/[|\n]/g, " ");
        }
        return out;
    }

    function buildPitch(scene, field, shadowGenerator) {
        const scale = PITCH_METRES_X / field.w;
        const w = field.w * scale;
        const h = field.h * scale;

        const grass = new BABYLON.StandardMaterial("grass", scene);
        grass.diffuseColor = new BABYLON.Color3(0.16, 0.34, 0.19);
        grass.specularColor = new BABYLON.Color3(0.02, 0.02, 0.02);
        const ground = BABYLON.MeshBuilder.CreateGround(
            "pitch",
            { width: w * 1.15, height: h * 1.3 },
            scene,
        );
        ground.material = grass;
        ground.receiveShadows = true;

        // Pitch markings as thin boxes rather than a texture: a texture would be
        // one draw call and hide the line cost, and the LÖVE renderer draws real
        // geometry here too.
        const paint = new BABYLON.StandardMaterial("paint", scene);
        paint.diffuseColor = new BABYLON.Color3(0.85, 0.88, 0.9);
        paint.specularColor = BABYLON.Color3.Black();
        const line = (name, x, z, lw, lh) => {
            const box = BABYLON.MeshBuilder.CreateBox(
                name,
                { width: lw, height: 0.02, depth: lh },
                scene,
            );
            box.position.set(x, 0.011, z);
            box.material = paint;
            return box;
        };
        const t = 0.14;
        line("touch_top", 0, -h / 2, w, t);
        line("touch_bottom", 0, h / 2, w, t);
        line("goal_left", -w / 2, 0, t, h);
        line("goal_right", w / 2, 0, t, h);
        line("halfway", 0, 0, t, h);
        const boxDepth = field.penalty_box_depth * scale;
        const boxH = field.penalty_box_h * scale;
        for (const side of [-1, 1]) {
            const x = side * (w / 2 - boxDepth / 2);
            line(`box_face_${side}`, side * (w / 2 - boxDepth), 0, t, boxH);
            line(`box_top_${side}`, x, -boxH / 2, boxDepth, t);
            line(`box_bottom_${side}`, x, boxH / 2, boxDepth, t);
        }
        const circle = BABYLON.MeshBuilder.CreateTorus(
            "centre_circle",
            { diameter: 18, thickness: t, tessellation: 48 },
            scene,
        );
        circle.position.y = 0.011;
        circle.material = paint;

        const frame = new BABYLON.StandardMaterial("goalframe", scene);
        frame.diffuseColor = new BABYLON.Color3(0.9, 0.9, 0.93);
        for (const side of [-1, 1]) {
            const goalW = ((field.goal_home.h || 90) * scale) / 1;
            const post = (name, z) => {
                const p = BABYLON.MeshBuilder.CreateBox(
                    name,
                    { width: 0.2, height: 2.44, depth: 0.2 },
                    scene,
                );
                p.position.set(side * (w / 2), 1.22, z);
                p.material = frame;
                if (shadowGenerator) {
                    shadowGenerator.addShadowCaster(p);
                }
            };
            post(`post_a_${side}`, -goalW / 2);
            post(`post_b_${side}`, goalW / 2);
            const bar = BABYLON.MeshBuilder.CreateBox(
                `bar_${side}`,
                { width: 0.2, height: 0.2, depth: goalW },
                scene,
            );
            bar.position.set(side * (w / 2), 2.44, 0);
            bar.material = frame;
            if (shadowGenerator) {
                shadowGenerator.addShadowCaster(bar);
            }
        }
        return { scale, w, h };
    }

    async function loadContainer(scene, url) {
        if (typeof BABYLON.LoadAssetContainerAsync === "function") {
            return await BABYLON.LoadAssetContainerAsync(url, scene);
        }
        return await BABYLON.SceneLoader.LoadAssetContainerAsync("", url, scene);
    }

    // Everything a character needs, per instance. `action` is the single clip
    // slot the pose families share: only one non-locomotion pose is ever
    // selected for a player in a frame, so allocating more than one would
    // measure animation this game cannot produce.
    function buildCharacter(container, index, scene, shadowGenerator, variant, blending) {
        const entries = container.instantiateModelsToScene((name) => `${name}_${index}`, false, {
            doNotInstantiate: true,
        });
        const root = entries.rootNodes[0];
        root.scaling.setAll(1);

        const groups = new Map();
        for (const group of entries.animationGroups) {
            group.stop();
            // Strip the per-instance suffix instantiateModelsToScene appends so
            // the clip table keys stay the asset's own names.
            groups.set(group.name.replace(/_\d+$/, ""), group);
        }

        let meshes = root.getChildMeshes(false).filter((m) => m.getTotalVertices() > 0);
        // Weapons and shields: the asset ships a full loadout, and a footballer
        // carries none of it. Left visible they would inflate the draw-call
        // count with geometry the game will never show.
        const carried = /sword|shield|crossbow|axe|staff|wand|arrow|quiver|spellbook|mug|dagger/i;
        for (const mesh of meshes) {
            if (carried.test(mesh.name)) {
                mesh.setEnabled(false);
            }
        }
        meshes = meshes.filter((m) => m.isEnabled());

        let skinned = meshes.filter((m) => m.skeleton);
        let merged = false;
        if (variant === "merged" && skinned.length > 1) {
            const skeleton = skinned[0].skeleton;
            const parent = skinned[0].parent;
            const material = skinned[0].material;
            const target = BABYLON.Mesh.MergeMeshes(skinned, true, true, undefined, false, false);
            if (
                target &&
                target.isVerticesDataPresent(BABYLON.VertexBuffer.MatricesIndicesKind) &&
                target.isVerticesDataPresent(BABYLON.VertexBuffer.MatricesWeightsKind)
            ) {
                target.name = `merged_${index}`;
                target.parent = parent;
                target.material = material;
                target.skeleton = skeleton;
                target.numBoneInfluencers = 4;
                skinned = [target];
                merged = true;
            } else if (target) {
                // Refuse to report a merged number the merge did not produce.
                throw new Error("merged variant lost skinning data during MergeMeshes");
            }
            meshes = root
                .getChildMeshes(false)
                .filter((m) => m.getTotalVertices() > 0 && m.isEnabled());
        }

        if (shadowGenerator) {
            for (const mesh of meshes) {
                shadowGenerator.addShadowCaster(mesh, false);
                mesh.receiveShadows = true;
            }
        }

        const clips = blending ? LOCOMOTION_CLIPS : LOCOMOTION_CLIPS.slice(0, 1);
        const locomotion = clips.map((name) => {
            const group = groups.get(name);
            if (!group) {
                throw new Error(`asset is missing locomotion clip ${name}`);
            }
            group.play(true);
            group.setWeightForAllAnimatables(blending ? 0 : 1);
            return group;
        });

        return {
            root,
            groups,
            locomotion,
            meshes,
            merged,
            action: null,
            actionClip: null,
        };
    }

    function applyPose(character, poseId, poseWeight) {
        const clipName = POSE_CLIPS[poseId] || null;
        if (clipName !== character.actionClip) {
            if (character.action) {
                character.action.setWeightForAllAnimatables(0);
                character.action.stop();
            }
            character.actionClip = clipName;
            character.action = clipName ? character.groups.get(clipName) || null : null;
            if (character.action) {
                character.action.play(true);
            }
        }
        if (character.action) {
            character.action.setWeightForAllAnimatables(poseWeight);
        }
        return character.action ? poseWeight : 0;
    }

    function applyLocomotion(character, normalisedSpeed, headroom) {
        const s = Math.min(1, Math.max(0, normalisedSpeed));
        let idle;
        let walk;
        let run;
        if (s < 0.5) {
            idle = 1 - s * 2;
            walk = s * 2;
            run = 0;
        } else {
            idle = 0;
            walk = 2 - s * 2;
            run = s * 2 - 1;
        }
        character.locomotion[0].setWeightForAllAnimatables(idle * headroom);
        character.locomotion[1].setWeightForAllAnimatables(walk * headroom);
        character.locomotion[2].setWeightForAllAnimatables(run * headroom);
    }

    async function main() {
        setStatus("fetching payload");
        const response = await fetch(config.payload, { cache: "no-store" });
        if (!response.ok) {
            throw new Error(`payload fetch failed: ${response.status}`);
        }
        const payload = await response.json();
        if (payload.schema !== 1) {
            throw new Error(`unsupported capture schema ${payload.schema}`);
        }

        const canvas = document.getElementById("canvas");
        canvas.width = config.width;
        canvas.height = config.height;

        const engine = new BABYLON.Engine(canvas, true, {
            preserveDrawingBuffer: false,
            stencil: false,
            // The page owns its own loop, so Babylon must not also install one.
            disableWebGL2Support: false,
            powerPreference: "high-performance",
        });
        engine.setSize(config.width, config.height);
        const gpu = gpuIdentity(engine);

        const scene = new BABYLON.Scene(engine);
        scene.clearColor = new BABYLON.Color4(0.05, 0.07, 0.11, 1);
        scene.skipFrustumClipping = false;

        const sun = new BABYLON.DirectionalLight(
            "sun",
            new BABYLON.Vector3(-0.4, -1, 0.35),
            scene,
        );
        sun.position = new BABYLON.Vector3(30, 60, -30);
        sun.intensity = 2.2;
        const fill = new BABYLON.HemisphericLight("fill", new BABYLON.Vector3(0, 1, 0), scene);
        fill.intensity = 0.55;

        let shadowGenerator = null;
        if (config.shadows) {
            shadowGenerator = new BABYLON.ShadowGenerator(1024, sun);
            shadowGenerator.usePercentageCloserFiltering = false;
            shadowGenerator.useExponentialShadowMap = true;
            shadowGenerator.bias = 0.005;
        }

        const geometry = buildPitch(scene, payload.field, shadowGenerator);

        const camera = new BABYLON.ArcRotateCamera(
            "camera",
            -Math.PI / 2,
            0.95,
            96,
            new BABYLON.Vector3(0, 2, 0),
            scene,
        );
        camera.minZ = 0.5;
        camera.maxZ = 400;

        const ball = BABYLON.MeshBuilder.CreateSphere("ball", { diameter: 0.44, segments: 16 }, scene);
        const ballMaterial = new BABYLON.StandardMaterial("ballmat", scene);
        ballMaterial.diffuseColor = new BABYLON.Color3(0.95, 0.95, 0.98);
        ball.material = ballMaterial;
        if (shadowGenerator) {
            shadowGenerator.addShadowCaster(ball);
        }

        setStatus("loading character");
        const container = await loadContainer(scene, config.model);
        const wanted = requiredClips(payload.pose_ids);
        const keep = [];
        // Snapshot first: AnimationGroup.dispose() splices itself out of its
        // parent container, so disposing while iterating the live array silently
        // skips every other clip -- including half the locomotion set.
        for (const group of container.animationGroups.slice()) {
            if (wanted.has(group.name)) {
                keep.push(group);
            } else {
                group.dispose();
            }
        }
        container.animationGroups = keep;
        const assetClips = keep.length;

        const meshVariant = config.variant === "authored" ? "authored" : "merged";
        const blending = config.variant !== "merged_static";

        setStatus(`building ${config.count} characters`);
        const characters = [];
        for (let i = 0; i < config.count; i += 1) {
            const character = buildCharacter(
                container,
                i,
                scene,
                shadowGenerator,
                meshVariant,
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

        // Height-normalise against the asset as actually instantiated, so the
        // character reads as 1.8 m next to a 2.44 m crossbar whatever units the
        // pack was authored in. Measured after building, because the bounds of a
        // loaded-but-unparented container are not the bounds of a live rig.
        characters[0].root.computeWorldMatrix(true);
        const bounds = characters[0].root.getHierarchyBoundingVectors(true);
        const modelScale =
            CHARACTER_HEIGHT_METRES / Math.max(0.001, bounds.max.y - bounds.min.y);
        for (const character of characters) {
            character.root.scaling.setAll(modelScale);
        }

        // Normalise locomotion against what this capture actually contains
        // rather than a guessed top speed: the blend then spans the real range.
        let topSpeed = 0;
        for (const v of payload.players.speed) {
            if (v > topSpeed) {
                topSpeed = v;
            }
        }
        topSpeed = Math.max(1e-6, topSpeed);

        const instrumentation = new BABYLON.SceneInstrumentation(scene);
        instrumentation.captureFrameTime = true;

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
            const applied = applyPose(character, poseId, poseWeight);
            applyLocomotion(character, p.speed[idx] / topSpeed, 1 - applied);
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

        // The camera is IDENTICAL at 10, 20 and 40 characters, and frames the
        // whole pitch. Two things follow, both deliberate. Every character stays
        // inside the frustum, so the curve cannot be flattened by culling half
        // the roster. And per-character pixel coverage does not change with
        // count, so added characters add shading linearly instead of shrinking
        // to nothing. Whole-pitch framing is also what the native LÖVE baseline
        // in #328 measured, which is what lets the two sit in one table.
        camera.radius = 82;
        camera.beta = 1.0;
        camera.setTarget(new BABYLON.Vector3(0, 1, 0));

        marker(
            `GC_BENCH_ENV|runtime=babylon|babylon=${BABYLON.Engine.Version}` +
                `|api=${gpu.api}|gpu_renderer=${gpu.renderer}` +
                `|gpu_unmasked_renderer=${gpu.unmasked_renderer}` +
                `|gpu_vendor=${gpu.vendor}|gpu_unmasked_vendor=${gpu.unmasked_vendor}` +
                `|engine=${gpu.description}` +
                `|width=${config.width}|height=${config.height}|players=${config.count}` +
                `|user_agent=${navigator.userAgent.replace(/[|\n]/g, " ")}`,
        );

        setStatus(`warming up (${config.warmup} frames)`);
        state.status = "running";

        const drawMs = [];
        const frameMs = [];
        const updateMs = [];
        const drawCalls = [];
        let frameIndex = 0;
        let measured = 0;
        const gl = engine._gl;

        const channel = new MessageChannel();
        let lastFrameStart = 0;

        function tick() {
            const loopStart = performance.now();

            const updateStart = performance.now();
            for (const character of characters) {
                place(character, frameIndex);
            }
            placeBall(frameIndex);
            const updateEnd = performance.now();

            scene.render();
            const drawEnd = performance.now();

            // Force the GPU to retire the frame before the loop closes, so
            // `frame` is a real cost and not a queue depth.
            if (gl) {
                gl.finish();
            }
            const loopEnd = performance.now();

            frameIndex += 1;
            const warmed = frameIndex > config.warmup;
            if (warmed) {
                updateMs.push(updateEnd - updateStart);
                drawMs.push(drawEnd - updateEnd);
                if (lastFrameStart) {
                    frameMs.push(loopEnd - lastFrameStart);
                }
                const calls = instrumentation.drawCallsCounter;
                drawCalls.push(calls ? calls.current : 0);
                measured += 1;
            }
            lastFrameStart = loopStart;

            if (frameIndex === config.warmup) {
                setStatus(`measuring (${config.frames} frames)`);
            }
            if (measured >= config.frames) {
                finish();
                return;
            }
            channel.port2.postMessage(0);
        }

        function finish() {
            const meanCalls = drawCalls.reduce((a, b) => a + b, 0) / Math.max(1, drawCalls.length);
            const maxCalls = drawCalls.reduce((a, b) => Math.max(a, b), 0);
            marker(
                `GC_BENCH_RESULT|renderer=babylon-${config.variant}` +
                    `|variant=${config.variant}|characters=${config.count}` +
                    `|merged_characters=${mergedCount}|drawn_meshes=${drawnMeshes}` +
                    `|asset_clips=${assetClips}|roster_slots=${payload.count}` +
                    `|seed=${payload.seed}|warmup_frames=${config.warmup}` +
                    `|measured_frames=${measured}` +
                    `|${summaryFields(updateMs, "update")}` +
                    `|${summaryFields(drawMs, "draw")}` +
                    `|${summaryFields(frameMs, "frame")}` +
                    `|draw_calls_mean=${meanCalls.toFixed(1)}|draw_calls_max=${maxCalls}` +
                    `|active_meshes=${scene.getActiveMeshes().length}` +
                    `|total_vertices=${scene.getTotalVertices()}` +
                    `|shadows=${config.shadows ? "on" : "off"}` +
                    `|blended_clips=${blending ? LOCOMOTION_CLIPS.length + 1 : 1}` +
                    `|state_hash=${payload.final_state_hash}`,
            );
            marker(
                `GC_BENCH_SAMPLES|renderer=babylon-${config.variant}|kind=draw` +
                    `|unit=microseconds|samples=${drawMs.map((v) => Math.round(v * 1000)).join(",")}`,
            );
            marker(
                `GC_BENCH_SAMPLES|renderer=babylon-${config.variant}|kind=frame` +
                    `|unit=microseconds|samples=${frameMs.map((v) => Math.round(v * 1000)).join(",")}`,
            );
            marker(
                `GC_BENCH_SAMPLES|renderer=babylon-${config.variant}|kind=draw_calls` +
                    `|unit=count|samples=${drawCalls.join(",")}`,
            );
            state.status = "done";
            setStatus(
                `done: ${config.count} characters, ${meanCalls.toFixed(1)} draw calls, ` +
                    `draw p95 ${percentile(drawMs.slice().sort((a, b) => a - b), 0.95).toFixed(2)} ms`,
            );
        }

        channel.port1.onmessage = () => {
            try {
                tick();
            } catch (error) {
                fail("tick", error);
            }
        };
        channel.port2.postMessage(0);
    }

    main().catch((error) => fail("main", error));
})();
