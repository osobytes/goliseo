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
    // planted" (see the skate test below, which #575 turned from a pinned
    // deficit into an acceptance).
    //
    // Under the old unconditional smoothstep every rotation channel eased at
    // both ends of every segment, so the toe's sweep rate swung from ~0 at each
    // key to ~1.5x its own mean in between -- the foot repeatedly stopped dead
    // and then whipped. A foot doing that cannot read as contact at any stride,
    // because contact is a CONSTANT rate. Linear easing on the locomotion
    // rotations makes the sweep steady; the residual variation is the honest
    // trigonometry of a leg swinging through an arc.
    //
    // Measured across all three regimes before the bars below were chosen, so
    // they separate them rather than merely admitting today's numbers:
    //
    //                          spread/mean          sweep range
    //   old smoothstep         walk 1.51 run 1.57   0.06 .. 1.10
    //   per-channel ease #574  walk 0.39 run 0.56   0.37 .. 0.74
    //   duty-factor keys #575  walk 0.10 run 0.18   0.67 .. 0.97
    //
    // The old range is the whole story: a foot going from 0.06x body speed to
    // 1.10x within one contact is stopping dead and then whipping, twice per
    // step, on every player on screen. #575's stance keys are authored so the
    // grounded toe z falls LINEARLY across the stance window, which is why the
    // spread tightened again: the sweep is now constant by construction, not
    // merely un-eased.
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
    //
    // COMPARED MODULO HALF A CYCLE: the clip is authored once and mirrored, so
    // the hips bottom out at BOTH mid-stances -- two equal minima half a cycle
    // apart -- and which one `indexOf` lands on is decided by floating-point
    // noise in the baked tracks, not by the animation. Folding by the mirror
    // period asserts the claim that matters (the bottom is a mid-stance, not a
    // contact or an apex) without betting on that noise.
    const t = track(poseTable.RUN_SPEED);
    const window = stance(t);
    const midStance = window[Math.floor(window.length / 2)] ?? 0;
    const lowest = t.hipY.indexOf(Math.min(...t.hipY));
    const half = SAMPLES / 2;
    const apart = Math.abs(midStance - lowest) % half;
    expect(
      Math.min(apart, half - apart) / SAMPLES,
      "the run's lowest hip should coincide with a mid-stance",
    ).toBeLessThan(0.12);
  });

  it("still vaults the walk, which is correct for a walk and must not be swept along", () => {
    // Folded modulo half a cycle for the same reason as the run's bounce test
    // above: the vault apexes twice, once per mirrored mid-stance.
    const t = track(poseTable.WALK_SPEED);
    const window = stance(t);
    const midStance = window[Math.floor(window.length / 2)] ?? 0;
    const highest = t.hipY.indexOf(Math.max(...t.hipY));
    const half = SAMPLES / 2;
    const apart = Math.abs(midStance - highest) % half;
    expect(
      Math.min(apart, half - apart) / SAMPLES,
      "the walk's highest hip should coincide with a mid-stance",
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

  it("plants the run's stance foot: skate under 20% of body speed (#575)", () => {
    // THE TEST THAT USED TO PIN THE DEFICIT, rewritten because the deficit
    // closed -- which is exactly the rewrite its old comment demanded.
    //
    // For a foot to be plantable the clip must sweep it backward at body speed
    // while it is grounded. The old 4-key cycle spread the run's sweep across
    // half the cycle, so the grounded foot moved at ~0.63x body speed -- a 37%
    // skate on every runner. #575 concentrated the sweep into a real stance
    // window (contact at t = 0, mid-stance at 0.075, toe-off at 0.15 of the
    // 0.6 s cycle: duty 0.25), authored so the grounded toe z falls linearly
    // across it. The mean grounded sweep is now ~0.91x body speed: ~9% skate,
    // within the ~20% acceptance, at the SAME stride and the SAME cadence.
    //
    // The pose still cannot reach past the rig's own legs -- pinned below so a
    // retune that quietly shrinks the reach (and pays for it with a higher
    // duty) still has to answer to this test.
    const t = track(poseTable.RUN_SPEED);
    const window = stance(t);
    const rates = window.map((i) => t.sweep[i] ?? 0);
    const mean = rates.reduce((a, b) => a + b, 0) / rates.length;
    expect(mean, "run stance-foot skate must stay under ~20% of body speed").toBeGreaterThan(0.8);
    expect(mean, "and the foot must not overtake the ground it stands on").toBeLessThan(1.1);

    const reachWu = (Math.max(...t.z) - Math.min(...t.z)) / M_PER_WU;
    const legReachWu =
      (2 * (RIG.seg.upperleg + RIG.seg.lowerleg) * Math.sin(Math.PI / 4)) / M_PER_WU;
    expect(reachWu, "the run pose is at its own geometric ceiling").toBeGreaterThan(
      0.98 * legReachWu,
    );
  });

  it("records the walk's remaining foot-sweep deficit, which CANNOT close on this rig", () => {
    // THIS TEST PINS A KNOWN GAP RATHER THAN A FIX, deliberately, because the
    // gap was invisible before it was measured and that is how it survived.
    //
    // A walk's duty is ~0.55 -- both feet are down around each contact -- so at
    // WALK_STRIDE 100 the stance must cover ~55 wu, against the ~43 wu
    // geometric ceiling of this rig's 0.66 m leg at a +/-45 degree split. The
    // run closes because its duty is low; the walk cannot: even a stance pose
    // at the ceiling leaves a ~22% skate, and the walk's poses stop short of
    // the ceiling because a walking figure doing full splits is a worse read
    // than a mild skate. #575 widened the reach (~30 -> ~38 wu) and keyed a
    // real double-support window (toe-off at 1/30 s after the opposite
    // contact), which brought the skate from ~34% to ~29%; the rest is the
    // rig's legs (42% of body height where a human's are ~50%), and
    // lengthening them is a roster decision, not a render fix -- see
    // `docs/design/prototype_theme_roster.md`.
    //
    // Asserted as a RANGE so that closing the gap -- longer legs, a shorter
    // walk stride, or a duty retune -- fails this test and forces this comment
    // to be rewritten, rather than passing quietly.
    const t = track(poseTable.WALK_SPEED);
    const window = stance(t);
    const rates = window.map((i) => t.sweep[i] ?? 0);
    const mean = rates.reduce((a, b) => a + b, 0) / rates.length;
    expect(mean, "known walk deficit: see this test's comment").toBeGreaterThan(0.6);
    expect(mean, "known walk deficit: see this test's comment").toBeLessThan(0.8);
  });
});
