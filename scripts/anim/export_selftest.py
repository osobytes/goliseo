# Pose a few bones with known rotations on the v2 rig and save a 2-key test
# action, for clip_export.py's --selftest round-trip.
# Usage: blender --background <rig.blend> --python export_selftest.py -- <out.blend>
import bpy, math, sys

from mathutils import Quaternion

argv = sys.argv[sys.argv.index("--") + 1 :]
out = argv[0]

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")

POSES = {
    1: {},  # rest key
    15: {   # a kick-ish shape: hip back, knee folded, torso counter-rotated
        "thigh.R": Quaternion((1, 0, 0), math.radians(-55)),
        "shin.R": Quaternion((1, 0, 0), math.radians(80)),
        "spine": Quaternion((0, 0, 1), math.radians(18)),
        "upper_arm.L": Quaternion((0, 1, 0), math.radians(30)),
    },
}
for frame, poses in POSES.items():
    for pb in arm.pose.bones:
        pb.rotation_mode = "QUATERNION"
        pb.rotation_quaternion = poses.get(pb.name, Quaternion())
    for name in {n for p in POSES.values() for n in p} | {"thigh.R"}:
        arm.pose.bones[name].keyframe_insert("rotation_quaternion", frame=frame)

bpy.ops.object.mode_set(mode="OBJECT")
bpy.ops.wm.save_as_mainfile(filepath=out)
print("SELFTEST_RIG_OK")
