# Generate run.json: a 20-frame in-place run cycle (30 fps, ~0.67 s/cycle).
# The right-stance half is authored; the left half is its mirror (L<->R swap,
# x negated), so the gait is symmetric by construction. Frame 21 repeats
# frame 1 for loop closure. World frame: -Y forward, +Z up, +X character-left.
# Root does not translate: in-game ground speed comes from the sim.
import json, os

HERE = os.path.dirname(os.path.abspath(__file__))


def key(frame, root_z, aims, spine_x, chest_x, head_x):
    return {
        "frame": frame,
        "root_z": root_z,
        "pose": {"spine": [spine_x, 0, 0], "chest": [chest_x, 0, 0], "head": [head_x, 0, 0]},
        "aim": {n: {"d": d} for n, d in aims.items()},
    }


def mirror(k, frame):
    def flip(name):
        if name.endswith(".L"):
            return name[:-2] + ".R"
        if name.endswith(".R"):
            return name[:-2] + ".L"
        return name

    return {
        "frame": frame,
        "root_z": k["root_z"],
        "pose": dict(k["pose"]),
        "aim": {flip(n): {"d": [-a["d"][0], a["d"][1], a["d"][2]]}
                for n, a in k["aim"].items()},
    }


# --- right-stance half -------------------------------------------------------
# elbows: forearm dir = upper-arm dir at ~90 deg flexion, i.e. (y,z) -> (z,-y)
contact_r = key(1, -0.04, {
    "thigh.R": [0, -0.55, -0.84], "shin.R": [0, -0.28, -0.96],
    "foot.R": [0, -0.90, -0.44], "toe.R": [0, -0.90, -0.44],
    "thigh.L": [0, 0.50, -0.87], "shin.L": [0, 0.80, -0.60],
    "foot.L": [0, 0.45, -0.89], "toe.L": [0, 0.45, -0.89],
    "upper_arm.L": [0.08, -0.60, -0.80], "forearm.L": [0.08, -0.75, 0.66],
    "hand.L": [0.08, -0.75, 0.66],
    "upper_arm.R": [-0.08, 0.55, -0.84], "forearm.R": [-0.08, -0.45, -0.89],
    "hand.R": [-0.08, -0.45, -0.89],
}, 10, 6, -10)

midstance_r = key(4, -0.06, {
    "thigh.R": [0, -0.05, -1.0], "shin.R": [0, 0.15, -0.99],
    "foot.R": [0, -0.67, -0.74], "toe.R": [0, -1.0, -0.02],
    "thigh.L": [0, 0.15, -0.99], "shin.L": [0, 0.72, -0.69],
    "foot.L": [0, 0.30, -0.95], "toe.L": [0, 0.30, -0.95],
    "upper_arm.L": [0.06, -0.25, -0.97], "forearm.L": [0.06, -0.93, 0.35],
    "hand.L": [0.06, -0.93, 0.35],
    "upper_arm.R": [-0.06, 0.25, -0.97], "forearm.R": [-0.06, -0.60, -0.80],
    "hand.R": [-0.06, -0.60, -0.80],
}, 9, 6, -9)

toeoff_r = key(7, -0.005, {
    "thigh.R": [0, 0.42, -0.91], "shin.R": [0, 0.62, -0.78],
    "foot.R": [0, 0.42, -0.91], "toe.R": [0, -0.10, -0.99],
    "thigh.L": [0, -0.62, -0.78], "shin.L": [0, -0.05, -1.0],
    "foot.L": [0, -0.45, -0.89], "toe.L": [0, -0.45, -0.89],
    "upper_arm.L": [0.07, 0.05, -1.0], "forearm.L": [0.07, -0.97, -0.24],
    "hand.L": [0.07, -0.97, -0.24],
    "upper_arm.R": [-0.07, -0.05, -1.0], "forearm.R": [-0.07, -0.95, 0.31],
    "hand.R": [-0.07, -0.95, 0.31],
}, 10, 7, -10)

flight_rl = key(9, 0.03, {
    "thigh.L": [0, -0.60, -0.80], "shin.L": [0, -0.35, -0.94],
    "foot.L": [0, -0.80, -0.60], "toe.L": [0, -0.80, -0.60],
    "thigh.R": [0, 0.50, -0.87], "shin.R": [0, 0.85, -0.53],
    "foot.R": [0, 0.50, -0.87], "toe.R": [0, 0.50, -0.87],
    "upper_arm.R": [-0.08, -0.55, -0.84], "forearm.R": [-0.08, -0.84, 0.55],
    "hand.R": [-0.08, -0.84, 0.55],
    "upper_arm.L": [0.08, 0.50, -0.87], "forearm.L": [0.08, -0.55, -0.83],
    "hand.L": [0.08, -0.55, -0.83],
}, 11, 7, -11)

keys = [
    contact_r, midstance_r, toeoff_r, flight_rl,
    mirror(contact_r, 11), mirror(midstance_r, 14),
    mirror(toeoff_r, 17), mirror(flight_rl, 19),
    {**contact_r, "frame": 21},
]

clip = {"name": "run", "fps": 30, "keys": keys}
out = os.path.join(HERE, "run.json")
with open(out, "w") as fh:
    json.dump(clip, fh, indent=1)
print("GEN_RUN_OK", out, len(keys), "keys")
