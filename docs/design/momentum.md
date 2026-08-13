# Design: Player momentum & turning radius

> **Superseded in mechanism, not in intent, by `docs/design/locomotion.md`
> (#488).** The `run_vel` field, the separate accel/decel rates and the
> single-helper rule below all survive; the vector nudge this file describes
> is now a speed scalar plus a heading eased independently, and facing is its
> own target rather than a read of `run_vel`. This file stays as the record of
> the first pass — its "Watch out for" list still applies verbatim.

## Why

Constant-speed movement and instant reversals kill the core feel of a soccer
game: beating a player means nothing, tackles have no committed moment to
punish, and sprint carries no risk. Momentum makes changes of direction
readable and committed.

## Player-facing behavior

- Players **accelerate** to top speed over ~0.25s and **decelerate** when input
  stops (~0.18s to rest).
- Reversing direction requires shedding current velocity first — a sprinting
  player turns in a visible arc instead of on a dime.
- Sprint raises top speed (unchanged multiplier) but makes direction changes
  *heavier* (same accel fighting more velocity) — sprint becomes a commitment.
- A stunned player accelerates at 40% (existing STUN_SLOW feel preserved).

## Mechanics

Add a single movement helper in `sim/match.lua` and route ALL walking/running
movement through it (controlled player, AI owner dribble, keeper positioning,
off-ball AI). Slides, dodges, and dives keep their existing bespoke movement.

```lua
local MOVE_ACCEL = 1100 -- px/s^2 toward the desired velocity
local MOVE_DECEL = 1500 -- px/s^2 when the desired velocity is zero (stopping)

-- p.run_vel (Vec2, new field): the player's current locomotion velocity.
-- Each tick: desired = dir * speed; run_vel moves toward desired by at most
-- accel*dt (DECEL when desired is zero); pos += run_vel * dt (clamped to field).
```

- New `MatchPlayer` field `run_vel` (init/reset like `vel`; note `vel` stays the
  *realized* velocity derived after collisions — do not merge them).
- `facing` = normalized `run_vel` when its length > 20 (drifts with real motion),
  else last facing. The existing input-facing for aiming while stationary must
  keep working: when input is held but speed is tiny, facing = input dir.
- AI steering (`ai.steer` calls) currently returns a clamped position; convert
  those call sites to produce a desired *direction* + speed and feed the same
  helper. Do not modify `sim/ai.lua`.
- `place_kickoff` resets `run_vel` to zero.

## Watch out for

- Many existing specs assume distances covered in N frames. Prefer relaxing
  frame budgets/loosening distance assertions over weakening their intent. The
  sprint specs measure displacement ratios — those should still pass (both
  runs accelerate identically).
- Keeper dive movement, slide movement, juke movement: leave untouched.
- The whiff-stumble/settle/shield mechanics rely on relative positioning —
  re-run the full suite and the balance sanity below.

## Acceptance

1. New spec: from rest, displacement in the first 6 frames is < 60% of the
   displacement in frames 25–30 × 6 (acceleration exists).
2. New spec: a player running right at full speed who reverses input takes
   longer to travel 40px left than one starting from rest (turn commitment).
3. All existing specs pass (adjusted only where frame budgets are too tight).
4. `./scripts/check.sh` green.
