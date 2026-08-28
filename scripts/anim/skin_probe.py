# Quantify skin-weight bleed on a bound character blend: rotate one arm bone,
# then measure how far TORSO-core vertices move. On a clean bind the torso
# stays put (millimetres); bleeding weights drag the back/chest along —
# the exact defect reported from the pose editor.
#
# Prints, per case, max/mean world displacement of torso-core vertices and of
# the limb's own vertices (the limb number proves the pose was actually
# applied, so a zero torso reading can't come from a dead probe).
#
# Usage: blender --background <bound.blend> --python skin_probe.py -- <label> <out_dir>
import bpy, math, os, sys

from mathutils import Euler, Vector as V

argv = sys.argv[sys.argv.index("--") + 1 :]
label, out_dir = argv[0], argv[1]
os.makedirs(out_dir, exist_ok=True)

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
mesh = max((o for o in bpy.data.objects if o.type == "MESH"), key=lambda o: len(o.data.vertices))
if arm.animation_data:
    arm.animation_data_clear()
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"
    pb.rotation_quaternion = (1, 0, 0, 0)


def evaluated_verts():
    bpy.context.view_layer.update()
    dg = bpy.context.evaluated_depsgraph_get()
    ev = mesh.evaluated_get(dg)
    return [mesh.matrix_world @ v.co for v in ev.data.vertices]


rest = evaluated_verts()
# bands are height-normalised from the mesh's own bounds so the probe is
# comparable across bodies with different coordinate layouts
zs = [w.z for w in rest]
xs = [w.x for w in rest]
z0, z1 = min(zs), max(zs)
H, cx = z1 - z0, (min(xs) + max(xs)) / 2
# torso core: centre column between waist and sternum — excludes arms/shoulders
torso = [i for i, w in enumerate(rest)
         if abs(w.x - cx) < 0.075 * H and z0 + 0.55 * H < w.z < z0 + 0.80 * H]
# sanity column: vertices the left forearm/hand weights own — proves the pose applied
gi = {g.index for n, g in mesh.vertex_groups.items() if n in ("forearm.L", "hand.L")}
larm = [v.index for v in mesh.data.vertices
        if any(g.group in gi and g.weight > 0.5 for g in v.groups)]
print(f"[{label}] torso probe verts: {len(torso)}, arm probe verts: {len(larm)}")


def measure(case, bone, xyz_deg):
    for pb in arm.pose.bones:
        pb.rotation_quaternion = (1, 0, 0, 0)
    arm.pose.bones[bone].rotation_quaternion = Euler(
        [math.radians(a) for a in xyz_deg], "XYZ"
    ).to_quaternion()
    posed = evaluated_verts()
    t = [(posed[i] - rest[i]).length for i in torso]
    a = [(posed[i] - rest[i]).length for i in larm]
    print(f"[{label}] {case:24s} torso max {max(t)*1000:7.1f} mm  "
          f"mean {sum(t)/len(t)*1000:6.1f} mm | arm max {max(a)*1000:7.1f} mm")
    # render the pose for the eye
    scene = bpy.context.scene
    for o in [o for o in bpy.data.objects if o.name.startswith(("floor", "cam", "dbg_", "seg_"))]:
        bpy.data.objects.remove(o, do_unlink=True)
    cam_data = bpy.data.cameras.new("cam")
    cam_data.lens = 60
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    cam.location = (0.25, -2.6, 1.15)
    cam.rotation_euler = (V((0, 0, 1.15)) - V(cam.location)).to_track_quat("-Z", "Y").to_euler()
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.light = "STUDIO"
    scene.display.shading.show_shadows = False
    scene.render.resolution_x, scene.render.resolution_y = 460, 620
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = os.path.join(out_dir, f"probe_{label}_{case}.png")
    bpy.ops.render.render(write_still=True)


measure("elbow_bend_65", "forearm.L", (65, 0, 0))
measure("shoulder_raise_45", "upper_arm.L", (45, 0, 0))
measure("chest_twist_30", "chest", (0, 30, 0))
print("SKIN_PROBE_OK", label)
