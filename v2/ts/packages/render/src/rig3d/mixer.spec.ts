// Tier-1/2 tests for the `THREE.AnimationMixer` playback layer.
//
// Runs headless. `MixerLayer` builds `THREE.Object3D`s, `THREE.AnimationClip`s
// and a mixer, none of which need a GL context -- the same boundary
// `player_renderer_3d.spec.ts` already draws for `build()`.
//
// Three things are pinned here, each because it is an assumption about
// THREE.JS'S OWN behaviour that this design leans on and that an upstream
// change could quietly invalidate:
//
//   1. dotted node names (`upper_arm.L`) resolve as one node, not as
//      node `upper_arm` / object `L`;
//   2. a zero-delta `update` evaluates at the pinned `.time` rather than
//      advancing or wrapping it; and
//   3. accumulated weight below 1 blends toward the value bound at bind time
//      (rest) -- which is what `clips.layer`'s masked-overlay semantics need
//      and what makes the locomotion base layer need no explicit mask at all.

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import type { Quat } from "@gc/core";
import * as clips from "./clips.ts";
import { BAKE_FPS, MixerLayer, bake } from "./mixer.ts";

// Largest absolute difference between any two quaternions' components.
function quatDelta(a: Readonly<Record<string, Quat>>, b: Readonly<Record<string, Quat>>): number {
  let worst = 0;
  for (const bone of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const qa = a[bone] ?? [0, 0, 0, 1];
    const qb = b[bone] ?? [0, 0, 0, 1];
    // Quaternion double cover: q and -q are the same rotation, so compare the
    // closer of the two representations.
    const dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
    const sign = dot < 0 ? -1 : 1;
    for (let i = 0; i < 4; i += 1) {
      worst = Math.max(worst, Math.abs((qa[i] ?? 0) - sign * (qb[i] ?? 0)));
    }
  }
  return worst;
}

describe("rig3d/mixer.bake", () => {
  it("names tracks so three.js resolves a dotted bone name as one node", () => {
    const baked = bake(clips.WALK);
    const names = baked.tracks.map((t) => t.name);
    expect(names).toContain("upper_arm.L.quaternion");
    expect(names).toContain("root.position");
    const parsed = THREE.PropertyBinding.parseTrackName("upper_arm.L.quaternion");
    expect(parsed.nodeName).toBe("upper_arm.L");
    expect(parsed.propertyName).toBe("quaternion");
    expect(parsed.objectName).toBeUndefined();
  });

  it("carries one track per authored channel and nothing else", () => {
    const baked = bake(clips.GUARD_STANCE);
    expect(baked.tracks.filter((t) => t.name.endsWith(".quaternion"))).toHaveLength(clips.GUARD_STANCE.rotBones.size);
    expect(baked.tracks.filter((t) => t.name.endsWith(".position"))).toHaveLength(clips.GUARD_STANCE.moveBones.size);
    expect(baked.duration).toBe(clips.GUARD_STANCE.duration);
  });

  it("keeps a one-shot's final key instead of wrapping back to its first", () => {
    // `clips.sample` wraps with `time % duration`, so a naive bake would end
    // KEEPER_SLING's release on its cocked pose. Compare the last baked
    // keyframe against a sample taken just inside the clip.
    const baked = bake(clips.KEEPER_SLING);
    const track = baked.tracks.find((t) => t.name === "upper_arm.R.quaternion");
    if (track === undefined) {
      throw new Error("expected an upper_arm.R rotation track");
    }
    const n = track.times.length;
    expect(track.times[n - 1]).toBeCloseTo(clips.KEEPER_SLING.duration, 6);
    const settled = clips.sample(clips.KEEPER_SLING, clips.KEEPER_SLING.duration - 1e-6).rot["upper_arm.R"];
    const cocked = clips.sample(clips.KEEPER_SLING, 0).rot["upper_arm.R"];
    if (settled === undefined || cocked === undefined) {
      throw new Error("expected sampled rotations");
    }
    const last: Quat = [track.values[(n - 1) * 4] ?? 0, track.values[(n - 1) * 4 + 1] ?? 0, track.values[(n - 1) * 4 + 2] ?? 0, track.values[(n - 1) * 4 + 3] ?? 1];
    expect(quatDelta({ q: last }, { q: settled })).toBeLessThan(1e-4);
    expect(quatDelta({ q: last }, { q: cocked })).toBeGreaterThan(0.1);
  });
});

describe("rig3d/mixer.MixerLayer", () => {
  it("evaluates at the pinned phase rather than advancing a clock of its own", () => {
    const layer = new MixerLayer([{ id: "walk", clip: clips.WALK }]);
    layer.silence();
    layer.set("walk", 0.3, 1);
    const first = layer.evaluate();
    // Evaluating again with the same pin must give the same pose: a zero
    // delta is evaluation, not playback.
    const second = layer.evaluate();
    expect(quatDelta(first.rot, second.rot)).toBe(0);
    layer.set("walk", 0.5, 1);
    expect(quatDelta(first.rot, layer.evaluate().rot)).toBeGreaterThan(0.01);
  });

  // Baking exists so `clips.ts` stays the single authority on what a clip
  // looks like (three.js will not cubic-interpolate quaternion tracks, so the
  // authored smoothstep ease has to be resampled rather than reimplemented).
  // This is the tolerance that claim is worth.
  it("reproduces clips.sample to within the resampling error, on every clip", () => {
    for (const clip of [clips.IDLE, clips.WALK, clips.RUN, clips.GUARD_STANCE, clips.CHARGE, clips.KEEPER_GATHER, clips.KEEPER_SLING, clips.SWING]) {
      const layer = new MixerLayer([{ id: "c", clip }]);
      let worst = 0;
      for (let i = 0; i < 40; i += 1) {
        const t = (i / 40) * clip.duration;
        layer.silence();
        layer.set("c", t, 1);
        worst = Math.max(worst, quatDelta(layer.evaluate().rot, clips.sample(clip, t).rot));
      }
      expect(worst, `${clip.name} at ${BAKE_FPS}fps`).toBeLessThan(1.5e-3);
    }
  });

  it("blends its own actions the way clips.layer would, when their weights sum to 1", () => {
    const layer = new MixerLayer([
      { id: "idle", clip: clips.IDLE },
      { id: "walk", clip: clips.WALK },
    ]);
    const mix = 0.35;
    layer.silence();
    layer.set("idle", 1.1, 1 - mix);
    layer.set("walk", 0.24, mix);
    const blended = layer.evaluate();
    const expected = clips.layer(clips.sample(clips.IDLE, 1.1), clips.sample(clips.WALK, 0.24), new Set(layer.rotBones), mix);
    expect(quatDelta(blended.rot, expected.rot)).toBeLessThan(5e-3);
  });

  // The behaviour the whole layering design rests on: three.js blends toward
  // the value a binding held at bind time (rest, here) when the accumulated
  // weight is below 1. That is `clips.layer`'s masked-overlay semantics for
  // free, and it is why the locomotion base needs no `masks.FULL_BODY` pass.
  it("fades toward rest, not toward the previous value, when total weight is below 1", () => {
    const layer = new MixerLayer([{ id: "guard", clip: clips.GUARD_STANCE }]);
    layer.silence();
    layer.set("guard", 0, 1);
    const full = layer.evaluate().rot["forearm.R"];
    layer.silence();
    layer.set("guard", 0, 0);
    const none = layer.evaluate().rot["forearm.R"];
    layer.silence();
    layer.set("guard", 0, 0.5);
    const half = layer.evaluate().rot["forearm.R"];
    if (full === undefined || none === undefined || half === undefined) {
      throw new Error("expected a forearm.R rotation");
    }
    expect(quatDelta({ q: none }, { q: [0, 0, 0, 1] })).toBeLessThan(1e-6);
    expect(quatDelta({ q: half }, { q: full })).toBeGreaterThan(1e-3);
    expect(quatDelta({ q: half }, { q: [0, 0, 0, 1] })).toBeGreaterThan(1e-3);
  });

  // 22 characters share one layer instance per frame (see `animator.ts`), so
  // an evaluation must depend only on what was pinned for it.
  it("carries nothing between evaluations, so one shared layer can serve every character", () => {
    const layer = new MixerLayer([
      { id: "guard", clip: clips.GUARD_STANCE },
      { id: "charge", clip: clips.CHARGE },
    ]);
    layer.silence();
    layer.set("guard", 0.4, 1);
    const guardFirst = layer.evaluate();
    layer.silence();
    layer.set("charge", 0.2, 1);
    layer.evaluate();
    layer.silence();
    layer.set("guard", 0.4, 1);
    expect(quatDelta(layer.evaluate().rot, guardFirst.rot)).toBe(0);
  });

  it("rejects an unknown action id loudly rather than animating nothing", () => {
    const layer = new MixerLayer([{ id: "walk", clip: clips.WALK }]);
    expect(() => layer.set("sprint", 0, 1)).toThrow(/no action "sprint"/);
  });

  // THE PRECONDITION `evaluate()`'s zero-delta path depends on. Three.js's
  // `AnimationAction._updateTime` early-returns on a zero delta and hands back
  // `this.time` unchanged, so NONE of `LoopRepeat`'s wrap, `LoopOnce`'s clamp
  // or `clampWhenFinished` ever runs. An out-of-range phase would therefore
  // freeze on the first or last keyframe -- silently, and for a clock-driven
  // action only after several seconds of play. Enforced by `set()` rather than
  // left to its one current caller's habit.
  it("rejects a phase outside the clip, which zero-delta evaluation would otherwise freeze on", () => {
    const layer = new MixerLayer([{ id: "swing", clip: clips.SWING, loop: "clamp" }]);
    expect(() => layer.set("swing", -0.01, 1)).toThrow(/outside "swing"/);
    expect(() => layer.set("swing", clips.SWING.duration + 0.01, 1)).toThrow(/outside "swing"/);
    expect(() => layer.set("swing", Number.NaN, 1)).toThrow(/outside "swing"/);
    // Both ends of the closed interval are legal: a wrapped loop phase can be
    // exactly 0, and a clamped one-shot phase can be exactly the duration.
    expect(() => layer.set("swing", 0, 1)).not.toThrow();
    expect(() => layer.set("swing", clips.SWING.duration, 1)).not.toThrow();
  });

  // The same invariant, checked against the concrete mistake `set()` exists to
  // catch: an unwrapped wall-clock phase. It is the one a caller is most
  // likely to make, because `now` grows without bound.
  it("would catch an unwrapped wall-clock phase rather than freezing on the last key", () => {
    const layer = new MixerLayer([{ id: "idle", clip: clips.IDLE }]);
    const tenSecondsIn = 10 * 0.35;
    expect(tenSecondsIn).toBeGreaterThan(clips.IDLE.duration);
    expect(() => layer.set("idle", tenSecondsIn, 1)).toThrow(/outside "idle"/);
  });
});
