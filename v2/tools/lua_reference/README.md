# Differential testing against the Lua original

Porting a spec proves the port satisfies the *assertions someone wrote down*.
For determinism-critical modules that is not enough: the sim's contract is that
two clients produce identical bits, and a spec that checks "sample is in [0, 1)"
would pass on a port that is subtly wrong in the 17th digit.

So for anything on the determinism path, also **capture reference values from the
real Lua implementation and compare bit patterns.** This found nothing wrong in
`core/rng.lua`'s port — but confirming that took minutes, and it is the only
evidence that actually speaks to the guarantee.

## How

The Lua tree still runs. `love` is on `PATH` and works headless — no display, no
`xvfb`, no sudo.

1. Make a scratch directory **inside your own scratchpad** and copy in the Lua
   layers you need (`core/`, `sim/`, `data/`, ...) from the worktree root.

2. Drop in a `conf.lua` that disables every device module:

   ```lua
   function love.conf(t)
       t.window = false
       t.modules.window = false
       t.modules.graphics = false
       t.modules.audio = false
   end
   ```

3. Drop in a `main.lua` that requires your module, prints values, and quits:

   ```lua
   function love.load()
       local m = require("sim.your_module")
       -- Print floats with %.17g. It round-trips binary64 exactly, so parsing
       -- the text back in Rust or TypeScript gives the identical f64.
       print(string.format("%.17g", m.compute(1, 2)))
       love.event.quit(0)
   end
   ```

4. Run `love .` and capture stdout.

5. Compare **bit patterns**, not printed text — formatting differs between
   languages. In Rust parse the captured `%.17g` string into an `f64` and compare
   `x.to_bits() == expected.to_bits()`. In TypeScript compare via a
   `DataView`/`Float64Array` round trip, or `Object.is` after parsing (which
   distinguishes `-0` from `0`, unlike `===`).

## When it is worth doing

Required for: RNG, hashing, fixed-point/series math, input-frame encoding, state
snapshots and hashing, anything whose output crosses the wire or feeds a resim.

Not required for: layout, rendering, diagnostics, lobby coordination — anywhere a
one-ulp difference is invisible rather than a desync.

## Cover the degenerate inputs

Ordinary values usually agree by construction. Divergence hides at the edges, so
include: zero, negatives, values above the modulus, non-integers where the Lua
floors, empty collections, and the largest value the type admits. In `core/rng.lua`
those were exactly the cases worth checking — `seed(0)`, `seed(-42.9)` and
`seed(2147483647*3 + 0.7)` all take different branches.

## Beware of a trap

Lua's `%` is a floored modulo; Rust's `%` and TypeScript's `%` are truncated
remainders. They agree only when both operands are non-negative. Any port of a
`%` on a possibly-negative value needs `rem_euclid` (Rust) or an explicit
`((a % n) + n) % n` (TypeScript) — or a proof that the operand cannot be negative.
