# Probe + preview a purchased GLB character: dump rig/mesh/material/animation
# ground truth as JSON and render textured stills (rest pose, front/3q/closeup).
# Usage: blender --background --python glb_probe.py -- <in.glb> <out_dir>
import bpy, json, os, sys

from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
src, out_dir = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
scene = bpy.context.scene

report = {"file": src}
arms = [o for o in bpy.data.objects if o.type == "ARMATURE"]
meshes = [o for o in bpy.data.objects if o.type == "MESH"]
report["armatures"] = [
    {"name": a.name, "bones": len(a.data.bones),
     "bone_names": [b.name for b in a.data.bones],
     "roots": [b.name for b in a.data.bones if b.parent is None]}
    for a in arms
]
report["meshes"] = [
    {"name": m.name, "verts": len(m.data.vertices),
     "tris": sum(len(p.vertices) - 2 for p in m.data.polygons),
     "vertex_groups": len(m.vertex_groups),
     "uv_layers": len(m.data.uv_layers),
     "materials": [mat.name for mat in m.data.materials if mat],
     "skinned": any(mod.type == "ARMATURE" for mod in m.modifiers)}
    for m in meshes
]
report["images"] = [
    {"name": i.name, "size": list(i.size), "packed": i.packed_file is not None}
    for i in bpy.data.images if i.name != "Render Result"
]
report["actions"] = [
    {"name": a.name, "frame_range": list(a.frame_range)} for a in bpy.data.actions
]

# overall bounds (world)
lo = Vector((1e9, 1e9, 1e9))
hi = Vector((-1e9, -1e9, -1e9))
for m in meshes:
    for corner in m.bound_box:
        w = m.matrix_world @ Vector(corner)
        lo = Vector(map(min, lo, w))
        hi = Vector(map(max, hi, w))
report["bounds_min"] = [round(c, 3) for c in lo]
report["bounds_max"] = [round(c, 3) for c in hi]
height = hi.z - lo.z
report["height_z"] = round(height, 3)

with open(os.path.join(out_dir, "glb_report.json"), "w") as fh:
    json.dump(report, fh, indent=1)

# --- textured stills ---
floor = bpy.data.meshes.new("floor")
s = 50
floor.from_pydata([(-s, -s, lo.z), (s, -s, lo.z), (s, s, lo.z), (-s, s, lo.z)], [], [(0, 1, 2, 3)])
fo = bpy.data.objects.new("floor", floor)
scene.collection.objects.link(fo)
fo.color = (0.45, 0.48, 0.46, 1.0)

cam_data = bpy.data.cameras.new("cam")
cam_data.lens = 55
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light = "STUDIO"
sh.color_type = "TEXTURE"
sh.show_shadows = True
scene.render.resolution_x, scene.render.resolution_y = 900, 900
scene.render.image_settings.file_format = "PNG"

mid = Vector((0, 0, lo.z + height * 0.52))
head = Vector((0, 0, lo.z + height * 0.86))
shots = [
    ("front", mid, (0, -height * 2.6, lo.z + height * 0.55)),
    ("threequarter", mid, (height * 1.7, -height * 2.1, lo.z + height * 0.62)),
    ("closeup", head, (height * 0.55, -height * 0.75, lo.z + height * 0.9)),
]
for tag, target, loc in shots:
    cam.location = loc
    cam.rotation_euler = (target - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"glb_{tag}.png")
    bpy.ops.render.render(write_still=True)

print("GLB_PROBE_OK", len(arms), "armatures", len(meshes), "meshes", round(height, 2))
