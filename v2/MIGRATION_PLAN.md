# v2 migration — agent roster and status

The milestone: **every module and every unit test translated, and the tests pass.**
No browser glue, no wasm bindings, no bundling — that is a separate milestone.

Finish line: `cargo test` green in `v2/rust`, `pnpm test` green in `v2/ts`.

Scale: ~100,600 lines of source and ~56,600 lines of spec. Roughly 70,800 lines to
Rust and 29,800 to TypeScript.

Conventions live in `v2/README.md` and bind every agent.

---

## Waves

Waves exist because of compile dependencies, not scheduling preference. An agent
cannot typecheck against a crate that does not exist yet.

### Wave 1 — foundations *(in progress)*

Everything else compiles against these.

| # | target | Lua sources | src | spec |
| --- | --- | --- | ---: | ---: |
| R1 | `gc-core` | `core/{rng,deterministic_math,fnv1a64}.lua` | 188 | 322 |
| R2 | `gc-data` | `data/**` (incl. the 14,517-line `omp1_determinism` golden fixture → JSON) | 16,990 | 245 |
| T1 | `@gc/core` | `core/{vec2,mat4,quat}.lua` | 367 | — |

### Wave 2 — the simulation, in topological order

`gc-sim` is 37,957 source lines and 29,401 spec lines: the single largest body of
work in the port.

**It must be split by dependency layer, not by theme.** The internal `require`
graph runs the opposite way from intuition — `sim/match.lua` *consumes* `aerial`,
`ai`, `keeper`, `combat`, `offball_runs`, `outfield_*`, `passing`, `placement` and
`possession_transition`. It is the top of the graph, not the bottom. Porting it
first would leave an agent stubbing nineteen modules.

The layers, each depending only on the ones above it:

| # | sub-wave | modules | src | spec |
| --- | --- | --- | ---: | ---: |
| S1 | 2a | `fixed_clock`, `tuning`, `species`, `stats`, `input_frame`, `placement`, `passing`, `rating`, `content_validation`, `metrics`, `network_conditions`, `combat_rules` | ~4,150 | ~2,300 |
| S2 | 2a | `brain`, `ai`, `keeper`, `outfield_decision`, `outfield_press`, `possession_transition`, `offball_runs`, `bot`, `aerial` | ~3,950 | ~2,900 |
| S3 | 2b | `combat_snapshot`, `combat_intent`, `combat_identity`, `combat_feasibility`, `combat_observation`, `combat`, `combat_policy` | ~5,000 | ~2,500 |
| S4 | 2b | `match_snapshot`, `slot_input` | ~2,050 | ~2,350 |
| S5 | 2c | `match.lua` — 5,282 lines, the top of the graph, its own agent | 5,282 | ~4,100 |
| S6 | 2d | `input_tape`, `headless`, `replay`, `determinism_evidence`, `sweep`, `tripwire`, `lever_metrics`, `rating_validation`, `outfield_ai_policy`, `outfield_ai_baseline` | ~3,800 | ~1,000 |
| S7 | 2d | `rollback_*` (7 files) | ~6,370 | ~2,700 |
| S8 | 2d | `research_*` (7), `env*` (5) | ~6,960 | ~3,100 |
| S9 | 2e | the full-match integration specs only — `*_match_spec`, `ai_kick_execution`, `ai_dribble`, `keeper_*`, `ball_bounds`, `dribble` | — | ~5,300 |

S9 ports no source. Those specs drive a whole match through modules other agents
own, so they can only be written once S1–S5 exist.

Running alongside, with no dependency on the sim:

| # | target | Lua sources | src |
| --- | --- | --- | ---: |
| T2 | `@gc/ui` + `@gc/input` | `game/ui/**`, `game/input/**` | 1,393 |
| T3 | `@gc/presentation` | `game/presentation/**` | 1,012 |
| T4 | `@gc/transport` | `game/transport/**` | 3,990 |

### Wave 3 — the render boundary and the netcode

| # | target | Lua sources | src |
| --- | --- | --- | ---: |
| R3 | `gc-render` | `render/**` — `frame`, `frame_buffer`, `identity`, `player_pose` | 1,776 |
| N1 | `gc-netcode` protocol | `protocol`, `input_protocol` + their fixtures and conformance suites | ~3,700 |
| N2 | `gc-netcode` coordinator | `coordinator`, `coordinator_driver`, `live_slot` + fixtures/conformance | ~4,200 |
| N3 | `gc-netcode` driver + fault | `match_driver`, `match_session`, `match_manifest`, `fault_*`, `desync_package` | ~6,000 |

### Wave 4 — the presentation

| # | target | Lua sources | src |
| --- | --- | --- | ---: |
| T5 | `@gc/render` rig | `rig3d/**` — skeleton/body/meshbuilder/shapes replaced by three.js, clips/themes/poses ported | ~3,900 |
| T6 | `@gc/render` scene | `pitch`, `player_renderer*`, `effects`, `bloom`, `arena`, `match_hud`, `combat` | ~3,300 |
| T7 | `@gc/render` camera + replay | `camera`, `camera_follow`, `correction_smoothing`, `replay`, `view_state`, `release_follow`, `benchmark` | ~1,900 |
| T8 | `@gc/screens` core | `lobby_model`, `lobby`, `match`, `online_match*`, `online_lobby`, `real_match` | ~3,800 |
| T9 | `@gc/screens` rest | `squad`, `formation`, `tactic`, `settings`, `result`, `title`, `menu`, `pause`, `help`, `credits`, fixtures | ~1,600 |
| T10 | `@gc/online` | `net_diagnostics`, `diagnostics_schema`, `diagnostic_transport`, `lobby_link`, `match_presentation`, `fault_campaign` + fixture | ~4,000 |
| T11 | `@gc/app` | `game/*.lua` root — app, flow, screen_stack, settings, audio, session, adapters | 4,284 |

---

## Standing rules for every agent

- Work only in `/home/oscar/Coding/galactic-cup-worktrees/v2-migration`.
- Read `v2/README.md` first; it is the contract.
- **Run no git commands.** The orchestrator commits between waves.
- Touch nothing outside your assigned crate or package.
- Use your own scratchpad directory and your own `CARGO_TARGET_DIR`. Agents run
  concurrently and share a scratchpad root; reading another agent's log as your own
  is a real failure mode that has happened before.
- Do not run `pnpm install` — a concurrent lockfile write corrupts the workspace.
  Report a needed dependency instead of adding it.
- Never drop a spec assertion. If one cannot be expressed, port it as
  `#[ignore]` / `it.skip` with a reason and report it.

## Deletions

Roughly 2,900 lines of `game/render/` are hand-written engine features three.js
ships (`skeleton`, `body`, `meshbuilder`, `shapes`, `rig3d/renderer`, `bloom`,
`gl_probe`). Those are deleted rather than translated — but **every deletion must
be named in the agent's report**, with its three.js replacement.

IK and bone masking stay hand-written: three.js's `CCDIKSolver` is example-tier and
MMD-oriented, and `AnimationMixer` blends whole clips rather than masked bones.
With native dropped these get written once instead of twice.

---

## Status

| layer | tests | state |
| --- | ---: | --- |
| `gc-core` | 17 | done — differential-tested against Lua |
| `gc-data` | 9 | done — 7,204 fixture hashes verified as an exact multiset |
| `@gc/core` | 22 | done |
| `@gc/presentation` | 10 | done |
| `gc-sim` S1 primitives | — | in progress |
| `@gc/ui` + `@gc/input` | — | in progress |
| `@gc/transport` | — | in progress |
| `@gc/render` rig3d | — | in progress |

## Carry-forward

Work an agent correctly declined because it belongs to a layer someone else owns.
**Nothing here may be dropped.** Each line names who picks it up.

| item | owner | why it was deferred |
| --- | --- | --- |
| `spec/game/combat_feedback_rollback_spec.lua` | `@gc/screens` (T8) | its subject is the Match screen's rollback consumption, plus `render/effects` and `render/replay` — not presentation |
| pose-priority block of `spec/game/combat_presentation_spec.lua` | `gc-render` (R3) | exercises `render/player_pose.lua`, which is Rust |
| `game/presentation/combat_feedback_fixture.lua` | `@gc/screens` (T8), after `gc-sim` | builds its baseline from `sim.match.new` / `sim.combat.new_state` / `data.teams`; porting it before the sim exists would mean inventing sim output rather than translating it |
| `spec/ui/tuning_panel_spec.lua` "tuning presets data" block (2 assertions) | `@gc/screens` (T9) or a wasm-bridge milestone | validates real preset blobs against the real knob registry, which is `sim/tuning.lua` and `data/tuning_presets.lua` — both Rust. The F4 cycling *mechanism* is still tested against a synthetic registry |
| `spec/game/input_bindings_spec.lua` help-card assertion | `@gc/screens` (T9) | depends on `game/screens/help.lua` |
| add `"@gc/ui": "workspace:*"` to `packages/input/package.json` | orchestrator, at a consolidation point | `controller.lua` calls `game.ui.viewport.to_virtual`; injected as a `ViewportMapper` interface for now because agents must not touch the lockfile concurrently |
| `spec/game/transport_relay_spec.lua` second block, "relay topology probe: no peer is the sequencer" | `gc-netcode` (N3) | drives `coordinator`, `match_driver`, `fault_harness`, `input_protocol`, `live_slot`, `match_manifest` — all Rust |
| `game/transport.lua` (root facade, 62 lines) | **already done** — folded into `@gc/transport`'s `index.ts` | it is a `game/` root file, so `@gc/app` (T11) must NOT port it again |
| `transport_star_spec.lua`'s "keeps browser and WebRTC APIs out of core, data, and sim" | **retired, not deferred** | it scanned the Lua source tree with `love.filesystem`. In v2 the property is enforced by construction: `gc-sim` is a Rust crate and cannot import a TypeScript package at all |

## Findings in the Lua worth revisiting later

Not port defects — things the port surfaced about the original.

- `data/tuning_presets.lua` builds a blob with `table.concat` at load time. That is
  mechanism inside a data file, against AGENTS.md §8.
- In a `combat_feedback` spec fixture, a `target_index` expression evaluates to the
  Lua boolean `true` for the "commit" case, because of an `or`/`and` precedence
  quirk — it reads like a ternary but is not one. No assertion depends on it, so
  the behaviour is latent rather than broken, but the field is typed as a number.
