import { describe, expect, it } from "vitest";
import { quat, type Quat } from "@gc/core";
import * as clips from "./clips.ts";
import * as masks from "./masks.ts";

// Quaternion components, read with a runtime bounds check: `Quat` is a fixed
// 4-tuple, but indexing it with a non-literal loop variable widens to
// `number | undefined` under `noUncheckedIndexedAccess`.
function comp(v: readonly number[], i: number): number {
  const x = v[i];
  if (x === undefined) {
    throw new Error(`index ${i} out of range`);
  }
  return x;
}

describe("rig3d clips", () => {
  it("every clip loops: the last keyframe matches the first", () => {
    // Looping clips only. KEEPER_SLING is a one-shot with a `fallback`, so
    // its last keyframe is a follow-through and must NOT match its first.
    const looping = [clips.ORDER[0], clips.ORDER[1], clips.RUN, clips.CHARGE, clips.KEEPER_GATHER];
    for (const clip of looping) {
      expect(clip).toBeDefined();
      if (!clip) continue;
      expect(clip.loop, `${clip.name} is listed here but does not declare loop`).toBe(true);
      const first = clips.sample(clip, 0);
      const last = clips.sample(clip, clip.duration - 1e-6);
      for (const [bone, q] of Object.entries(first.rot)) {
        const other = last.rot[bone];
        expect(other, `${clip.name}: ${bone} missing at loop point`).toBeDefined();
        if (!other) continue;
        for (let i = 0; i < 4; i++) {
          expect(comp(q, i)).toBeCloseTo(comp(other, i), 3);
        }
      }
    }
  });

  it("sampling wraps rather than running off the end", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const a = clips.sample(walk, 0.1);
    const b = clips.sample(walk, walk.duration + 0.1);
    for (const [bone, q] of Object.entries(a.rot)) {
      const other = b.rot[bone];
      expect(other).toBeDefined();
      if (!other) continue;
      for (let i = 0; i < 4; i++) {
        expect(comp(q, i)).toBeCloseTo(comp(other, i), 9);
      }
    }
  });

  it("layer leaves bones outside the mask untouched", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.2);
    const overlay = clips.sample(clips.CHARGE, 0.1);
    const out = clips.layer(base, overlay, masks.UPPER_BODY, 1);
    // Legs are not in UPPER_BODY, so they must survive verbatim.
    for (const bone of ["thigh.R", "shin.R", "foot.L", "toe.L"]) {
      const baseQ = base.rot[bone];
      if (baseQ) {
        const outQ = out.rot[bone];
        expect(outQ).toBeDefined();
        if (!outQ) continue;
        for (let i = 0; i < 4; i++) {
          expect(comp(outQ, i)).toBeCloseTo(comp(baseQ, i), 12);
        }
      }
    }
  });

  it("layer at weight 0 is the base pose", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.3);
    const overlay = clips.sample(clips.CHARGE, 0.2);
    const out = clips.layer(base, overlay, masks.UPPER_BODY, 0);
    for (const [bone, q] of Object.entries(base.rot)) {
      const outQ = out.rot[bone];
      expect(outQ).toBeDefined();
      if (!outQ) continue;
      for (let i = 0; i < 4; i++) {
        expect(comp(outQ, i)).toBeCloseTo(comp(q, i), 9);
      }
    }
  });

  it("layer at weight 1 takes the overlay on masked bones", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.3);
    const overlay = clips.sample(clips.CHARGE, 0.2);
    const out = clips.layer(base, overlay, masks.UPPER_BODY, 1);
    for (const [bone, q] of Object.entries(overlay.rot)) {
      if (masks.UPPER_BODY.has(bone)) {
        const outQ = out.rot[bone];
        expect(outQ).toBeDefined();
        if (!outQ) continue;
        for (let i = 0; i < 4; i++) {
          expect(comp(outQ, i)).toBeCloseTo(comp(q, i), 9);
        }
      }
    }
  });

  // `compose` against `layer`, on the same base pose, because the difference
  // between them is the whole reason both exist (#445). `layer` OVERRIDES
  // through a mask, so a bone the mask owns and the overlay is silent about
  // goes to REST -- which is what would delete a stride if a sparse crouch were
  // masked over the legs. `compose` ADDS, and touches nothing the overlay does
  // not name.
  it("compose adds where layer overrides, and leaves unnamed bones alone", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.3);
    // A sparse overlay: one leg bone and a root translation, nothing else.
    const overlay = {
      rot: { "shin.R": quat.fromEuler(Math.PI / 6, 0, 0) },
      move: { root: [0, -0.05, 0] as const },
    };
    const restQ = quat.identity();

    // Layered through a mask that OWNS the legs, the bones the overlay never
    // mentions are sent to rest -- the stride on `thigh.R` is gone.
    const layered = clips.layer(base, overlay, masks.LOWER_BODY, 1);
    for (let i = 0; i < 4; i++) {
      expect(
        comp(layered.rot["thigh.R"] ?? restQ, i),
        "layer sends an unmentioned masked bone to rest",
      ).toBeCloseTo(comp(restQ, i), 9);
    }

    // Composed, the same overlay leaves it exactly as the walk resolved it.
    const composed = clips.compose(base, overlay);
    // Non-vacuous: the walk really does pose that bone, so "untouched" is a
    // claim about something rather than about an absent key.
    expect(
      base.rot["thigh.R"],
      "the walk poses the thigh, or neither half means anything",
    ).toBeDefined();
    expect(composed.rot["thigh.R"], "compose leaves an unnamed bone untouched").toBe(
      base.rot["thigh.R"],
    );

    // And where it DOES speak, it adds rather than replaces -- both channels.
    const expected = quat.multiply(overlay.rot["shin.R"], base.rot["shin.R"] ?? restQ);
    for (let i = 0; i < 4; i++) {
      expect(comp(composed.rot["shin.R"] ?? restQ, i), "compose pre-multiplies").toBeCloseTo(
        comp(expected, i),
        9,
      );
    }
    expect(composed.move["root"]?.[1] ?? 0, "and sums translations").toBeCloseTo(
      (base.move["root"]?.[1] ?? 0) - 0.05,
      12,
    );
  });

  // THE INVARIANT `compose`'s EXACTNESS RESTS ON, and which nothing else
  // enforces (#445).
  //
  // `compose` pre-multiplies, and `quat.fromEuler` composes as `Ry * Rx * Rz`.
  // So `Rx(-c) * (Rx(rx) * Rz(rz))` collapses to `Rx(rx - c) * Rz(rz)` -- the
  // fold lands exactly in the authored x angle -- ONLY while the leg keys carry
  // no y. Put a twist in the chain and `Rx(-c) * (Ry(ry) * Rx(rx) * Rz(rz))`
  // does not collapse: it is still a fold about the hip's flexion axis, but no
  // longer the one `crouch.angleFor` sized, so the fold and the drop stop
  // cancelling and the soles drift off the turf by an amount nobody measured.
  //
  // Every locomotion and action clip in the tree happens to key the legs in x
  // and z alone. This is what turns "happens to" into "has to": author a thigh
  // twist and you get a failing test naming the reason, rather than a subtly
  // wrong crouch.
  //
  // ENUMERATED FROM THE MODULE'S OWN EXPORTS rather than from a list written
  // here, so a clip added tomorrow is covered without being remembered.
  it("keys no y rotation on the bones a crouch folds, which is what makes compose exact", () => {
    const CROUCH_BONES = ["thigh.L", "thigh.R", "shin.L", "shin.R", "foot.L", "foot.R"];
    const all = Object.values(clips as unknown as Record<string, unknown>).filter(
      (v): v is clips.Clip =>
        typeof v === "object" &&
        v !== null &&
        Array.isArray((v as clips.Clip).keys) &&
        "rotBones" in v,
    );
    expect(
      all.length,
      "every clip the module exports, or this proves nothing",
    ).toBeGreaterThanOrEqual(8);

    let checked = 0;
    for (const clip of all) {
      for (const key of clip.keys) {
        for (const bone of CROUCH_BONES) {
          const q = key.q[bone];
          if (q === undefined) {
            continue;
          }
          checked += 1;
          // Recover the x and z halves on the assumption there is no y, rebuild,
          // and require the rebuild to be the original: a y-twist survives the
          // round trip as a mismatch. Well conditioned because no leg key comes
          // near 180 degrees -- asserted, so a future key that did would fail
          // here loudly instead of being waved through by a degenerate atan2.
          expect(comp(q, 3), `${clip.name}/${bone}: a leg key past 180 degrees`).toBeGreaterThan(
            0.1,
          );
          const rx = 2 * Math.atan2(comp(q, 0), comp(q, 3));
          const rz = 2 * Math.atan2(comp(q, 2), comp(q, 3));
          const rebuilt = quat.fromEuler(rx, 0, rz);
          for (let i = 0; i < 4; i++) {
            expect(comp(rebuilt, i), `${clip.name} keys y on ${bone}`).toBeCloseTo(comp(q, i), 12);
          }
        }
      }
    }
    // NON-VACUOUS: the clips really do pose these bones, so "no y on them" is a
    // claim about keys that exist rather than about an empty scan.
    expect(checked, "the leg bones really are keyed, in many clips").toBeGreaterThan(20);

    // AND THE CHECK ITSELF CATCHES ONE, so a broken round trip cannot pass
    // silently: the same reconstruction applied to a deliberately y-keyed
    // rotation must NOT come back equal.
    const twisted = quat.fromEuler(0.3, 0.4, 0.2);
    const rxT = 2 * Math.atan2(comp(twisted, 0), comp(twisted, 3));
    const rzT = 2 * Math.atan2(comp(twisted, 2), comp(twisted, 3));
    const rebuiltT = quat.fromEuler(rxT, 0, rzT);
    let worst = 0;
    for (let i = 0; i < 4; i++) {
      worst = Math.max(worst, Math.abs(comp(rebuiltT, i) - comp(twisted, i)));
    }
    expect(
      worst,
      "a y-twist must fail the round trip, or the sweep above is blind",
    ).toBeGreaterThan(0.01);
  });

  it("masks include the sockets attached to the hands they cover", () => {
    // A socket left out of the mask keeps the base layer's transform while
    // the arm follows the overlay, and the weapon detaches from the fist.
    for (const mask of [masks.UPPER_BODY, masks.ARMS]) {
      expect(
        mask.has("hand.R") && mask.has("socket_hand.R"),
        "socket must accompany its hand",
      ).toBe(true);
      expect(
        mask.has("hand.L") && mask.has("socket_hand.L"),
        "socket must accompany its hand",
      ).toBe(true);
    }
  });
});

// ---------------------------------------------------------------------------
// Per-bone key schedules (#580): a keyframe marked `sparse: true` speaks only
// for the bones it names, so each bone interpolates between ITS OWN
// neighbouring keys and keyframe density becomes a per-bone choice.
// ---------------------------------------------------------------------------

const DEG = Math.PI / 180;

// Asserts a sampled quaternion against an expected one, component-wise.
function expectQuat(got: Quat | undefined, want: Quat, label: string): void {
  expect(got, label).toBeDefined();
  if (!got) return;
  for (let i = 0; i < 4; i++) {
    expect(comp(got, i), `${label} [${i}]`).toBeCloseTo(comp(want, i), 9);
  }
}

// The pre-#580 sampler, verbatim: one segment search over the CLIP's keys,
// every bone slerped across that same segment, absent bones read as rest.
// Kept here as the reference the new per-track sampler must reproduce bit for
// bit on all-dense clips -- which is every clip the module ships.
function legacySample(
  clip: clips.Clip,
  time: number,
): { rot: Record<string, Quat>; move: Record<string, clips.EulerTriple> } {
  const ease = (u: number, kind: clips.Easing): number => {
    switch (kind) {
      case "linear":
        return u;
      case "accel":
        return u * u;
      case "decel":
        return u * (2 - u);
      case "smooth":
        return u * u * (3 - 2 * u);
    }
  };
  const IDENTITY: Quat = quat.identity();
  const ZERO: clips.EulerTriple = [0, 0, 0];
  const t = time % clip.duration;
  const keys = clip.keys;
  let i = 0;
  while (i < keys.length - 2) {
    const next = keys[i + 1];
    if (!next || next.t > t) {
      break;
    }
    i += 1;
  }
  const a = keys[i];
  const b = keys[i + 1];
  if (!a || !b) {
    throw new Error(`${clip.name}: clip has no keyframes to sample`);
  }
  const span = b.t - a.t;
  const u = span > 1e-6 ? (t - a.t) / span : 0;
  const uRot = ease(u, a.easeRot);
  const rot: Record<string, Quat> = {};
  for (const name of clip.rotBones) {
    rot[name] = quat.slerp(a.q[name] ?? IDENTITY, b.q[name] ?? IDENTITY, uRot);
  }
  const uMove = ease(u, a.easeMove);
  const move: Record<string, clips.EulerTriple> = {};
  for (const name of clip.moveBones) {
    const av = a.move[name] ?? ZERO;
    const bv = b.move[name] ?? ZERO;
    move[name] = [
      av[0] + (bv[0] - av[0]) * uMove,
      av[1] + (bv[1] - av[1]) * uMove,
      av[2] + (bv[2] - av[2]) * uMove,
    ];
  }
  return { rot, move };
}

describe("rig3d clips per-bone key schedules (#580)", () => {
  // THE BACKWARDS-COMPATIBILITY CLAIM, verified at full strength: on a clip
  // with only dense keys, every bone's track carries the clip's own key times,
  // so the per-track sampler runs the identical arithmetic in the identical
  // order and the poses are BIT-identical, not merely close. `toBe` is
  // Object.is, so even a stray -0 would fail here.
  //
  // ALL-DENSE CLIPS ONLY, since #575: the walk and run now carry sparse
  // stance-window keys, which is the exact behaviour the legacy sampler cannot
  // express (it would park every unnamed bone at rest at a sparse key's time),
  // so they are excluded by the same predicate that defines the claim. The
  // filter is written against each clip's own keys rather than as a hand-kept
  // list, and the count below keeps it from going vacuous.
  it("reproduces the pre-#580 sampler bit for bit on every all-dense shipped clip", () => {
    const all = [
      clips.IDLE,
      clips.WALK,
      clips.RUN,
      clips.GUARD_STANCE,
      clips.CHARGE,
      clips.KEEPER_GATHER,
      clips.KEEPER_SLING,
      clips.SWING,
    ].filter((clip) => clip.keys.every((key) => !key.sparse));
    expect(all.length, "most shipped clips are still all-dense").toBeGreaterThanOrEqual(6);
    for (const clip of all) {
      for (let k = 0; k <= 97; k += 1) {
        // 1.37 also exercises the wrap, and 97 lands between keys, on no
        // authored key spacing.
        const time = (k / 97) * clip.duration * 1.37;
        const ours = clips.sample(clip, time);
        const legacy = legacySample(clip, time);
        expect(Object.keys(ours.rot).sort()).toEqual(Object.keys(legacy.rot).sort());
        expect(Object.keys(ours.move).sort()).toEqual(Object.keys(legacy.move).sort());
        for (const [bone, q] of Object.entries(legacy.rot)) {
          const got = ours.rot[bone];
          expect(got, `${clip.name}/${bone} at ${time}`).toBeDefined();
          if (!got) continue;
          for (let i = 0; i < 4; i++) {
            expect(comp(got, i), `${clip.name}/${bone}[${i}] at ${time}`).toBe(comp(q, i));
          }
        }
        for (const [bone, v] of Object.entries(legacy.move)) {
          const got = ours.move[bone];
          expect(got, `${clip.name}/${bone} at ${time}`).toBeDefined();
          if (!got) continue;
          for (let i = 0; i < 3; i++) {
            expect(got[i], `${clip.name}/${bone}[${i}] at ${time}`).toBe(v[i]);
          }
        }
      }
    }
  });

  // THE ACCEPTANCE CASE: one bone keyed at a time another bone is not, with
  // correct independent interpolation on both channels.
  it("interpolates each bone between its own keys: a sparse key bends only the bones it names", () => {
    const clip = clips.prepare({
      name: "test_two_schedules",
      loop: false,
      root_motion: false,
      fallback: "idle",
      duration: 1,
      keys: [
        {
          t: 0,
          rot: { coarse: [0, 0, 0], fine: [0, 0, 0] },
          move: { root: [0, 0, 0] },
          ease: "linear",
        },
        { t: 0.25, sparse: true, rot: { fine: [40, 0, 0] }, ease: "linear" },
        { t: 1, rot: { coarse: [80, 0, 0], fine: [0, 0, 0] }, move: { root: [0, 0.2, 0] } },
      ],
    });

    // `fine` hits its own key exactly, and interpolates between ITS
    // neighbours on either side of it.
    expectQuat(clips.sample(clip, 0.25).rot["fine"], quat.fromEuler(40 * DEG, 0, 0), "fine@0.25");
    expectQuat(clips.sample(clip, 0.125).rot["fine"], quat.fromEuler(20 * DEG, 0, 0), "fine@0.125");
    expectQuat(clips.sample(clip, 0.625).rot["fine"], quat.fromEuler(20 * DEG, 0, 0), "fine@0.625");

    // `coarse` has no key at 0.25: it is 25% along its own single segment
    // there, NOT at rest -- the pre-#580 sampler would have read the absent
    // bone as IDENTITY and parked it at rest at that time.
    expectQuat(
      clips.sample(clip, 0.25).rot["coarse"],
      quat.fromEuler(20 * DEG, 0, 0),
      "coarse@0.25",
    );
    expectQuat(clips.sample(clip, 0.5).rot["coarse"], quat.fromEuler(40 * DEG, 0, 0), "coarse@0.5");

    // The move channel is independent the same way: `root` has no entry on
    // the sparse key, so it runs one linear segment from 0 to 0.2.
    expect(clips.sample(clip, 0.25).move["root"]?.[1], "root@0.25").toBeCloseTo(0.05, 12);
    expect(clips.sample(clip, 0.75).move["root"]?.[1], "root@0.75").toBeCloseTo(0.15, 12);
  });

  it("scopes a sparse key's easing to the bones it names -- the segment is bone-local", () => {
    const clip = clips.prepare({
      name: "test_bone_local_ease",
      loop: false,
      root_motion: false,
      fallback: "idle",
      duration: 1,
      keys: [
        { t: 0, rot: { coarse: [0, 0, 0], fine: [40, 0, 0] }, ease: "linear" },
        { t: 0.5, sparse: true, rot: { fine: [0, 0, 0] }, ease: "accel" },
        { t: 1, rot: { coarse: [80, 0, 0], fine: [40, 0, 0] } },
      ],
    });
    // `fine`'s segment [0.5, 1] runs the sparse key's `accel`: at u = 0.5 the
    // eased progress is 0.25, so it has covered 10 of its 40 degrees.
    expectQuat(clips.sample(clip, 0.75).rot["fine"], quat.fromEuler(10 * DEG, 0, 0), "fine@0.75");
    // `coarse`'s one segment [0, 1] still runs the dense key's `linear`
    // straight through the sparse key's time: 75% along at t = 0.75.
    expectQuat(
      clips.sample(clip, 0.75).rot["coarse"],
      quat.fromEuler(60 * DEG, 0, 0),
      "coarse@0.75",
    );
  });

  // #575's shape: the leg chain carries 5 keys per cycle while the torso
  // keeps 2, without the torso being re-keyed at any leg time. NOT the
  // shipped walk -- authoring that clip is #575's job -- just the proof the
  // format expresses it.
  it("expresses #575's leg densification without re-keying the torso", () => {
    const clip = clips.prepare({
      name: "test_leg_densify",
      loop: true,
      root_motion: false,
      duration: 0.8,
      keys: [
        { t: 0, rot: { chest: [3, 0, 0], "thigh.R": [-26, 0, 0] }, move: {}, ease: "linear" },
        { t: 0.2, sparse: true, rot: { "thigh.R": [6, 0, 0] }, ease: "linear" },
        { t: 0.4, sparse: true, rot: { "thigh.R": [22, 0, 0] }, ease: "linear" },
        { t: 0.6, sparse: true, rot: { "thigh.R": [-10, 0, 0] }, ease: "linear" },
        { t: 0.8, rot: { chest: [3, 0, 0], "thigh.R": [-26, 0, 0] }, move: {} },
      ],
    });
    // The leg hits every one of its five keys...
    expectQuat(clips.sample(clip, 0.2).rot["thigh.R"], quat.fromEuler(6 * DEG, 0, 0), "thigh@0.2");
    expectQuat(clips.sample(clip, 0.4).rot["thigh.R"], quat.fromEuler(22 * DEG, 0, 0), "thigh@0.4");
    expectQuat(
      clips.sample(clip, 0.6).rot["thigh.R"],
      quat.fromEuler(-10 * DEG, 0, 0),
      "thigh@0.6",
    );
    // ...while the torso holds its own two-key schedule, unmoved at each leg
    // key time instead of dipping toward rest there.
    for (const t of [0, 0.2, 0.4, 0.6, 0.79]) {
      expectQuat(clips.sample(clip, t).rot["chest"], quat.fromEuler(3 * DEG, 0, 0), `chest@${t}`);
    }
  });

  // #576's shape: a follow-through refinement key on the striking arm alone,
  // after the hit lands. Adding it must leave every OTHER bone's motion
  // bit-identical -- that is what "without re-keying uninvolved bones" means.
  it("expresses #576's arm follow-through: the extra key leaves other bones bit-identical", () => {
    const k0: clips.RawKeyframe = {
      t: 0,
      rot: { spine: [-7, 0, 0], "upper_arm.R": [-125, 0, 0] },
      ease: "accel",
    };
    const k1: clips.RawKeyframe = {
      t: 0.5,
      rot: { spine: [15, 0, 0], "upper_arm.R": [-45, 0, 0] },
      ease: "decel",
    };
    const k2: clips.RawKeyframe = { t: 1.4, rot: { spine: [0, 0, 0], "upper_arm.R": [-18, 0, 0] } };
    const followThrough: clips.RawKeyframe = {
      // 190ms after contact, on the striking limb alone.
      t: 0.69,
      sparse: true,
      rot: { "upper_arm.R": [-10, 0, 0] },
      ease: "decel",
    };
    const shared = { loop: false, root_motion: false, fallback: "idle", duration: 1.4 } as const;
    const base = clips.prepare({ ...shared, name: "test_swing_base", keys: [k0, k1, k2] });
    const refined = clips.prepare({
      ...shared,
      name: "test_swing_refined",
      keys: [k0, k1, followThrough, k2],
    });

    // The arm passes through its new refinement key exactly...
    expectQuat(
      clips.sample(refined, 0.69).rot["upper_arm.R"],
      quat.fromEuler(-10 * DEG, 0, 0),
      "arm@0.69",
    );
    // ...and really moved: the base clip has it elsewhere at that time.
    const baseArm = clips.sample(base, 0.69).rot["upper_arm.R"];
    const refinedArm = clips.sample(refined, 0.69).rot["upper_arm.R"];
    expect(
      baseArm && refinedArm && Math.abs(comp(baseArm, 0) - comp(refinedArm, 0)),
    ).toBeGreaterThan(0.01);

    // Every bone the sparse key does not name is bit-identical across the two
    // clips at every sampled time.
    for (let k = 0; k <= 55; k += 1) {
      const t = (k / 55) * 1.4;
      const a = clips.sample(base, t).rot["spine"];
      const b = clips.sample(refined, t).rot["spine"];
      expect(a, `spine@${t}`).toBeDefined();
      if (!a || !b) continue;
      for (let i = 0; i < 4; i++) {
        expect(comp(b, i), `spine[${i}]@${t}`).toBe(comp(a, i));
      }
    }
  });

  it("rejects a sparse first or last key and out-of-order keys, loudly", () => {
    const dense = (t: number): clips.RawKeyframe => ({ t, rot: { a: [0, 0, 0] } });
    expect(() =>
      clips.prepare({
        name: "bad_first",
        loop: true,
        root_motion: false,
        duration: 1,
        keys: [{ t: 0, sparse: true, rot: { a: [0, 0, 0] } }, dense(1)],
      }),
    ).toThrow(/dense/);
    expect(() =>
      clips.prepare({
        name: "bad_last",
        loop: true,
        root_motion: false,
        duration: 1,
        keys: [dense(0), { t: 1, sparse: true, rot: { a: [0, 0, 0] } }],
      }),
    ).toThrow(/dense/);
    expect(() =>
      clips.prepare({
        name: "bad_order",
        loop: true,
        root_motion: false,
        duration: 1,
        keys: [dense(0), dense(0.6), { t: 0.4, sparse: true, rot: { a: [10, 0, 0] } }, dense(1)],
      }),
    ).toThrow(/increasing/);
  });

  it("rejects a clip with fewer than two keys", () => {
    // The only shape that reaches this guard: with a nonzero duration a lone
    // key at t = 0 fails "last key must be at the duration" first.
    expect(() =>
      clips.prepare({
        name: "one_key",
        loop: true,
        root_motion: false,
        duration: 0,
        keys: [{ t: 0, rot: { a: [0, 0, 0] } }],
      }),
    ).toThrow(/two keys/);
  });
});
