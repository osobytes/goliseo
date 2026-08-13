# Design: Pass-target preview

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

## Why

The receiver of a pass is chosen invisibly at release, so every pass is an act
of faith — most of the reported "aim/meter mistrust". FIFA-style: show WHO will
receive while the pass button is held.

## Player-facing behavior

- While the controlled player holds K (charging a pass) — outfielder or keeper —
  a subtle team-colored marker appears under the teammate who would receive if
  released *this frame*. It live-updates as aim (held direction) and charge
  (range) change.
- No marker when idle or when nobody would be targeted.

## Mechanics

- Refactor, do not duplicate: extract the receiver-selection logic out of
  `try_pass` (cone → safety filter → range → cross override → open-fallback)
  into a pure function, e.g.
  `select_pass_target(s, owner_idx, lofted, aim, range) -> player_index?`,
  and the same for `keeper_throw`'s aim/safety scoring
  (`select_throw_target(s, keeper_idx, range, aim) -> player_index?`).
  `try_pass` / `keeper_throw` call them; behavior must be bit-identical
  (existing specs are the proof).
- New `MatchPlayer` field `pass_target integer?` (annotate), beside that
  player's `pass_charge`. In `update_ball`, while an input-owned player carries:
  - carrying outfielder with `input.pass_held` → recompute via
    `select_pass_target` with the live aim (`input.move` or facing), the
    carrier's current `pass_charge` range, and `input.lob`;
  - human keeper with `input.pass_held` → via `select_throw_target`;
  - otherwise `player.pass_target = nil`. Also nil it on release/loss.
- Renderer (`game/render/pitch.lua`): after entities, if the selected player's
  `pass_target` is set, draw a small pulsing double-ring at that preview target
  (use `love.timer` if available, else static — the smoke test stubs graphics,
  so guard any `love.timer` access).

## Watch out for

- Determinism: the preview must be a pure recompute (no RNG draws — do NOT
  touch `s.rng`).
- The preview must equal the actual receiver: add a spec that charges for N
  frames, records the carrier's `pass_target`, releases, and asserts the
  recorded index gets `receive_timer > 0`.
- Update `draw_smoke_spec` to exercise the marker path
  (`s.players[s.controlled].pass_target = 2`).

## Acceptance

1. Spec: preview equals the actual receiver (outfielder case and keeper case).
2. Spec: `pass_target` is nil when not charging.
3. Existing pass/throw specs unchanged and green (refactor is behavior-neutral).
4. `./scripts/check.sh` green.
