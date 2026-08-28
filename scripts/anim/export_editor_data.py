# Export pose-editor data from a clip blend: the skinned character as GLB and
# the clip's key poses as per-bone LOCAL quaternions.
# Usage: blender --background <clip.blend> --python export_editor_data.py -- <out_dir> <clip_name>
import bpy, json, os, sys

argv = sys.argv[sys.argv.index("--") + 1 :]
out_dir, clip_name = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
mesh = max((o for o in bpy.data.objects if o.type == "MESH"), key=lambda o: len(o.data.vertices))

# clean scene clutter so the GLB holds exactly armature + character
for o in list(bpy.data.objects):
    if o not in (arm, mesh):
        bpy.data.objects.remove(o, do_unlink=True)

act = arm.animation_data.action if arm.animation_data else None
key_frames = sorted({
    round(kp.co[0])
    for layer in act.layers for strip in layer.strips for bag in strip.channelbags
    for fc in bag.fcurves for kp in fc.keyframe_points
}) if act and not hasattr(act, "fcurves") else sorted({
    round(kp.co[0]) for fc in act.fcurves for kp in fc.keyframe_points
} if act else set())

BONES = [b.name for b in arm.data.bones]
keys = []
for f in key_frames:
    bpy.context.scene.frame_set(f)
    bones = {}
    for pb in arm.pose.bones:
        q = pb.rotation_quaternion
        if abs(q.w - 1.0) > 1e-6 or abs(q.x) > 1e-6 or abs(q.y) > 1e-6 or abs(q.z) > 1e-6:
            bones[pb.name] = [round(c, 5) for c in (q.w, q.x, q.y, q.z)]
    keys.append({"frame": f, "bones": bones,
                 "root_z": round(arm.location.z, 4)})

with open(os.path.join(out_dir, f"{clip_name}.clip.json"), "w") as fh:
    json.dump({"name": clip_name, "fps": bpy.context.scene.render.fps,
               "frame_start": key_frames[0] if key_frames else 1,
               "frame_end": key_frames[-1] if key_frames else 1,
               "bone_order": BONES, "keys": keys}, fh, indent=1)

# rest pose for the GLB
bpy.context.scene.frame_set(0)
for pb in arm.pose.bones:
    pb.rotation_quaternion = (1, 0, 0, 0)
arm.location = (0, 0, 0)
arm.animation_data_clear()
bpy.ops.object.select_all(action="DESELECT")
arm.select_set(True)
mesh.select_set(True)
bpy.ops.export_scene.gltf(
    filepath=os.path.join(out_dir, "character.glb"),
    use_selection=True, export_animations=False, export_skins=True,
    export_yup=True,
)
print("EDITOR_DATA_OK", len(keys), "keys", len(BONES), "bones")
