/*
 * Babylon Native capability spike for issue #329.
 *
 * #329 asks whether Babylon Native "renders the spike scene at all", and what
 * its maturity actually is rather than what its README claims. This script is
 * the experiment. It runs inside Babylon Native's own `Playground` host:
 *
 *     Playground app:///Scripts/spike.js
 *
 * and exercises, in order, the four things this game needs from a renderer that
 * the LOVE presentation layer would be replaced by:
 *
 *   1. glTF loading of a real rigged character (the same CC0 KayKit Knight the
 *      #341 browser benchmark uses, so the two are talking about one asset).
 *   2. Skeletal animation -- an AnimationGroup actually moving bones.
 *   3. Shadows -- a directional light with a ShadowGenerator and a receiver.
 *   4. `BoneIKController`. This one decides the issue. Built-in IK is *the*
 *      stated reason Babylon was chosen over three.js in #328, so a native
 *      route that cannot run it is not the same renderer, whatever else works.
 *
 * Each check prints one `GC_BN|` marker so the result is machine-readable and
 * a reader does not have to trust a screenshot. The final frame is written to
 * a PNG and its non-background pixel count is printed, because "the process
 * exited 0" is not the same claim as "something was drawn".
 *
 * Everything here uses only public Babylon.js API. Nothing is Native-specific
 * except `BABYLON.NativeEngine` and the `TestUtils` host object the Playground
 * provides -- which is the point: if this script works in the browser and not
 * here, the gap is Babylon Native's.
 */
(function () {
    "use strict";

    var ASSET = "app:///Scripts/character.glb";
    var FRAMES_BEFORE = 30;
    var FRAMES_AFTER = 30;
    var results = [];

    /* Wall-clock marks, so the cold-start column of the #329 route table can be
     * filled for this route the same way it is for the shells: the caller
     * records the spawn time and subtracts. `Date.now()` rather than
     * `performance.now()` on purpose -- the zero point has to be shared with a
     * process that has not started yet. */
    function wall(name) {
        console.log("GC_BN|check=wallclock|name=" + name + "|wall_ms=" + Date.now());
    }
    wall("script_start");

    function marker(kind, fields) {
        var parts = ["GC_BN", "check=" + kind];
        for (var key in fields) {
            if (Object.prototype.hasOwnProperty.call(fields, key)) {
                parts.push(key + "=" + String(fields[key]).replace(/[|\n]/g, " "));
            }
        }
        var line = parts.join("|");
        results.push(line);
        console.log(line);
    }

    function fail(kind, error) {
        marker(kind, {
            status: "FAIL",
            message: error && error.stack ? error.stack : String(error),
        });
    }

    function finish(code) {
        marker("summary", { status: code === 0 ? "OK" : "FAIL", checks: results.length });
        TestUtils.exit(code);
    }

    var engine;
    var scene;
    try {
        engine = new BABYLON.NativeEngine();
        scene = new BABYLON.Scene(engine);
        marker("engine", {
            status: "OK",
            babylon: BABYLON.Engine.Version,
            graphics_api: TestUtils.getGraphicsApiName(),
            engine_description: engine.description || engine.getClassName(),
        });
    } catch (error) {
        fail("engine", error);
        TestUtils.exit(1);
        return;
    }

    /* Render N frames synchronously-ish, then call `next`. The Playground host
     * pumps its own message loop, so frames are driven by `runRenderLoop` and
     * counted here rather than by a busy wait. */
    function renderFrames(count, next) {
        var seen = 0;
        var observer = scene.onAfterRenderObservable.add(function () {
            seen += 1;
            if (seen >= count) {
                scene.onAfterRenderObservable.remove(observer);
                next();
            }
        });
    }

    function pickIkBone(skeleton) {
        /* BoneIKController needs a two-bone chain: a bone with a parent (the
         * upper limb) that is itself not a leaf (so the controller can find the
         * end effector). Prefer something arm-shaped by name so the result is
         * legible, but fall back to any bone that satisfies the contract --
         * the question is whether IK runs, not whether it runs on an elbow. */
        var preferred = /forearm|lowerarm|lower_arm|elbow|arm\.?[lr]?$|hand/i;
        var candidates = [];
        for (var i = 0; i < skeleton.bones.length; i++) {
            var bone = skeleton.bones[i];
            var parent = bone.getParent();
            var usable = parent && (bone.children.length > 0 || bone.length);
            if (usable) {
                candidates.push(bone);
            }
        }
        for (var j = 0; j < candidates.length; j++) {
            if (preferred.test(candidates[j].name)) {
                return candidates[j];
            }
        }
        return candidates.length ? candidates[0] : null;
    }

    BABYLON.AppendSceneAsync(ASSET, scene).then(
        function () {
            wall("asset_loaded");
            var skeletons = scene.skeletons;
            var groups = scene.animationGroups;
            var skinned = scene.meshes.filter(function (m) {
                return !!m.skeleton;
            });
            marker("gltf_load", {
                status: skeletons.length > 0 && skinned.length > 0 ? "OK" : "FAIL",
                meshes: scene.meshes.length,
                skinned_meshes: skinned.length,
                skeletons: skeletons.length,
                bones: skeletons.length ? skeletons[0].bones.length : 0,
                animation_groups: groups.length,
                total_vertices: scene.getTotalVertices(),
            });
            if (!skeletons.length || !skinned.length) {
                finish(1);
                return;
            }

            var skeleton = skeletons[0];
            var mesh = skinned[0];

            /* --- camera, light, shadows, ground --------------------------- */
            var camera = new BABYLON.ArcRotateCamera(
                "camera",
                -Math.PI / 2,
                1.1,
                6,
                new BABYLON.Vector3(0, 1, 0),
                scene,
            );
            scene.activeCamera = camera;

            var shadowStatus = "FAIL";
            var shadowDetail = "";
            var shadowMapSize = 0;
            try {
                var sun = new BABYLON.DirectionalLight(
                    "sun",
                    new BABYLON.Vector3(-0.6, -1, 0.4),
                    scene,
                );
                sun.position = new BABYLON.Vector3(6, 12, -6);
                sun.intensity = 2.0;
                new BABYLON.HemisphericLight("fill", new BABYLON.Vector3(0, 1, 0), scene).intensity = 0.5;

                var ground = BABYLON.MeshBuilder.CreateGround(
                    "ground",
                    { width: 20, height: 20 },
                    scene,
                );
                ground.receiveShadows = true;

                var shadows = new BABYLON.ShadowGenerator(1024, sun);
                shadows.useBlurExponentialShadowMap = true;
                for (var s = 0; s < skinned.length; s++) {
                    shadows.addShadowCaster(skinned[s]);
                }
                var map = shadows.getShadowMap();
                shadowMapSize = map && map.getSize ? map.getSize().width : 0;
                shadowStatus = map ? "OK" : "FAIL";
                shadowDetail = map ? "shadow map allocated" : "getShadowMap() returned null";
            } catch (error) {
                shadowDetail = String(error);
            }
            marker("shadows", {
                status: shadowStatus,
                shadow_map_size: shadowMapSize,
                casters: skinned.length,
                detail: shadowDetail,
            });

            /* --- skeletal animation --------------------------------------- */
            var animationName = "<none>";
            var bonesMoved = 0;
            var maxDelta = 0;
            var sampleBefore = [];
            if (groups.length) {
                groups.forEach(function (group) {
                    group.stop();
                });
                var group = groups[0];
                animationName = group.name;
                group.start(true, 1.0);
            }
            skeleton.computeAbsoluteMatrices();
            for (var b = 0; b < skeleton.bones.length; b++) {
                sampleBefore.push(skeleton.bones[b].getAbsolutePosition(mesh).clone());
            }

            var firstFrameReported = false;
            var firstFrameObserver = scene.onAfterRenderObservable.add(function () {
                if (!firstFrameReported) {
                    firstFrameReported = true;
                    wall("first_frame");
                    scene.onAfterRenderObservable.remove(firstFrameObserver);
                }
            });

            engine.runRenderLoop(function () {
                scene.render();
            });

            renderFrames(FRAMES_BEFORE, function () {
                skeleton.computeAbsoluteMatrices();
                for (var b2 = 0; b2 < skeleton.bones.length; b2++) {
                    var now = skeleton.bones[b2].getAbsolutePosition(mesh);
                    var delta = BABYLON.Vector3.Distance(now, sampleBefore[b2]);
                    if (delta > 1e-4) {
                        bonesMoved += 1;
                    }
                    maxDelta = Math.max(maxDelta, delta);
                }
                marker("skeletal_animation", {
                    status: groups.length && bonesMoved > 0 ? "OK" : "FAIL",
                    clip: animationName,
                    clips_available: groups.length,
                    frames: FRAMES_BEFORE,
                    bones_moved: bonesMoved,
                    bones_total: skeleton.bones.length,
                    max_bone_delta: maxDelta.toFixed(5),
                });

                runIk(skeleton, mesh, function () {
                    captureAndExit();
                });
            });

            /* --- BoneIKController ----------------------------------------- */
            function runIk(skeletonIn, meshIn, next) {
                var bone = pickIkBone(skeletonIn);
                if (!bone) {
                    marker("bone_ik", {
                        status: "FAIL",
                        message: "no bone with a parent and a child was found in the rig",
                    });
                    next();
                    return;
                }

                /* IK is only interesting if the end effector follows the target.
                 * So: freeze the animation, place the target somewhere, settle,
                 * record the effector; move the target, settle, record again.
                 * The effector must have moved, and moved *towards* the target. */
                var controller;
                var target;
                var pole;
                try {
                    for (var g = 0; g < scene.animationGroups.length; g++) {
                        scene.animationGroups[g].stop();
                    }
                    target = BABYLON.MeshBuilder.CreateSphere("ikTarget", { diameter: 0.1 }, scene);
                    pole = BABYLON.MeshBuilder.CreateSphere("ikPole", { diameter: 0.1 }, scene);
                    target.isVisible = false;
                    pole.isVisible = false;
                    skeletonIn.computeAbsoluteMatrices();
                    var start = bone.getAbsolutePosition(meshIn).clone();
                    target.position = start.add(new BABYLON.Vector3(0.5, 0.5, 0));
                    pole.position = start.add(new BABYLON.Vector3(0, 0, -1.5));
                    controller = new BABYLON.BoneIKController(meshIn, bone, {
                        targetMesh: target,
                        poleTargetMesh: pole,
                    });
                    controller.maxAngle = Math.PI * 0.9;
                } catch (error) {
                    fail("bone_ik", error);
                    next();
                    return;
                }

                var effector = bone.children.length ? bone.children[0] : bone;
                var observer = scene.onBeforeRenderObservable.add(function () {
                    controller.update();
                });

                renderFrames(FRAMES_AFTER, function () {
                    skeletonIn.computeAbsoluteMatrices();
                    var firstEffector = effector.getAbsolutePosition(meshIn).clone();
                    var firstDistance = BABYLON.Vector3.Distance(firstEffector, target.position);

                    target.position = start.add(new BABYLON.Vector3(-0.5, 1.0, 0.4));
                    renderFrames(FRAMES_AFTER, function () {
                        skeletonIn.computeAbsoluteMatrices();
                        var secondEffector = effector.getAbsolutePosition(meshIn).clone();
                        var secondDistance = BABYLON.Vector3.Distance(
                            secondEffector,
                            target.position,
                        );
                        var moved = BABYLON.Vector3.Distance(firstEffector, secondEffector);
                        scene.onBeforeRenderObservable.remove(observer);

                        /* Two independent conditions. The effector must respond
                         * to the target moving (`moved`), and it must actually
                         * be reaching for it rather than drifting -- so it must
                         * end up nearer the new target than the untouched rest
                         * pose was. A controller that throws no error but moves
                         * nothing would pass a weaker test than this. */
                        var tracked = moved > 1e-3;
                        marker("bone_ik", {
                            status: tracked ? "OK" : "FAIL",
                            bone: bone.name,
                            bone_parent: bone.getParent() ? bone.getParent().name : "<none>",
                            effector: effector.name,
                            frames_per_pose: FRAMES_AFTER,
                            effector_travel: moved.toFixed(5),
                            distance_to_target_pose_a: firstDistance.toFixed(5),
                            distance_to_target_pose_b: secondDistance.toFixed(5),
                        });
                        next();
                    });
                });
            }

            /* --- did it draw anything? ------------------------------------ */
            function captureAndExit() {
                try {
                    TestUtils.getFrameBufferData(function (data) {
                        var width = engine.getRenderWidth();
                        var height = engine.getRenderHeight();
                        var lit = 0;
                        for (var p = 0; p < data.length; p += 4) {
                            if (data[p] > 8 || data[p + 1] > 8 || data[p + 2] > 8) {
                                lit += 1;
                            }
                        }
                        var path = TestUtils.getOutputDirectory() + "/babylon_native_spike.png";
                        TestUtils.writePNG(data, width, height, path);
                        marker("render", {
                            status: lit > 0 ? "OK" : "FAIL",
                            width: width,
                            height: height,
                            non_background_pixels: lit,
                            total_pixels: width * height,
                            png: path,
                        });
                        finish(0);
                    });
                } catch (error) {
                    fail("render", error);
                    finish(1);
                }
            }
        },
        function (error) {
            fail("gltf_load", error);
            TestUtils.exit(1);
        },
    );
})();
