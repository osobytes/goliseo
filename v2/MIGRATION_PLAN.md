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

### Wave 2 — the simulation, and the TS layers with no math dependency

`gc-sim` is 37,957 source lines and 29,401 spec lines: the single largest body of
work in the port, split by domain. `sim/match.lua` alone is 5,282 lines and gets
its own agent.

| # | target | Lua sources | src |
| --- | --- | --- | ---: |
| S1 | `gc-sim` foundation | `fixed_clock`, `input_frame`, `input_tape`, `slot_input`, `stats`, `species`, `tuning`, `placement`, `metrics`, `tripwire`, `rating`, `rating_validation`, `content_validation` | ~4,000 |
| S2 | `gc-sim` match core | `match.lua`, `match_snapshot.lua` | 6,950 |
| S3 | `gc-sim` match physics | `aerial`, `keeper`, `passing`, `possession_transition`, `offball_runs`, `replay`, `headless` | ~2,900 |
| S4 | `gc-sim` AI | `ai`, `bot`, `brain`, `outfield_ai_baseline`, `outfield_ai_policy`, `outfield_decision`, `outfield_press` | ~2,800 |
| S5 | `gc-sim` combat | `combat*.lua` (9 files) | ~5,000 |
| S6 | `gc-sim` rollback | `rollback_*.lua` (7 files), `network_conditions` | ~6,400 |
| S7 | `gc-sim` research/env | `research_*.lua` (7), `env*.lua` (5), `determinism_evidence`, `sweep`, `lever_metrics` | ~7,000 |
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
