/*
 * The red demonstration for `spike.js`'s render check (#329).
 *
 * AGENTS.md section 9: "every gate must come with a demonstration that it can
 * go red". This is that demonstration for the one check in the spike that
 * asserts something was actually drawn -- and it exists because the first
 * version of that check could NOT go red. It counted a pixel as drawn if any
 * channel exceeded 8, while Babylon's default `Scene.clearColor` is
 * (0.2, 0.2, 0.3, 1) = R51 G51 B76, so an entirely empty frame scored 240000
 * of 240000 and the spike reported "it drew" on a blank screen.
 *
 * Load this file BEFORE `spike.js`. The X11 Playground host treats every argv
 * entry as a script to load, in order:
 *
 *     ./Playground app:///Scripts/prove_flat_fill.js app:///Scripts/spike.js
 *
 * `spike.js` then builds its scene exactly as normal -- same glTF, same
 * skeleton, same IK, same shadow map -- and disables every mesh immediately
 * before the frame is captured. Everything except the render check still runs
 * and still reports OK, which is the point: only the pixel check is being
 * falsified, and it must report FAIL and exit non-zero.
 *
 * If this run ever reports `check=render|status=OK`, the render check has
 * stopped discriminating and the "it drew" claim in
 * docs/design/native_route_decision.md is void again.
 */
globalThis.GC_BN_PROVE_FLAT_FILL = true;
console.log("GC_BN|check=prove_flat_fill|status=ARMED|expect=render FAIL and a non-zero exit");
