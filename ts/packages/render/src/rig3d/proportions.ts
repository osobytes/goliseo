// The single conformant rig.
//
// An earlier pass in this prototype explored three proportion sets (a 7-head
// legionary, a 5-head hero and a 2.6-head mascot). That exploration is retired:
// docs/design/prototype_theme_roster.md puts "unrestricted character
// proportions or a second production rig" explicitly OUT OF SCOPE, and requires
// that "All six use one validated Rig_Medium skeleton contract".
//
// So there is one rig here, and the three themes differ only in shape language,
// material and palette -- which is exactly the claim the roster makes and this
// prototype exists to test.
//
// `seg` values are bone lengths in metres and drive the skeleton.
// `form` values drive the body shapes.
// `gear.over` is how far armour stands proud of the limb it wraps: the single
// number that decides whether kit reads as *equipped* or as body paint. Below
// about 1.10 the shell intersects the limb and the eye reads one blob.

/** Bone lengths in metres, driving the skeleton. */
export interface RigSegments {
  readonly hips_y: number;
  readonly spine: number;
  readonly chest: number;
  readonly neck: number;
  readonly head: number;
  readonly shoulder_x: number; // clavicle position on the chest
  readonly shoulder_y: number;
  readonly arm_x: number; // upper arm offset from the clavicle
  readonly upperarm: number;
  readonly lowerarm: number;
  readonly thigh_x: number;
  readonly thigh_y: number;
  readonly upperleg: number;
  readonly lowerleg: number;
}

/** Body-shape parameters. */
export interface RigForm {
  readonly segments: number;
  readonly head_r: number;
  readonly head_round: number;
  readonly eye_y: number;
  readonly neck_r: number;
  readonly torso_r: number;
  readonly torso_bulge: number;
  readonly arm_r: number;
  readonly arm_taper: number;
  readonly leg_r: number;
  readonly leg_taper: number;
  readonly hand_r: number;
  readonly foot_len: number;
  readonly foot_w: number;
  readonly cap: number;
}

/** How far armour stands proud of the limb it wraps. */
export interface RigGear {
  readonly over: number;
}

/** A full set of proportions describing one skeleton/body contract. */
export interface RigProportions {
  readonly key: string;
  readonly label: string;
  readonly seg: RigSegments;
  readonly form: RigForm;
  readonly gear: RigGear;
  readonly motion_scale: number;
}

// A stylised medium humanoid: broad enough to carry plate, readable at match
// camera scale, roughly five heads tall.
export const RIG_MEDIUM: RigProportions = {
  key: "rig_medium",
  label: "Rig_Medium",
  seg: {
    hips_y: 0.8,
    spine: 0.09,
    chest: 0.19,
    neck: 0.1,
    head: 0.05,
    shoulder_x: 0.085,
    shoulder_y: 0.15,
    arm_x: 0.155,
    upperarm: 0.24,
    lowerarm: 0.21,
    thigh_x: 0.105,
    thigh_y: -0.04,
    upperleg: 0.36,
    lowerleg: 0.3,
  },
  form: {
    segments: 12,
    head_r: 0.145,
    head_round: 0.62,
    eye_y: 0.52,
    neck_r: 0.062,
    torso_r: 0.185,
    torso_bulge: 1.14,
    arm_r: 0.074,
    arm_taper: 0.86,
    leg_r: 0.098,
    leg_taper: 0.8,
    hand_r: 0.058,
    foot_len: 0.24,
    foot_w: 0.12,
    cap: 0.55,
  },
  gear: { over: 1.24 },
  motion_scale: 0.88,
};

// Approximate standing height in metres: the bone chain up to the head plus the
// skull the head builder puts on top of it. Used to frame the camera.
export function height(rig: RigProportions): number {
  const s = rig.seg;
  const f = rig.form;
  return s.hips_y + s.spine + s.chest + s.neck + s.head + 0.02 + f.head_r * (2.5 - 0.5 * f.head_round);
}
