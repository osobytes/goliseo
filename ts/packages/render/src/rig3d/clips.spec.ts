import { describe, expect, it } from "vitest";
import { quat } from "@gc/core";
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
