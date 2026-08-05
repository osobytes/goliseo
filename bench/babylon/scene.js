/*
 * GOLISEO — the Babylon benchmark scene, shared by both drivers (#328).
 *
 * WHY THIS FILE EXISTS
 *
 * There are two ways to feed the same scene, and the whole point of #328 is
 * that they are the same scene:
 *
 *   bench.js       reads a file of captured `RenderFrame` payloads (#341).
 *   wasm_bench.js  reads the live simulation across the wasm boundary (#332).
 *
 * If those two pages built their pitches, characters, clip tables and
 * measurement loops separately, a difference between their numbers could be a
 * difference in the harness rather than in the data source — which is the one
 * comparison #328 exists to make. So everything that is not the data source
 * lives here and neither driver is allowed a private copy.
 *
 * The contents were lifted from `bench.js` as it shipped in #341, unchanged in
 * behaviour, so the #341 numbers remain the numbers this code produces. The
 * only addition is the `merged_all` variant, and it is documented where it is
 * implemented.
 *
 * WHAT `draw` AND `frame` MEAN, precisely, because the comparison depends on it
 *
 *   update = CPU time to produce and apply one frame of poses. For bench.js
 *            that is reading captured arrays; for wasm_bench.js it is the ONE
 *            boundary crossing plus the placement it feeds. Kept separate from
 *            draw so the payload's cost is never hidden inside the renderer's.
 *   draw   = CPU time inside scene.render(): culling, material binds,
 *            bone-matrix uploads, draw-call issue. The direct counterpart of
 *            the native baseline's `draw` sample, which also times only the CPU
 *            side of one LÖVE draw.
 *   frame  = wall time for one loop iteration INCLUDING a gl.finish(), so the
 *            GPU has actually retired the frame. This is the number a player
 *            feels. It has no counterpart in the native baseline and must not
 *            be compared against it.
 *
 * WHY THE RENDER LOOP IS NOT requestAnimationFrame
 *
 * rAF is vsync-locked. Under it every configuration reports 16.67 ms whether it
 * has 90% headroom or none, which is the exact trap game/render/benchmark.lua
 * disables vsync to avoid. `driveLoop` is driven by a MessageChannel port (a
 * 0 ms task browsers do not clamp), so frame time is free to fall below the
 * refresh interval and free to rise above it.
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

    function setStatus(text) {
        const node = document.getElementById("status");
        if (node) {
            node.textContent = text;
        }
    }

    function fail(where, error) {
        const text = `${where}: ${error && error.stack ? error.stack : error}`;
        state.errors.push(text);
        state.status = "error";
        marker(`GC_BENCH_ERROR|where=${where}|message=${String(error).replace(/[|\n]/g, " ")}`);
        setStatus(text);
    }

    window.addEventListener("error", (event) => fail("window", event.message));
    window.addEventListener("unhandledrejection", (event) => fail("promise", event.reason));

    /*
     * VARIANTS. Only the first two are candidates.
     *
     * `authored`     the asset as its author shipped it: six skinned meshes per
     *                character (body, head, two arms, two legs) plus the two
     *                bone-parented pieces of gear.
     * `merged`       the six skinned meshes collapsed into one — legal only
     *                because the whole pack shares a single material. This is
     *                the Babylon-side equivalent of the rigid-GPU-skinning
     *                optimisation #330 costs for LÖVE. Gear stays separate,
     *                so a character is THREE meshes, not one.
     * `merged_all`   #328: gear folded into the skin as well, so a character is
     *                ONE mesh. See `bakeIntoSkin`. This exists because #337
     *                slice 2's optimised LÖVE renderer merged all 28 rigid
     *                parts INCLUDING gear into one mesh, and comparing that
     *                against Babylon's `merged` compares LÖVE at its floor with
     *                Babylon above its own.
     * `merged_static` a CONTROL, not a candidate: the same merged meshes playing
     *                ONE clip at full weight instead of three blended locomotion
     *                clips plus a pose clip, so the difference between it and
     *                `merged` is the cost of skeletal animation and nothing
     *                else.
     */
    const VARIANTS = new Set(["authored", "merged", "merged_all", "merged_static"]);

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
     * reports what it found and lets the Python runner refuse, so the refusal
     * lives in one place and cannot be talked out of by a page.
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

    function envMarker(engine, gpu, config, extra) {
        return (
            `GC_BENCH_ENV|runtime=babylon|babylon=${BABYLON.Engine.Version}` +
            `|api=${gpu.api}|gpu_renderer=${gpu.renderer}` +
            `|gpu_unmasked_renderer=${gpu.unmasked_renderer}` +
            `|gpu_vendor=${gpu.vendor}|gpu_unmasked_vendor=${gpu.unmasked_vendor}` +
            `|engine=${gpu.description}` +
            `|width=${config.width}|height=${config.height}|players=${config.count}` +
            `|user_agent=${navigator.userAgent.replace(/[|\n]/g, " ")}` +
            (extra ? `|${extra}` : "")
        );
    }

    /**
     * The pitch, goals and markings. `field` is the flat shape both data
     * sources agree on: `{ w, h, penalty_box_depth, penalty_box_h, goal_h }`.
     */
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
            const goalW = (field.goal_h || 90) * scale;
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

    // Snapshot first: AnimationGroup.dispose() splices itself out of its parent
    // container, so disposing while iterating the live array silently skips
    // every other clip -- including half the locomotion set.
    function pruneClips(container, wanted) {
        const keep = [];
        for (const group of container.animationGroups.slice()) {
            if (wanted.has(group.name)) {
                keep.push(group);
            } else {
                group.dispose();
            }
        }
        container.animationGroups = keep;
        return keep.length;
    }

    /*
     * Fold a bone-parented, UNSKINNED mesh into a skinned one (#328).
     *
     * WHY. `merged` collapses the six skinned meshes and stops, because the
     * helmet and cape carry no bone weights and Babylon's MergeMeshes refuses
     * to mix the two. That leaves a character at three meshes — six draw calls
     * with the shadow pass — while #337 slice 2's optimised LÖVE renderer put
     * ALL 28 rigid parts, gear included, into one mesh. Comparing those two is
     * comparing LÖVE at its floor with Babylon above its own, so this closes the
     * gap rather than caveating it.
     *
     * HOW. A mesh parented to bone B and never deformed is the degenerate case
     * of skinning: every vertex is weighted 1.0 to B. So the accessory's
     * vertices are expressed in the skinned mesh's local space AT REST, given
     * weight 1 on B, and appended to the skin's own vertex arrays. Babylon
     * uploads `restInverse * absolute` per bone, which is identity at rest, so
     * the baked vertices sit exactly where the bone-parented mesh sat and then
     * follow B for the rest of the animation.
     *
     * Row-vector convention throughout, which is Babylon's: `p_world = p_local *
     * World`, so the rebasing matrix is `accessoryWorld * inverse(skinWorld)`.
     *
     * WHAT IT IS NOT. This is a benchmark transform, not an authoring pipeline.
     * It gives the gear rigid attachment, which is what it already had; real
     * authored weights would let a cape deform, cost the same per frame, and
     * are #318's problem rather than this one's. Non-uniform scale in the chain
     * would skew normals — the pack has none, and this throws rather than
     * guess if the vertex layout is not the one it knows how to append to.
     */
    const BAKEABLE_KINDS = [
        BABYLON.VertexBuffer.PositionKind,
        BABYLON.VertexBuffer.NormalKind,
        BABYLON.VertexBuffer.UVKind,
        BABYLON.VertexBuffer.MatricesIndicesKind,
        BABYLON.VertexBuffer.MatricesWeightsKind,
    ];

    function boneIndexFor(mesh, skeleton) {
        const linked = new Map();
        for (let index = 0; index < skeleton.bones.length; index += 1) {
            const node = skeleton.bones[index].getTransformNode();
            if (node) {
                linked.set(node.uniqueId, index);
            }
        }
        for (let node = mesh.parent; node; node = node.parent) {
            if (linked.has(node.uniqueId)) {
                return linked.get(node.uniqueId);
            }
        }
        return -1;
    }

    function bakeIntoSkin(skin, accessories, skeleton) {
        const unexpected = skin
            .getVerticesDataKinds()
            .filter((kind) => !BAKEABLE_KINDS.includes(kind));
        if (unexpected.length > 0) {
            throw new Error(`merged_all cannot append vertex kinds ${unexpected.join(",")}`);
        }
        skin.computeWorldMatrix(true);
        const skinInverse = BABYLON.Matrix.Invert(skin.getWorldMatrix());

        const data = {};
        for (const kind of BAKEABLE_KINDS) {
            const values = skin.getVerticesData(kind);
            if (!values) {
                throw new Error(`merged_all needs ${kind} on the skinned mesh`);
            }
            data[kind] = Array.from(values);
        }
        const indices = Array.from(skin.getIndices() || []);

        for (const accessory of accessories) {
            const bone = boneIndexFor(accessory, skeleton);
            if (bone < 0) {
                throw new Error(`${accessory.name} is not parented to any bone; cannot bake it`);
            }
            accessory.computeWorldMatrix(true);
            const rebase = accessory.getWorldMatrix().multiply(skinInverse);
            const positions = accessory.getVerticesData(BABYLON.VertexBuffer.PositionKind);
            const normals = accessory.getVerticesData(BABYLON.VertexBuffer.NormalKind);
            const uvs = accessory.getVerticesData(BABYLON.VertexBuffer.UVKind);
            const accessoryIndices = accessory.getIndices();
            if (!positions || !accessoryIndices) {
                throw new Error(`${accessory.name} has no geometry to bake`);
            }
            const base = data[BABYLON.VertexBuffer.PositionKind].length / 3;
            const point = new BABYLON.Vector3();
            for (let v = 0; v < positions.length; v += 3) {
                point.set(positions[v], positions[v + 1], positions[v + 2]);
                const world = BABYLON.Vector3.TransformCoordinates(point, rebase);
                data[BABYLON.VertexBuffer.PositionKind].push(world.x, world.y, world.z);
                if (normals) {
                    point.set(normals[v], normals[v + 1], normals[v + 2]);
                    const n = BABYLON.Vector3.TransformNormal(point, rebase);
                    n.normalize();
                    data[BABYLON.VertexBuffer.NormalKind].push(n.x, n.y, n.z);
                } else {
                    data[BABYLON.VertexBuffer.NormalKind].push(0, 1, 0);
                }
                const vertex = v / 3;
                data[BABYLON.VertexBuffer.UVKind].push(
                    uvs ? uvs[vertex * 2] : 0,
                    uvs ? uvs[vertex * 2 + 1] : 0,
                );
                data[BABYLON.VertexBuffer.MatricesIndicesKind].push(bone, 0, 0, 0);
                data[BABYLON.VertexBuffer.MatricesWeightsKind].push(1, 0, 0, 0);
            }
            for (const index of accessoryIndices) {
                indices.push(index + base);
            }
            accessory.dispose(false, false);
        }

        for (const kind of BAKEABLE_KINDS) {
            skin.setVerticesData(kind, data[kind], false);
        }
        skin.setIndices(indices);
        skin.numBoneInfluencers = 4;
        skin.refreshBoundingInfo(true);
        return skin;
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
        if (variant !== "authored" && skinned.length > 1) {
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
            if (variant === "merged_all") {
                const gear = meshes.filter((m) => !m.skeleton);
                bakeIntoSkin(target, gear, skeleton);
                meshes = root
                    .getChildMeshes(false)
                    .filter((m) => m.getTotalVertices() > 0 && m.isEnabled());
                if (meshes.length !== 1) {
                    throw new Error(
                        `merged_all left ${meshes.length} meshes on a character; expected 1`,
                    );
                }
            }
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

    /**
     * Engine, scene, lights, camera, ball. The camera is IDENTICAL at every
     * character count and frames the whole pitch. Two things follow, both
     * deliberate. Every character stays inside the frustum, so a curve cannot be
     * flattened by culling half the roster. And per-character pixel coverage
     * does not change with count, so added characters add shading linearly
     * instead of shrinking to nothing. Whole-pitch framing is also what the
     * native LÖVE baseline in #328 measured, which is what lets the two sit in
     * one table.
     */
    function createScene(config, field) {
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

        const geometry = buildPitch(scene, field, shadowGenerator);

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
        camera.radius = 82;
        camera.beta = 1.0;
        camera.setTarget(new BABYLON.Vector3(0, 1, 0));

        const ball = BABYLON.MeshBuilder.CreateSphere(
            "ball",
            { diameter: 0.44, segments: 16 },
            scene,
        );
        const ballMaterial = new BABYLON.StandardMaterial("ballmat", scene);
        ballMaterial.diffuseColor = new BABYLON.Color3(0.95, 0.95, 0.98);
        ball.material = ballMaterial;
        if (shadowGenerator) {
            shadowGenerator.addShadowCaster(ball);
        }

        return { engine, scene, camera, shadowGenerator, geometry, ball };
    }

    // Height-normalise against the asset as actually instantiated, so the
    // character reads as 1.8 m next to a 2.44 m crossbar whatever units the pack
    // was authored in. Measured after building, because the bounds of a
    // loaded-but-unparented container are not the bounds of a live rig.
    function normaliseHeights(characters) {
        characters[0].root.computeWorldMatrix(true);
        const bounds = characters[0].root.getHierarchyBoundingVectors(true);
        const modelScale = CHARACTER_HEIGHT_METRES / Math.max(0.001, bounds.max.y - bounds.min.y);
        for (const character of characters) {
            character.root.scaling.setAll(modelScale);
        }
        return modelScale;
    }

    /**
     * The measurement loop. `update(frameIndex)` produces and applies one frame
     * of poses; `finish(stats)` is called once the requested number of measured
     * frames is in. Errors from either land on the page's error channel rather
     * than vanishing into a MessageChannel callback.
     */
    function driveLoop(options) {
        const { engine, scene, warmup, frames, update, finish } = options;
        const instrumentation = new BABYLON.SceneInstrumentation(scene);
        instrumentation.captureFrameTime = true;

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
            update(frameIndex);
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
            const warmed = frameIndex > warmup;
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

            if (frameIndex === warmup) {
                setStatus(`measuring (${frames} frames)`);
            }
            if (measured >= frames) {
                finish({ updateMs, drawMs, frameMs, drawCalls, measured });
                return;
            }
            channel.port2.postMessage(0);
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

    function resultMarker(stats, extra) {
        const meanCalls =
            stats.drawCalls.reduce((a, b) => a + b, 0) / Math.max(1, stats.drawCalls.length);
        const maxCalls = stats.drawCalls.reduce((a, b) => Math.max(a, b), 0);
        return {
            meanCalls,
            maxCalls,
            line:
                `GC_BENCH_RESULT|${extra}` +
                `|measured_frames=${stats.measured}` +
                `|${summaryFields(stats.updateMs, "update")}` +
                `|${summaryFields(stats.drawMs, "draw")}` +
                `|${summaryFields(stats.frameMs, "frame")}` +
                `|draw_calls_mean=${meanCalls.toFixed(1)}|draw_calls_max=${maxCalls}`,
        };
    }

    function sampleMarkers(renderer, stats) {
        return [
            `GC_BENCH_SAMPLES|renderer=${renderer}|kind=draw|unit=microseconds` +
                `|samples=${stats.drawMs.map((v) => Math.round(v * 1000)).join(",")}`,
            `GC_BENCH_SAMPLES|renderer=${renderer}|kind=frame|unit=microseconds` +
                `|samples=${stats.frameMs.map((v) => Math.round(v * 1000)).join(",")}`,
            `GC_BENCH_SAMPLES|renderer=${renderer}|kind=draw_calls|unit=count` +
                `|samples=${stats.drawCalls.join(",")}`,
        ];
    }

    globalThis.GoliseoBenchScene = {
        state,
        marker,
        fail,
        setStatus,
        VARIANTS,
        PITCH_METRES_X,
        CHARACTER_HEIGHT_METRES,
        POSE_CLIPS,
        LOCOMOTION_CLIPS,
        requiredClips,
        percentile,
        countAbove,
        summaryFields,
        gpuIdentity,
        envMarker,
        buildPitch,
        loadContainer,
        pruneClips,
        buildCharacter,
        bakeIntoSkin,
        applyPose,
        applyLocomotion,
        createScene,
        normaliseHeights,
        driveLoop,
        resultMarker,
        sampleMarkers,
    };
})();
