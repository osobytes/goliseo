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

Pass `write` to either script only after intentionally reviewing a renderer
change. A baseline nobody looked at is worse than no baseline.
