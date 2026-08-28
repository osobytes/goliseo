# Apply a pose-editor .clip.json back onto the bound rig and save the clip
# blend — the bridge from the visual editor to the game pipeline:
#   editor save -> apply_editor_clip -> clip_export -> clips.ts
# Usage: blender --background <meshy_bound.blend> --python apply_editor_clip.py -- \
#            <clip.clip.json> <out.blend>
import bpy, json, sys

argv = sys.argv[sys.argv.index("--") + 1 :]
clip_path, out_blend = argv[0], argv[1]
with open(clip_path) as fh:
    clip = json.load(fh)

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
if arm.animation_data:
    arm.animation_data_clear()
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

keyed = set()
for k in clip["keys"]:
    keyed |= set(k["bones"].keys())

for k in clip["keys"]:
    f = k["frame"]
    for pb in arm.pose.bones:
        if pb.name not in keyed:
            continue
        q = k["bones"].get(pb.name, [1, 0, 0, 0])
        pb.rotation_quaternion = (q[0], q[1], q[2], q[3])
        pb.keyframe_insert("rotation_quaternion", frame=f)
    arm.location = (0.0, 0.0, k.get("root_z", 0.0))
    arm.keyframe_insert("location", frame=f)

scene = bpy.context.scene
scene.frame_start = clip["frame_start"]
scene.frame_end = clip["frame_end"]
scene.render.fps = clip.get("fps", 30)
bpy.ops.wm.save_as_mainfile(filepath=out_blend)
print("APPLY_OK", clip["name"], len(clip["keys"]), "keys")
