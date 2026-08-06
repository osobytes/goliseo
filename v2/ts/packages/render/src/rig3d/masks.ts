// Bone masks: which part of the body a layer is allowed to drive.
//
// GOLISEO is a fast soccer game, so a player striking, guarding, aiming or
// reaching while still running is the NORMAL case, not an edge case. Authoring
// a full-body clip per (locomotion x action) pair is combinatorial: 12
// locomotion states x 10 actions is 120 clips that all have to be re-authored
// when the run cycle changes.
//
// The alternative is one clip per locomotion state and one clip per action,
// composed at runtime with a mask deciding which bones each layer owns. 12 + 10
// instead of 120, and the run cycle stays a single source of truth.
//
// A mask is just a set of bone names, so it costs nothing to define -- but it
// does mean bone naming is an interface, not an internal detail.
//
// IMPORTANT: sockets must be inside any mask that includes the hand they hang
// off. A socket left out of the mask keeps the base layer's transform while the
// arm follows the overlay, and the weapon visibly detaches from the fist.

function set(list: readonly string[]): Set<string> {
  return new Set(list);
}

function sided(prefixes: readonly string[]): string[] {
  const out: string[] = [];
  for (const prefix of prefixes) {
    out.push(`${prefix}.L`, `${prefix}.R`);
  }
  return out;
}

function merge(...sets: readonly ReadonlySet<string>[]): Set<string> {
  const out = new Set<string>();
  for (const s of sets) {
    for (const name of s) {
      out.add(name);
    }
  }
  return out;
}

// Everything from the spine up. `hips` is deliberately NOT here: it is the
// root of both halves and belongs to whatever drives locomotion, or the
// character's stride starts fighting the action.
export const UPPER_BODY: ReadonlySet<string> = merge(
  set(["spine", "chest", "neck", "head", "socket_ball"]),
  set(sided(["shoulder", "upper_arm", "forearm", "hand", "socket_hand"])),
  set(["socket_shield.L"]),
);

// Arms only: an action that must not disturb the torso's stride lean.
export const ARMS: ReadonlySet<string> = merge(
  set(sided(["shoulder", "upper_arm", "forearm", "hand", "socket_hand"])),
  set(["socket_shield.L"]),
);

// The sword arm alone -- a strike that leaves the shield arm holding guard.
export const ARM_R: ReadonlySet<string> = set([
  "shoulder.R",
  "upper_arm.R",
  "forearm.R",
  "hand.R",
  "socket_hand.R",
]);

export const LOWER_BODY: ReadonlySet<string> = merge(set(["hips"]), set(sided(["thigh", "shin", "foot", "toe"])));

// Single-leg masks, and the reason the naive upper/lower split is not enough
// for this game: a PASS or a SHOT is a lower-body action performed while
// running. It cannot be an upper-body overlay, and making it a full-body clip
// would throw away the stride. Masking one leg plus the spine lets a player
// strike the ball mid-run while the other leg keeps planting.
export const KICK_R: ReadonlySet<string> = merge(
  set(["spine", "hips"]),
  set(["thigh.R", "shin.R", "foot.R", "toe.R"]),
);
export const KICK_L: ReadonlySet<string> = merge(
  set(["spine", "hips"]),
  set(["thigh.L", "shin.L", "foot.L", "toe.L"]),
);

export const FULL_BODY: ReadonlySet<string> = merge(UPPER_BODY, LOWER_BODY, set(["root"]));
