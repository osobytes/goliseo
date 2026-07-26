# Learning environment (deterministic, player-observable)

Issue: [#138](https://github.com/osobytes/goliseo/issues/138).

This document is the contract record for the pure Lua learning-environment core in
`sim/env*.lua`. It covers what an agent may see, what it may do, how episodes start
and end, which reward channels exist and which may never be optimized, and what a
reproducibility manifest must contain.

## Scope of this record

**Landed here (pure Lua core):**

- `sim/env_config.lua` — versioned, strict-validated episode configuration and the
  canonical config digest.
- `sim/env_observation.lua` — the three observation profiles and the canonical
  observation encoding.
- `sim/env_action.lua` — the abstract action contract, quantization into the
  canonical `InputSample`, and client-knowable legality masks.
- `sim/env_reward.lua` — the named reward channel registry (objective / shaping /
  diagnostic) and per-transition evaluation.
- `sim/env.lua` — `reset` / `observe` / `action_masks` / `step` / `manifest` /
  `tape` / `snapshot`.

**Deliberately not here:** the external tooling bridge (stdio/FFI/socket) and
vectorized batching. Those are transport concerns and live outside `sim/`; see
"Deferred" at the end.

## Layer position

```
sim/env.lua            reset / step orchestration        (pure)
  ├── sim/env_config       episode identity              (pure data validation)
  ├── sim/env_observation  what an agent may see          (pure projection)
  ├── sim/env_action       what an agent may do           (pure quantization)
  └── sim/env_reward       named channels                 (pure scoring)
```

The core requires only other `sim/`, `core/`, and `data/` modules. It never
requires `love`, never touches the filesystem or a socket, and knows nothing about
any learning framework. A bridge process may only call this API; because
`sim.match` remains the sole authority and every action is quantized into a
canonical `InputFrame` before it reaches the simulation, no transport layer can
change simulation authority.

The environment composes the seams that already existed rather than adding new
ones:

| Seam                     | Role in the environment                                |
| ------------------------ | ------------------------------------------------------ |
| `sim.fixed_clock`        | canonical tick numbering and the single simulation `dt` |
| `sim.slot_input`         | materializes the complete effective row for all 8 slots |
| `sim.input_frame`        | the quantized wire form of every action                 |
| `sim.match`              | the only simulation authority                           |
| `sim.match_snapshot`     | boundary hashes and the privileged snapshot             |
| `sim.metrics`            | evaluation diagnostics (#128 fun-proxy family)          |
| `sim.input_tape`         | export of the episode as a canonical tape               |
| `sim.replay`             | independent re-derivation of the boundary hashes        |

`sim/headless.lua`, `sim/match.lua`, `sim/input_tape.lua`, and `sim/replay.lua`
were **not modified**. The environment is additive.

## Configuration and reset

`env.reset(config, tape_frames?)` returns an `EnvInstance` or `nil, err, code`.
Configuration is external input, so every rejection is a recoverable return.

A config names, explicitly: contract version, build/source/content provenance,
fixture label (derived from the teams when omitted), seed, duration, goal cap,
field size, both team ids, optional formations, both tactic ids, whether the
combat companion state is enabled, the observation profile, the eight per-slot
sources, the reward perspective, the objective and shaping channel selections, and
an optional episode tick budget.

Per-slot sources are the multi-agent mapping:

| Kind      | Meaning                                                            |
| --------- | ------------------------------------------------------------------ |
| `policy`  | the caller supplies this slot's action every step (`policy_id` optional, recorded in the manifest) |
| `tape`    | the slot replays a supplied recorded `InputFrame` row (human trace) |
| `bot`     | the deterministic built-in slot bot, seeded per slot               |
| `neutral` | the canonical neutral row                                          |

Any deterministic mix is legal as long as at least one slot is a `policy` slot.
Because sources are per slot, side/role rotations and paired common-seed
evaluation are config edits, not code edits.

## Observation profiles

Three profiles, each explicitly tagged (`env_observation.PROFILES`):

| Profile          | `player_observable` | `human_proxy_valid` | Slots |
| ---------------- | ------------------- | ------------------- | ----- |
| `representative` | true                | true                | exactly 1 |
| `team`           | true                | true                | 1..8  |
| `privileged`     | **false**           | **false**           | 1..8  |

`privileged` exists for oracle, debug, and adversarial-search experiments. It adds
the authoritative RNG state, the canonical `MatchSnapshot` (a deep copy, never a
live reference), and the boundary hash on top of the player-observable body.
`human_proxy_valid = false` is the machine-readable form of the rule that a policy
trained on this profile may not be reported as a human proxy.

`team` is `representative` per slot, side-relative for each slot independently.
Every slot's view is built from the same boundary state before any action for the
next tick exists, so no view can carry another slot's same-tick choice.

An observation is always `{ version, profile, player_observable, human_proxy_valid,
tick, slots, views }` where `views[slot]` is an `EnvSlotView`. Everything in it is
freshly allocated plain data: an observation holds no reference to `MatchState`.

### Field table

Unit conventions: positions and lengths are world pixels on a `field.w x field.h`
pitch; velocities are px/s; times are seconds; tick counts are canonical 60 Hz
ticks. No normalization is applied — normalization is a policy-side choice, and
doing it here would bake a scale into the contract. `geometry` supplies the
denominators (`field_w`, `field_h`, goal rects) a policy needs to normalize.

| Field | Source | Unit / range | Visibility rule | History | Missing |
| ----- | ------ | ------------ | --------------- | ------- | ------- |
| `match.tick` | `state.input_tick` | integer ≥ 0 | public clock | current | never |
| `match.tick_rate` | `fixed_clock.TICK_RATE` | 60 | public constant | static | never |
| `match.phase` | derived | `kickoff`/`live`/`finished` | presented match state | current | never |
| `match.stoppage` | `state.kickoff_hold > 0` | boolean | visible restart hold | current | never |
| `match.time_left` | `state.time_left` | seconds ≥ 0 | scoreboard | current | never |
| `match.score_own` / `score_opponent` | `state.score` | integer | scoreboard | current | never |
| `match.max_goals` | `state.max_goals` | integer | rules, announced | static | never |
| `match.finished` | `state.finished` | boolean | scoreboard | current | never |
| `geometry.field_w` / `field_h` | `state.field` | px | pitch geometry | static | never |
| `geometry.own_goal` / `target_goal` | `state.goal_*` | px rect | pitch geometry | static | never |
| `ball.x/y/z` | `state.ball`, `ball_z` | px | on screen | current | never |
| `ball.vx/vy/vz` | `state.ball_vel`, `ball_vz` | px/s | visible motion | current | never |
| `ball.spin` | `state.ball_spin` | signed | visible curve | current | never |
| `ball.airborne` | `ball_z > 0` | boolean | on screen | current | never |
| `ball.loose` | `state.owner == nil` | boolean | on screen | current | never |
| `ball.owner_slot` | `slot_for_player[owner]` | 1..8 or nil | visible carrier | current | `nil` when loose or keeper-held |
| `ball.owner_side` | derived | `own`/`opponent` | visible carrier | current | `nil` when loose |
| `ball.owner_is_keeper` | `player.is_keeper` | boolean | visible carrier | current | false when loose |
| `own.player_id` | `player.id` | string | own identity | static | never |
| `own.slot` / `side` / `is_keeper` | routing | — | own identity | static | never |
| `own.x/y`, `vx/vy`, `facing_x/y`, `radius` | `player` | px, px/s, unit | own body | current | never |
| `own.move_speed` | `player.move_speed` | px/s | own stat sheet | static | never |
| `own.has_ball` | `state.owner` | boolean | own body | current | never |
| `own.sprinting`, `jockeying`, `sliding`, `tackling`, `dodging`, `stunned`, `diving`, `airborne`, `winding_up`, `charging` | timers → boolean | boolean | own visible pose | current | never |
| `own.sprint_meter`, `charge`, `pass_charge` | `player` | 0..1 | own HUD meter | current | never |
| `own.dash_ready`, `dodge_ready`, `tackle_ready`, `header_ready` | cooldowns == 0 | boolean | own readiness | current | never |
| `own.*_cooldown_s`, `stun_s`, `dodge_s`, `slide_s`, `tackle_s`, `jockey_s`, `windup_s`, `aerial_recovery_s` | `player` timers | seconds ≥ 0 | own readiness readout | current | never |
| `own.pass_preview_slot` / `pass_preview_keeper` | `player.pass_target` | 1..8 / boolean | the presented pass preview, own carries only | current | `nil` unless own player holds the ball |
| `own.equipment.*` | `CombatPlayerState` | see below | own equipment readout | current | `nil` without a loadout |
| `teammates[]` / `opponents[]` | `state.players` | canonical player order | see below | current | keepers report `slot = nil` |
| `events[]` | `state.events`, `combat_state.events` | see below | confirmed, past | previous tick only | empty at reset |

Other players (`teammates` / `opponents`) expose position, velocity, facing,
radius, side, keeper flag, slot (nil for keepers), `has_ball`, and the visible
poses `sprinting`, `jockeying`, `sliding`, `tackling`, `dodging`, `stunned`,
`diving`, `airborne`, `winding_up`, `charging`. They expose **no** remaining timer
values and **no** meters: an opponent's animation is visible, their private clock is
not. Their `equipment` telegraph is `{ family_id, phase, forced_state }` — the
silhouette, the visible animation phase, and a visible stagger — with no
`phase_ticks`, no `cooldown_ticks`, and no `loadout_id`.

`own.equipment` additionally carries `loadout_id`, `phase_ticks`,
`cooldown_ticks`, `ready`, `control_held`, `forced_ticks`, and `immunity_ticks`:
own readiness is exactly what a HUD shows its own player.

`events[]` entries are `{ kind, tick, x, y, actor_slot?, actor_side?, on_target?,
result?, family_id? }`. `state.events` is cleared at the top of every
`match.step`, so an observation's events always belong to the tick that already
completed and `tick` is always strictly less than the boundary tick. Presentation
detail that has no player-legible analogue (`difficulty`, `save_style`, aerial
`style`/`outcome`, `keeper_state`, `keeper_depth`, `source_sequence`) is withheld
from the observation and available only in the step result's evaluation events.

### What is deliberately excluded

`state.rng`, raw `MatchState`, any `MatchSnapshot`, `save_pending`, `save_vx`,
`save_timer`, `save_style`, `windup_shot` payloads, `outfield_decision` (AI
cadence/intent), other players' `pass_target`, `marks`, `marking`, `press`,
`outfield_press`, `keeper_release_*`, `aerial_outcome`/`aerial_style`, any future
tick, any same-tick input from any slot, and any speculative or
rollback-presentation event.

Where the current 2D game shows the whole pitch, "observable" still means
*presented and current*: the observation may name where a player is and what pose
they are in, never what they intend, what the resolver already decided, or what
happens next.

`spec/sim/env_leakage_spec.lua` asserts each of these, and pairs the negative
assertion with a positive control on the `privileged` profile so a scan that stops
working fails instead of passing silently.

## Action contract

An `EnvSlotAction` is `{ version?, move?, held?, edges? }`:

- `move` — continuous `{ x, y }` inside the unit disc. Quantized through
  `input_frame.quantize_move` (7-bit signed per axis), exactly like a recorded
  human row. `nil` means "still".
- `held` — `shoot`, `pass`, `sprint`, `jockey`, `lob`, `aerial_strike`,
  `aerial_acrobatic`, `equipment`. Held for this tick.
- `edges` — `shoot`, `pass`, `switch`, `dash`, `dodge`, `equipment_pressed`,
  `equipment_released`. One-shot, this tick only.

Combat family intents are expressed exclusively through `equipment` /
`equipment_pressed` / `equipment_released`; the family that responds is the one the
slot's loadout grants, so `unarmed`, `guard`, `light_melee`, and `ranged` all share
one intent surface. A policy cannot select a family, a phase, a contact, or an
outcome.

**Player switch** does not exist under fixed-slot routing: a slot owns one player
for the whole fixture. `sim.match` already ignores `switch` in slot mode; the
contract makes that explicit by masking it out and rejecting it with a reason
rather than silently dropping it.

**Neutral / no-op** is `env_action.neutral()` — zero move, no held, no edges. It
quantizes to `input_frame.neutral_sample()`.

Illegal actions return `nil, err, code` with codes `malformed`,
`move_out_of_range`, `unknown_held_action`, `unknown_edge_action`, and
`unavailable_action`. `env.step` prefixes the slot and surfaces the reason.

### Legality masks

`env_action.mask(view)` derives legality **from an observation**, not from
`MatchState`. Because a representative view contains no private state, the mask
cannot encode privileged legality by construction. A mask built from the
`privileged` profile carries `privileged = true`.

| Intent | Gate (all from the view) |
| ------ | ------------------------ |
| `move` | always |
| `held.shoot/pass/sprint/jockey/lob` | always |
| `held.aerial_strike` / `aerial_acrobatic` | `own.header_ready` and `ball.airborne` and not `own.stunned` |
| `held.equipment` | the slot has a loadout |
| `edges.shoot` / `pass` | always |
| `edges.switch` | never (fixed-slot routing) |
| `edges.dash` | `own.tackle_ready` and not `own.stunned` |
| `edges.dodge` | `own.dodge_ready` and not `own.stunned` |
| `edges.equipment_pressed` | `own.equipment.ready` (ready phase, no cooldown, not staggered) |
| `edges.equipment_released` | the slot has a loadout |

The mask is advisory about *availability*; the simulation remains the authority
about *effect*. Cooldowns, recovery, windups, and phase gating are enforced by
`sim.match` and `sim.combat` regardless of what the mask said.

## Step semantics

`env.step(instance, actions, ticks?)`:

1. Refuses if the instance is faulted (`faulted`) or the episode already ended
   (`episode_over`).
2. Requires an action for **every** controlled slot at once (`missing_slot`) and
   rejects actions for slots it does not own (`unknown_slot`). This is how
   simultaneity is enforced: there is no interleaving point at which one policy
   could observe another's choice for the same tick.
3. Validates each action and checks it against the mask taken at the boundary
   where the action was chosen (`illegal_action`).
4. For each of `ticks` canonical ticks (default 1): builds the base frame (policy
   rows from actions, tape rows from the supplied recording, other rows left for
   the producer), materializes the complete effective row for all eight slots
   through `slot_input.materialize`, and advances exactly one tick via
   `fixed_clock.step` → `match.step(state, TICK_SECONDS, frame, combat_state)`
   followed by `metrics.observe`.
5. Records the effective frame, the boundary hash, and the encoded wire row.

**Action repeat**: when `ticks > 1`, held intents persist across every tick while
one-shot edges fire only on the first. An edge means "this tick only"; repeating it
would fabricate presses the policy never made.

**Termination vs truncation** are separate fields and are never both true:

| Outcome | Field | Reason |
| ------- | ----- | ------ |
| regulation time ran out | `terminated` | `time_expired` |
| a side reached the goal cap | `terminated` | `goal_cap` |
| episode tick budget spent | `truncated` | `step_limit` |
| a tape slot ran out of rows | `truncated` | `tape_exhausted` |

**Stoppage** is neither: `stoppage` / `stoppage_reason = "kickoff_hold"` reports
that play is held at a restart while the episode continues.

**Invalid action** never advances the boundary. **Simulation fault**: the tick is
run under `pcall`, and an invariant violation inside `sim.match` is returned as
`nil, err, "sim_fault"` naming the tick, the boundary hash before it, and the exact
encoded input row — a reproduction recipe. The instance latches faulted and every
later step returns `faulted`. Nothing is swallowed: the assert message is part of
the returned error.

**Evaluation data** rides along with every step: `events` (full-fidelity confirmed
`MatchEvent` / `CombatEvent` records with the tick and acting side) and
`diagnostics` (`role = "evaluation"`, per-step tick and event counts, and the
registered `sim.metrics` `MatchMetrics` once the episode ends). Diagnostics are
never a reward.

## Reward boundary

`sim/env_reward.lua` is a registry, not a formula. Every channel declares a role:

| Channel | Role | Meaning |
| ------- | ---- | ------- |
| `match_outcome` | objective | +1 win / -1 loss / 0 draw, paid once at termination |
| `goal_scored` | objective | +1 per goal for the reward team |
| `goal_conceded` | objective | -1 per goal against |
| `goal_difference_delta` | objective | change in goal difference |
| `possession_gain` | shaping | +1 gaining the ball, -1 losing it |
| `shot_attempt` | shaping | +1 per confirmed own shot/header/volley/bicycle |
| `equipment_contact` | shaping | +1 per unguarded combat contact landed |
| `experience_proxy_metrics` | diagnostic | the #128 fun-proxy metric family, evaluation only |

Rules enforced in code and tested:

- **No channel may be named `fun`.** `env_reward.FORBIDDEN_CHANNEL_ID` is asserted
  at module load, and the spec additionally rejects any id containing "fun".
- Diagnostic channels are not optimizable. Selecting
  `experience_proxy_metrics` as an objective or a shaping term is a config
  validation error (`wrong_role`). The #128 metrics are returned as evaluation
  data, never as a default reward.
- Objective and shaping totals are reported separately (`objective_total`,
  `shaping_total`, `total`). An ablation is a subtraction, not a rerun: the same
  transition evaluated without the shaping selection yields identical objectives
  and an empty shaping table.
- Shaping channels are preregistered in the registry, so a run cannot invent one.
- Imitation loss is not here at all: it belongs to training code, which has the
  tape and the observations it needs.
- The default selection is `{ "match_outcome" }` and nothing else.

Rewards are expressed from `config.reward_team`, so a self-play population scores
both sides from the same registry.

## Reproducibility

`env.manifest(instance)` returns every input a reproduction needs:

- contract versions: env, config, observation, action, reward, input frame,
  snapshot, tape;
- `tick_rate`;
- provenance: `build`, `source`, `content`, `fixture`;
- the canonical `config` digest (`env_config.digest`) and the serialized `tuning`
  blob;
- `seed`, and the `combat` mechanics identity when combat is enabled;
- `ownership` (the canonical slot → player assignment), `observation_profile`,
  `controlled_slots`, `policy_ids`, and the per-slot `slot_sources`;
- `objective_channels`, `shaping_channels`, and the `diagnostic_channel`;
- `initial_boundary_hash`, the latest `boundary_hash`, and `episode_ticks`.

`env.tape(instance)` exports the episode as a canonical `InputTape` built from the
reset snapshot, the effective rows the simulation consumed, and the environment's
own boundary hashes. `sim.replay` then re-derives those hashes independently, which
is the equivalence proof: `spec/sim/env_spec.lua` asserts the same hashes from
(a) the environment, (b) a plain `match.new` + `match.step` loop fed the same rows,
and (c) `replay.run` on the exported tape.

`env.snapshot(instance)` returns the canonical `MatchSnapshot` for save/checkpoint
use. Restore is **not** landed in this change (see Deferred).

## Worked examples

### Minimal scripted / random policy

```lua
local env = require("sim.env")
local env_action = require("sim.env_action")
local rng = require("core.rng")

local config = assert(env.reference_config("soccer_only", {
    build = "example-scripted-v1",
    seed = 1234,
    duration = 20,
}))
local instance = assert(env.reset(config))
local slot = instance.controlled_slots[1]
local state = rng.seed(1234)

-- A deterministic "chase the ball, challenge when close" policy. It reads only the
-- representative view and only proposes intents the mask allows.
while not (instance.terminated or instance.truncated) do
    local view = assert(env.observe(instance).views[slot])
    local mask = assert(env_action.mask(view))
    local dx, dy = view.ball.x - view.own.x, view.ball.y - view.own.y
    local length = math.sqrt(dx * dx + dy * dy)
    local move = length > 1 and { x = dx / length, y = dy / length } or { x = 0, y = 0 }
    local roll
    state, roll = rng.roll(state)
    local action = {
        move = move,
        held = { sprint = length > 100 },
        edges = { dash = mask.edges.dash and length < 34 and roll < 0.2 or false },
    }
    local result, err = env.step(instance, { [slot] = action })
    if not result then
        error(err) -- a rejected action is a bug in the policy, not in the sim
    end
end

local manifest = env.manifest(instance)
print(manifest.boundary_hash, assert(env.tape(instance)).identity.config)
```

Replacing the vector above with `env_action.neutral()` or with a uniformly random
`move` plus mask-filtered edges gives a random policy; nothing else changes.

### Tape-replay policy (human trace as the opponent, or as the policy)

```lua
local env = require("sim.env")
local input_frame = require("sim.input_frame")
local env_action = require("sim.env_action")

-- `recorded` is an InputTape produced by a human session: contiguous frames from
-- tick zero. Slot 5 replays the recording; slot 1 is the policy under test.
local config = assert(env.reference_config("soccer_only", {
    build = "example-tape-replay-v1",
    seed = 77,
}))
config.slot_sources[5] = { kind = "tape" }
local instance = assert(env.reset(config, recorded.frames))

-- A pure tape-replay policy: drive the controlled slot from the same recording,
-- decoding the canonical row back into the abstract action contract.
local slot = instance.controlled_slots[1]
for index = 1, #recorded.frames do
    local row = recorded.frames[index].slots[slot]
    local action = assert(env_action.from_sample(row))
    local result = assert(env.step(instance, { [slot] = action }))
    if result.terminated or result.truncated then
        break
    end
end

-- The environment's boundary hashes must equal the recording's.
for i, hash in ipairs(instance.boundary_hashes) do
    assert(hash == recorded.boundary_hashes[i], "replay divergence at boundary " .. i)
end
```

An incomplete recording truncates the episode with `tape_exhausted` instead of
faulting, so a short human trace is a legal, clearly labelled episode end.

### Reproducibility manifest

Record the manifest next to any result. Two runs reproduce when these agree:

```
env_version, config_version, observation_version, action_version, reward_version,
input_version, snapshot_version, tape_version, tick_rate
build, source, content, fixture
config   (the env_config.digest string)
tuning   (tuning.serialize())
seed, combat
ownership, observation_profile, controlled_slots, policy_ids, slot_sources
objective_channels, shaping_channels, diagnostic_channel
initial_boundary_hash, boundary_hash, episode_ticks
```

Plus the action tape itself (`env.tape`), which pins the trajectory exactly.

## Reference environments

`env.reference_config(id, overrides?)` provides two synthetic reference fixtures:

- `soccer_only` — combat companion state disabled; soccer rules only.
- `combat_all_families` — combat enabled on the `nebula` vs `orion` fixture, whose
  four outfield loadouts per side cover `unarmed`, `guard`, `light_melee`, and
  `ranged`. `spec/sim/env_spec.lua` asserts all four families appear in the
  observation and that each leaves the `ready` phase through the shared equipment
  intent.

## Deferred to a follow-up

The following acceptance items from #138 are intentionally **not** in this change,
because they are transport and lifecycle concerns rather than contract concerns:

1. **External tooling bridge** (stdio / FFI / socket) — must live outside `sim/`.
   The core is already the complete surface it would serialize.
2. **Vectorized batching** of many environments — an outside-`sim/` scheduler over
   independent `EnvInstance` values.
3. **Save/restore**: `env.snapshot` exists; `env.restore` does not. Restoring
   mid-episode has to re-establish tape continuity, the metrics collector, and the
   boundary-hash chain so that a restored episode is provably indistinguishable
   from an uninterrupted one, and it has to prove it cannot leak holdout outcomes.
   That deserves its own tests.
4. **Deterministic action latency/noise wrappers** for human-proxy experiments —
   these belong in a wrapper module over `env.step`, and must not change the base
   environment.
5. **Headless-path hash equality**: `sim/headless.lua` reports metrics, not
   boundary hashes, so this change asserts hash equality across the direct sim, the
   environment, and replay. Extending `headless` to emit boundary hashes would mean
   changing a module this change deliberately leaves alone.
