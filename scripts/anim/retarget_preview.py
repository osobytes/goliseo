# LOCAL DIAGNOSTIC ONLY: play an actual reference-pack clip on the bound
# character, to separate rig/skinning problems from authored-pose problems.
# Output goes to the session scratchpad, never into the repo or the game --
# reference-pack motion data must not ship (#424 rule; owner decision).
#
# Method: for every frame, each source (Biped) bone's parent-relative rotation
# DELTA vs its rest is applied to the mapped target bone on top of the
# target's own rest relation; the pelvis world translation delta transfers
# scaled by body height. Same world-relative-delta math as clip_export.py,
# run in reverse.
#
# Usage: blender --background <meshy_bound.blend> --python retarget_preview.py \
#            -- <pack_clip.fbx> <out_dir>
import bpy, os, sys

from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
src_fbx, out_dir = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)

ours = next(o for o in bpy.data.objects if o.type == "ARMATURE")
before = set(bpy.data.objects)
bpy.ops.import_scene.fbx(filepath=src_fbx)
theirs = next(o for o in (set(bpy.data.objects) - before) if o.type == "ARMATURE")

BONE_MAP = {
    "Bip001Pelvis": "hips",
    "Bip001Spine": "spine",
    "Bip001Spine1": "spine2",
    "Bip001Spine2": "chest",
    "Bip001Neck": "neck",
    "Bip001Head": "head",
}
for s, side in (("L", "L"), ("R", "R")):
    BONE_MAP[f"Bip001{s}Clavicle"] = f"shoulder.{side}"
    BONE_MAP[f"Bip001{s}UpperArm"] = f"upper_arm.{side}"
    BONE_MAP[f"Bip001{s}Forearm"] = f"forearm.{side}"
    BONE_MAP[f"Bip001{s}Hand"] = f"hand.{side}"
    BONE_MAP[f"Bip001{s}Thigh"] = f"thigh.{side}"
    BONE_MAP[f"Bip001{s}Calf"] = f"shin.{side}"
    BONE_MAP[f"Bip001{s}Foot"] = f"foot.{side}"
    BONE_MAP[f"Bip001{s}Toe0"] = f"toe.{side}"
missing = [a for a in BONE_MAP if a not in theirs.data.bones]
print("UNMAPPED_SOURCE_BONES", missing)
inv_map = {v: k for k, v in BONE_MAP.items() if k in theirs.data.bones}

act = theirs.animation_data.action
f0, f1 = int(act.frame_range[0]), int(act.frame_range[1])
scene = bpy.context.scene
scene.frame_start, scene.frame_end = f0, f1

# rest data
A_rest = {b.name: theirs.matrix_world @ b.matrix_local for b in theirs.data.bones}
A_parent = {b.name: (b.parent.name if b.parent else None) for b in theirs.data.bones}
B_rest = {b.name: ours.matrix_world @ b.matrix_local for b in ours.data.bones}
B_parent = {b.name: (b.parent.name if b.parent else None) for b in ours.data.bones}
B_order = [b.name for b in ours.data.bones]  # parent-before-child

pelvis_a = next(n for n in theirs.data.bones.keys() if "Pelvis" in n)
height_scale = (B_rest["head_tip"].translation.z - B_rest["toe.L"].translation.z) / max(
    A_rest[pelvis_a].translation.z * 2.0, 1e-6
)
# pelvis height ratio is a decent proxy for body scale
height_scale = B_rest["hips"].translation.z / max(A_rest[pelvis_a].translation.z, 1e-6)

for pb in ours.pose.bones:
    pb.rotation_mode = "QUATERNION"

for f in range(f0, f1 + 1):
    scene.frame_set(f)
    A_posed = {pb.name: theirs.matrix_world @ pb.matrix for pb in theirs.pose.bones}

    # world-space rotation transfer: each mapped bone's WORLD orientation
    # change equals the source bone's; unmapped bones follow their parent's
    # world delta. Avoids the chain-accumulation error of parent-local deltas
    # between rigs with different rest poses.
    R = {}
    T = {}
    for name in B_order:
        par = B_parent[name]
        src = inv_map.get(name)
        if src:
            d = (A_posed[src].to_3x3() @ A_rest[src].to_3x3().inverted())
            R[name] = d @ B_rest[name].to_3x3()
        elif par:
            d = R[par] @ B_rest[par].to_3x3().inverted()
            R[name] = d @ B_rest[name].to_3x3()
        else:
            R[name] = B_rest[name].to_3x3()
        if par is None:
            T[name] = B_rest[name].translation.copy()
        elif name == "hips":
            t_delta = A_posed[pelvis_a].translation - A_rest[pelvis_a].translation
            T[name] = B_rest["hips"].translation + t_delta * height_scale
        else:
            off = B_rest[name].translation - B_rest[par].translation
            T[name] = T[par] + (R[par] @ B_rest[par].to_3x3().inverted()) @ off
    inv_world = ours.matrix_world.inverted()
    for name in B_order:
        M = R[name].to_4x4()
        M.translation = T[name]
        pb = ours.pose.bones[name]
        pb.matrix = inv_world @ M
        bpy.context.view_layer.update()
    for name in B_order:
        pb = ours.pose.bones[name]
        pb.keyframe_insert("rotation_quaternion", frame=f)
        pb.keyframe_insert("location", frame=f)

# remove the source rig and any of its imported children
for o in list(set(bpy.data.objects) - before):
    bpy.data.objects.remove(o, do_unlink=True)

# render
mesh = max((o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("floor")),
           key=lambda o: len(o.data.vertices))
lo = Vector((1e9,) * 3)
hi = Vector((-1e9,) * 3)
for c in mesh.bound_box:
    w = mesh.matrix_world @ Vector(c)
    lo = Vector(map(min, lo, w))
    hi = Vector(map(max, hi, w))
for o in [o for o in bpy.data.objects if o.name.startswith(("floor", "cam"))]:
    bpy.data.objects.remove(o, do_unlink=True)
floor = bpy.data.meshes.new("floor")
floor.from_pydata([(-60, -60, lo.z), (60, -60, lo.z), (60, 60, lo.z), (-60, 60, lo.z)], [], [(0, 1, 2, 3)])
fo = bpy.data.objects.new("floor", floor)
scene.collection.objects.link(fo)
cd = bpy.data.cameras.new("cam")
cd.lens = 45
cam = bpy.data.objects.new("cam", cd)
scene.collection.objects.link(cam)
scene.camera = cam
h = hi.z - lo.z
# clips travel: follow the pelvis like the pack previews
anchor = bpy.data.objects.new("anchor", None)
scene.collection.objects.link(anchor)
con = anchor.constraints.new("COPY_LOCATION")
con.target = ours
con.subtarget = "hips"
cam.parent = anchor
cam.location = (2.6, -3.2, 0.6)
track = cam.constraints.new("TRACK_TO")
track.target = anchor
track.track_axis = "TRACK_NEGATIVE_Z"
track.up_axis = "UP_Y"
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light = "STUDIO"
sh.color_type = "TEXTURE"
sh.show_shadows = True
scene.render.resolution_x, scene.render.resolution_y = 640, 480
scene.render.image_settings.file_format = "PNG"
scene.render.filepath = os.path.join(out_dir, "rt_")
bpy.ops.render.render(animation=True)
print("RETARGET_OK", f0, f1)
