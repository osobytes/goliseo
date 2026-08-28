# Render the REAL in-game character mesh (dumped world-space colored vertices)
# side by side at two proportion sets — the honest "stylized ceiling" preview.
# Usage: blender --background --python mesh_preview.py -- <a.json> <b.json> <out_dir>
import bpy, json, sys

from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
a_path, b_path, out_dir = argv[0], argv[1], argv[2]

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


def load(path, name, x_off):
    with open(path) as fh:
        d = json.load(fh)
    data, stride = d["data"], d["stride"]
    n = len(data) // stride
    verts, cols = [], []
    for i in range(n):
        px, py, pz = data[i * stride : i * stride + 3]
        r, g, b = data[i * stride + 6 : i * stride + 9]
        # game Y-up, faces +Z  ->  blender Z-up, faces -Y
        verts.append((px + x_off, -pz, py))
        cols.append((r, g, b, 1.0))
    faces = [(i, i + 1, i + 2) for i in range(0, n, 3)]
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    attr = mesh.color_attributes.new(name="Col", type="FLOAT_COLOR", domain="POINT")
    for i, c in enumerate(cols):
        attr.data[i].color = c
    obj = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(obj)
    return d["height"]


h1 = load(a_path, "current", -0.62)
h2 = load(b_path, "candidate", 0.62)
height = max(h1, h2)

floor = bpy.data.meshes.new("floor")
floor.from_pydata([(-40, -40, 0), (40, -40, 0), (40, 40, 0), (-40, 40, 0)], [], [(0, 1, 2, 3)])
fo = bpy.data.objects.new("floor", floor)
scene.collection.objects.link(fo)
fo.color = (0.42, 0.47, 0.44, 1.0)

cam_data = bpy.data.cameras.new("cam")
cam_data.lens = 50
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
scene.render.engine = "BLENDER_WORKBENCH"
sh = scene.display.shading
sh.light = "STUDIO"
sh.color_type = "VERTEX"
sh.show_shadows = True
scene.render.resolution_x, scene.render.resolution_y = 1280, 720
scene.render.image_settings.file_format = "PNG"

mid = Vector((0, 0, height * 0.58))
for tag, loc in (
    ("front", (0, -5.6, height * 0.6)),
    ("threequarter", (3.4, -4.4, height * 0.7)),
    ("matchcam", (2.6, -10.5, height * 0.5 + 3.3)),
):
    cam.location = loc
    cam.rotation_euler = (mid - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = f"{out_dir}/mesh_{tag}.png"
    bpy.ops.render.render(write_still=True)

print("MESH_PREVIEW_OK", h1, h2)
