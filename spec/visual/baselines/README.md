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

## The three image sets above are NOT gates

They are pixel-exact, so they pin the GPU that produced them as much as they pin the
code. Re-rendered under Mesa llvmpipe — what a CI runner has — against these committed
baselines, every one of them fails: the two pose PNG sets by byte inequality, and the
rig3d pair with 93% of pixels differing at a max deviation of 39/255. That is why none
of the three appears in `scripts/check.sh` or `.github/workflows/ci.yml`, and per
AGENTS.md §9 that makes them evidence run by hand on the machine that owns the
baselines. Run them before changing a renderer; never read a green CI run as having
checked them. What to do about it — re-pin to a software rasteriser, or add
machine-independent gates beside the images — is #351, and it is a real trade, not
an oversight to fix in passing.

## rig3d player draw (issue #340) — this one IS a gate

- `rig3d_player_draw.txt` — not an image. It is what
  `game/render/player_renderer_3d.lua` hands the GPU across nine fixed scenarios:
  the resolved palette, the character's model matrix, and all 78 bone rows,
  captured at `renderer.beginPass` / `renderer.draw`.

That renderer was executed by nothing in this repository until this gate existed. The
tier-1 specs require `themes`/`meshbuilder`/`skeleton`/`body` directly, and the palette
gate above calls `body.build` directly, so `build()`, `poseFor`, `clipFor`,
`draw_player` and the `pose → skeleton.apply → boneRows → renderer.draw` integration had
never run under any test. The baseline is numbers rather than pixels precisely so it
can be enforced: it is pure Lua arithmetic, identical on an NVIDIA card and under
llvmpipe, which is what lets `./scripts/check_rig3d_player_draw.sh` run in `check.sh`
and in CI. Pixels are still checked, just not pinned — the gate requires that a
character actually rasterised and that the two team palettes produce different frames.

Refresh with `./scripts/check_rig3d_player_draw.sh write`, and only after reviewing
why a pose moved.
