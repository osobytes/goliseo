# Design: Defensive jockey / contain

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

> This record names the keys that were bound when it was written. "Space" is the
> **ACTION** role and "L" the **MODIFIER** role; `docs/controls.md` and
> `game/input/bindings.lua` are authoritative for what they are bound to now.

## Why

Defending is chase-and-poke only; there is no deliberate stance. Every soccer
game has "hold to contain": shadow the carrier, stay goal-side, commit with the
tackle only when you choose.

## Player-facing behavior

Space, while not carrying, becomes hold-vs-tap (mirroring how Space already
works with the ball: hold = charge, release = fire):

- **Tap Space** (release) → the standing poke / sprint slide, exactly as today.
- **Hold Space** → **jockey stance** while held: the defender slows to 0.75×,
  faces the opposing carrier (or the loose ball), and gains a slightly wider
  poke on the release (+6 reach) as the reward for containing first.
- Releasing Space always fires the poke/slide — so "hold to shadow, release to
  strike" is one continuous motion.

## Mechanics

- `MatchInput` gains `jockey boolean` (Space held while not carrying).
- Screen (`game/screens/match.lua`): Space is already edge-`dash` on press when
  not carrying — change to: track `_space_held_prev` off the ball; while held →
  `input.jockey = true`; on release → `self._dash = true` (poke fires on
  release now). Update the full-time guard and existing screen specs.
- Sim (`sim/match.lua`), controlled branch of `move_players`:
  - When `input.jockey` and the controlled player is not the owner: movement
    speed × `JOCKEY_SLOW = 0.75`, and facing locks toward `s.ball`.
  - Set `p.jockey_timer = 0.2` each held frame (new field, decays like the
    other timers) — `attempt_steals` grants `STAND_REACH + 6` when
    `jockey_timer > 0` at poke time.
- Sprint and jockey are mutually exclusive (jockey wins while held).
- Do NOT change AI defender behavior in this task.

## Watch out for

- Existing screen spec "Space tackles only when not carrying" asserts `_dash`
  set on the press event — update it for release-fired pokes.
- The sim spec "the human can poke the ball loose from behind at contact
  range" feeds `dash = true` directly to the sim — sim-level `dash` semantics
  are unchanged, so it should still pass.
- Keep the tackle initiation conditions (`tackle_cd`, `stun_timer`, sprint →
  slide) exactly as they are.

## Acceptance

1. New sim spec: with `jockey = true`, the controlled defender's displacement
   over 30 frames is ~0.75× the plain-run displacement, and facing tracks the
   ball.
2. New sim spec: a poke released from jockey wins the ball from
   `STAND_REACH + 6` away (where a plain poke misses).
3. Screen specs updated: hold = jockey input, release = dash.
4. `docs/controls.md` documents the stance. `./scripts/check.sh` green.
