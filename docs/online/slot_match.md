# OMP-1 fixed-slot match adapter

> **Module names below are pre-port `require` paths.** The contract this document
> describes is current; the way it names files is not. Read `sim.foo` as
> `gc_sim::foo` (`rust/crates/gc-sim/src/foo.rs`), `game.online.foo` as
> `gc_netcode::foo`, `core.foo` as `gc_core::foo`, `data.foo` as `gc_data::foo`,
> and any `game/**` or `spec/**` path as its `ts/packages/**` counterpart. A
> `love .` command, a `love.*` API or a love.js measurement is **pre-port
> evidence** — commit `2c0d449` (#467) deleted that tree.

`sim.match` has a slot mode for an `InputOwnership` fixture. Its state retains
the validated ownership record, `slot_players` (canonical frame index to match
player index), the inverse `slot_for_player`, and the next expected input tick.
Those values are fixture diagnostics: they do not change after kickoff, a goal,
a turnover, an aerial contact, or a legacy `controlled` metadata change.

In slot mode `match.step` accepts only a valid, complete `InputFrame` whose
tick matches `input_tick`. A legacy `MatchInput` is rejected loudly. Every
outfielder is routed from its permanent slot; neither possession nor selected
player state participates in the routing. Both keepers have no slot and stay
under deterministic keeper AI.

Controller-specific action state follows the assigned player rather than the
match. Each `MatchPlayer` owns its shot/punt charge, pass-range charge,
pass-target preview, wind-up payload/timer, tackle and dodge cooldowns, and
other action recovery. Only the current carrier can accumulate a shot or pass
charge, and losing possession clears that player's charge, preview, and
pending wind-up before another owner acts. Complete frame rows are still
consumed independently in canonical player order, so one slot's simultaneous
hold or release cannot overwrite another slot's movement, tackle, dodge, or
aerial intent.

The legacy `controlled` index remains offline presentation/input and legacy
metrics metadata. Slot mode ignores its switch edge and never changes it for a
pass, turnover, aerial assist, keeper possession, or kickoff. The immutable
`slot_players` and `slot_for_player` tables are the only slot-routing authority.
Slot-mode aggregate metrics retain the existing single-proxy `controlled`
label; they are not per-slot ownership metrics.

## Explicit empty-slot policy

Every frame still carries all eight samples. The fixture also records one
source for each canonical slot in an **upstream producer state**, beside the
tape or adapter rather than inside `MatchState`:

- `frame` consumes the corresponding recorded `InputFrame` sample;
- `bot` replaces that row with the deterministic bot stream seeded by that
  source's required integer `seed`; and
- `neutral` replaces it with the canonical neutral sample.

Only a slot explicitly configured as `bot` receives bot behavior. Each bot
owns its own RNG state, so changing one bot's decisions does not consume or
perturb another slot's seed stream. A finite integer bot seed is canonicalized
to its Park–Miller state when the producer is configured; `frame` and `neutral`
sources cannot carry a seed.

`slot_input.materialize(producer, state, base_frame)` produces the complete
effective frame for one tick. Frame rows are copied, neutral rows are rewritten
to canonical neutral input, and bot `MatchInput`s are round-tripped through
`to_sample` before being inserted. `sim.match` then consumes every effective
row uniformly and holds no source policy or bot RNG state. The effective frame
is therefore the record/replay artifact, not an implementation detail hidden in
the match.

The conversion carries `equipment_held`, `equipment_pressed`, and
`equipment_released` for every human, recorded, neutral, and bot row. Neutral
and current deterministic bot producers emit all three as false; choosing when
AI should use equipment remains downstream behavior work rather than an input
schema decision.

## Offline showcase adapter

The showcase product screen remains on its existing fixed-clock, legacy-input
path. Its render-side `MatchInput` adapter is deliberately separate from the
fixed-slot simulation boundary while the per-player mode and presentation work
is defined later. The headless harness also preserves that legacy `MatchInput`
path whenever both `frames` and `slot_sources` are omitted; this is the path
the frozen gameplay baselines in CI are recorded and verified on.

`game.match_input_adapter` is the explicit offline compatibility adapter. If a
later producer converts one of its tick `MatchInput`s with
`slot_input.to_sample`, the canonical frame's `lob` held bit preserves the
adapter's effective modifier intent: it may remain set on the shoot/pass release
tick when the render-side latch pairs a just-released action with its preceding
loft modifier. The same conversion records equipment held, press, and release
without deriving transitions from adjacent frames. The simulation consumes
those recorded tick values directly.
Offline switching, pass-follow control, cross/aerial assistance, and temporary
human keeper distribution remain enabled only outside slot mode; those rules
select which player's own action state the single legacy input drives.
Slot-mode heavy-touch losses also cancel the former carrier's pending wind-up
at that tick boundary. The offline compatibility path intentionally does not
add this new cancellation to legacy match AI, whose historical heavy-touch
outcomes remain pinned by the frozen gameplay baselines; existing tackle and
smother wind-up cancellations are unchanged in both modes.

## Headless recordings

`headless.run_match({ frames = complete_recording })` treats a complete tape as
the source for all eight rows when `slot_sources` is omitted. Callers that need
recorded/bot or recorded/neutral mixtures must provide the explicit producer
policy. Providing either `frames` or `slot_sources` opts into fixed-slot mode.
When only `slot_sources` is supplied, every base row starts neutral and the
producer applies exactly the declared frame, bot, and neutral sources—there is
no implicit human-proxy or bot injection. This keeps offline legacy behavior
and fixed-slot recording behavior independently reproducible.
