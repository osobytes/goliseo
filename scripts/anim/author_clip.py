# Author a clip on the bound character from a JSON key-pose definition, save
# the actioned .blend, and render per-key stills + a contact sheet strip.
#
# Clip JSON: { "name", "fps", "keys": [ { "frame": N,
#   "pose": { "<bone>": [x_deg, y_deg, z_deg] } } ] }
# Rotations are bone-LOCAL XYZ euler degrees on the canonical-roll rig
# (local X = side/pitch, Y = along bone/twist, Z = forward/lean).
#
# Usage: blender --background <bound.blend> --python author_clip.py -- \
#            <clip.json> <out_dir>
import bpy, json, math, os, sys

from mathutils import Euler, Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
clip_path, out_dir = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)
with open(clip_path) as fh:
    clip = json.load(fh)

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
scene = bpy.context.scene
scene.render.fps = clip.get("fps", 30)

# --- directional posing -----------------------------------------------------
# A key's "aim" block poses bones by WORLD direction instead of bone-local
# eulers: {"bone": {"d": [x,y,z], "twist": deg}} rotates the bone so its rest
# direction points along d (shortest arc, world space), then twists about d.
# Immune to per-bone local-frame differences -- the fix for swings landing on
# the wrong axes. World frame: -Y is the character's front, +Z is up.
from mathutils import Quaternion as Q, Vector as V

rest_dir = {}
rest_world_rot = {}
for b in arm.data.bones:
    m = arm.matrix_world @ b.matrix_local
    rest_world_rot[b.name] = m.to_3x3()
    rest_dir[b.name] = (m.to_3x3() @ V((0, 1, 0))).normalized()

def desired_world_rot(name, d, twist_deg):
    """World orientation that points the bone's rest direction along d."""
    target = V(d).normalized()
    swing = rest_dir[name].rotation_difference(target)
    tw = Q(target, math.radians(twist_deg))
    return (tw @ swing).to_matrix() @ rest_world_rot[name]


# Aims are applied ANALYTICALLY: accumulate each bone's world rotation from
# quaternions and set rotation_quaternion directly. Assigning pb.matrix (the
# previous approach) decomposes against a stale parent state and silently
# bakes centimetre-scale junk into pb.location — offsets Blender renders with
# but a rotations-only game export loses (the striker limb-offset bug).
assert arm.matrix_world.to_quaternion().angle < 1e-6, "armature object must be unrotated"
rest_rel_q = {}
for b in arm.data.bones:
    if b.parent:
        rest_rel_q[b.name] = (b.parent.matrix_local.inverted() @ b.matrix_local).to_quaternion()
    else:
        rest_rel_q[b.name] = b.matrix_local.to_quaternion()

# wipe any prior action, key the new one
if arm.animation_data:
    arm.animation_data_clear()
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
posed_bones = {n for k in clip["keys"] for n in k.get("pose", {})}
aimed_bones = {n for k in clip["keys"] for n in k.get("aim", {})}
order = [b.name for b in arm.data.bones]  # parent-before-child
# quaternion mode: Blender interpolates quaternion channels near-slerp, which
# avoids the long-arc sweeps euler-channel interpolation produces between
# far-apart poses (the palm-out forearm arc in the settle)
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"
    pb.location = (0.0, 0.0, 0.0)
    pb.scale = (1.0, 1.0, 1.0)
    pb.rotation_quaternion = (1.0, 0.0, 0.0, 0.0)
for key in clip["keys"]:
    f = key["frame"]
    pose = key.get("pose", {})
    aims = key.get("aim", {})
    # reset the FULL basis of every authored bone, apply local eulers first
    for pb in arm.pose.bones:
        if pb.name in posed_bones or pb.name in aimed_bones:
            pb.location = (0.0, 0.0, 0.0)
            pb.scale = (1.0, 1.0, 1.0)
            deg = pose.get(pb.name, [0, 0, 0])
            pb.rotation_quaternion = Euler(
                [math.radians(a) for a in deg], "XYZ"
            ).to_quaternion()
    arm.location = (0.0, 0.0, key.get("root_z", 0.0))
    # aims: walk parent-before-child accumulating world rotations, so children
    # account for already-posed parents without any depsgraph round-trip
    world = {}
    for name in order:
        pb = arm.pose.bones[name]
        par = pb.parent.name if pb.parent else None
        pre = (world[par] if par else Q()) @ rest_rel_q[name]
        if name in aims:
            spec_a = aims[name]
            d = spec_a["d"] if isinstance(spec_a, dict) else spec_a
            twist = spec_a.get("twist", 0.0) if isinstance(spec_a, dict) else 0.0
            r_w = desired_world_rot(name, d, twist).to_quaternion()
            pb.rotation_quaternion = pre.inverted() @ r_w
        world[name] = pre @ pb.rotation_quaternion
    for pb in arm.pose.bones:
        if pb.name in posed_bones or pb.name in aimed_bones:
            pb.keyframe_insert("rotation_quaternion", frame=f)
    arm.keyframe_insert("location", frame=f)
bpy.ops.object.mode_set(mode="OBJECT")
f0 = clip["keys"][0]["frame"]
f1 = clip["keys"][-1]["frame"]
scene.frame_start, scene.frame_end = f0, f1
bpy.ops.wm.save_as_mainfile(filepath=os.path.join(out_dir, f"{clip['name']}.blend"))

# --- render: one still per key, side view (kick reads in profile) ---
# the loaded blend may carry a floor/cam from the bind session — drop them
for o in [o for o in bpy.data.objects if o.name.startswith(("floor", "cam", "dbg_", "seg_"))]:
    bpy.data.objects.remove(o, do_unlink=True)
mesh = max((o for o in bpy.data.objects if o.type == "MESH"),
           key=lambda o: len(o.data.vertices))
lo = Vector((1e9,) * 3)
hi = Vector((-1e9,) * 3)
for c in mesh.bound_box:
    w = mesh.matrix_world @ Vector(c)
    lo = Vector(map(min, lo, w))
    hi = Vector(map(max, hi, w))
h = hi.z - lo.z
floor = bpy.data.meshes.new("floor")
S = 40
floor.from_pydata([(-S, -S, lo.z), (S, -S, lo.z), (S, S, lo.z), (-S, S, lo.z)], [], [(0, 1, 2, 3)])
fo = bpy.data.objects.new("floor", floor)
scene.collection.objects.link(fo)
cam_data = bpy.data.cameras.new("cam")
cam_data.lens = 50
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light = "STUDIO"
sh.color_type = "TEXTURE"
sh.show_shadows = True
scene.render.resolution_x, scene.render.resolution_y = 420, 470
scene.render.image_settings.file_format = "PNG"
mid = Vector((0, 0, lo.z + h * 0.5))
# side view: kicks travel toward -Y, camera on +X side
cam.location = (h * 2.3, -0.3, lo.z + h * 0.55)
cam.rotation_euler = (mid - Vector(cam.location)).to_track_quat("-Z", "Y").to_euler()
for i, key in enumerate(clip["keys"]):
    scene.frame_set(key["frame"])
    scene.render.filepath = os.path.join(out_dir, f"{clip['name']}_k{i}_f{key['frame']}.png")
    bpy.ops.render.render(write_still=True)
print("AUTHOR_OK", clip["name"], len(clip["keys"]), "keys")
