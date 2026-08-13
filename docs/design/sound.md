# Design: Sound

> **Pre-port record (LÖVE/Lua), kept as history.** Everything below was written
> against the Lua tree on LÖVE that commit `2c0d449` (#467) deleted when the
> Rust + TypeScript port reached parity. Its file paths, module names, commands
> and measurements describe that tree: they are accurate for the work they
> record and **name nothing you can open or run today**. The live tree is
> `rust/crates/gc-*` and `ts/packages/*` — see `ARCHITECTURE.md`.

## Why

Audio is half of game feel, and the simulation already emits an event for
every discrete action. Synthesized match sound is one of the cheapest
high-impact presentation improvements.

## Player-facing behavior

- Every sim event has a short synthesized sound: pass (soft tick), touch,
  shot (low thump), tackle (thud), block, catch/claim (slap), parry (sharp),
  header (flick), volley (heavy thump), goal (rising two-note + noise swell),
  kickoff (whistle).
- A quiet looping crowd bed (filtered noise) under the match; swells briefly on
  goals.
- `M` toggles mute (screen key handler; document in `docs/controls.md`).

## Mechanics

- New module `game/audio.lua` (impure, game layer):
  - **No asset files.** Synthesize everything at load with
    `love.sound.newSoundData(samples, rate, bits, channels)` + `setSample` —
    sines/triangles with exponential decay for tonal hits, white noise bursts
    for thuds/crowd. Keep each SFX ≤ 0.4s at 22050 Hz mono. Build
    `love.audio.newSource(data, "static")` per SFX; clone on play so hits can
    overlap.
  - API: `audio.load()`, `audio.update(state, dt)` (drains `state.events`,
    detects score changes for the goal sound, keeps the crowd loop alive),
    `audio.reset()`, `audio.toggle_mute()`.
  - **Headless guard**: tests run with `t.modules.audio = false` and
    `t.modules.sound = false` — every entry point must no-op cleanly when
    `love.audio` or `love.sound` is nil. This is the module's contract, not an
    afterthought.
- `game/screens/match.lua`: call `audio.load()` lazily, `audio.update(state,
  dt)` after `effects.update`, `audio.reset()` in `restart()`, and handle the
  `m` key in `event()`.

## Watch out for

- AGENTS.md layering: audio lives in `game/`, reads sim state, never the other
  way. Do not add fields to MatchState.
- Volume discipline: SFX peak ~0.5, crowd bed ~0.08 — this is ambience, not a
  slot machine.
- Add a headless spec: with `love.audio == nil`, `audio.load/update/reset`
  run without error over a real match state (mirrors `draw_smoke_spec`'s
  stubbing pattern).

## Acceptance

1. Headless no-op spec green; full `./scripts/check.sh` green.
2. Every event kind in `MatchEvent` maps to a sound; goal + kickoff covered
   via score/state edges.
3. Mute toggle works and is documented.
