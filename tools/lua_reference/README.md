# Reference vectors: pinned cross-language determinism evidence

This directory holds frozen numeric vectors, captured once from this game's
original Lua implementation before it was retired. They are the only
surviving record of that implementation's floating-point behavior, and they
cannot be regenerated: there is no Lua interpreter anywhere in this
repository any more, and no Lua source left to run one against. **A failing
differential test against one of these vectors is a finding about the Rust
or TypeScript code under test — never a stale fixture that needs
refreshing.**

## Why this exists

Passing a unit test proves a module satisfies the assertions someone wrote
down. For determinism-critical modules that is not enough: the simulation's
contract is that two clients produce identical bits, and a test that checks
"the sample is in [0, 1)" would pass on an implementation that is subtly
wrong in the 17th digit. So for anything on the determinism path, this
codebase additionally captured reference values from the real Lua
implementation and compared bit patterns against them. This found nothing
wrong in `rng`'s Rust port — but confirming that took minutes, and it was
the only evidence that actually spoke to the guarantee.

## What's here

- `diagnostics_schema_vectors.txt` — captured `fnv1a64` digest vectors for
  `diagnostics_schema`'s canonical serializer: for each case, the encoded
  bytes (hex) and the resulting digest. Read and asserted directly by
  `gc-netcode/src/diagnostics_schema.rs`'s `shared_vectors_agree_with_lua`
  test, which rebuilds each case's value tree in Rust and checks that
  `encode` and `digest` reproduce the pinned bytes exactly — not merely that
  `fnv1a64` itself agrees, which `gc-core`'s own tests already cover
  separately. The equivalent cross-language guarantee on the TypeScript
  side (`@gc/online`'s `diagnostics_schema.ts`) is covered by
  `diagnostics_schema_crosslang.spec.ts`, which carries its own
  separately-captured cases for the same two historical defects this whole
  exercise exists to catch (see below) rather than reading this file
  directly.
- `research_schema_vectors.txt` — the same shape of evidence, for
  `research_schema`'s canonical serializer/digest. Read and asserted by
  `gc-sim/tests/research_schema_differential.rs`'s
  `shared_vectors_agree_with_lua`. `research_schema` has no TypeScript
  counterpart, so this one is Rust-only.

These two files are what ARCHITECTURE.md §1.2 means by requiring
`fnv1a64`/`diagnostics_schema` to be "pinned by a shared vector file": a
digest computed in Rust on one client and in TypeScript on another must
agree bit-for-bit, because a desync package is evidence peers exchange, and
a hash function duplicated across two languages with nothing pinning them
together is exactly how that stops being true.

The same capture methodology produced roughly eighteen sibling
`*_lua_reference.txt` fixtures elsewhere in this tree
(`rust/crates/*/tests/fixtures/`, `ts/packages/render/src/fixtures/`), each
read directly by a differential test (`include_str!` on the Rust side, a
generated `.ts` module on the TypeScript side), plus one case embedded as a
string literal rather than a checked-in fixture file
(`ts/packages/render/src/pitch.spec.ts` — see
`tools/render_reference/README.md` for that one; it compares draw-command
sequences rather than scalar values, so it needed a different capture
shape). All of it is frozen the same way, for the same reason, and none of
it can be regenerated either.

## What is pinned

Bit-exact agreement on the determinism path: RNG draws, hashing,
fixed-point and series math, input-frame encoding, match-snapshot state and
hashing, rollback bookkeeping, and full session/match runs — anything whose
output crosses the wire, feeds a resim, or is asserted to agree across two
peers.

Not pinned, and not required to be: layout, rendering, diagnostics, lobby
coordination — anywhere a one-ulp difference is invisible rather than a
desync.

## How these vectors were captured (historical)

The Lua implementation ran headlessly under `love` — no display, no
`xvfb`, no sudo required. A module was loaded in isolation, its values
printed with `%.17g`, and stdout captured. `%.17g` was chosen because it
round-trips a `binary64` value exactly, so parsing the text back in Rust or
TypeScript reproduces the identical `f64`.

Comparison was always on **bit patterns, never printed text**, because
formatting differs between languages: in Rust, the captured `%.17g` string
was parsed into an `f64` and compared with `x.to_bits() ==
expected.to_bits()`; in TypeScript, the equivalent was a
`DataView`/`Float64Array` round trip, or `Object.is` after parsing (which
distinguishes `-0` from `0`, unlike `===`).

Vectors deliberately covered the degenerate inputs, because ordinary values
usually agree by construction and divergence hides at the edges: zero,
negatives, values above the modulus, non-integers where Lua floors, empty
collections, and the largest value the type admits. In `rng`, those were
exactly the cases worth checking — `seed(0)`, `seed(-42.9)` and
`seed(2147483647*3 + 0.7)` all took different branches, and a vector set
covering only ordinary seeds would have missed all three.

## The floored-modulo trap

Lua's `%` is a floored modulo; Rust's `%` and TypeScript's `%` are
truncated remainders. The two agree only when both operands are
non-negative. Anything computing a `%` on a possibly-negative value needs
`rem_euclid` (Rust) or an explicit `((a % n) + n) % n` (TypeScript) — or a
proof that the operand cannot be negative. This is exactly the class of
divergence a vector set covering negative inputs catches, and a vector set
covering only positive ones would silently miss.
