// Tier-2 tests for the locomotion cues that decided the "giants in slow
// motion" complaint (#574): whether the planted foot holds a steady sweep, and
// whether the run bounces rather than vaults.
//
// WHY THIS IS A MEASUREMENT AND NOT AN INSPECTION. Perceived body scale is read
// from motion, and the dominant cue after cadence is what the feet do against
// the ground. That is invisible in the authored keyframes -- they are angles in
// one file, and the stride they have to satisfy is world units in another -- so
// everything here derives the toe's motion through the real pose path
// (`animator.basePose` -> `skeleton.apply` -> `skeleton.jointPosition`) rather
// than re-deriving it from the numbers the clips were authored with.
//
// Headless: that path is pure arithmetic, no GL and no window.

import { describe, expect, it } from "vitest";
import * as animator from "./animator.ts";
import * as poseTable from "./pose_table.ts";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";

const RIG = RIG_MEDIUM;

// Metres per world unit, derived the way `player_renderer_3d.metresPerWorldUnit`
// derives it rather than restated as a constant: the rig is authored in metres
// and drawn `PLAYER_RADIUS * HEIGHT_IN_RADII * 2` world units tall. Restated
// here with the same algebra (rig3d points only upward and must not import the
// renderer), so a change on either side surfaces as a failure rather than
// drifting silently.
const PLAYER_RADIUS = 12;
const HEIGHT_IN_RADII = 3;
const RIG_HEIGHT_M = 0.8 + 0.09 + 0.19 + 0.1 + 0.05 + 0.02 + 0.145 * (2.5 - 0.5 * 0.62);
const M_PER_WU = RIG_HEIGHT_M / (PLAYER_RADIUS * HEIGHT_IN_RADII * 2);

const SAMPLES = 480;

interface Track {
  /** Forward (+Z) toe position in body frame, metres, per sample. */
  readonly z: readonly number[];
  /** Toe height, metres, per sample. */
  readonly y: readonly number[];
  /** Hip height, metres, per sample. */
  readonly hipY: readonly number[];
  /** Backward toe sweep as a multiple of body speed, per sample. */
  readonly sweep: readonly number[];
}

// Tracks the right toe through one full gait cycle at `speed`.
//
// Locomotion clips are in-place (`root_motion: false`, per #101), so the toe's
// WORLD velocity is the body's speed minus the rate this in-body position
// sweeps backward: a foot sweeping backward at exactly 1.0x body speed is
// standing still on the ground, which is what "planted" means.
function track(speed: number): Track {
  const rig = skeleton.newRig(RIG);
  const z: number[] = [];
  const y: number[] = [];
  const hipY: number[] = [];
  for (let i = 0; i < SAMPLES; i += 1) {
    skeleton.apply(rig, animator.basePose({ speed, gait: i / SAMPLES }, 0));
    const toe = skeleton.jointPosition(rig, "toe.R");
    z.push(toe[2]);
    y.push(toe[1]);
    hipY.push(skeleton.jointPosition(rig, "hips")[1]);
  }
  const cycleSeconds = poseTable.strideFor(speed) / speed;
  const dt = cycleSeconds / SAMPLES;
  const speedM = speed * M_PER_WU;
  const sweep: number[] = [];
  for (let i = 0; i < SAMPLES; i += 1) {
    const next = (i + 1) % SAMPLES;
    sweep.push(-(((z[next] ?? 0) - (z[i] ?? 0)) / dt) / speedM);
  }
  return { z, y, hipY, sweep };
}

// ONE stance: the contiguous run of samples around this toe's lowest point for
// which it stays in the bottom quarter of its own vertical range.
//
// Contiguity is the part that matters. A plain threshold over the whole cycle
// picks up the far side of the swing as well -- a walk's toe is near the ground
// for most of the cycle -- and averaging a backward stance sweep together with
// a forward swing return gives a mean near zero and a meaningless ratio. This
// walks outward from the minimum instead, and wraps, so it is one contact.
//
// Defined off the toe's measured height rather than a named phase window, so
// retiming a clip cannot quietly point it at the airborne half.
function stance(t: Track): readonly number[] {
  const min = Math.min(...t.y);
  const max = Math.max(...t.y);
  const bar = min + 0.25 * (max - min);
  const lowest = t.y.indexOf(min);
  const below = (i: number): boolean => (t.y[(i + SAMPLES) % SAMPLES] ?? 0) <= bar;

  let first = lowest;
  while (below(first - 1) && lowest - first < SAMPLES - 1) {
    first -= 1;
  }
  let last = lowest;
  while (below(last + 1) && last - lowest < SAMPLES - 1) {
    last += 1;
  }
  const out: number[] = [];
  for (let i = first; i <= last; i += 1) {
    out.push((i + SAMPLES) % SAMPLES);
  }
  return out;
}

describe("rig3d locomotion: what the feet do against the ground", () => {
  it("sweeps the grounded foot at a STEADY rate instead of lurching between stop and whip", () => {
    // This is the cue #574's easing change actually bought, and it is worth
    // stating precisely because it is not the same claim as "the foot is
    // planted" (see the skate test below, which is still red-by-design).
    //
    // Under the old unconditional smoothstep every rotation channel eased at
    // both ends of every segment, so the toe's sweep rate swung from ~0 at each
    // key to ~1.5x its own mean in between -- the foot repeatedly stopped dead
    // and then whipped. A foot doing that cannot read as contact at any stride,
    // because contact is a CONSTANT rate. Linear easing on the locomotion
    // rotations makes the sweep steady; the residual variation is the honest
    // trigonometry of a leg swinging through an arc.
    //
    // Measured both ways before the bars below were chosen, so they separate
    // the two regimes rather than merely admitting today's numbers:
    //
    //                     spread/mean          sweep range
    //   old smoothstep    walk 1.51 run 1.57   0.06 .. 1.10
    //   per-channel ease  walk 0.39 run 0.56   0.37 .. 0.74
    //
    // The old range is the whole story: a foot going from 0.06x body speed to
    // 1.10x within one contact is stopping dead and then whipping, twice per
    // step, on every player on screen.
    for (const speed of [poseTable.WALK_SPEED, poseTable.RUN_SPEED]) {
      const t = track(speed);
      const window = stance(t);
      const rates = window.map((i) => t.sweep[i] ?? 0);
      const mean = rates.reduce((a, b) => a + b, 0) / rates.length;
      const spread = Math.max(...rates) - Math.min(...rates);
      expect(
        spread / Math.abs(mean),
        `sweep rate should be steady while grounded at speed ${String(speed)}`,
      ).toBeLessThan(0.8);
      expect(
        Math.min(...rates),
        `the grounded foot should never stop dead at speed ${String(speed)}`,
      ).toBeGreaterThan(0.25);
    }
  });

  it("bounces the run rather than vaulting it: hips lowest at mid-stance", () => {
    // A walk VAULTS (hips highest at mid-stance, rising over a straight support
    // leg); a run COMPRESSES (hips lowest at mid-stance, highest in flight).
    // Both were authored the walk's way round, which is why a sprint read as a
    // hurried walk: no impact bottom and no airborne top. Measured on the
    // composed result rather than asserted on the keyframe.
    const t = track(poseTable.RUN_SPEED);
    const window = stance(t);
    const midStance = window[Math.floor(window.length / 2)] ?? 0;
    const lowest = t.hipY.indexOf(Math.min(...t.hipY));
    const apart = Math.abs(midStance - lowest) / SAMPLES;
    expect(
      Math.min(apart, 1 - apart),
      "the run's lowest hip should coincide with mid-stance",
    ).toBeLessThan(0.12);
  });

  it("still vaults the walk, which is correct for a walk and must not be swept along", () => {
    const t = track(poseTable.WALK_SPEED);
    const window = stance(t);
    const midStance = window[Math.floor(window.length / 2)] ?? 0;
    const highest = t.hipY.indexOf(Math.max(...t.hipY));
    const apart = Math.abs(midStance - highest) / SAMPLES;
    expect(
      Math.min(apart, 1 - apart),
      "the walk's highest hip should coincide with mid-stance",
    ).toBeLessThan(0.12);
  });

  it("keeps cadence in the human band, so none of this is a speed-up in disguise", () => {
    // The premise of #574 is that the complaint was NOT that the clips play too
    // slowly, and cadence is the number that would betray a fix that cheated by
    // playing them faster. Pinned here so the next retune has to argue with it.
    const runHz = poseTable.RUN_SPEED / poseTable.strideFor(poseTable.RUN_SPEED);
    const walkHz = poseTable.WALK_SPEED / poseTable.strideFor(poseTable.WALK_SPEED);
    expect(runHz).toBeGreaterThan(1.3);
    expect(runHz).toBeLessThan(1.6);
    expect(walkHz).toBeGreaterThan(0.8);
    expect(walkHz).toBeLessThan(1.2);
  });

  it("records the foot-sweep deficit the clips cannot yet close", () => {
    // THIS TEST PINS A KNOWN GAP RATHER THAN A FIX, deliberately, because the
    // gap was invisible before it was measured and that is how it survived.
    //
    // For a foot to be plantable the clip must sweep it backward by
    // `duty x stride` during stance. The run's authored sweep is ~43 wu, which
    // is already the geometric maximum for this rig's 0.66 m leg at a +/-45
    // degree split -- the pose cannot reach further. At RUN_STRIDE 185 and a
    // run's ~0.27 duty the requirement is ~50 wu, so the clip is ~14% short
    // even before the 4-key cycle spreads that sweep across half the cycle
    // instead of concentrating it into a real stance window.
    //
    // Neither lever available here fixes it: widening the pose has no room, and
    // shortening the stride buys the ground back by raising cadence, which is
    // the exact fast-forward read the fix is required to avoid. The remaining
    // lever is a longer stance window, i.e. keyframes these cycles do not have.
    // Asserted as a RANGE so that closing the gap fails this test and forces
    // the comment to be rewritten, rather than passing quietly.
    const t = track(poseTable.RUN_SPEED);
    const reachWu = (Math.max(...t.z) - Math.min(...t.z)) / M_PER_WU;
    const legReachWu =
      (2 * (RIG.seg.upperleg + RIG.seg.lowerleg) * Math.sin(Math.PI / 4)) / M_PER_WU;
    expect(reachWu, "the run pose is at its own geometric ceiling").toBeGreaterThan(
      0.98 * legReachWu,
    );

    const needed = 0.27 * poseTable.RUN_STRIDE;
    expect(reachWu / needed, "known deficit: see this test's comment").toBeGreaterThan(0.8);
    expect(reachWu / needed, "known deficit: see this test's comment").toBeLessThan(0.95);
  });
});
