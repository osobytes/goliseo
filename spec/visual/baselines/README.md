# Pose baselines

These opt-in 960×540 full-pitch images pin live match-scale review views. Every
figure uses the same projected 6–10px radius as normal play.

## Goalkeeper (issue #46)

- `central_dive_catch.png`
- `stretch_dive_parry.png`

Run `./scripts/check_keeper_pose_snapshots.sh` to compare the current procedural
renderer with the checked-in images.

## Outfield (issue #58)

- `contain_vs_tackle.png` — an AI presser holding contain beside an outfielder
  committed to a standing challenge, so the two closest ground poses can be
  compared side by side.

Run `./scripts/check_outfield_pose_snapshots.sh` to compare.

## rig3d palette slots (issue #337)

- `rig3d_palette_crimson.png` / `rig3d_palette_azure.png` — the SAME medieval
  Rig_Medium character and mesh drawlist, rendered with two different resolved
  team palettes. Colour is a per-vertex palette slot resolved to a uniform at
  draw time (not baked geometry), so these two images differing while coming
  from one shared mesh build is the actual point of the pair.

Run `./scripts/check_rig3d_palette_snapshots.sh` to compare, or `self-test` to
prove the compare itself rejects a crimson/azure mismatch (a demonstration
this gate can go red, per AGENTS.md #9).

Pass `write` to any of these scripts only after intentionally reviewing a
renderer change. A baseline nobody looked at is worse than no baseline.
