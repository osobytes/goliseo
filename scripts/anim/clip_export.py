# Export a posed clip from the goliseo_v2 authoring rig into game clip data.
#
# Reads the armature's action keyframes, and for each key emits every posed
# bone's rotation as a game-space quaternion + YXZ Euler degrees, relative to
# the v2 rest convention (identity rest rotations, character-aligned frames --
# the convention the skeleton.ts v2 port will adopt). Axis mapping
# (Blender authoring -> game): x -> x, z -> y (up), -y -> z (facing).
#
# Output: <out>.json with { name, fps, duration, keys: [ { t, rot: {bone:
# [x,y,z] deg YXZ}, move: {bone: [x,y,z]} } ] } -- the RawClip shape
# clips.ts's prepare() consumes, plus a "v2_quat" block for the future rig.
#
# Usage:
#   blender --background <rig_or_clip.blend> --python clip_export.py -- \
#       <clip_name> <out.json> [--selftest]
import bpy, json, math, sys

from mathutils import Matrix, Quaternion, Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
clip_name, out_path = argv[0], argv[1]
SELFTEST = "--selftest" in argv

# Blender->game basis change: columns are where blender X/Y/Z land in game space.
C = Matrix(((1, 0, 0), (0, 0, 1), (0, -1, 0)))
C4 = C.to_4x4()
CI = C4.inverted()


def to_game_vec(v):
    return C @ Vector(v)


def to_game_quat(m):
    """World-orientation matrix in blender space -> game-space quaternion."""
    return (C4 @ m.to_4x4() @ CI).to_quaternion()


def game_euler_yxz_deg(q):
    """Game quat -> [x, y, z] degrees applied as Y*X*Z (the quat.ts order).

    Blender order strings are extrinsic application order, so the game's
    RY*RX*RZ matrix product is Blender's "ZXY" (verified numerically)."""
    e = q.to_matrix().to_euler("ZXY")
    return [round(math.degrees(e.x), 3), round(math.degrees(e.y), 3), round(math.degrees(e.z), 3)]


arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
act = arm.animation_data.action if arm.animation_data else None
assert act, "authoring rig has no action to export"
scene = bpy.context.scene
fps = scene.render.fps or 30

# authored key times: the union of keyframe x-positions across the action
key_frames = sorted({
    round(kp.co[0])
    for layer in act.layers
    for strip in layer.strips
    for bag in strip.channelbags
    for fc in bag.fcurves
    for kp in fc.keyframe_points
} if not hasattr(act, "fcurves") else {
    round(kp.co[0]) for fc in act.fcurves for kp in fc.keyframe_points
})
assert key_frames, "action has no keyframes"
f0 = key_frames[0]

# rest orientations/positions and all pose math in ARMATURE space: the object
# transform carries only the animated root bob (exported as move.root below),
# and mixing the evaluated object matrix into per-bone math is what let stale
# pose-bone location residue slip through unnoticed
assert arm.matrix_world.to_quaternion().angle < 1e-6, "armature object must be unrotated"
rest_world = {b.name: b.matrix_local.copy() for b in arm.data.bones}
parent_of = {b.name: (b.parent.name if b.parent else None) for b in arm.data.bones}

keys = []
for f in key_frames:
    scene.frame_set(f)
    rot, rot_q, move = {}, {}, {}
    root_bob = Vector(arm.location)
    if root_bob.length > 1e-4:
        move["root"] = [round(c, 5) for c in to_game_vec(root_bob)]
    for pb in arm.pose.bones:
        posed = pb.matrix
        rest = rest_world[pb.name]
        par = parent_of[pb.name]
        if par:
            # bone orientation relative to its posed parent, measured against
            # the same relation at rest -- character-aligned identity-rest frames
            rel_posed = arm.pose.bones[par].matrix.inverted() @ posed
            rel_rest = rest_world[par].inverted() @ rest
            delta = rel_posed @ rel_rest.inverted()
            # rigidity guard: a rotations-only export is meaningless if pose
            # translations moved the head (the pb.matrix-setter residue bug)
            pred = arm.pose.bones[par].matrix @ rest_world[par].inverted() @ rest.translation
            drift = (pb.head - pred).length
            assert drift < 1e-4, f"{pb.name} head drifted {drift:.4f} m off the rigid chain at f{f}"
        else:
            delta = posed @ rest.inverted()
        q = to_game_quat(delta.to_3x3())
        if q.angle > 1e-4:
            rot_q[pb.name] = [round(c, 6) for c in (q.w, q.x, q.y, q.z)]
            rot[pb.name] = game_euler_yxz_deg(q)
    keys.append({"t": round((f - f0) / fps, 4), "rot": rot, "move": move, "quat": rot_q})

clip = {
    "name": clip_name,
    "fps": fps,
    "duration": keys[-1]["t"],
    "keys": [{"t": k["t"], "rot": k["rot"], "move": k["move"]} for k in keys],
    "v2_quat": [{"t": k["t"], "rot": k["quat"]} for k in keys],
}
with open(out_path, "w") as fh:
    json.dump(clip, fh, indent=1)

if SELFTEST:
    # Round-trip proof: re-derive every bone's head position purely from the
    # exported game-space data (quats + move.root) + rest geometry, and compare
    # with Blender's own posed positions (axis-converted). Agreement proves the
    # axis mapping, the relative-rotation math, and the root-bob channel
    # together.
    worst = 0.0
    for ki, f in enumerate(key_frames):
        scene.frame_set(f)
        bob = clip["keys"][ki]["move"].get("root", [0, 0, 0])
        bob = Vector(bob)
        game_world = {}
        order = [b.name for b in arm.data.bones]
        for name in order:
            par = parent_of[name]
            q = clip["v2_quat"][ki]["rot"].get(name)
            dq = Quaternion((q[0], q[1], q[2], q[3])) if q else Quaternion()
            rest_h = to_game_vec(rest_world[name].translation)
            rest_r = (C4 @ rest_world[name].to_4x4() @ CI).to_quaternion()
            if par:
                pq, ph, prh = game_world[par]
                prest_r = (C4 @ rest_world[par].to_4x4() @ CI).to_quaternion()
                rel_rest_q = prest_r.inverted() @ rest_r
                off = prest_r.inverted() @ (rest_h - prh)
                w_q = pq @ dq @ rel_rest_q
                w_h = ph + (pq @ off)
            else:
                w_q = dq @ rest_r
                w_h = rest_h + bob
            game_world[name] = (w_q, w_h, rest_h)
        for pb in arm.pose.bones:
            expect = to_game_vec(Vector(pb.head) + Vector(arm.location))
            got = game_world[pb.name][1]
            worst = max(worst, (expect - got).length)
    print(f"SELFTEST worst head error: {worst:.6f} m")
    assert worst < 1e-3, "round-trip mismatch"

print("EXPORT_OK", clip_name, len(keys), "keys")
