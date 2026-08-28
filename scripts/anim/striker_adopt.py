# Adopt the Meshy striker body as the authoring character: keep the mesh and
# Meshy's ML-painted skin weights, but rename its Mixamo-style bones to our
# skeleton names and rebuild tails + canonical rolls so the aim-based clip
# pipeline (author_clip.py / clip_export.py) drives it unchanged.
#
# Rationale over meshy_fit.py's analytic bind: Meshy's weights are painted for
# this exact mesh from a true T-pose, so elbow/shoulder rotation stays out of
# the torso — the failure mode of the region-grown weights.
#
# Usage: blender --background --python striker_adopt.py -- <in.glb> <out_dir>
import bpy, json, math, os, sys

from mathutils import Vector as V

argv = sys.argv[sys.argv.index("--") + 1 :]
src, out_dir = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)
CANON_H = 1.728  # authoring height shared with the previous body / root_z scale

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
mesh = max((o for o in bpy.data.objects if o.type == "MESH"), key=lambda o: len(o.data.vertices))
for o in list(bpy.data.objects):
    if o not in (arm, mesh):  # drop props (ball icosphere) and empties
        bpy.data.objects.remove(o, do_unlink=True)
if arm.animation_data:
    arm.animation_data_clear()

# --- flatten object transforms into the data, in metres ----------------------
# unparent first: transform_apply on a parent does not compensate children
mw = mesh.matrix_world.copy()
mesh.parent = None
mesh.matrix_world = mw


def apply_all():
    for o in (arm, mesh):
        bpy.ops.object.select_all(action="DESELECT")
        o.select_set(True)
        bpy.context.view_layer.objects.active = o
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)


apply_all()


def bounds():
    lo, hi = V((1e9,) * 3), V((-1e9,) * 3)
    for v in mesh.data.vertices:
        w = mesh.matrix_world @ v.co
        lo, hi = V(map(min, lo, w)), V(map(max, hi, w))
    return lo, hi


lo, hi = bounds()
f = CANON_H / (hi.z - lo.z)
for o in (arm, mesh):
    o.scale = (f, f, f)
apply_all()
lo, hi = bounds()
# feet exactly on the floor
for o in (arm, mesh):
    o.location.z -= lo.z
apply_all()

# --- fold the face-anchor bone into the head before renaming -----------------
vg = mesh.vertex_groups
if "headfront" in vg and "Head" in vg:
    hf_i, hd = vg["headfront"].index, vg["Head"]
    for v in mesh.data.vertices:
        for g in v.groups:
            if g.group == hf_i and g.weight > 1e-6:
                hd.add([v.index], g.weight, "ADD")
    vg.remove(vg["headfront"])
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")
eb = arm.data.edit_bones
if "headfront" in eb:
    eb.remove(eb["headfront"])
bpy.ops.object.mode_set(mode="OBJECT")

# --- rename to our skeleton names -------------------------------------------
MAP = {
    "Hips": "hips", "Spine02": "spine", "Spine01": "spine2", "Spine": "chest",
    "neck": "neck", "Head": "head", "head_end": "head_tip",
    "LeftShoulder": "shoulder.L", "LeftArm": "upper_arm.L",
    "LeftForeArm": "forearm.L", "LeftHand": "hand.L",
    "RightShoulder": "shoulder.R", "RightArm": "upper_arm.R",
    "RightForeArm": "forearm.R", "RightHand": "hand.R",
    "LeftUpLeg": "thigh.L", "LeftLeg": "shin.L", "LeftFoot": "foot.L", "LeftToeBase": "toe.L",
    "RightUpLeg": "thigh.R", "RightLeg": "shin.R", "RightFoot": "foot.R", "RightToeBase": "toe.R",
}
for old, new in MAP.items():
    arm.data.bones[old].name = new
# bone rename normally syncs vertex-group names; enforce it in case it didn't
for old, new in MAP.items():
    if old in mesh.vertex_groups:
        mesh.vertex_groups[old].name = new

# --- rebuild tails along the anatomy, then canonical rolls -------------------
# glTF has no bone tails; the importer's guesses are unusable, and both the aim
# system (bone Y = head->tail) and local-euler pose blocks depend on the frame.
CHAIN = {
    "hips": "spine", "spine": "spine2", "spine2": "chest", "chest": "neck",
    "neck": "head", "head": "head_tip",
    "shoulder.L": "upper_arm.L", "upper_arm.L": "forearm.L", "forearm.L": "hand.L",
    "shoulder.R": "upper_arm.R", "upper_arm.R": "forearm.R", "forearm.R": "hand.R",
    "thigh.L": "shin.L", "shin.L": "foot.L", "foot.L": "toe.L",
    "thigh.R": "shin.R", "shin.R": "foot.R", "foot.R": "toe.R",
}
bpy.ops.object.mode_set(mode="EDIT")
eb = arm.data.edit_bones
for name, child in CHAIN.items():
    eb[name].tail = eb[child].head
for side in ("L", "R"):
    hand = eb[f"hand.{side}"]
    d = (hand.head - eb[f"forearm.{side}"].head).normalized()
    hand.tail = hand.head + d * 0.09
    toe = eb[f"toe.{side}"]
    toe.tail = toe.head + V((0, -0.07, 0))
eb["head_tip"].tail = eb["head_tip"].head + V((0, 0, 0.06))
for b in eb:
    d = (b.tail - b.head).normalized()
    # canonical roll: local Z forward (-Y); near-forward bones use Z-up instead
    b.align_roll(V((0, 0, 1)) if abs(d.y) > 0.9 else V((0, -1, 0)))
bpy.ops.object.mode_set(mode="OBJECT")

# --- verify + report ---------------------------------------------------------
group_names = {g.name for g in mesh.vertex_groups}
bone_names = {b.name for b in arm.data.bones}
orphans = sorted(group_names - bone_names)
assert not orphans, f"vertex groups with no bone: {orphans}"
unweighted = sum(1 for v in mesh.data.vertices if sum(g.weight for g in v.groups) < 0.5)
report = {
    "height_m": round(bounds()[1].z, 4),
    "bones": len(arm.data.bones),
    "verts": len(mesh.data.vertices),
    "unweighted_verts": unweighted,
    "vertex_groups": sorted(group_names),
}
with open(os.path.join(out_dir, "striker_report.json"), "w") as fh:
    json.dump(report, fh, indent=1)
assert unweighted == 0, f"{unweighted} vertices with no skin weight"

bpy.ops.wm.save_as_mainfile(filepath=os.path.join(out_dir, "striker_bound.blend"))
print("STRIKER_ADOPT_OK", report["bones"], "bones", report["verts"], "verts",
      "height", report["height_m"])
