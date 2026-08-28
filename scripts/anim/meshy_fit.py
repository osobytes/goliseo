# Fit the goliseo v2 skeleton into the purchased Meshy body, bind with
# automatic weights, and render fit-overlay / bend-test evidence.
#
# The FIT table below is the tuning surface: canonical skeleton_v2.json joint
# positions are scaled to the mesh height, then per-region corrections applied.
# Iterate: run --overlay, look, adjust FIT, repeat; then --bind for the
# weight-painted .blend and bend-test renders.
#
# Usage:
#   blender --background --python meshy_fit.py -- <in.glb> <skeleton_v2.json> \
#       <out_dir> (--overlay | --bind)
import bpy, json, math, os, sys

from mathutils import Quaternion, Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
src, skel_path, out_dir = argv[0], argv[1], argv[2]
MODE = "bind" if "--bind" in argv else "overlay"
os.makedirs(out_dir, exist_ok=True)

# Per-joint corrections AFTER uniform height scaling, in mesh-space metres.
# Tuned by eye against overlay renders of this specific body.
FIT = {
    "scale_to_height": True,
    "z_shift": {},      # e.g. {"thigh": -0.02} lowers hip joints
    "x_widen": {},      # e.g. {"thigh": 0.01} widens leg stance
    "arm_drop_deg": 74, # mesh arms hang nearly straight down (vs rig's 40)
}

# Owner-directed corrections, applied LAST, after every measurement: metres in
# mesh space, {joint: (dx, dy, dz)} moving that joint's head and tail.
# dy < 0 is toward the character's front. ".L"/".R" apply per side; a bare
# name without suffix applies mirrored to both sides (dx flipped for .R).
FIT_NUDGE = {}

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
scene = bpy.context.scene
mesh_obj = next(o for o in bpy.data.objects if o.type == "MESH")

lo = Vector((1e9,) * 3)
hi = Vector((-1e9,) * 3)
for corner in mesh_obj.bound_box:
    w = mesh_obj.matrix_world @ Vector(corner)
    lo = Vector(map(min, lo, w))
    hi = Vector(map(max, hi, w))
mesh_h = hi.z - lo.z

with open(skel_path) as fh:
    spec = json.load(fh)
s = (mesh_h / spec["art_height_m"]) if FIT["scale_to_height"] else 1.0

def fitted(name, p):
    v = Vector((p[0] * s, p[1] * s, p[2] * s + lo.z))
    base = name.split(".")[0]
    v.z += FIT["z_shift"].get(base, 0.0)
    if name.endswith(".L"):
        v.x += FIT["x_widen"].get(base, 0.0)
    elif name.endswith(".R"):
        v.x -= FIT["x_widen"].get(base, 0.0)
    return v

# re-drop the arms: rotate arm-chain joints around the shoulder to match the
# mesh's hanging arms
def redrop(joints):
    extra = math.radians(FIT["arm_drop_deg"] - 40)
    for side, sx in (("L", 1.0), ("R", -1.0)):
        pivot = joints[f"upper_arm.{side}"][0]
        rot = Quaternion((0, 1, 0), extra * sx)
        chain = [f"upper_arm.{side}", f"forearm.{side}", f"hand.{side}", f"socket_hand.{side}"]
        chain += [f"{f}_{i}.{side}" for f in ("thumb", "index", "middle", "ring", "pinky") for i in (1, 2, 3)]
        for name in chain:
            h, t = joints[name]
            joints[name] = (pivot + rot @ (h - pivot), pivot + rot @ (t - pivot))

joints = {b["name"]: (fitted(b["name"], b["head"]), fitted(b["name"], b["tail"])) for b in spec["bones"]}
redrop(joints)

# --- measure leg landmarks from the mesh and z-warp the leg joints to them ---
verts_w = [mesh_obj.matrix_world @ v.co for v in mesh_obj.data.vertices]

def slice_stats(z, band=0.012):
    xs = [v.x for v in verts_w if abs(v.z - z) < band]
    if not xs:
        return None
    xs.sort()
    # widest x-gap around the centreline = two separate legs
    gap = max((b - a) for a, b in zip(xs, xs[1:])) if len(xs) > 1 else 0.0
    return {"width": xs[-1] - xs[0], "gap": gap}

def find_crotch():
    z = lo.z + 0.30 * mesh_h
    while z < lo.z + 0.65 * mesh_h:
        st = slice_stats(z)
        if st and st["gap"] < 0.02:
            return z
        z += 0.01
    return lo.z + 0.5 * mesh_h

def find_narrowest(z_from, z_to):
    best, best_w = None, 1e9
    z = z_from
    while z < z_to:
        st = slice_stats(z)
        if st and st["width"] < best_w:
            best, best_w = z, st["width"]
        z += 0.01
    return best

crotch_z = find_crotch()
ankle_z = find_narrowest(lo.z + 0.02 * mesh_h, lo.z + 0.12 * mesh_h)
knee_z = find_narrowest(lo.z + 0.22 * mesh_h, crotch_z - 0.08)
print(f"LANDMARKS crotch={crotch_z:.3f} knee={knee_z:.3f} ankle={ankle_z:.3f}")

skel_hip = joints["thigh.L"][0].z
skel_knee = joints["shin.L"][0].z
skel_ankle = joints["foot.L"][0].z
hip_target = crotch_z + (skel_hip - crotch_z) * 0.3  # hip joints sit just above crotch

def warp_z(z):
    pts = [(skel_ankle, ankle_z), (skel_knee, knee_z), (skel_hip, hip_target)]
    if z <= pts[0][0]:
        return ankle_z + (z - skel_ankle)
    for (a0, b0), (a1, b1) in zip(pts, pts[1:]):
        if z <= a1:
            t = (z - a0) / (a1 - a0)
            return b0 + t * (b1 - b0)
    return hip_target + (z - skel_hip)

for b in spec["bones"]:
    n = b["name"]
    if n.split(".")[0] in ("thigh", "shin", "foot", "toe", "toe_tip"):
        h, t = joints[n]
        joints[n] = (Vector((h.x, h.y, warp_z(h.z))), Vector((t.x, t.y, warp_z(t.z))))

# --- refine ALL joints from measured region centroids -----------------------
# The v0 fit assumes the body is a y=0 column; this mesh has a real
# forward/back profile (jutting head, curved back), so every joint gets its
# x/y from the centroid of ITS limb region's cross-section, per side.
me = mesh_obj.data
n_v = len(me.vertices)
vco = [mesh_obj.matrix_world @ v.co for v in me.vertices]
adj = [[] for _ in range(n_v)]
for e in me.edges:
    a, b2 = e.vertices
    d = (vco[a] - vco[b2]).length
    adj[a].append((b2, d))
    adj[b2].append((a, d))
# weld glTF UV-seam splits so BFS can cross them
from collections import defaultdict

normals_w = [(mesh_obj.matrix_world.to_3x3() @ v.normal).normalized() for v in me.vertices]
colocated = defaultdict(list)
for i, v in enumerate(vco):
    colocated[(round(v.x, 5), round(v.y, 5), round(v.z, 5))].append(i)
for group in colocated.values():
    for a, b2 in zip(group, group[1:]):
        # weld only same-surface splits (UV seams share normals); opposing
        # normals mean two different surfaces merely TOUCHING (a hand resting
        # near the hip), and welding those leaks limb regions into the torso
        if normals_w[a].dot(normals_w[b2]) > 0.8:
            adj[a].append((b2, 0.0))
            adj[b2].append((a, 0.0))

import heapq

def geodesic_region(seed_point, limit):
    start = min(range(n_v), key=lambda i: (vco[i] - seed_point).length)
    dist = {start: 0.0}
    heap = [(0.0, start)]
    while heap:
        d, i = heapq.heappop(heap)
        if d > dist.get(i, 1e9) or d > limit:
            continue
        for j, w in adj[i]:
            nd = d + w
            if nd < dist.get(j, 1e9) and nd <= limit:
                dist[j] = nd
                heapq.heappush(heap, (nd, j))
    return dist

def chain_len(names):
    return sum((joints[n][1] - joints[n][0]).length for n in names)

CHAINS = {}
for side in ("L", "R"):
    arm_chain = [f"upper_arm.{side}", f"forearm.{side}", f"hand.{side}"]
    leg_chain = [f"thigh.{side}", f"shin.{side}", f"foot.{side}", f"toe.{side}"]
    hand_mid = (joints[f"hand.{side}"][0] + joints[f"hand.{side}"][1]) / 2
    foot_mid = (joints[f"foot.{side}"][0] + joints[f"foot.{side}"][1]) / 2
    CHAINS[f"arm{side}"] = (arm_chain + [f"shoulder.{side}"],
                            geodesic_region(hand_mid, chain_len(arm_chain) * 1.25))
    CHAINS[f"leg{side}"] = (leg_chain,
                            geodesic_region(foot_mid, chain_len(leg_chain) * 1.3))
head_top = joints["head_tip"][1]
CHAINS["head"] = (["neck", "head"],
                  geodesic_region(head_top, (joints["head_tip"][1] - joints["neck"][0]).length * 1.5))

TORSO = ["hips", "spine", "spine2", "chest", "neck", "shoulder.L", "shoulder.R"]
region_of = ["torso"] * n_v
region_d = [1e9] * n_v
for rname, (_, dist) in CHAINS.items():
    for i, d in dist.items():
        if d < region_d[i]:
            region_d[i] = d
            region_of[i] = rname
# anatomical clips: legs may not own anything above the crotch line, and
# arms may not own the torso core — geodesic reach is not ownership
for i in range(n_v):
    r = region_of[i]
    if r.startswith("leg") and vco[i].z > crotch_z - 0.01:
        region_of[i] = "torso"
    elif r.startswith("arm") and abs(vco[i].x) < 0.15 and vco[i].z > crotch_z:
        region_of[i] = "torso"
region_ids = defaultdict(list)
for i, r in enumerate(region_of):
    region_ids[r].append(i)

def slice_pts(rname, z, band=0.025):
    return [vco[i] for i in region_ids[rname] if abs(vco[i].z - z) < band]

def slice_centroid(rname, z, band=0.025):
    pts = slice_pts(rname, z, band)
    if not pts:
        return None
    c = Vector((0, 0, 0))
    for p in pts:
        c += p
    return c / len(pts)

def move_joint(name, new_head=None, new_tail=None):
    h, t = joints[name]
    joints[name] = (new_head if new_head else h, new_tail if new_tail else t)

# torso column is recentred AFTER the limbs are measured (see the smoothing
# pass below) so the pelvis can anchor to the legs' hip line instead of the
# glute bulge, and the column stays smooth instead of zigzagging per slice.
for jn in ("head", "head_tip"):
    h, t = joints[jn]
    ch = slice_centroid("head", h.z, 0.04)
    ct = slice_centroid("head", t.z, 0.04) or ch
    joints[jn] = (Vector((0, ch.y if ch else h.y, h.z)), Vector((0, ct.y if ct else t.y, t.z)))

def narrowest_z(rname, z_from, z_to):
    best, best_w = None, 1e9
    z = z_from
    while z < z_to:
        pts = slice_pts(rname, z, 0.012)
        if pts:
            xs = [p.x for p in pts]
            w = max(xs) - min(xs)
            if w < best_w:
                best, best_w = z, w
        z += 0.01
    return best

ANKLE_Z = {}
for side in ("L", "R"):
    leg = f"leg{side}"
    ankle_s = narrowest_z(leg, lo.z + 0.02 * mesh_h, lo.z + 0.12 * mesh_h) or ankle_z
    ANKLE_Z[side] = ankle_s
    knee_s = narrowest_z(leg, lo.z + 0.22 * mesh_h, crotch_z - 0.08) or knee_z
    hip_h = joints[f"thigh.{side}"][0]
    hip_c = slice_centroid(leg, hip_target - 0.02, 0.03)
    hip_p = Vector((hip_c.x if hip_c else hip_h.x, hip_c.y if hip_c else hip_h.y, hip_h.z))
    knee_c = slice_centroid(leg, knee_s, 0.02) or Vector((hip_p.x, 0, knee_s))
    knee_p = Vector((knee_c.x, knee_c.y, knee_s))
    ankle_c = slice_centroid(leg, ankle_s, 0.02) or Vector((knee_p.x, 0, ankle_s))
    ankle_p = Vector((ankle_c.x, ankle_c.y, ankle_s))
    joints[f"thigh.{side}"] = (hip_p, knee_p)
    joints[f"shin.{side}"] = (knee_p, ankle_p)
    # feet: follow the foot's OWN direction (this mesh's toes splay), from the
    # actual heel and toe-tip points near the ground
    foot_pts = [vco[i] for i in region_ids[leg] if vco[i].z < lo.z + 0.07]
    if foot_pts:
        y_min = min(p.y for p in foot_pts)
        y_max = max(p.y for p in foot_pts)
        y_len = y_max - y_min
        # band centroids give a stable foot axis; a single extreme vertex
        # (the big toe) skews the direction inward
        front = [p for p in foot_pts if p.y < y_min + 0.2 * y_len]
        back = [p for p in foot_pts if p.y > y_max - 0.2 * y_len]
        toe_pt = sum(front, Vector()) / len(front)
        heel_pt = sum(back, Vector()) / len(back)
        heel = Vector((heel_pt.x, heel_pt.y, lo.z + 0.02))
        along = Vector((toe_pt.x - heel.x, toe_pt.y - heel.y, 0.0))
        # ankle re-anchored above the heel-to-ball midfoot, in a tight band
        ankle_band = narrowest_z(leg, lo.z + 0.045, lo.z + 0.10) or ankle_s
        ac = slice_centroid(leg, ankle_band, 0.015)
        ankle_p = Vector((ac.x, ac.y, ankle_band)) if ac else ankle_p
        joints[f"shin.{side}"] = (knee_p, ankle_p)
        ball_p = heel + along * 0.62
        ball_p.z = lo.z + 0.02
        toe_p = heel + along * 0.85
        toe_p.z = lo.z + 0.015
        tip_p = Vector((toe_pt.x, toe_pt.y, lo.z + 0.012))
        joints[f"foot.{side}"] = (ankle_p, ball_p)
        joints[f"toe.{side}"] = (ball_p, toe_p)
        joints[f"toe_tip.{side}"] = (toe_p, tip_p)
    # arms: shoulder from region top, wrist/elbow along the measured hang
    arm = f"arm{side}"
    arm_ids = region_ids[arm]
    if arm_ids:
        top_z = max(vco[i].z for i in arm_ids)
        sho_c = slice_centroid(arm, top_z - 0.03, 0.035)
        hand_bot = min((vco[i] for i in arm_ids), key=lambda p: p.z)
        sho_p = sho_c if sho_c else joints[f"upper_arm.{side}"][0]
        span = (hand_bot - sho_p).length
        old_hand = joints[f"hand.{side}"]
        hand_seg = (old_hand[1] - old_hand[0]).length
        finger_seg = (joints[f"middle_3.{side}"][1] - joints[f"middle_1.{side}"][0]).length
        hand_len = hand_seg + finger_seg
        wrist_p = sho_p.lerp(hand_bot, max(0.1, 1 - hand_len / span))
        # elbow: the arm's NARROWEST cross-section between biceps and forearm
        # bulges, not a fixed ratio down a straight line
        best_z, best_w = None, 1e9
        z0 = sho_p.z - 0.30 * (sho_p.z - hand_bot.z)
        z1 = sho_p.z - 0.70 * (sho_p.z - hand_bot.z)
        z = min(z0, z1)
        while z < max(z0, z1):
            pts = slice_pts(arm, z, 0.015)
            if len(pts) > 3:
                xs = [p.x for p in pts]
                ys = [p.y for p in pts]
                w = (max(xs) - min(xs)) + (max(ys) - min(ys))
                if w < best_w:
                    best_z, best_w = z, w
            z += 0.008
        elbow_p = sho_p.lerp(hand_bot, 0.52)
        if best_z is not None:
            ec = slice_centroid(arm, best_z, 0.02)
            if ec:
                elbow_p = Vector((ec.x, ec.y, best_z))
        old_wrist = joints[f"hand.{side}"][0]
        joints[f"upper_arm.{side}"] = (sho_p, elbow_p)
        joints[f"forearm.{side}"] = (elbow_p, wrist_p)
        hand_tip = wrist_p.lerp(hand_bot, 1.0)
        joints[f"hand.{side}"] = (wrist_p, wrist_p.lerp(hand_bot, hand_seg / max(hand_len, 1e-6)))
        # clavicle points at the measured shoulder
        cl_h = joints[f"shoulder.{side}"][0]
        joints[f"shoulder.{side}"] = (Vector((cl_h.x * 0.4, joints["chest"][1].y, cl_h.z)), sho_p)
        # drag fingers/socket along by the wrist delta
        delta = wrist_p - old_wrist
        for fn in [f"socket_hand.{side}"] + [f"{f}_{i}.{side}" for f in ("thumb", "index", "middle", "ring", "pinky") for i in (1, 2, 3)]:
            h, t = joints[fn]
            joints[fn] = (h + delta, t + delta)

# --- torso column: smooth recentre, anchored to the legs' hip line ----------
# Independent per-slice snapping zigzagged the spine (the pelvis pulled toward
# the glute bulge, the belly slice forward), which is what bent the lower back
# under the kick's spine rotations. Instead: measure once per column node,
# anchor the base between the hip joints, then low-pass the whole column.
hip_line_y = (joints["thigh.L"][0].y + joints["thigh.R"][0].y) / 2
node_zs = [
    joints["hips"][0].z,
    joints["spine"][1].z,
    joints["spine2"][1].z,
    joints["chest"][1].z,
    joints["neck"][1].z,
]
node_ys = []
for nz in node_zs:
    c = slice_centroid("torso", nz, 0.03) or slice_centroid("head", nz, 0.03)
    node_ys.append(c.y if c else 0.0)
node_ys[0] = 0.55 * hip_line_y + 0.45 * node_ys[0]
for _ in range(2):
    sm = list(node_ys)
    for i in range(1, len(node_ys) - 1):
        sm[i] = 0.25 * node_ys[i - 1] + 0.5 * node_ys[i] + 0.25 * node_ys[i + 1]
    node_ys = sm

def col_y(z):
    if z <= node_zs[0]:
        return node_ys[0]
    pairs = list(zip(node_zs, node_ys))
    for (z0, y0), (z1, y1) in zip(pairs, pairs[1:]):
        if z <= z1:
            u = (z - z0) / max(z1 - z0, 1e-9)
            return y0 + u * (y1 - y0)
    return node_ys[-1]

for jn in ("hips", "spine", "spine2", "chest", "neck"):
    h, t = joints[jn]
    joints[jn] = (Vector((0, col_y(h.z), h.z)), Vector((0, col_y(t.z), t.z)))

# --- owner-directed nudges, applied last ------------------------------------
for name, d in FIT_NUDGE.items():
    targets = [(name, 1.0)] if name in joints else [(f"{name}.L", 1.0), (f"{name}.R", -1.0)]
    for tn, sx in targets:
        if tn in joints:
            dv = Vector((d[0] * sx, d[1], d[2]))
            h, t = joints[tn]
            joints[tn] = (h + dv, t + dv)

arm_data = bpy.data.armatures.new("goliseo_v2_fit")
arm_obj = bpy.data.objects.new("goliseo_v2_fit", arm_data)
scene.collection.objects.link(arm_obj)
bpy.context.view_layer.objects.active = arm_obj
bpy.ops.object.mode_set(mode="EDIT")
edit = {}
for b in spec["bones"]:
    eb = arm_data.edit_bones.new(b["name"])
    eb.head, eb.tail = joints[b["name"]]
    # canonical roll: local Z forward (-Y) so local X is the side axis and a
    # local-X rotation is always a pitch (knee/elbow/hip fold); near-horizontal
    # bones (feet, toes) get Z up instead
    d = (eb.tail - eb.head).normalized()
    eb.align_roll(Vector((0, 0, 1)) if abs(d.y) > 0.9 else Vector((0, -1, 0)))
    if b["parent"]:
        eb.parent = edit[b["parent"]]
    edit[b["name"]] = eb
bpy.ops.object.mode_set(mode="OBJECT")

# --- shared render setup ---
floor = bpy.data.meshes.new("floor")
S = 50
floor.from_pydata([(-S, -S, lo.z), (S, -S, lo.z), (S, S, lo.z), (-S, S, lo.z)], [], [(0, 1, 2, 3)])
fo = bpy.data.objects.new("floor", floor)
scene.collection.objects.link(fo)
cam_data = bpy.data.cameras.new("cam")
cam_data.lens = 55
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light = "STUDIO"
sh.show_shadows = True
scene.render.resolution_x, scene.render.resolution_y = 900, 900
scene.render.image_settings.file_format = "PNG"
mid = Vector((0, 0, lo.z + mesh_h * 0.52))

def shoot(tag, target, loc):
    cam.location = loc
    cam.rotation_euler = (target - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"{tag}.png")
    bpy.ops.render.render(write_still=True)

if MODE == "overlay":
    # bone markers as bright cones, mesh x-rayed so joints show through
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
    for name, (h, t) in joints.items():
        d = t - h
        ln = max(d.length, 0.01)
        r = min(0.02, ln * 0.2)
        ob = bpy.data.objects.new(f"seg_{name}", unit)
        ob.rotation_mode = "QUATERNION"
        scene.collection.objects.link(ob)
        ob.location = h
        ob.rotation_quaternion = d.to_track_quat("Y", "Z")
        ob.scale = (r, ln, r)
        ob.color = (1.0, 0.25, 0.05, 1.0)
    sh.color_type = "OBJECT"
    mesh_obj.color = (0.65, 0.7, 0.72, 1.0)
    sh.show_xray = True
    sh.xray_alpha = 0.32
    shoot("fit_front", mid, (0, -mesh_h * 2.4, lo.z + mesh_h * 0.52))
    shoot("fit_side", mid, (mesh_h * 2.4, 0, lo.z + mesh_h * 0.52))
    # closeups for the joint-feedback loop: pelvis, left arm, feet
    hips_h = joints["hips"][0]
    shoot("fit_close_pelvis", Vector((0, hips_h.y, hips_h.z)), (1.15, -0.2, hips_h.z + 0.12))
    el = joints["forearm.L"][0]
    shoot("fit_close_armL", Vector((el.x, el.y, el.z)), (el.x + 0.8, el.y - 0.4, el.z + 0.1))
    ank = (joints["foot.L"][0] + joints["foot.R"][0]) / 2
    shoot("fit_close_feet", Vector((ank.x, ank.y, ank.z - 0.03)), (0.6, ank.y - 0.85, ank.z + 0.32))
    print("FIT_OVERLAY_OK")
else:
    sh.color_type = "TEXTURE"

    # Region graph, welding, and per-limb regions were computed during the
    # joint-refinement pass above; the weighting below reuses them directly.

    def seg_dist(p, a, b):
        ab = b - a
        t = max(0.0, min(1.0, (p - a).dot(ab) / max(ab.length_squared, 1e-9)))
        return (p - (a + ab * t)).length

    bone_list = sorted({n for chain, _ in CHAINS.values() for n in chain} | set(TORSO))
    bone_idx = {n: i for i, n in enumerate(bone_list)}
    # BRIDGE bones join regions across their boundary joints, so a waist
    # vertex can follow hips AND thigh, a shoulder vertex chest AND arm --
    # without them a body twist shears the mesh along the region seams.
    BRIDGE = {
        "legL": ["hips"],
        "legR": ["hips"],
        "armL": ["chest"],
        "armR": ["chest"],
        "head": ["chest"],
        "torso": ["thigh.L", "thigh.R", "upper_arm.L", "upper_arm.R", "neck"],
    }
    weights = [dict() for _ in range(n_v)]
    for i in range(n_v):
        region = region_of[i]
        chain = (CHAINS[region][0] if region != "torso" else TORSO) + BRIDGE.get(region, [])
        bridge_set = set(BRIDGE.get(region, []))
        scored = []
        for n in chain:
            h, t = joints[n]
            d = seg_dist(vco[i], h, t)
            # 1/d^2.5: sharp enough to keep limbs crisp, soft enough that
            # joint bands blend across two bones instead of snapping.
            # Bridge bones are damped: they exist to soften the boundary
            # band, not to grab distant vertices (shoulder spikes when the
            # shorts rim rode the chest at full strength).
            w = 1.0 / (d ** 2.5 + 1e-6)
            if n in bridge_set:
                # bridges only soften the joint band itself; past it they
                # cause lag (the shorts-hem skirt flare)
                w *= 0.3 if d < 0.20 else 0.0
            scored.append((w, n))
        scored.sort(reverse=True)
        top = scored[:4]
        tot = sum(w for w, _ in top)
        weights[i] = {n: w / tot for w, n in top}

    allowed_by_region = {
        r: set((CHAINS[r][0] if r != "torso" else TORSO)) | set(BRIDGE.get(r, []))
        for r in list(CHAINS.keys()) + ["torso"]
    }
    for _ in range(4):
        nxt = []
        for i in range(n_v):
            allowed = allowed_by_region[region_of[i]]
            acc = dict(weights[i])
            cnt = 1.0
            for j, _ in adj[i]:
                for n, w in weights[j].items():
                    # region-constrained: a neighbour cannot push a foreign
                    # bone across the seam (cross-leg bleed through the
                    # shorts' crotch fabric = the skirt flare)
                    if n in allowed:
                        acc[n] = acc.get(n, 0.0) + w
                cnt += 1.0
            top = sorted(acc.items(), key=lambda kv: -kv[1])[:4]
            tot = sum(w for _, w in top)
            nxt.append({n: w / tot for n, w in top})
        weights = nxt

    # ankle sharpening: below the ankle the foot owns its vertices outright and
    # above a 4 cm band the shin does — a wide shin/foot blend candy-wrappers
    # the ankle when the leg reaches full extension with a pointed toe
    for side in ("L", "R"):
        az = ANKLE_Z[side]
        foot_set = {f"foot.{side}", f"toe.{side}"}
        for i in region_ids[f"leg{side}"]:
            z = vco[i].z
            if z < az - 0.005:
                w = {n: v for n, v in weights[i].items() if n in foot_set}
            elif z > az + 0.055:
                w = {n: v for n, v in weights[i].items() if n not in foot_set}
            else:
                continue
            if w:
                tot = sum(w.values())
                weights[i] = {n: v / tot for n, v in w.items()}

    for n in bone_list:
        mesh_obj.vertex_groups.new(name=n)
    for i in range(n_v):
        for n, w in weights[i].items():
            mesh_obj.vertex_groups[n].add([i], w, "REPLACE")
    mod = mesh_obj.modifiers.new("Armature", "ARMATURE")
    mod.object = arm_obj
    mesh_obj.parent = arm_obj
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(out_dir, "meshy_bound.blend"))
    shoot("bind_rest", mid, (mesh_h * 1.6, -mesh_h * 2.0, lo.z + mesh_h * 0.58))
    # bend test: knee fold + elbow fold + hip pitch + spine twist
    bpy.context.view_layer.objects.active = arm_obj
    bpy.ops.object.mode_set(mode="POSE")
    POSE = {
        "thigh.R": Quaternion((1, 0, 0), math.radians(-70)),
        "shin.R": Quaternion((1, 0, 0), math.radians(95)),
        "forearm.L": Quaternion((1, 0, 0), math.radians(-80)),
        "spine": Quaternion((0, 0, 1), math.radians(20)),
        "head": Quaternion((0, 0, 1), math.radians(-15)),
    }
    for pb in arm_obj.pose.bones:
        pb.rotation_mode = "QUATERNION"
        if pb.name in POSE:
            pb.rotation_quaternion = POSE[pb.name]
    bpy.ops.object.mode_set(mode="OBJECT")
    shoot("bind_bend", mid, (mesh_h * 1.6, -mesh_h * 2.0, lo.z + mesh_h * 0.58))
    shoot("bind_bend_front", mid, (0, -mesh_h * 2.3, lo.z + mesh_h * 0.55))

    # DEBUG 1: posed bone cones over x-rayed mesh — where did the bones go?
    unit = bpy.data.meshes.new("dbg_cone")
    uverts = [(0.0, 0.0, 0.0), (0.0, 1.0, 0.0)]
    ufaces = []
    for i in range(6):
        a = i * math.tau / 6
        uverts.append((math.cos(a), 0.18, math.sin(a)))
    for i in range(6):
        j, k = 2 + i, 2 + (i + 1) % 6
        ufaces += [(0, j, k), (1, k, j)]
    unit.from_pydata(uverts, [], ufaces)
    dg = bpy.context.evaluated_depsgraph_get()
    for pb in arm_obj.pose.bones:
        h = arm_obj.matrix_world @ pb.head
        t = arm_obj.matrix_world @ pb.tail
        d = t - h
        ln = max(d.length, 0.01)
        r = min(0.018, ln * 0.2)
        ob = bpy.data.objects.new(f"dbg_{pb.name}", unit)
        ob.rotation_mode = "QUATERNION"
        scene.collection.objects.link(ob)
        ob.location = h
        ob.rotation_quaternion = d.to_track_quat("Y", "Z")
        ob.scale = (r, ln, r)
        ob.color = (1.0, 0.2, 0.05, 1.0)
    sh.color_type = "OBJECT"
    mesh_obj.color = (0.6, 0.65, 0.7, 1.0)
    sh.show_xray = True
    sh.xray_alpha = 0.3
    shoot("dbg_bones_posed", mid, (mesh_h * 1.6, -mesh_h * 2.0, lo.z + mesh_h * 0.58))
    sh.show_xray = False

    # DEBUG 2: dominant-bone vertex colors at rest
    for pb in arm_obj.pose.bones:
        pb.rotation_quaternion = Quaternion()
    import colorsys
    hue = {n: colorsys.hsv_to_rgb((i * 0.61803) % 1.0, 0.85, 0.95) for i, n in enumerate(bone_list)}
    attr = me.color_attributes.new(name="DbgBone", type="FLOAT_COLOR", domain="POINT")
    for i in range(n_v):
        dom = max(weights[i].items(), key=lambda kv: kv[1])[0]
        c = hue[dom]
        attr.data[i].color = (c[0], c[1], c[2], 1.0)
    me.color_attributes.active_color = attr
    sh.color_type = "VERTEX"
    shoot("dbg_weights", mid, (mesh_h * 1.6, -mesh_h * 2.0, lo.z + mesh_h * 0.58))
    print("BIND_OK")
