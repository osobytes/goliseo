# OMP-1 fixed simulation tick

> **Module names below are pre-port `require` paths.** The contract this document
> describes is current; the way it names files is not. Read `sim.foo` as
> `gc_sim::foo` (`rust/crates/gc-sim/src/foo.rs`), `game.online.foo` as
> `gc_netcode::foo`, `core.foo` as `gc_core::foo`, `data.foo` as `gc_data::foo`,
> and any `game/**` or `spec/**` path as its `ts/packages/**` counterpart. A
> `love .` command, a `love.*` API or a love.js measurement is **pre-port
> evidence** — commit `2c0d449` (#467) deleted that tree.

Live matches advance on exactly 60 simulation ticks per second. Rendering,
input sampling, audio ambience, replay playback, and renderer-owned effects
continue on the display's normal render cadence; none of them may pass a
render-frame delta into `sim.match.step`.

`sim.fixed_clock` is the simulation-facing authority:

- `TICK_RATE` is `60` and `TICK_SECONDS` is exactly `1 / 60`.
- `FixedClockState.tick` is the next tick to simulate. It starts at `0`, and
  every input provided to the clock is consumed exactly once with that tick
  number before the counter advances.
- `advance(clock, render_dt, input_for_tick, step)` accumulates render time and
  calls `step(tick, input)` only with canonical ticks. `input_for_tick(tick)`
  is intentionally tick-indexed, so it is compatible with the versioned
  [`InputFrame`](input_frame.md) contract from #34.
- `step(clock, input, step)` runs one exact tick directly. The headless runner
  uses this same API instead of maintaining a separate fixed-delta loop.

## Catch-up policy

One render update may run at most eight simulation ticks. If its accumulated
time contains more whole ticks than that budget allows, the clock runs eight,
discards the remaining *whole* tick debt, and retains the fractional remainder
below one tick. It records both the overload count and discarded-tick count.

This is deliberate: under a long frame the match slows rather than building an
unbounded catch-up queue or silently converting the elapsed render time into a
variable simulation step. The policy is part of the deterministic timing
contract and has direct unit coverage.

## Offline input bridge

The current showcase path still has one locally controlled player, so
`game.match_input_adapter` turns render-sampled controls into one legacy
`MatchInput` per fixed tick. It retains release/action edges across zero-tick
render updates, emits those edges only on the first tick of a catch-up batch,
and keeps holds live on every tick. Equipment uses a held signal plus distinct
press and release edges. The adapter folds render-rate button chatter into the
six canonical transitions defined by the combat contract, including an ordered
press-then-release pair for a complete tap between ticks.

This is a compatibility bridge, not an ownership model. Issue #36 replaces it
with the eight stable outfield streams in `InputFrame`; the fixed clock will
continue to call its tick-indexed input provider unchanged. No transport,
prediction, rollback, snapshots, or replay hashing is introduced here.
