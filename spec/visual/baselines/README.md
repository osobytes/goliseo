# Goalkeeper pose baselines

These opt-in 960×540 full-pitch images pin the two live match-scale review
views required by issue #46. The goalkeeper uses the same projected 6–10px
radius as normal play:

- `central_dive_catch.png`
- `stretch_dive_parry.png`

Run `./scripts/check_keeper_pose_snapshots.sh` to compare the current procedural
renderer with the checked-in images. Pass `write` only after intentionally
reviewing a renderer change.
