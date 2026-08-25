# Reference vectors: pinned determinism evidence

This directory, and roughly eighteen sibling fixtures elsewhere in the tree,
hold frozen numeric vectors captured once from this game's original Lua
implementation before it was retired. None of them can be regenerated: there
is no Lua interpreter anywhere in this repository any more, and no Lua source
left to run one against.

They are **not all the same kind of evidence**, and until #500 this file said
they were. That mistake had a cost, so the split comes first.

| | **Encoding vectors** | **Behavioral vectors** |
| --- | --- | --- |
| Files | the two `*_schema_vectors.txt` in this directory | the `*_lua_reference.txt` fixtures listed below |
| What they pin | canonical serializer bytes and digests that **Rust and TypeScript must both reproduce today** | one retired implementation's per-tick simulation behavior, **bugs included** |
| Is the thing they pin still alive? | yes — two live implementations must agree with each other | no — the implementation they describe is deleted |
| Rule when one goes red | **always a defect. Never refresh. No exceptions.** | a defect *unless* a deliberate, reviewed change supersedes it — see below |

The Lua origin of the encoding vectors is incidental. Those bytes would have
to be reproduced by both languages even if Lua had never existed. The
behavioral vectors are the opposite: their entire content is "what Lua did",
and Lua is gone.

**A third axis, found by #536: "not trajectory-coupled" does not mean
"immune to a schema change."** The table above has one row for whether a
vector's evidence is still alive, and the behavioral half of this file has
one axis — trajectory-coupled vs. not — for whether a *gameplay* change can
legitimately retire it (§2 below). Neither asks the question that matters for
a *wire-schema* change: does this vector's captured bytes embed a version
word, or hash something that does? `match_snapshot_case_a_lua_reference.txt`,
`match_snapshot_case_b_lua_reference.txt`, `match_driver_lua_reference.txt`'s
boundary-zero digest, and `input_tape_lua_reference.txt`'s
`identity.snapshot_version` and `boundary_hash[0]` were all captured from
**zero-tick** or **pre-step** scenarios — genuinely not trajectory-coupled,
exactly as documented — and every one of them still went red the first time
`match_snapshot::VERSION` moved, because the version integer is on the wire
and the hash of anything containing it moves with it. Filing a vector as
"behavioral" by filename (`*_lua_reference.txt`) said nothing about this;
being trajectory-independent was mistaken for being schema-independent, and
that gap stood undetected because `match_snapshot::VERSION` had not moved
since Lua was deleted in `2c0d449` — until #531's phase 1 moved it for the
first time and all four went red in the same PR.

This is **not** a third bucket beside encoding/behavioral — a schema-coupled
vector is still behavioral (Lua's implementation is still what it recorded,
and the same retirement procedure in §2 still applies to it), and it is still
capturable at zero ticks (so §2's trajectory-coupled/not split still applies
independently). It is an orthogonal question worth asking explicitly before
trusting "not trajectory-coupled" to mean "will survive a schema bump":

> **Does this vector's captured bytes contain a version word, or hash
> something that does?** If yes, it is schema-coupled — a change to
> `match_snapshot::VERSION`, `combat_snapshot::VERSION`, `input_frame::VERSION`
> or `input_tape::VERSION` will redden it even with zero simulation ticks
> involved, and that is a legitimate, no-fault-of-the-code reason to retire
> it under §2's procedure, not evidence the vector was miscategorized as
> behavioral in the first place.

The four vectors above were the ones known to be schema-coupled when #536
retired them — see `gc-sim/tests/match_snapshot_differential.rs`,
`gc-netcode/tests/match_driver.rs`, and `gc-sim/tests/input_tape_differential.rs`
for the per-file account, and the retirement table in §2 below for the
paperwork. A single #489 `match_snapshot::COMBAT_VERSION` bump then found
three more, all in `gc-netcode`, none previously known to be schema-coupled:
`coordinator_desync_lua_reference.txt`'s `transcript_id` line,
`desync_package_identity_vector.txt`'s `manifest_id` field, and
`protocol_lua_reference.txt`'s manifest-carrying values — see those rows in
§2. That one bump reaching four separate frozen files this way is itself
evidence worth flagging: every fixture that embeds a manifest, not only the
ones already known to, is schema-coupled to this constant. A future
`*_lua_reference.txt` fixture should ask the question
above before its first schema bump, not after.

## Why any of this exists

Passing a unit test proves a module satisfies the assertions someone wrote
down. For determinism-critical modules that is not enough: the simulation's
contract is that two clients produce identical bits, and a test that checks
"the sample is in [0, 1)" would pass on an implementation that is subtly
wrong in the 17th digit. So for anything on the determinism path, this
codebase additionally captured reference values from the real Lua
implementation and compared bit patterns against them. This found nothing
wrong in `rng`'s Rust port — but confirming that took minutes, and it was
the only evidence that actually spoke to the guarantee.

---

## 1. Encoding vectors — never refreshed, ever

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

**A failing assertion against one of these vectors is a finding about the
Rust or TypeScript code under test — never a stale fixture that needs
refreshing.** That rule is unconditional here and is not weakened by
anything below. There is no such thing as a "deliberate change" that
supersedes one of these files, because the file is not a record of a past
decision — it is the only thing keeping two currently-shipping
implementations honest about a format they both write. Refresh it to clear a
red check and the check still passes, the two languages still disagree, and
the disagreement now surfaces where it always does: as a desync in a real
match, blamed on the network.

Also unconditional: ARCHITECTURE.md §3 rule 2. Reassociating floating-point
arithmetic on the determinism path invalidates these vectors and requires
re-justifying against them, not re-capturing them.

---

## 2. Behavioral vectors — frozen records of a superseded implementation

These are the `*_lua_reference.txt` fixtures, each read directly by a
differential test (`include_str!` on the Rust side, a generated `.ts` module
on the TypeScript side), plus one case embedded as a string literal rather
than a checked-in fixture file (`ts/packages/render/src/pitch.spec.ts` — see
`tools/render_reference/README.md` for that one; it compares draw-command
sequences rather than scalar values, so it needed a different capture shape).

They recorded Lua's behavior **including Lua's bugs**. That was the right
thing to capture while the job was validating the port. It is the wrong thing
to be permanently bound by now that the job is improving the game: a correct
fix to a ported gameplay path necessarily diverges the vector that recorded
the incorrect behavior. Worse, these vectors are cumulative — once the RNG
stream diverges at one tick, every subsequent tick differs, so there is no
way to annotate an individual intended divergence.

### The rule

**A divergence nobody intended is still a defect.** This is the common case
and it has not changed. If you did not deliberately set out to change what
the simulation does, a red differential means you changed it anyway — go
find out how. Do not reach for the recorder.

**A divergence caused by a deliberate, reviewed change supersedes the
vector.** Only then, and only with the paperwork:

1. **Name the intent before you look at the fixture.** "I am changing what
   the keeper predicts" is a decision. "The fixture is red and my change is
   probably fine" is not — that is the same sentence someone says in front of
   a real defect.
2. **Record the retirement**: the decision (an issue, with the owner's call
   on it), the superseding change (the PR/commit that makes the old behavior
   wrong), and **the last commit at which the vector held** — verified green
   there, not assumed.
3. **Leave the vector file in place, unmodified.** It is the historical
   record of a capture that can never be taken again. Never edit one in
   place, and never regenerate one: there is nothing to regenerate it from.
4. **Do not delete the test to make the fixture go away.** Several of these
   tests are the only coverage of a layer that has nothing to do with Lua.
   Convert the test to assert against a baseline recorded from the current
   Rust build, keep the scenario and the bit-exact comparison, and state in
   its module doc what it now proves and what it no longer proves.
5. **Then ask what the test was actually for, and get that out of the frozen
   part.** Re-recording in Rust makes a fixture *regenerable*; it does not
   make it *immune*. A per-tick trajectory recorded from build X still breaks
   on any deliberate gameplay change in build X+1 — so a test whose real
   subject is a wire format, a round trip, or determinism itself should
   assert that directly, by comparing two live runs against each other rather
   than against a record. Those assertions survive every rework untouched.
   Retiring a vector without doing this leaves the next rework facing the
   same wall with a longer diff.
6. **Say out loud that the replacement is weaker.** A self-recorded baseline
   detects change; it cannot detect "wrong but consistently wrong". Nothing
   in this repository replaces the cross-implementation coverage, because
   there is no second implementation left to disagree with. A converted test
   that reads as if nothing was lost is a worse artifact than a red one.

The distinction in the first two paragraphs is the whole point of this
section. If you find yourself deciding which one applies *after* seeing the
red output, you are answering the wrong question in the wrong order.

### What has been retired so far

| Vector | Decision | Superseded by | Last commit where it held | Replacement |
| --- | --- | --- | --- | --- |
| `session_legacy_ordinary_lua_reference.txt` | #500 (repository owner) | the goalkeeper forward-prediction rework for #490, which replaces the hand-rolled gravity-only extrapolation #486 exists to eliminate | `9127c5c` (verified green) | `session_legacy_ordinary_baseline.txt`, recorded from Rust by the `#[ignore]`d `record_session_legacy_ordinary_baseline` in the same test file |
| `match_step_ai_ai_lua_reference.txt` | #520 (repository owner) | #516, the locomotion rework for #488: it changes what every body on the pitch does per tick, so a per-tick trajectory diverges by construction | `3f8f4a3` (verified green) | `match_step_ai_ai_baseline.txt`, recorded by `record_match_step_ai_ai_baseline`; plus `match_step_is_bit_reproducible_across_two_independent_runs`, which needs no record |
| `session_ai_driven_lua_reference.txt` | #520 (repository owner) | #516, as above | `3f8f4a3` (verified green) | `session_ai_driven_baseline.txt`, recorded by `record_session_ai_driven_baseline`; read by BOTH `session_ai_driven_differential.rs` and `ai_driven_evidence.rs`, as this table's earlier note warned |
| `rollback_session_lua_reference.txt` | #520 (repository owner) | #516, as above | `3f8f4a3` (verified green) | `rollback_session_baseline.txt`, recorded by `record_rollback_session_baseline`; plus `rollback_session_resimulation_reaches_what_direct_simulation_reaches`, which needs no record |
| `input_tape_lua_reference.txt` — **the stepped-boundary assertion only** | #520 (repository owner), extended after #523 | #516, as above | `4c6d1eb` (verified green) | none, and that is the point: the assertion is retired outright. Its four sibling format claims in the same file were *rewritten* by #523 to read the frozen wire instead of a stepped match, and they keep gating |
| `match_snapshot_case_a_lua_reference.txt`, `match_snapshot_case_b_lua_reference.txt` — **schema-coupled, see the third-axis note near the top of this file** | #536 (repository owner) | `0c94cee`, phase 1 of #531: bumps `match_snapshot::VERSION` 11 → 12 and adds `pass_intent` to every serialized `MatchPlayer` | `2ce0ca0` (verified green) | `match_snapshot_case_a_baseline.txt`/`..._case_b_baseline.txt`, recorded by the `#[ignore]`d `record_match_snapshot_case_a_baseline`/`..._case_b_baseline` in the same test file |
| `match_driver_lua_reference.txt` — **the boundary-zero digest assertion only, schema-coupled** | #536 (repository owner) | `0c94cee`, as above | `2ce0ca0` (verified green) | `BOUNDARY_ZERO_BASELINE_HASH` in `gc-netcode/tests/match_driver.rs` — a single value, re-recorded by reading the assertion's own failure output; no separate recorder test |
| `input_tape_lua_reference.txt` — **`identity.snapshot_version` and `boundary_hash[0]` only, schema-coupled** | #536 (repository owner), found during phase 2 of #531 and added to that issue's scope after its original audit | `0c94cee`, as above | `2ce0ca0` (verified green) | `input_tape_baseline.txt` — two fields re-recorded, `boundary_hash[1..5]` carried over unchanged from the already-recorded `STEPPED_BASELINE` in the same file so the fixture stays internally consistent with one build |
| `coordinator_desync_lua_reference.txt` — **the `transcript_id` line only, schema-coupled** | #489, found by the implementing PR rather than pre-decided — **flagged for explicit reviewer confirmation**, the same class of call #536 made but not yet made by the repository owner for this instance | the committed-actions PR for #489: bumps `match_snapshot::COMBAT_VERSION` 13 → 14 for the new `action` field on every serialized `MatchPlayer`, which `protocol::validate_manifest` embeds in every session manifest, moving every session's `transcript_id` | `b53a5c0` (verified green: this branch's own merge base) | `TRANSCRIPT_ID_BASELINE` in `gc-netcode/tests/coordinator.rs` — a single value, re-recorded by reading the assertion's own failure output; no separate recorder test. `coordinator_conformance.rs`'s Rust-embedded `Golden` (same file class, not a separate fixture — see its own module doc) moved the same four transcript ids for the same reason, in the same commit |
| `desync_package_identity_vector.txt` — **the `manifest_id` field only, schema-coupled; `snapshot_version` also moved but is a raw version integer, the same sanctioned header exception `gc_data::omp1_determinism`'s `identity.snapshot_version` already documents, not a retirement** | #489, same flag as the row above | the same #489 `COMBAT_VERSION` bump: `protocol::manifest_id` hashes the whole canonical manifest, which carries `snapshot_version` | `b53a5c0` (verified green: this branch's own merge base) | `MANIFEST_ID_BASELINE` in `gc-netcode/tests/desync_package.rs` — a single value, re-recorded the same way. `snapshot_version` itself is asserted directly against the live `match_snapshot::COMBAT_VERSION` constant instead of a baked-in literal, so it never needs touching again |
| `protocol_lua_reference.txt` — **`MIN_WIRE(_LEN/_HASH)`, `MAXIMAL_MANIFEST_ID`, `MAXIMAL_WIRE(_LEN/_HASH)`, `MANIFEST_ID` and `TRANSCRIPT_ID` only, schema-coupled** | #489, same flag as the two rows above | the same #489 `COMBAT_VERSION` bump, reached this time through `gc-netcode/tests/protocol.rs`'s own differential suite | `b53a5c0` (verified green: this branch's own merge base) | Seven `*_BASELINE_489` constants in `gc-netcode/tests/protocol.rs`, next to the section doc comment explaining the split. `HANDSHAKE_WIRE` carries no manifest content and is unaffected, still read from the frozen fixture unmodified. `VOCAB_ID` was ALSO unaffected by #489 for the same reason — but see the row directly below, which retires it for an unrelated, later reason |
| `protocol_lua_reference.txt` — **`VOCAB_ID` only, mechanism-coupled rather than schema-coupled (see this file's third-axis note near the top — this is a case that note's own question does not cover: nothing here is a version word)** | #612, found by the implementing PR rather than pre-decided — **flagged for explicit reviewer confirmation**, the same class of call #489's three rows above made | this PR's own change: `Start`'s allowed phases widen from `[Countdown]` to `[Countdown, Running]` (`gc_netcode::protocol::allowed_phases`) so a resend of the canonical start boundary, or a duplicate echo of it, is a phase-legal no-op instead of a protocol violation — `protocol::vocabulary_id()` digests the phase table directly, and the retired Lua implementation never had a resend at all (a one-shot, 2-second handshake with no recovery), so there is nothing left to stay byte-identical with for this one value | `36b0964` (the true merge base once #614, part 1 of the same #612, landed on `main` mid-PR — verified byte-identical to this branch's original merge base `b7ab896` for every file this row touches) | `VOCAB_ID_BASELINE_612` in `gc-netcode/tests/protocol.rs`, next to its own doc comment. `MANIFEST_ID`/`TRANSCRIPT_ID` and the seven `*_BASELINE_489` constants above are unaffected — none of them read the phase table. `protocol_conformance::GOLDEN.vocabulary_id` (same file class as `coordinator_conformance::Golden` above, not a separate fixture) moved the same value for the same reason, in the same commit |

The last-green commit for the three original #520 rows was verified by
checking out `3f8f4a3` and running all four affected tests there; the fourth
row's was verified the same way at `4c6d1eb`, running `input_tape_differential`
(4 passed); the #536 rows' was verified the same way at `2ce0ca0`, running
`match_snapshot_differential` (2 passed), `match_driver`'s
`match_driver_differential_matches_the_lua_reference` (1 passed), and
`input_tape_differential` (4 passed). Not assumed from the catalogue or from
the version-history diff alone — that check is what makes a retirement
auditable rather than merely documented.

**The #536 rows are partial retirements, same shape as the fourth #520 row
below.** In each of the three files, only the field(s) actually derived from
`match_snapshot::VERSION` or a hash containing it were retired; every other
assertion in the same file — frame wires, identity fields other than
`snapshot_version`, the delivery-protocol assertions in `match_driver`'s
case, the structural/canonical-form checks on `boundary_hash[1..5]` — still
reads the original frozen `*_lua_reference.txt` fixture and is unaffected.
See the third-axis note near the top of this file and each file's own module
doc for which field moved and why.

**The fourth row is a partial retirement, and the only one so far.** #520
classified the whole of `input_tape_differential` as class B — format claims,
to be rewritten rather than retired. #523 did the rewrite and discovered that
one of the five assertions cannot be rewritten: the vector recorded *digests
of stepped states* and never the states themselves, so no amount of decoding
recovers them. #523 isolated it into its own case with its own name and said
so in its docstring, calling it "a correction to the issue's premise rather
than to its decision". The decision was extended to cover it. The file's other
four assertions are green and stay.

### What #520 did with rule 5, in each case

Rule 5 is the one that is easy to skip, so here is what "get the real subject
out of the frozen part" produced. In every case the new assertion compares two
live runs and therefore needs no re-recording, ever:

- **`match_differential`** — `match::step` is a pure function of state and
  inputs, so two independently constructed runs of the same scenario agree on
  all 31 fields at all 7,201 ticks. Catches a hidden global, a clock read or
  an iteration-order dependence that a recorded trajectory would happily
  enshrine.
- **`session_ai_driven_differential`** — the same, over the full
  bot + `input_frame` encode/decode/validate + `slot_input` + `match::step`
  pipeline. Its old "the bot actually PLAYED" assertion was **deleted rather
  than moved**, deliberately: it read the FIXTURE's own final row, so
  re-recording the fixture re-recorded the assertion's input too and it could
  never fail a PR that re-records. The live-run form of that claim survives
  once, in `ai_driven_evidence`, where #518 owns it.
- **`ai_driven_evidence`** — the digest chain's Lua end is gone, but the end
  that never depended on Lua is now the point: the pinned constants are what
  `packages/wasm/src/ai_driven.spec.ts` asserts against the compiled wasm
  module, so the pair is the only thing in the workspace that would catch this
  scenario's native and wasm builds parting company. That matters more than it
  used to — see #517.
- **`rollback_session_differential`** — a rollback that resimulates a
  corrected tick must land exactly where a session that never mispredicted
  lands. That is what rollback IS, and it is stated against a live twin.

A caution learned while writing the last of those: the first draft of the twin
mispredicted several ticks and rebuilt the tail from neutral input, which
failed — because after a correction the session's own prediction repeats the
CORRECTED sample. Making the twin match would have meant encoding the
prediction policy into a test that exists to check the code implementing it.
An oracle derived from the code under test is exactly the trap this file warns
about two sections up; the fix was to end the scenario at the corrected tick
so the question does not arise. The old vector diverged at `tick 743: rng state
mismatch`; the test that read it, `gc-sim/tests/session_legacy_differential.rs`,
was **converted rather than deleted**, because it exercises
`input_frame::encode`/`decode`/`validate` plus `slot_input::to_match_input`
into the live-state path `gc-wasm`'s `Session::step` actually runs. It is not
the only test covering that pipeline today — `session_ai_driven_differential`
does too — but that one reads a behavioral vector listed below and will face
this same decision under #488–#491, so it is not a durable home for a wire
guarantee. The re-recorded baseline is bit-identical to
the retired Lua vector across all 7,201 × 31 fields, so that conversion moved
the claim and not one value.

A second lesson from the same file, worth stating because it generalizes:
**an assertion that names a field proves nothing about that field unless the
field varies, and unless the expectation is derived independently of the code
under test.** Three review rounds each found a different instance in this one
test -- neutral move axes, an oracle reading the slot constant it was meant to
check, then all-zero button bitmasks with the oracle calling the decoder. Each
fix closed one dimension and left the next. The durable answer was a
meta-assertion, `every_wire_field_varies_across_the_corpus`, which fails and
names any field that is constant across the corpus, with an explicit
allowlist for a disclosed gap. Prefer that shape over patching instances.

Rule 5 above was learned here, and the transcript is worth keeping: with the
#490 keeper commits cherry-picked on top of the *converted* test, it still
failed at the same `tick 743`. A Rust-recorded trajectory is regenerable, not
immune. So that test now also carries two assertions that compare live runs
against each other instead of against a record — the same match stepped twice
must agree bit-for-bit, and the wire-driven session must agree bit-for-bit
with one stepped from a directly constructed `MatchInput`. Both pass
unchanged under the keeper change. The wire coverage, which is the reason the
file survives at all, therefore keeps gating through every rework in
#488–#491 with nothing to re-record; only the trajectory baseline moves.

### The remaining behavioral fixtures

**Three more were retired by #520 and have been removed from this list;
everything still listed passes.** They are catalogued so the next gameplay
rework knows which ones it might trip and what that would mean — not so it
can pre-emptively retire them. `match_differential.rs` in particular survived
the keeper change because it runs `human_controlled: Some(false)`, a
different scenario: the divergence was scenario-specific, not universal.

Three rows in the "Format and protocol" table below carry a marker,
**partially retired (#536)**: a single schema-coupled field or assertion in
each was retired per the table in §2 above, while everything else the same
row describes still reads the original frozen fixture unmodified and is
still exactly as load-bearing as it reads here. See the third-axis note near
the top of this file for why "not trajectory-coupled" did not save them from
a `match_snapshot::VERSION` bump.

**Gameplay trajectory — a deliberate sim change can legitimately trip these:**

| Fixture | Read by | Records |
| --- | --- | --- |
| `brain_keeper_lua_reference.txt` | `gc-sim/tests/brain_keeper_differential.rs` | per-call outputs of `brain` and `keeper`'s decision functions |
| `aerial_lua_reference.txt` | `gc-sim/tests/aerial_differential.rs` | `aerial::resolve`'s four RNG draws and the trajectory arithmetic they feed |
| `rollback_input_history_lua_reference.txt` | `gc-sim/tests/rollback_input_history_differential.rs` | the input ring buffer the re-simulation replays from |
| `rollback_snapshot_history_lua_reference.txt` | `gc-sim/tests/rollback_snapshot_history_differential.rs` | the snapshot ring the re-simulation restores from, and its eviction floor |

**Format and protocol — these are behavioral vectors by capture, but what
they pin is a wire contract, and a live TypeScript implementation is on the
other end of most of it. Treat a red one as a defect. "Deliberate gameplay
change" is almost never the explanation, and where it is, the TypeScript side
needs the same change in the same PR:**

| Fixture | Read by | Records |
| --- | --- | --- |
| `network_input_frame_lua_reference.txt` | `gc-sim/tests/differential.rs` | `input_frame`'s wire format and `network_conditions`' RNG use |
| `match_snapshot_case_a_lua_reference.txt`, `..._case_b_...` — **partially retired (#536)** | `gc-sim/tests/match_snapshot_differential.rs` | serialized snapshot state and its hash. Both fields were schema-coupled to `match_snapshot::VERSION` and are now compared against a self-recorded baseline instead — see that file's module doc |
| `input_tape_lua_reference.txt` — **partially retired (#536)** | `gc-sim/tests/input_tape_differential.rs` | the tape's identity words, its five frame wires, and its tick-zero boundary hash. **This fixture is split across both classes** — its four post-step boundary hashes are trajectory, and are compared in that file's separately named `the_stepped_boundaries_reproduce_their_recorded_baseline`, which is class A and was itself retired under #520. Of the class-B format claims, `identity.snapshot_version` and `boundary_hash[0]` were additionally schema-coupled and are now compared against a self-recorded baseline; the frame wires and the other identity fields are not and still read real Lua. See "A fixture may be split" below and that file's module doc |
| `protocol_lua_reference.txt`, `fake_relay_lua_reference.txt`, `match_driver_lua_reference.txt`, `coordinator_desync_lua_reference.txt` — **`match_driver_lua_reference.txt`'s boundary-zero digest partially retired (#536)** | `gc-netcode/tests/{protocol,fake_relay,match_driver,coordinator}.rs` | the online protocol's encodings, relay ordering, driver stepping, and desync-package construction. `match_driver`'s consumer compares the delivery protocol — status, confirmation arithmetic, checkpoint cadence, boundary zero — not the correction counts or post-kickoff digests recorded beside them. The boundary-zero digest was schema-coupled to `match_snapshot::VERSION` and is now compared against a self-recorded constant instead of the fixture's digest column; every other comparison in that row is unaffected |
| `frame_buffer_lua_reference.txt` (Rust) and `frame_buffer_lua_reference.ts` (TypeScript) | `gc-render/tests/frame_buffer_differential.rs`, `ts/packages/render/src/frame_buffer.spec.ts` | the `RenderFrame` payload's field order, widths and version word — read by two languages, so this is the closest of the behavioral vectors to an encoding vector in kind. Both consumers now read the frozen rows rather than re-simulating them |

### A format vector's consumer must not step the simulation

Four of the tests in this table used to build a match, step it, and compare
the result to the vector. That makes a format test fail on a gameplay change
with the format intact — which is what happened under the locomotion rework
(#520): `frame_buffer_differential` reported `t37: word 56 is 14.303, Lua
produced 13.843` at the same word index, the same width and the same version
word, while its TypeScript twin — which *decodes* the same frozen row instead
of re-simulating it — stayed green.

So the rule these consumers now follow, and the one to apply to any new
format vector: **recover the state from the vector; do not reproduce it.**
Read the frozen row back and re-encode it, check the wire in both directions,
assert the protocol's shape rather than a simulated magnitude, and where a
hash is genuinely worth pinning across languages, pin the one taken *before*
the first step. Almost all of that needed no retirement — rule 5's "get what
the test was actually for out of the frozen part" is the fix.

### A fixture may be split between the two classes

`input_tape_lua_reference.txt` is the worked example, and the reason the
rule above says "almost all". Recovering state from the vector works for
anything the vector *encodes*; it cannot work for anything the vector only
*hashes*. That file's `boundary_hash[1..5]` are digests of stepped state and
the states themselves were never captured, so nothing decodes them back —
the only way to check them is to step the simulation and hash the result,
which is a trajectory claim by construction.

So do not force a fixture into one class. Split the CONSUMER instead: keep
the format assertions in cases that cannot be reddened by gameplay, and put
the trajectory assertion in its own named case that says in its name and its
failure message that it is trajectory-coupled. The payoff is diagnostic — a
red tells you which kind of thing moved without anyone reading an assertion
index — and procedural: the class-A part can then be retired under rule 2 on
its own, without taking the format guarantees with it.

Do not paper over the split by pointing at a nearby function that sounds
like it covers the gap. This file's first draft dropped those four hashes
and claimed `input_tape::validate` covered them; `validate` replays every
frame but never hashes a boundary, so a tape with `boundary_hashes[1..]`
zeroed still validates. An assertion you have described but not written is
worse than one you have openly dropped.

---

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

A Rust-recorded replacement baseline follows the same discipline, with one
substitution: floats are written with Rust's shortest round-trippable
`Display` form instead of `%.17g`. Both parse back to the identical `f64`;
Lua simply had no shortest-round-trip formatter. Comparison stays on bit
patterns.

## The floored-modulo trap

Lua's `%` is a floored modulo; Rust's `%` and TypeScript's `%` are
truncated remainders. The two agree only when both operands are
non-negative. Anything computing a `%` on a possibly-negative value needs
`rem_euclid` (Rust) or an explicit `((a % n) + n) % n` (TypeScript) — or a
proof that the operand cannot be negative. This is exactly the class of
divergence a vector set covering negative inputs catches, and a vector set
covering only positive ones would silently miss.

This trap is also the sharpest argument for the asymmetry at the top of this
file. It was found by a vector, and no self-recorded baseline could ever have
found it — a baseline recorded from the buggy side would have frozen the
truncated remainder as the expected answer and defended it.
