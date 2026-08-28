# Skeleton v2 authoring-rig generator (provisional authority for the v2 bone set).
#
# Builds the GOLISEO skeleton-v2 armature in Blender: every current game bone
# name unchanged (masks/crouch/action_pose keep working), plus the humanoid
# additions the reference study called for (second spine link, head/toe tips,
# full finger chains). Proportions are a blend between the shipped arcade
# RIG_MEDIUM values (ts/packages/render/src/rig3d/proportions.ts) and a
# realistic ~1.75 m anthropometric set — HUMANOID=0.0 is today's arcade body,
# 1.0 is realistic, default 0.5 is the agreed middle point.
#
# Authoring conventions (converted by the exporter, not here): Blender-native
# Z-up, character faces -Y, character's left at +X, .L/.R suffixes so
# Blender's mirror/pose-flip tooling works.
#
# Usage:
#   blender --background --python scripts/anim/blender_rig_gen.py -- \
#       <out_dir> [humanoid_blend] [--stills]
# Writes <out_dir>/goliseo_rig_v2.blend, <out_dir>/skeleton_v2.json
# and, with --stills, front/three-quarter verification renders.
import bpy, json, math, os, sys

argv = sys.argv[sys.argv.index("--") + 1 :]
out_dir = argv[0]
HUMANOID = float(argv[1]) if len(argv) > 1 and not argv[1].startswith("--") else 0.5
# MASS blends body VOLUMES (head/torso/limb girth) separately from the skeleton,
# so limbs can lengthen while the arcade chunk survives. Defaults below skeleton.
MASS = float(argv[argv.index("--mass") + 1]) if "--mass" in argv else HUMANOID * 0.7
STILLS = "--stills" in argv
VOLUMES = "--volumes" in argv
os.makedirs(out_dir, exist_ok=True)


def lerp(a, b):
    return a + (b - a) * HUMANOID


def mlerp(a, b):
    return a + (b - a) * MASS


# body volumes: (arcade RIG_MEDIUM.form, realistic) anchors
FORM = {k: mlerp(a, b) for k, (a, b) in {
    "head_r": (0.145, 0.108),
    "neck_r": (0.062, 0.052),
    "torso_r": (0.185, 0.150),
    "arm_r": (0.074, 0.050),
    "leg_r": (0.098, 0.072),
    "hand_r": (0.058, 0.046),
    "foot_w": (0.120, 0.100),
}.items()}


# (arcade, realistic) anchors. Arcade values trace to RIG_MEDIUM.seg; realistic
# values are standard body-segment ratios for a 1.75 m frame.
SEG = {k: lerp(a, b) for k, (a, b) in {
    "pelvis_z": (0.80, 0.95),
    "spine": (0.090, 0.130),
    "spine2": (0.095, 0.130),
    "chest": (0.095, 0.160),
    "neck": (0.100, 0.100),
    "head": (0.050, 0.060),
    "skull": (0.300, 0.220),
    "shoulder_x": (0.085, 0.090),
    "arm_x": (0.155, 0.170),
    "upperarm": (0.240, 0.300),
    "lowerarm": (0.210, 0.270),
    "hand": (0.080, 0.100),
    "finger": (0.055, 0.075),
    "thigh_x": (0.105, 0.090),
    "upperleg": (0.360, 0.460),
    "lowerleg": (0.300, 0.440),
    "foot_len": (0.170, 0.190),
    "toe": (0.060, 0.070),
    "toe_tip": (0.030, 0.035),
    "hand_w": (0.070, 0.080),
}.items()}

ARM_DROP = math.radians(40)  # A-pose: arms 40 degrees below horizontal

bones = []  # (name, parent, head(x,y,z), tail(x,y,z))


def add(name, parent, head, tail):
    bones.append((name, parent, head, tail))
    return name


pz = SEG["pelvis_z"]
add("root", None, (0, 0, 0), (0, -0.25, 0))
add("hips", "root", (0, 0, pz), (0, 0, pz + 0.05))
z = pz
for nm, parent, ln in [
    ("spine", "hips", SEG["spine"]),
    ("spine2", "spine", SEG["spine2"]),
    ("chest", "spine2", SEG["chest"]),
    ("neck", "chest", SEG["neck"]),
    ("head", "neck", SEG["head"]),
    ("head_tip", "head", SEG["skull"]),
]:
    add(nm, parent, (0, 0, z), (0, 0, z + ln))
    z += ln
chest_top = pz + SEG["spine"] + SEG["spine2"] + SEG["chest"]

FINGERS = ["thumb", "index", "middle", "ring", "pinky"]
for side, sx in (("L", 1.0), ("R", -1.0)):
    sh = (sx * SEG["shoulder_x"], -0.01, chest_top - 0.02)
    ax = (sx * SEG["arm_x"], -0.01, chest_top - 0.02)
    add(f"shoulder.{side}", "chest", sh, ax)
    d = (sx * math.cos(ARM_DROP), 0.0, -math.sin(ARM_DROP))
    ua = tuple(ax[i] + d[i] * SEG["upperarm"] for i in range(3))
    fa = tuple(ua[i] + d[i] * SEG["lowerarm"] for i in range(3))
    hd = tuple(fa[i] + d[i] * SEG["hand"] for i in range(3))
    add(f"upper_arm.{side}", f"shoulder.{side}", ax, ua)
    add(f"forearm.{side}", f"upper_arm.{side}", ua, fa)
    add(f"hand.{side}", f"forearm.{side}", fa, hd)
    add(f"socket_hand.{side}", f"hand.{side}", hd, tuple(hd[i] + d[i] * 0.04 for i in range(3)))
    # fingers fan across the hand's width (local Y), continuing the arm line
    for fi, fname in enumerate(FINGERS):
        fy = (fi / (len(FINGERS) - 1) - 0.5) * SEG["hand_w"]
        seg3 = SEG["finger"] / 3
        base = (hd[0], hd[1] + fy, hd[2])
        prev = f"hand.{side}"
        for j, tag in enumerate(("1", "2", "3")):
            b0 = tuple(base[i] + (d[i] * seg3 * j if i != 1 else 0) for i in range(3))
            b0 = (b0[0], base[1], b0[2])
            b1 = (b0[0] + d[0] * seg3, base[1], b0[2] + d[2] * seg3)
            prev = add(f"{fname}_{tag}.{side}", prev, b0, b1)

# shield socket: mid-forearm on the left arm
_fa_head = (math.cos(ARM_DROP) * (SEG["upperarm"]) + SEG["arm_x"], -0.01,
            chest_top - 0.02 - math.sin(ARM_DROP) * SEG["upperarm"])
add("socket_shield.L", "forearm.L", (_fa_head[0], _fa_head[1] - 0.05, _fa_head[2]),
    (_fa_head[0], _fa_head[1] - 0.10, _fa_head[2]))
add("socket_ball", "chest", (0, -0.16, chest_top - 0.06), (0, -0.22, chest_top - 0.06))

for side, sx in (("L", 1.0), ("R", -1.0)):
    hip = (sx * SEG["thigh_x"], 0.0, pz)
    knee_z = pz - SEG["upperleg"]
    ankle_z = max(knee_z - SEG["lowerleg"], 0.05)
    add(f"thigh.{side}", "hips", hip, (hip[0], 0.0, knee_z))
    add(f"shin.{side}", f"thigh.{side}", (hip[0], 0.0, knee_z), (hip[0], 0.0, ankle_z))
    ball = (hip[0], -SEG["foot_len"], 0.015)
    add(f"foot.{side}", f"shin.{side}", (hip[0], 0.0, ankle_z), ball)
    toe_end = (hip[0], ball[1] - SEG["toe"], 0.01)
    add(f"toe.{side}", f"foot.{side}", ball, toe_end)
    add(f"toe_tip.{side}", f"toe.{side}", toe_end, (hip[0], toe_end[1] - SEG["toe_tip"], 0.01))

# --- build the armature ---
bpy.ops.wm.read_factory_settings(use_empty=True)
arm_data = bpy.data.armatures.new("goliseo_v2")
arm_obj = bpy.data.objects.new("goliseo_v2", arm_data)
bpy.context.scene.collection.objects.link(arm_obj)
bpy.context.view_layer.objects.active = arm_obj
bpy.ops.object.mode_set(mode="EDIT")
from mathutils import Vector as _V

edit = {}
for name, parent, head, tail in bones:
    eb = arm_data.edit_bones.new(name)
    eb.head, eb.tail = head, tail
    # canonical roll: local Z forward (-Y), so local X is always the side axis
    d = (_V(tail) - _V(head)).normalized()
    eb.align_roll(_V((0, 0, 1)) if abs(d.y) > 0.9 else _V((0, -1, 0)))
    if parent:
        eb.parent = edit[parent]
    edit[name] = eb
bpy.ops.object.mode_set(mode="OBJECT")

height = SEG["pelvis_z"] + SEG["spine"] + SEG["spine2"] + SEG["chest"] + SEG["neck"] + SEG["head"] + SEG["skull"]
report = {
    "humanoid_blend": HUMANOID,
    "art_height_m": round(height, 4),
    "bone_count": len(bones),
    "segments": {k: round(v, 4) for k, v in SEG.items()},
    "bones": [{"name": n, "parent": p, "head": [round(c, 4) for c in h],
               "tail": [round(c, 4) for c in t]} for n, p, h, t in bones],
}
with open(os.path.join(out_dir, "skeleton_v2.json"), "w") as fh:
    json.dump(report, fh, indent=1)
bpy.ops.wm.save_as_mainfile(filepath=os.path.join(out_dir, "goliseo_rig_v2.blend"))

if VOLUMES:
    # quick silhouette body: primitives sized by the blended form params —
    # this is what the middle-point judgment is actually about
    from mathutils import Vector

    scene = bpy.context.scene
    BODY = (0.42, 0.50, 0.62, 1.0)
    heads = {n: (Vector(h), Vector(t)) for n, _, h, t in bones}

    def paint(ob):
        ob.color = BODY

    def capsule(p0, p1, r0, r1):
        d = Vector(p1) - Vector(p0)
        mid = (Vector(p0) + Vector(p1)) / 2
        bpy.ops.mesh.primitive_cone_add(vertices=14, radius1=r0, radius2=r1,
                                        depth=max(d.length, 0.01), location=mid)
        ob = bpy.context.active_object
        ob.rotation_mode = "QUATERNION"
        ob.rotation_quaternion = d.to_track_quat("Z", "Y")
        paint(ob)

    def ball(center, rx, ry, rz):
        bpy.ops.mesh.primitive_uv_sphere_add(segments=18, ring_count=12, location=center)
        ob = bpy.context.active_object
        ob.scale = (rx, ry, rz)
        paint(ob)

    # head sits on the head bone, sized by head_r
    h0, h1 = heads["head_tip"]
    ball((h0.x, h0.y, h0.z + FORM["head_r"] * 0.95), FORM["head_r"], FORM["head_r"], FORM["head_r"] * 1.06)
    capsule(heads["neck"][0], heads["neck"][1], FORM["neck_r"], FORM["neck_r"] * 0.9)
    # torso: pelvis block + chest mass
    sp0 = heads["spine"][0]
    ct1 = heads["chest"][1]
    torso_len = ct1.z - sp0.z
    ball((0, 0, sp0.z + torso_len * 0.62), FORM["torso_r"] * 1.12, FORM["torso_r"] * 0.82, torso_len * 0.62)
    ball((0, 0, sp0.z + 0.01), FORM["torso_r"] * 0.94, FORM["torso_r"] * 0.72, 0.11)
    for side in ("L", "R"):
        capsule(*heads[f"upper_arm.{side}"], FORM["arm_r"], FORM["arm_r"] * 0.86)
        capsule(*heads[f"forearm.{side}"], FORM["arm_r"] * 0.86, FORM["arm_r"] * 0.66)
        hd0, hd1 = heads[f"hand.{side}"]
        ball(((hd0 + hd1) / 2), FORM["hand_r"], FORM["hand_r"] * 0.8, FORM["hand_r"])
        capsule(*heads[f"thigh.{side}"], FORM["leg_r"], FORM["leg_r"] * 0.8)
        capsule(*heads[f"shin.{side}"], FORM["leg_r"] * 0.8, FORM["leg_r"] * 0.55)
        f0, f1 = heads[f"foot.{side}"]
        t1 = heads[f"toe.{side}"][1]
        foot_mid = (f0 + t1) / 2
        ball((foot_mid.x, foot_mid.y, 0.045), FORM["foot_w"] * 0.55, (t1 - f0).length * 0.62, 0.05)

if STILLS:
    scene = bpy.context.scene
    from mathutils import Vector
    if not VOLUMES:
        unit = bpy.data.meshes.new("unit_cone")
        uverts = [(0.0, 0.0, 0.0), (0.0, 1.0, 0.0)]
        ufaces = []
        for i in range(6):
            a = i * math.tau / 6
            uverts.append((math.cos(a), 0.18, math.sin(a)))
        for i in range(6):
            j, k = 2 + i, 2 + (i + 1) % 6
            ufaces += [(0, j, k), (1, k, j)]
        unit.from_pydata(uverts, [], ufaces)
        for name, parent, head, tail in bones:
            d = Vector(tail) - Vector(head)
            ln = max(d.length, 0.015)
            r = min(0.03, ln * 0.22)
            ob = bpy.data.objects.new(f"seg_{name}", unit)
            ob.rotation_mode = "QUATERNION"
            scene.collection.objects.link(ob)
            ob.location = head
            ob.rotation_quaternion = d.to_track_quat("Y", "Z")
            ob.scale = (r, ln, r)
            ob.color = (0.9, 0.45, 0.12, 1.0)
    floor = bpy.data.meshes.new("floor")
    floor.from_pydata([(-30, -30, 0), (30, -30, 0), (30, 30, 0), (-30, 30, 0)], [], [(0, 1, 2, 3)])
    fo = bpy.data.objects.new("floor", floor)
    scene.collection.objects.link(fo)
    fo.color = (0.55, 0.6, 0.55, 1.0)
    cam_data = bpy.data.cameras.new("cam")
    cam_data.lens = 50
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "OBJECT"
    scene.display.shading.show_shadows = True
    scene.render.resolution_x = scene.render.resolution_y = 540
    scene.render.image_settings.file_format = "PNG"
    mid = Vector((0, 0, height * 0.52))
    shots = [("front", (0, -3.4, height * 0.55)), ("threequarter", (2.4, -2.6, height * 0.62)),
             # match camera: ~17 degree elevation at distance, the read that matters
             ("matchcam", (2.6, -9.2, height * 0.5 + 2.9))]
    for tag, loc in shots:
        cam.location = loc
        q = (mid - Vector(loc)).to_track_quat("-Z", "Y")
        cam.rotation_euler = q.to_euler()
        scene.render.filepath = os.path.join(out_dir, f"rig_v2_{tag}.png")
        bpy.ops.render.render(write_still=True)

print("RIG_OK", len(bones), "bones", round(height, 3), "m")
