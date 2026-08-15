# Design: Fun metrics & simulation-based balance search

> **Every number in this document was measured before the port, and none of
> them has been re-measured since.** The mechanism described here is live —
> `gc_sim::metrics`, `gc_sim::bot`, `gc_sim::headless`, `gc_sim::sweep`,
> `gc_sim::tripwire` and `gc_data::fun_baseline` are the Rust modules that
> carry it — but the tables, sweeps, candidates and drift-log entries below
> were all produced between **2026-07-05 and 2026-08-10** by the Lua tree on
> LÖVE that commit `2c0d449` (#467) deleted, through a `love . --sim` /
> `--sweep` / `--search` / `--eval` / `--tripwire` CLI that no longer exists.
>
> Two consequences, both of which have been read backwards at least once:
>
> - **The Phase 2 sensitivity table is not present-day evidence.** It was
>   measured on 2026-07-05 against the Lua `sim/bot.lua` human proxy, on that
>   tree's balance and that tree's mechanics. Reuse of its rankings to justify
>   a knob today needs a fresh sweep, not a citation. §9 of `AGENTS.md` is the
>   standard a knob claim has to meet now: `gc_sim::knob_contract::assert_moves`
>   against a *measured* noise floor.
> - **The fun tripwire is not currently wired into any gate.** The prose below
>   says it runs in `check.sh` and fails the build on drift; that was true of
>   the Lua tree. `gc_sim::tripwire` ports the measurement and comparison, but
>   nothing calls `tripwire::measure` today, so the 30-seed signature is not
>   checked by `./scripts/check.sh` or CI. The frozen combat-disabled Outfield
>   AI baseline **is** still enforced, as an ordinary Rust test
>   (`gc-sim/tests/outfield_ai_baseline.rs`, run by gate 3).
>
> The history is kept, not deleted: it records why the bands, the candidates
> and the non-refresh rule are what they are.

**Scope:** `gc_sim::metrics`, `gc_sim::bot`, `gc_sim::headless`, `gc_sim::sweep`,
`gc_sim::tripwire`, and their tests under `rust/crates/gc-sim/tests/`
(pre-port: `sim/metrics.lua`, `sim/bot.lua`, `sim/headless.lua`,
`sim/sweep.lua`, `main.lua`, `spec/sim/*`)

Combat evidence is governed separately by
[`combat_fun_evidence_contract.md`](combat_fun_evidence_contract.md). The
soccer-only tripwire and its historical `fun` name remain regression tools;
neither is a measurement of human enjoyment or a combat-active baseline.

**Pre-port CLI (deleted with the Lua tree; kept so the runs cited below can be
read).** Every command in this document is one of these:

```
love . --sim [n]          n matches at current defaults, metric report
love . --sweep [n]        per-knob min/max sensitivity, ranked by fun impact
love . --search K1,K2 [n] greedy coordinate ascent over the named knobs
love . --eval FILE [n]    a tuning blob vs defaults on held-out seeds (paired)
love . --tripwire [write] fun-signature snapshot vs data/fun_baseline.lua
                          (in check.sh; exit 1 on drift; `write` refreshes)
```

**Today there is no CLI.** The same measurements are library calls in
`gc-sim`, reached from a Rust test or a scratch harness:
`headless::run_batch` plays a seeded batch and `headless::report` prints it;
`sweep::sensitivity` and `sweep::paired_delta` do the per-knob sweep;
`tripwire::measure` / `compare` / `report` produce and check the fun
signature, and `tripwire::serialize` emits a `gc_data::fun_baseline` literal
to paste over `rust/crates/gc-data/src/fun_baseline.rs`;
`outfield_ai_baseline::measure` reproduces the frozen control. Rebuilding a
runnable entry point over them is not done — see the banner above.

**Status (2026-07-10, pre-port):** phases 1–4 done. The tripwire (`sim/tripwire.lua`)
runs 30 seeded matches in check.sh and fails the gate when any banded-metric
mean (or the composite) drifts more than 5% from the checked-in baseline —
the sim is deterministic per seed, so a behavior-neutral change reproduces
the snapshot exactly. On intended drift: re-run `--sim 100`, log the shift in
the drift log below, then `love . --tripwire write`. Candidate A is validated
(re-validated twice); B died in the 2026-07-10 re-validation. A ships as an
F1 preset — awaiting a hands-on playtest verdict before any change to
`gc_sim::tuning` defaults.

## Why

Balance tuning today is play-by-hand with the F1 panel: change a knob, play a
match, trust your gut. That works for feel but doesn't scale to ~30 interacting
knobs, and it can't answer "did this pass buff quietly double goals per match?"
The sim is pure, deterministic (seeded `core.rng`), and headless-capable — so we
can play thousands of unattended matches and measure the *statistical signature*
of the resulting gameplay.

We cannot measure fun. We can measure whether a match produces the shape of
matches we know are fun, and flag configurations that drift out of that shape.

## The fun proxy

Each match produces a metrics table; each metric gets a **target band** — a
trapezoid desirability function worth 1.0 inside the band, falling linearly to
0.0 at the hard limits. The per-match **fun score** is the *geometric mean* of
the desirabilities, so no config can score well by maxing five metrics while
zeroing a sixth (a weighted average would happily trade a 0 for two 1.2s; the
geometric mean will not).

Provisional bands (per 120 s match — first-to-3 when these were set, no goal
limit since #268; revisit after the baseline run):

| Metric               | What it protects                       | Good band  | Zero at    |
| -------------------- | -------------------------------------- | ---------- | ---------- |
| `goals_total`        | matches resolve, but stay scarce       | 2 – 5      | 0 / 8      |
| `shots_per_goal`     | shots feel dangerous but not automatic | 2.5 – 6    | 1 / 25     |
| `save_rate`          | keepers matter, aren't walls           | 0.45 – 0.75| 0.15 / 0.95|
| `pass_completion`    | passing is viable but contestable      | 0.55 – 0.85| 0.25 / 1.0 |
| `turnovers_per_min`  | contested but not ping-pong (settled)  | 1 – 5      | 0.3 / 10   |
| `possession_balance` | neither side steamrolls (share of max) | 0.35 – 0.65| 0.1 / 0.9  |
| `longest_drought_s`  | no dead stretches without a chance     | 0 – 35     | –  / 80    |
| `decided_late`       | tension survives into the match        | 0.4 – 1.0  | 0.05 / –   |

Zero edges sit at *catastrophic*, not merely bad — a hard-zero plateau gives a
future optimizer no gradient to climb out of. `turnovers_per_min` counts
**settled** possession changes (a team must hold the ball 0.7 s before it
"has" it): raw ownership flicker runs ~40× higher — the ball changes hands
every second or two in poke-scrambles — and measures nothing a player would
call a turnover.

`decided_late` = time of the goal that finally decided the winner, as a
fraction of the match (draws count 1.0 — undecided to the end). Caveat when
reading the per-metric table: goalless matches therefore inflate the
`decided_late` column mean. The fun score is unaffected — `goals_total`
hard-zeros those matches — but the column looks healthier than it is whenever
goalless matches are common. `lead_changes`
is reported but unbanded for now: it was too coarse to score under the 3-goal
cap these runs were measured with, where the honest range was 0–2. #268 removed
the cap, so a re-measurement is free to widen that range — but nothing here has
been re-measured, and the column stays unbanded until something is.

## The human proxy (the big caveat)

AI-vs-AI matches measure the AI ecosystem, not the player's hands. The
controlled slot is driven by `gc_sim::bot` (measured pre-port as
`sim/bot.lua`), a deliberately human-ish input
driver: it re-decides only every ~0.2 s (reaction latency), adds aim noise from
its own seeded RNG, dribbles/charges/shoots/passes with simple heuristics, and
chases/jockeys off the ball. It can also reactively juke a committed defender,
charge and loft long outlets, chip a shot, and request aerial/acrobatic strikes,
so the proxy exercises the same verbs available in a hands-on match. It is not
a good player; it is a *predictable mediocre* player, which is what balance
work needs.

Dribble diagnostics are reported separately as `controlled_dribble_*` and
`ai_dribble_*`: carry time, close-control/sprint/juke shares, touch cadence,
and heavy losses. The AI bucket includes every non-controlled outfield carrier,
so compare the normalized shares and per-carry-minute rates rather than raw
action counts.

Consequence: treat all results as **relative** (config A vs config B under the
same bot), never as absolute predictions of human experience. Big metric swings
are signal; small ones are bot artifacts until verified by hand with F1.

## Determinism & variance

- Same seed ⇒ same match ⇒ identical metrics (bot RNG is derived from the
  match seed; nothing reads `math.random` or the clock).
- Configs are compared on the **same seed set** (common random numbers), so
  differences come from the knobs, not seed luck.
- Metrics are reported as mean ± sd over N seeds; single matches are anecdotes.

## Roadmap

1. **Baseline — done.** `love . --sim [n]` plays n seeded matches on the
   default knobs and prints the metric distribution + fun score. This is the
   reference signature and a manual regression tool for any future sim change.
2. **Knob sweep — done.** `--sweep` perturbs every knob to its min and max
   over common seeds and ranks by paired fun impact. Results below.
3. **Search — done.** `--search` runs greedy coordinate ascent over chosen
   knobs; `--eval` re-checks any blob on held-out seeds. Candidates below —
   verified by humans, never auto-shipped (Goodhart's law: an optimizer will
   happily invent coin-flip keepers to farm `decided_late`).
4. **Tripwire — done.** A 30-seed smoke batch in `scripts/check.sh` fails loudly
   when a sim change moves a banded metric beyond the checked-in tolerance.

## Baseline signature (defaults) — pre-port, 2026-07-09

`love . --sim 100`, all knobs at defaults, 2026-07-09 (post keeper-fix — the
original 2026-07-05 table is superseded; the shift is recorded in the drift
log below):

```
metric                     mean        sd       min       max   desir  band
fun                       0.246     0.373     0.000     0.950       -
goals_total               0.930     0.832     0.000     3.000    0.45  [2 .. 5]
shots_per_goal           20.379     7.905     6.667    38.000    0.31  [2.5 .. 6] (n=65)
save_rate                 0.820     0.219     0.000     1.000    0.42  [0.45 .. 0.75]
pass_completion           0.522     0.054     0.367     0.658    0.88  [0.55 .. 0.85]
turnovers_per_min         1.090     0.889     0.000     3.000    0.71  [1 .. 5]
possession_balance        0.325     0.056     0.208     0.461    0.85  [0.35 .. 0.65]
longest_drought_s        17.240     5.496     9.067    38.783    1.00  [0 .. 35]
decided_late              0.719     0.331     0.046     1.000    0.88  [0.4 .. 1]
lead_changes              0.020     0.141     0.000     1.000       -
margin                    0.750     0.744     0.000     3.000       -
shots                    25.590     4.486    17.000    39.000       -
passes                   84.360     6.576    68.000   106.000       -
```

What the baseline says (under this bot — relative claims only):

- **Scoring is the weak dimension.** 0.93 goals/match against a 2–5 target,
  and 20 shots per goal: teams shoot plenty (~26/match) but conversion is dire
  and the keeper fix made it worse. 35 of 100 matches were goalless. The three
  worst desirabilities (goals 0.45, conversion 0.31, save rate 0.42) are the
  same underlying issue: shots rarely threaten.
- **The bot's team holds ~33% possession** — the human proxy is weaker than
  the AI it replaces, as expected. Keep it fixed while comparing configs.
- **Flow metrics are healthy**: droughts, decided-late, and settled turnovers
  all sit in or near band. The game's problem is not stagnation, it's payoff.
- `lead_changes` ≈ 0 follows directly from goal scarcity; it will only become
  meaningful once goals_total lives in its band.

## Phase 2: sensitivity sweep — pre-port, measured 2026-07-05

**Provenance, because this table is the one most often mistaken for current
evidence.** It was produced on **2026-07-05** by `love . --sweep 30` on the Lua
tree, against that tree's `sim/bot.lua` human proxy, that tree's knob ranges,
and a build that still capped matches at three goals (#268 removed the cap on
2026-07-30, and the mechanics batches logged in the drift log below moved the
baseline repeatedly afterwards). It has not been re-measured on `gc-sim`. Read
it as the record of how the *method* was validated — including the honest
sanity check that knobs the sim ignores came back at exactly 0.000 — not as a
ranking you may cite for a knob today. A present-day claim that a knob moves a
metric is made with `gc_sim::knob_contract::assert_moves` (`AGENTS.md` §9).

`love . --sweep 30` — every knob to its min and max, paired against the
default baseline (fun 0.238, goals 1.10) on seeds 1–30. Knobs that matter,
ranked by paired ΔFun (±se ≈ 0.08–0.11):

| Knob                 | Best direction    | ΔFun   | Goals there |
| -------------------- | ----------------- | ------ | ----------- |
| `AI_SHOOT_RANGE`     | max (340)         | +0.56  | 3.50        |
| `AI_PASS_PRESSURE`   | min (30)          | +0.32  | 2.07        |
| `AI_STEAL_CD`        | max (2.5)         | +0.26  | 1.27        |
| `AI_HEADER_RANGE`    | max (300)         | +0.23  | 1.97        |
| `CARRIER_SETTLE`     | min (0)           | +0.20  | 1.80        |
| `JOCKEY_SLOW`        | min (0.5)         | +0.20  | 1.47        |
| `SAVE_SPEED_REF`     | min (700)         | +0.14  | 2.20        |
| `KEEPER_RESPECT_DIST`| **max is BAD**    | −0.15  | 0.53        |

Sanity checks that the harness is honest: knobs the sim ignores for an all-AI
match (`REPLAY_*`, `PASS_CHARGE_RATE`, `KEEPER_HOLD_HUMAN`, `PUNT_MAX`) came
back at *exactly* 0.000 — the pairing removes all seed noise. The story is
one-directional: everything that lets attacks finish (shoot earlier, header
from further, press less, poke less) helps, because the baseline's failure
mode is attacks that never resolve.

## Phase 3: candidates (validated on held-out seeds 1001–1060)

**Candidate A — "direct play" (the ascent winner, fun 0.912 on search seeds):**

```
AI_SHOOT_RANGE=340
AI_HEADER_RANGE=300
AI_PASS_PRESSURE=75
SAVE_SPEED_REF=700
AI_STEAL_CD=1.5
CARRIER_SETTLE=0.6
```

Held-out: **fun 0.770 vs 0.302 at defaults — paired ΔFun +0.468 ± 0.065**
(~7 se; survives the overfit haircut from 0.912). Goals 3.68, save rate 0.60,
all banded metrics ≥ 0.75 desirability.

**Candidate B — "one-knob sweet spot":**

```
AI_SHOOT_RANGE=300
```

Held-out: **ΔFun +0.388 ± 0.062** (fun 0.690, goals 2.53). One change buys
~80% of Candidate A's gain — the low-risk first ship.

Caveats before shipping either:

- ~~**Range-edge optima.**~~ Resolved by round 2 (below): with widened ranges
  the ascent kept `AI_SHOOT_RANGE=340` and `SAVE_SPEED_REF=700` — both are
  interior optima, not fence artifacts.
- ~~**Matches got shorter.**~~ Resolved by [#268](https://github.com/osobytes/goliseo/issues/268):
  under A the 3-goal cap ended matches at ~78 s mean (min 21 s) and under B at
  ~106 s, and the advice here was to raise `max_goals` if full-length matches
  mattered. There is no goal cap any more — a match runs its 120 seconds and is
  decided on score — so neither candidate shortens one. The measurements above
  were taken under the cap and are left as recorded.
- **Bot-relative.** All numbers are under the pre-port `sim/bot.lua` proxy
  (now `gc_sim::bot`). Verify by playing: both candidates ship as F1-panel
  presets (`gc_data::tuning_presets`, F4 cycles Defaults → A → B; F2 persists
  the choice across runs). Defaults in `gc_sim::tuning` stay untouched until a
  candidate survives hands-on play.
- `pass_completion` (~0.49) stays just below its band in every config tested —
  no knob in the current panel moves it much. If passing should feel more
  reliable, that's a *mechanics* change (lead, receiver magnetism), not a knob.

## Round 2: the optimum is real (2026-07-05)

Round 1's winner sat on three range fences, so the fences moved:
`AI_SHOOT_RANGE` max 340→480, `AI_HEADER_RANGE` max 300→420,
`SAVE_SPEED_REF` min 700→400 (the F1 sliders widened accordingly). A second
ascent then warm-started **from Candidate A** over twelve knobs — round 1's
eight plus `WHIFF_STUMBLE`, `CHARGE_RATE`, `KEEPER_RESPECT_DIST`,
`CATCH_EVEN_QUALITY` (`--search ... 30 /tmp/candidate_a.tune`).

Result: **the search found nothing.** One accepted move in two passes
(`AI_HEADER_RANGE` 300→350, +0.005 on search seeds), which a paired held-out
head-to-head (`--eval A' 60 A`) measured at **+0.009 ± 0.058 — noise**.
`AI_SHOOT_RANGE` refused 400 and 480 despite now being allowed there;
`SAVE_SPEED_REF` refused 400; none of the four new knobs improved on A.

Conclusions:

- **Candidate A is a converged local optimum**, robust to a wider search
  space. `AI_SHOOT_RANGE≈340` and `SAVE_SPEED_REF≈700` are interior sweet
  spots, not clipped values.
- The fun ridge around A is flat (~0.91 ± 0.07 on search seeds): nearby knob
  nudges neither help nor hurt much, which is what you want from a shipped
  balance — it won't be fragile to small future tweaks.
- Further gains now require either **new mechanics** (the stuck
  `pass_completion`), **a better human proxy**, or **band revisions** — not
  more knob search. Ship A (or B), play it, and recalibrate the bands against
  how it actually feels.

## Frozen combat-disabled Outfield AI baseline (#59)

Everything above is the **soccer fun tripwire**: a 30-seed human-proxy smoke
test with a 5% tolerance band, checked in as `gc_data::fun_baseline`. This
section is a *different* artifact with a different job, and the two must not be
confused or merged. The locked evidence contract
(`docs/design/combat_fun_evidence_contract.md` §4.4) requires the soccer
tripwire to stay combat-disabled and to never be refreshed from a combat
fixture; nothing in #59 touches `gc_data::fun_baseline`.

### Why it exists

Combat-family calibration (#149) compares combat-active play against
combat-disabled play. Without a frozen, citable control, "combat changed X" is
unfalsifiable: any inconvenient result can be explained away by re-running the
control. #59's orchestrator refresh therefore requires a versioned
combat-disabled gameplay-AI **policy id** plus a common-seed **baseline** that
#112/#148/#149 reference *without copying or silently refreshing it*.

### The policy id

`gc_sim::outfield_ai_policy` (pre-port: `sim/outfield_ai_policy.lua`) publishes
the identity, e.g.

```text
outfield_ai_policy/v1/combat_disabled/303228d776b65a19
```

It is `schema / schema version / combat mode / FNV-1a-64 of a canonical
serialization`. What gets hashed is an explicitly **declared** surface —
`outfield_ai_policy.SURFACE` plus the `AI`-category tuning defaults — not
whatever happens to be in the AI modules. The surface covers five modules:
`gc_sim::outfield_decision`, `gc_sim::outfield_press`, `gc_sim::offball_runs`,
`gc_sim::possession_transition`, and `gc_sim::ai`, whose off-ball support
weights (`IMPORTANCE_K`, `CENTER_SIGMA`, `LANE_WIDTH`, `LANE_BLOCK`) feed
`offball_runs`' pass-lane scoring. Declaring the surface is what makes the id
stable in both directions:

- adding, renaming, or reordering an *undeclared* field does not move the id,
  so it does not churn on refactors;
- changing a *declared* constant, an `AI` knob default, or a module `VERSION`
  does move it;
- deleting or renaming a declared field raises instead of hashing `nil` into a
  plausible-looking id.

A live tuning-panel nudge does **not** move the id: the policy is the shipped
balance, so `knob.default` is hashed and `tuning.values` is not.

Some genuinely behavioural constants stay file-local — `gc_sim::ai`'s
intercept-sampling grid, `gc_sim::offball_runs`' run-shape geometry,
`gc_sim::outfield_press`' contain and lane weights. Changing one of those
moves the recorded metrics but not the id on its own. **Every** module in the
surface therefore exposes a `VERSION` as the declared landing spot for such a
change, and `gc_sim::outfield_ai_policy` asserts that each one exists
and leads its field list, so the promise cannot quietly rot. A test checks the
same thing. Bumping `VERSION` is how a file-local policy change is declared.

Whether or not anyone remembers to bump it, behaviour that moves play is caught
by the metric signature below. It is never absorbed.

### The baseline artifact

`gc_data::outfield_ai_baseline::RECORD`
(`rust/crates/gc-data/src/outfield_ai_baseline.rs`), produced and verified by
`gc_sim::outfield_ai_baseline`.

| Field | Value |
| --- | --- |
| Fixture | `combat_disabled_control_a` — fixture A of the locked matrix (§3.2) |
| Seeds | `20001..20060`, the locked paired calibration block (§3.3) |
| Sides | all-AI (`bot = "none"`); the human proxy is a separate policy |
| Config | `field=960x540; duration=120; max_goals=3; tick_rate=60; tactic=balanced` |
| Combat | disabled — `sim.headless` never constructs a `CombatMatchState` |
| Knobs | every knob at its default (applied and restored per match) |
| Recorded | `n`, `mean`, `sd`, `min`, `max` for 23 metrics |

Seeds `20001..20060` are the same block #149 runs its combat-active arm on, so
this is a **paired** control under common random numbers rather than an
independent sample. It deliberately avoids the tripwire's `1..30` and the spent
evaluation seeds `1001..1060`.

The file records the full identity #59 asks for: `policy_id`, `fixture_hash`,
`config_hash`, `content_hash` (teams, rosters, player stats, species
modifiers, formation anchors, tactic), `tuning_hash` (all 40 knob defaults),
`snapshot_version`, `input_version`, `tick_rate`, `seed_first`, `seed_count`,
and `seed_hash`. `signature` covers identity plus every recorded statistic.

### Commands

**Today.** The verification is an ordinary Rust test — no CLI, no flag:

```sh
cd rust && cargo test -p gc-sim --test outfield_ai_baseline
```

`outfield_ai_baseline_reproduces_the_frozen_fixture_exactly` re-runs the
fixture through `gc_sim::outfield_ai_baseline::measure` and compares it against
the frozen `gc_data::outfield_ai_baseline::RECORD`, so it runs inside
`cargo test --workspace` — gate 3 of `./scripts/check.sh`, which
`.github/workflows/ci.yml`'s `gate` job invokes rather than mirroring, so the
two cannot drift (AGENTS.md §9). A deliberate re-freeze means running
`outfield_ai_baseline::serialize` over a fresh `measure` and pasting the result
over `rust/crates/gc-data/src/outfield_ai_baseline.rs`; the ceremony below is
what the acknowledgement flag used to enforce, and it is now enforced by review
rather than by an argument parser.

**Pre-port**, for reading the history below:

```sh
love . --ai-baseline                          # verify (exit 1 when a metric moved)
love . --ai-baseline write --refreeze-ack     # deliberately re-freeze
```

Both `./scripts/check.sh` and the `quality` job in `.github/workflows/ci.yml`
ran the verification on that tree; `--ai-baseline` was also listed in
`conf.lua`'s `headless_flags`, because this check belongs to the no-display
tier of AGENTS.md §9 and must not try to open a GL context on a CI runner.

### The non-refresh rule

**A failing baseline comparison is a finding, not a chore.** #148 and #149
cite this artifact as their control, so refreshing it to go green destroys the
only record that the control moved.

Unlike the fun tripwire this comparison is **exact**. The batch is
deterministic per seed, and recorded values round-trip through `%.17g`, so any
metric movement at all is real.

Two outcomes are possible, and only one of them blocks:

**`AI BASELINE MOVED` — fails, exit 1.** A tracked metric changed, so this
build no longer plays the frozen control. Confirm the change is intended, add a
drift-log entry below naming the moved metrics, then re-freeze.

**`AI BASELINE STALE` — warns, exit 0.** Every tracked metric held but the
recorded identity is out of date. This happens because identity is deliberately
wider than play: `tuning_hash` covers all 40 knob defaults, not just the nine
`AI`-category ones in the policy id, and `content_hash` covers every authored
roster, formation, and tactic. Registering an unrelated knob or renaming a
reserve player moves the identity while combat-disabled play stays bit-for-bit
identical. Such an edit still owes a drift-log entry and a re-freeze, but it
does **not** fail the shared gate — taxing unrelated branches with the
deliberately awkward `--refreeze-ack` ceremony would make that ceremony routine
and hollow out the one guardrail this artifact has.

In both cases the resolution is the same order of operations: confirm the
change is intended, log it, and only then run
`love . --ai-baseline write --refreeze-ack`. Writing without the
acknowledgement flag is refused. Every re-freeze bumps `baseline_version`, and
the `signature` deliberately excludes it, so a re-freeze that changes nothing
shows up in git as a lone version bump rather than hiding inside a churned
file.

Downstream issues cite the `policy_id` and `fixture_hash` strings. They do not
copy the numbers, and they do not re-record the control themselves.

### What this baseline does not cover

It is observability and identity only — #59's full instrumentation contract
(runs by type/role, passes to active runners, open options at pass release,
press commit reasons, reaction time, counter-press/counter-attack timing,
angular error by technique profile), its matched mental-2-vs-mental-8
comparison, its technique and tactic comparisons, its constant sweeps, and its
visual review all remain open. This section freezes the control those
experiments — and #149's calibration — measure against.

## Baseline drift log — every entry is pre-port (2026-07-08 to 2026-08-10)

The ritual still stands: a sim change that moves the fun signature owes a
100-match validation and an entry here before the baseline is refreshed. The
commands each entry cites are the deleted Lua CLI (see the banner at the top),
and no entry has been re-measured on `gc-sim`. Every number below is the Lua
tree's, from **2026-07-08** (oldest) to **2026-08-10** (newest).

- **2026-08-14 — nine `gc-sim`/`gc-data` transcendental call sites converted
  to `gc_core::deterministic_math::cos_sin` or a precomputed constant, so
  native and the compiled wasm module compute the same simulation state
  (#517's mechanical sweep).** `baseline_version` **10 → 11**, signature
  `95850e11e242ea9b` → `fcb849367c7c26e4`; `identity.tuning_hash`,
  `identity.content_hash`, `identity.fixture_hash` and `snapshot_version`
  unchanged — no knob defaults or content moved, only mechanics. Re-frozen
  via `record_outfield_ai_baseline`, per that module's own re-freeze
  protocol.

  This is not a gameplay change: `f64::cos`/`f64::sin` (not correctly
  rounded, and a different libm on `wasm32-unknown-unknown` than native)
  moved to `gc_core::deterministic_math::cos_sin` at `match.rs`'s dribble
  touch and AI-outfield execution-error sites, `aerial.rs`'s contact-angle
  rotation, and `bot.rs`'s aim noise; `match.rs`'s four-literal-angle support
  triangle and `gc-data`'s `ActionFamilyData::front_arc_cos` (read by
  `combat.rs`/`combat_feasibility.rs`) moved to precomputed constants. Each
  site's own output shifts by an amount on the order of `cos_sin`'s ~1e-13
  relative accuracy against libm, but the simulation is a long-running
  nonlinear system (7,200 ticks, collisions, AI decisions) — once one
  dribble touch or support run lands a player a fraction of a pixel
  differently, the rest of that possession sequence runs on genuinely
  different state, not just genuinely different bits. `gc-sim`'s
  `KNOWN_DIVERGENCES` allowlist in `scripts/check_wasm_native_corpus.mjs`
  went from two tracked native/wasm divergences (attributed to two of these
  nine sites) to zero, which is the point of the change; this baseline's
  drift is that same mechanism's cost on the native side, since the frozen
  fixture pins exact hashes and does not distinguish "moved for a bug" from
  "moved because a call that used to disagree with wasm now agrees with it."

  | metric | frozen (v10) | re-frozen (v11) | delta |
  | --- | --- | --- | --- |
  | `fun` | 0.267473 | 0.256633 | −0.010841 |
  | `goals_total` | 1.883333 | 1.783333 | −0.100000 |
  | `goals_home` | 0.650000 | 0.750000 | +0.100000 |
  | `goals_away` | 1.233333 | 1.033333 | −0.200000 |
  | `shots` | 29.000000 | 30.166667 | +1.166667 |
  | `shots_per_goal` | 16.433642 | 19.657738 | +3.224096 |
  | `save_rate` | 0.889098 | 0.899940 | +0.010842 |
  | `passes` | 32.683333 | 31.633333 | −1.050000 |
  | `pass_completion` | 0.524982 | 0.536465 | +0.011483 |
  | `turnovers_per_min` | 9.778232 | 9.263996 | −0.514236 |
  | `possession_balance` | 0.518344 | 0.525805 | +0.007461 |
  | `longest_drought_s` | 13.325278 | 13.563889 | +0.238611 |
  | `decided_late` | 0.667803 | 0.627862 | −0.039941 |
  | `lead_changes` | 0.016667 | 0.033333 | +0.016667 |
  | `margin` | 1.050000 | 1.116667 | +0.066667 |
  | `duration` | 117.205000 | 117.481667 | +0.276667 |
  | `ai_dribble_carry_s` | 28.371944 | 28.540556 | +0.168611 |
  | `ai_dribble_close_share` | 0.843196 | 0.851183 | +0.007987 |
  | `ai_dribble_sprint_share` | 0.140410 | 0.133638 | −0.006773 |
  | `ai_dribble_juke_share` | 0.075924 | 0.075090 | −0.000834 |
  | `ai_dribble_touches_per_min` | 99.758856 | 97.251863 | −2.506993 |
  | `ai_dribble_heavy_losses_per_min` | 0.437461 | 0.458042 | +0.020580 |
  | `ai_jukes` | 30.933333 | 30.900000 | −0.033333 |

  `gc_sim::keeper_shadow_classifier`'s frozen 60-seed counts moved in the
  same commit, for the same reason: `candidates` 9285 → 9438,
  `disagree_height` 18 → 40, `disagree_deferred` 172 → 159, `new_only`
  unchanged at 0 (the structural argument for it does not depend on which
  libm computed the angle that got a carrier to a given position).

- **2026-08-14 — the gameplay AI's pass/throw decisions moved onto the same
  `MatchInput` charge-and-release seam a human uses, instead of calling
  `release_pass` directly (#531 phase 2, re-recording phase 1's #535
  blast radius).** `baseline_version` **9 → 10**, signature
  `ac397926cf724b7b` → `95850e11e242ea9b`, `snapshot_version` **11 → 12**
  (`pass_intent` added to `MatchPlayer`; `tuning_hash` and `content_hash`
  unchanged — no knob defaults or content moved), `fixture_hash`
  `eda80b6ca32829a2` → `110e1af740715032`. Re-frozen via
  `record_outfield_ai_baseline`, per that module's own re-freeze protocol.

  This is a genuine trajectory change, not a schema artifact: an AI pass or
  throw now costs the same charge time and hold-duration interception
  exposure a human always paid, and the soft cone
  (`select_pass_target`/`select_throw_target`) resolves the receiver instead
  of the AI's own scorer. See #531/#535 for the seam's design and the full
  enumerated blast radius.

  | metric | frozen (v9) | re-frozen (v10) | delta |
  | --- | --- | --- | --- |
  | `fun` | 0.264791 | 0.267473 | +0.002682 |
  | `goals_total` | 1.683333 | 1.883333 | +0.200000 |
  | `goals_home` | 0.616667 | 0.650000 | +0.033333 |
  | `goals_away` | 1.066667 | 1.233333 | +0.166667 |
  | `shots` | 33.016667 | 29.000000 | −4.016667 |
  | `shots_per_goal` | 22.572327 | 16.433642 | −6.138685 |
  | `save_rate` | 0.911370 | 0.889098 | −0.022272 |
  | `passes` | 33.700000 | 32.683333 | −1.016667 |
  | `pass_completion` | 0.553037 | 0.524982 | −0.028055 |
  | `turnovers_per_min` | 8.784997 | 9.778232 | +0.993235 |
  | `possession_balance` | 0.560170 | 0.518344 | −0.041826 |
  | `longest_drought_s` | 11.537500 | 13.325278 | +1.787778 |
  | `decided_late` | 0.642449 | 0.667803 | +0.025354 |
  | `lead_changes` | 0.066667 | 0.016667 | −0.050000 |
  | `margin` | 1.183333 | 1.050000 | −0.133333 |
  | `duration` | 116.805000 | 117.205000 | +0.400000 |
  | `ai_dribble_carry_s` | 25.724444 | 28.371944 | +2.647500 |
  | `ai_dribble_close_share` | 0.845731 | 0.843196 | −0.002535 |
  | `ai_dribble_sprint_share` | 0.147908 | 0.140410 | −0.007498 |
  | `ai_dribble_juke_share` | 0.081623 | 0.075924 | −0.005700 |
  | `ai_dribble_touches_per_min` | 105.131959 | 99.758856 | −5.373103 |
  | `ai_dribble_heavy_losses_per_min` | 0.636747 | 0.437461 | −0.199285 |
  | `ai_jukes` | 31.933333 | 30.933333 | −1.000000 |

  `fun` is comparable across these two rows — no metric was registered or
  retired between v9 and v10, only the producer's mechanics changed — but it
  should not be read as a verdict on balance. `pass_completion` drops from
  0.553 to 0.525, moving toward, not yet past, the 0.52 floor #531's issue
  body predicted; `ai_dribble_carry_s` and `longest_drought_s` both rise,
  consistent with an AI that now has to hold the ball through a charge
  instead of releasing on the spot. **This entry is the drift record the
  recorder's own re-freeze protocol requires, not the phase-3 balance
  re-measurement #531 tracks separately** — phase 3 re-runs the harness from
  scratch and publishes the new numbers as the reference rather than as a
  delta against the AI-exempt baseline above, which measured a different
  producer's rules.

- **2026-08-13 — passing: soft-scored receiver selection, a registered
  distance-to-speed curve, and a lead solver measured against the real
  locomotion profile (#491, sim half).** `baseline_version` **8 → 9**,
  signature `614ed81d38e82116` → `ac397926cf724b7b`, `tuning_hash`
  `84908592d5981f4a` → `4a1d2ea76cd7481c` (eleven new tier-1 knobs),
  `fixture_hash` `d6463f56f154f710` → `eda80b6ca32829a2`. Re-frozen via
  `record_outfield_ai_baseline`, per that module's own re-freeze protocol.

  The hard 60-degree acceptance cone in `gc_sim::passing` is replaced by a
  soft blend of distance and a chord-weighted angular term with no acceptance
  test at all; ground-pass launch speed now comes from three registered
  breakpoints instead of three `const`s; and a driven ground pass to a moving
  receiver is aimed at a solved lead point when one is admissible.

  | metric | frozen (v8) | re-frozen (v9) | delta |
  | --- | --- | --- | --- |
  | `fun` | 0.269890 | 0.264791 | −0.005099 |
  | `goals_total` | 1.500000 | 1.683333 | +0.183333 |
  | `goals_home` | 0.716667 | 0.616667 | −0.100000 |
  | `goals_away` | 0.783333 | 1.066667 | +0.283333 |
  | `shots` | 34.083333 | 33.016667 | −1.066667 |
  | `shots_per_goal` | 22.988333 | 22.572327 | −0.416006 |
  | `save_rate` | 0.927511 | 0.911370 | −0.016140 |
  | `passes` | 34.366667 | 33.700000 | −0.666667 |
  | `pass_completion` | 0.558467 | 0.553037 | −0.005430 |
  | `turnovers_per_min` | 8.740769 | 8.784997 | +0.044228 |
  | `possession_balance` | 0.547505 | 0.560170 | +0.012665 |
  | `longest_drought_s` | 11.449444 | 11.537500 | +0.088056 |
  | `decided_late` | 0.694121 | 0.642449 | −0.051672 |
  | `lead_changes` | 0.033333 | 0.066667 | +0.033333 |
  | `margin` | 1.000000 | 1.183333 | +0.183333 |
  | `duration` | 118.884167 | 116.805000 | −2.079167 |
  | `ai_dribble_carry_s` | 25.846944 | 25.724444 | −0.122500 |
  | `ai_dribble_close_share` | 0.833517 | 0.845731 | +0.012214 |
  | `ai_dribble_sprint_share` | 0.147692 | 0.147908 | +0.000216 |
  | `ai_dribble_juke_share` | 0.085907 | 0.081623 | −0.004283 |
  | `ai_dribble_touches_per_min` | 107.620664 | 105.131959 | −2.488705 |
  | `ai_dribble_heavy_losses_per_min` | 0.510557 | 0.636747 | +0.126189 |
  | `ai_jukes` | 32.716667 | 31.933333 | −0.783333 |

  **`fun` is again not comparable across these two rows**, for exactly the
  reason the v2 → v3 entry below gives: this change registers a **tenth and
  eleventh** metric (`pass_aim_error`, `pass_lead_time`), and `fun` is a
  geometric mean over however many extract a value.

  The same-build number that entry gives, given here on the same terms.
  Scored on **one identical build** over these same 60 seeds, holding
  everything but the registered metric set fixed:

  | fixture | metrics folded | `fun` | inflation |
  | --- | --- | --- | --- |
  | this all-AI control | 9 (v8's set) | 0.263588 | — |
  | this all-AI control | 10 | 0.264791 | **+0.001203 (+0.46%)** |
  | bot-driven harness default | 9 (v8's set) | 0.344160 | — |
  | bot-driven harness default | 11 | 0.358275 | **+0.014115 (+4.10%)** |

  So the like-for-like v8 → v9 movement is **0.269890 → 0.263588 = −0.006302**,
  and the −0.005099 in the table above understates it by the +0.001203 the
  extra metric adds for free.

  **Only ONE of the two new metrics arms on this control, and that is not a
  detail.** `pass_aim_error` is present in **0 of 60** matches here and in
  60 of 60 on the bot-driven fixture, because an all-AI match contains no
  aimed release at all. That is the same fact the next paragraph argues from,
  measured rather than asserted — and it is why this row inflates by 0.46%
  while a build that exercises both metrics inflates by 4.10%, close to the
  ~4.9% #488 measured for adding one.

  Both inflations are **structural, not earned**: at the shipped defaults
  `pass_aim_error` measures 0.383 inside its [0.15, 0.75] band and
  `pass_lead_time` 0.408 inside its [0.1, 0.6], so both score desirability
  ≈ 1.0 per match and raise the geometric mean of every future build
  regardless of whether that build passes any better.

  ### These two metrics fold into `fun` as an INTERIM state, superseded by #528

  Whether a newly-registered metric folds into the score immediately or waits
  was #487's explicitly undecided question. #488 settled it by default; this
  entry would have been the second to do so. **It is now decided: #528 puts
  newly-registered metrics on probation** — reported alongside the score but
  excluded from the geometric mean until they have a hands-on pilot and bands
  that are not self-fit. The +4.10% and +0.46% figures above are the evidence
  that decision was made on.

  So the v9 row's `fun` **is folded over eleven metrics and should not be**,
  and that is a known, measured, time-boxed condition rather than an
  oversight. #491 deliberately does not build the probation mechanism: it is a
  registry change plus a promotion procedure plus its own go-red test, and
  burying a governance mechanism inside a passing-feature PR is the mistake
  #506/#515 exist to avoid.

  **What #528 will move when it lands**, so nobody has to rediscover it. The
  folded `fun` value reaches exactly two re-recorded artifacts and no others:

  - `gc_data::outfield_ai_baseline`'s `RECORD.stats.fun`
    (mean `0.2647910531282728`, sd `0.3568961197946737`, min `0.0`, max
    `0.8918478433191761`) — and therefore its `signature`
    `ac397926cf724b7b`, since `TRACKED` includes `"fun"` and the signature
    folds every tracked key's statistics, and therefore `baseline_version`,
    which the recorder bumps.
  - The `fun` row of the table above.

  It reaches **none** of the trajectory fixtures
  (`session_legacy_ordinary_baseline.txt`, `match_step_ai_ai_baseline.txt`,
  `session_ai_driven_baseline.txt`), none of `omp1_determinism.json`, none of
  the six mirrored digest sites, and none of
  `keeper_shadow_classifier`'s counts — those are positions, hashes and event
  tallies, with no metric fold anywhere in them. So #528's re-record is one
  baseline and one drift-log row, not another full sweep.

  **The selection change contributes NOTHING to this table, and that is worth
  stating rather than leaving implicit.** This control is all-AI
  (`HeadlessBot::None`), and `r#match::select_pass_target` — the soft cone — is
  reached only from the human/bot-driven input path. The match AI picks its
  own receiver through `outfield_decision` and never consults the cone. Every
  row above is the lead solver and the speed curve; the numbers were measured
  identically before and after the selection half was finished, which is how
  this was confirmed rather than assumed.

  **`pass_completion` did not move, in either direction, and #491's premise
  about it no longer holds.** The issue opens with "pass completion is 0.52
  against a band floor of 0.55". Measured at `c0fc6cf` — before any of this
  work — over 48 full-length matches it is **0.6200 ± 0.0098**, already inside
  the 0.55–0.85 band; #488, #489 and #490 all landed after the issue was
  written. After this change it is **0.6189 ± 0.0114**: unchanged, well inside
  one standard error. The load-bearing half of the issue's claim is the other
  one — that no tunable moves it — and that is confirmed and only partly
  fixed. All four pre-existing pass knobs measure DECORATION against
  `pass_completion`, and so do all eleven new ones. What changed is that the
  subsystem now has knobs with measurable effects on the quantities they
  govern; see `gc-sim/tests/knob_contract.rs`'s census block for the numbers
  and `gc_sim::r#match::PassShadowTally` for why two new metrics were
  registered rather than the knobs being argued fine.

  **`save_rate` moved further from #487's 0.45–0.75 band, not closer**
  (0.9275 → 0.9114 — toward it, but from far outside). `goals_total` 1.50 →
  1.68 moves toward the 2.0–4.5 band. Neither is a balance pass; balance
  tuning is out of scope for this change and both are reported rather than
  tuned away.

  `gc-sim/tests/keeper_shadow_classifier.rs` was re-pinned in the same commit
  and against these same 60 seeds: candidates 10507 → 9208, `disagree_height`
  27 → 10, `disagree_deferred` 210 → 171, `new_only` still structurally 0.
  Fewer shots reach a keeper because a led pass keeps possession sequences
  alive longer between loose balls.

- **2026-08-13 — the locomotion primitive: momentum, turn arcs, decoupled
  facing, and carry composed with direction (#488, PR #516).** `baseline_version`
  **2 → 3**, signature `9bf9c999d7b077f8` → `614ed81d38e82116`. The first entry
  in this log measured on `gc-sim` rather than the deleted Lua tree, and the
  first re-freeze driven by a recorder
  (`record_outfield_ai_baseline`) instead of a hand edit — the runner this
  module's own doc recorded as missing.

  Bodies now carry momentum, turn through bounded arcs instead of pivoting,
  and face independently of where they move; carrying is a modifier composed
  onto the direction context rather than one of seven mutually exclusive
  contexts, so a shielding carrier keeps carry's handling penalty instead of
  silently getting the empty-handed profile.

  | metric | frozen (v2) | re-frozen (v3) | delta |
  | --- | --- | --- | --- |
  | `fun` | 0.283436 | 0.269890 | −0.013546 |
  | `goals_total` | 1.750000 | 1.500000 | −0.250000 |
  | `shots` | 32.200000 | 34.083333 | +1.883333 |
  | `shots_per_goal` | 21.732390 | 22.988333 | +1.255943 |
  | `save_rate` | 0.910569 | 0.927511 | +0.016942 |
  | `pass_completion` | 0.570084 | 0.558467 | −0.011618 |
  | `turnovers_per_min` | 8.258094 | 8.740769 | +0.482675 |
  | `possession_balance` | 0.542468 | 0.547505 | +0.005037 |
  | `longest_drought_s` | 11.718056 | 11.449444 | −0.268611 |
  | `decided_late` | 0.596003 | 0.694121 | +0.098118 |
  | `ai_dribble_close_share` | 0.821165 | 0.833517 | +0.012351 |
  | `ai_dribble_sprint_share` | 0.179538 | 0.147692 | −0.031846 |
  | `ai_dribble_touches_per_min` | 118.829505 | 107.620664 | −11.208841 |
  | `ai_dribble_heavy_losses_per_min` | 0.530382 | 0.510557 | −0.019825 |
  | `ai_jukes` | 31.966667 | 32.716667 | +0.750000 |

  **Read `fun` carefully: it is not comparable across these two rows.** `fun`
  is a geometric mean over however many metrics extract a value, and this
  change registers a ninth (`time_to_reverse`). Measured on one build, the
  same game scored 0.184 over eight metrics and 0.193 over nine. The −0.0135
  above therefore understates the like-for-like movement, and any future
  comparison against v3 must hold the metric set fixed.

  **`possession_balance` did not move.** It reads +0.005 against a per-match
  sd of 0.063 and a 60-seed standard error of about 0.008 — inside the noise,
  and it would take roughly 900 seeds to call. This matters beyond one row:
  #488 was justified by a claim that possession balance "sits at 0.33 against
  a 0.35–0.65 band", and the v2 baseline had it at 0.542, already mid-plateau.
  The metric was never out of band and the primitive does not move it. Two
  earlier re-freezes were declined while this regression stood at −0.132;
  what changed is the carry-composition fix, not the appetite.

  Fewer, heavier touches, more shielding, fewer heavy losses, shorter
  droughts: the possession *mechanics* moved in the intended direction even
  though the possession *metric* did not.

- **2026-08-11 — the keeper's dive-timing/contact-point query replaces a
  gravity-only quadratic with a real sampled trajectory (#486, sliced from
  #490).** `crates/gc-sim/src/match.rs`'s `attempt_save` used to compute the
  ball's height at the keeper's line as
  `s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz` — a closed form that
  never modeled the ground bounce, air drag, or the cage ceiling, and (for a
  ball that has already landed) is not even bounded below zero. It now asks
  `ball_prediction::BallPredictor::position_at_time`, an authoritative query
  against the same `ball_flight::step` the live ball actually runs. A query
  that cannot resolve inside `predict.max_horizon` (2.0s) — only reachable
  for a shot decaying so close to `keeper::travel_time`'s own dead-ball
  cutoff that `eta` is technically `Some` but implausibly large — defers the
  commit rather than guessing; `attempt_save` runs every live tick and `tz`
  only shrinks as the ball closes in, so the shot still resolves once it's
  inside the horizon (`crates/gc-sim/tests/keeper_prediction.rs`).

  This moved the `combat_disabled_control_a` baseline
  (`gc_sim::outfield_ai_baseline`, seeds 20001..20060, n=60, paired: same
  seeds, same fixture, before/after this change only):

  | metric | before (v1) | after (v2) | delta |
  | --- | --- | --- | --- |
  | fun | 0.291774 | 0.283436 | -0.008338 |
  | goals_total | 1.916667 | 1.750000 | -0.166667 |
  | goals_home | 0.633333 | 0.633333 | +0.000000 |
  | goals_away | 1.283333 | 1.116667 | -0.166667 |
  | shots | 31.766667 | 32.200000 | +0.433333 |
  | shots_per_goal | 19.656918 | 21.732390 | +2.075472 |
  | save_rate | 0.892550 | 0.910569 | +0.018019 |
  | passes | 32.633333 | 33.966667 | +1.333333 |
  | pass_completion | 0.583350 | 0.570084 | -0.013266 |
  | turnovers_per_min | 8.222948 | 8.258094 | +0.035146 |
  | possession_balance | 0.544171 | 0.542468 | -0.001703 |
  | longest_drought_s | 11.483333 | 11.718056 | +0.234722 |
  | decided_late | 0.647152 | 0.596003 | -0.051148 |
  | lead_changes | 0.100000 | 0.083333 | -0.016667 |
  | margin | 1.116667 | 1.083333 | -0.033333 |
  | duration | 113.612778 | 116.696667 | +3.083889 |

  (`ai_dribble_*` and `ai_jukes` also moved; omitted here as noise
  downstream of the same shots/possession shift, not evidence about the
  keeper itself.)

  **The direction is not what #487 wants, and that is reported here rather
  than hidden.** #487 records `save_rate` at 0.82-0.89 against a healthy
  band of 0.45-0.75 — already far too high — and asks a physically honest
  keeper to plausibly move it. It did move, materially (+0.018, about 2% of
  its own value), but *up*, away from the band, not down; `goals_total` fell
  in step (-0.17) and `shots_per_goal` rose (+2.08). Balance tuning itself is
  explicitly out of scope for this change (owned in parallel by #493's
  tunable-registry work), so nothing here was adjusted to chase the band —
  but the direction deserves an honest account rather than a shrug.

  **Tested with the evidence contract's own instrument, not a bare
  significance threshold.** An earlier draft of this entry used
  `knob_contract`'s `NOISE_SIGMAS = 2.0` — a magnitude-only "is this knob
  wired" threshold — and concluded "not a regression" from failing to clear
  it. A statistics review caught the error:
  `docs/design/combat_fun_evidence_contract.md:563-564` states outright that
  *"'Not significant' is not evidence of non-inferiority or equivalence"*,
  and that document's own §4.4 already carries a **preregistered**,
  locked-before-this-change one-sided non-inferiority margin for `save rate`
  (`95% upper bound B-A < +0.04; B absolute < 0.95`) and for `goals`
  (`95% lower bound B-A > -0.10`). Re-run against those, not an invented
  margin — reusing a real, already-locked instrument rather than choosing a
  new margin after looking at the data, which is exactly what a NI test is
  supposed to prevent.

  **The structural argument comes first — this is a proof for its subcase,
  not a sample, and it is scoped honestly: it covers the grounded/landed
  case, not every case.** For a grounded or already-landed ball
  (`ball_z <= 0, ball_vz <= 0`, true of every candidate this fixture's shots
  reach), the deleted formula `old_z = ball_z + ball_vz*tz -
  0.5*GRAVITY*tz*tz` is strictly decreasing and unbounded below in `tz`: it
  never models the ground bounce, so once the real ball has landed, `old_z`
  keeps falling through the floor while the real trajectory settles or
  bounces back up, meaning `old_z <= z_real` for every `tz` past landing.
  The on-target check (`z_cross < CROSSBAR && z_cross <= KEEPER_AIR_GRAB`)
  is upper-bound-only, so `old_z <= z_real` implies `new_on_target =>
  old_on_target` — for this subcase, the deleted formula can only ever be
  wrongly *permissive*, never wrongly restrictive. **This is a proof for the
  landed subcase specifically, not a universal guarantee.** The pre-bounce,
  still-rising or still-falling-but-not-yet-landed case is a different
  shape (`old_z` and the live discrete step both integrate the same pure
  gravity, so they track each other up to the `+0.5 * GRAVITY * dt * t`
  discretization bias discussed further down) and is argued informally
  there, not folded into this formal claim. `keeper_shadow_classifier.rs`'s
  frozen-fixture `new_only == 0` (0 of 9,376 candidates go the other way)
  is empirical confirmation covering both cases as this fixture happens to
  exercise them, not a substitute for extending the proof itself.

  **The classifier, now committed and pinned
  (`crates/gc-sim/tests/keeper_shadow_classifier.rs`).** Every candidate save
  evaluation on the official 60-seed fixture, real `attempt_save` logic,
  deleted formula computed alongside (never read for a decision):
  9,376 candidates — 3,105 `agree_true`, 6,021 `agree_false`, 227
  `disagree_deferred` (old says on target, the query hadn't resolved yet —
  resolves into an identical real commit one tick later almost every time,
  per `keeper_prediction.rs`'s constructed case), 23 `disagree_height` (old
  says on target, the resolved real height says otherwise — the failure mode
  #486 names, directly measured), 0 `new_only`. Per-match: 6/60 matches touch
  `disagree_height`, 25/60 touch `disagree_deferred`, 30/60 (50%) touch
  either.

  **Byte-identical reconciliation (this was previously asserted, not shown
  — the arithmetic).** 43/60 (72%) matches are byte-identical between old
  and new code on the paired run below; 17/60 (28%) differ. If only
  `disagree_height` caused divergence, "touched" would be 6/60 (10%),
  nowhere near 28%. Folding in `disagree_deferred` reaches 30/60 (50%) —
  on the correct side of 28% to explain it, since a one-tick RNG-stream
  shift (which is what a `disagree_deferred` episode resolving one tick
  later than the old formula would have IS) cascades into a different state
  hash for the rest of a deterministic match without changing anything a
  viewer would call different, and not every such cascade need move the
  final rounded `save_rate`/`goals_total` pair. Deferred episodes, not
  disagreement episodes, are the dominant driver of the split — confirming
  what the review flagged, not what the earlier draft implied.

  **Non-inferiority results, paired, both seed blocks (control arm: the
  deleted formula bypassed in place, per the recipe below):**

  | metric | block | n | paired Δ (B-A) | SE | one-sided 95% bound | margin | NI verdict | approx. MDE (80% power) |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | save_rate | official 20001..20060 | 60 | +0.0180 | 0.0109 | upper +0.0362 | < +0.04 | **passes** | 0.030 |
  | save_rate | supplementary 50001..50200 | 200 | -0.0025 | 0.0027 | upper +0.0019 | < +0.04 | **passes** | 0.008 |
  | goals_total | official 20001..20060 | 60 | -0.1667 | 0.0895 | lower -0.3162 | > -0.10 | **fails** (inconclusive) | 0.251 |
  | goals_total | supplementary 50001..50200 | 200 | +0.0800 | 0.0508 | lower -0.0040 | > -0.10 | **passes** | 0.142 |

  MDE convention: `Δ ± (z_0.975 + z_0.80) · SE ≈ 2.80 · SE` — a **two-sided**
  z at 80% power (`1.96 + 0.84`), not the one-sided `1.645 + 0.84 ≈ 2.49`
  more usual for an NI test's own MDE. The two-sided choice is the more
  conservative of the two (it reports a slightly larger, harder-to-clear
  MDE); it changes no verdict above, but is named here so the number does
  not need reverse-engineering.

  `save_rate` clears the repository's own preregistered harm bound on BOTH
  blocks — the one-sided 95% bound stays under +0.04 even on the least
  favorable evidence available, and the B-absolute catastrophe floor
  (`<0.95`; measured 0.9106 / 0.9076) holds on both too. That is a genuinely
  strong claim, not a power statement: save_rate is non-inferior to the old
  code by this repository's own standard.

  `goals_total` does **not** clear its margin on the official 60-seed block
  alone — the 95% lower bound (-0.316) sits well below the -0.10 harm
  floor, so that evidence cannot rule out a real drop. It does clear on the
  200-seed block. Read together with the MDE column: at n=60 the smallest
  goals_total shift this test could reliably resolve is ≈0.25 goals/match,
  well above the classifier's own bound on the mechanism's size (23
  `disagree_height` candidates, touching 6/60 matches, only 2 tied to a goal
  within 3s) — so the honest reading is **underpowered on the official
  block, not evidence of harm**, corroborated by the 200-seed block not
  reproducing any drop at all.

  **The asymmetry between the two verdicts is the load-bearing fact here,
  and it is worth stating on its own rather than leaving it implicit in the
  MDE column.** `save_rate`'s NI test is **adequately powered**: its MDE
  (0.030 at 60 seeds, 0.008 at 200) sits *below* the `0.04` margin on both
  blocks, so this test could actually have detected harm near the boundary
  and did not — the pass is a real result about the metric, not an artifact
  of a test too weak to fail. `goals_total`'s NI test is **structurally
  underpowered**: its MDE (0.251 at 60 seeds, 0.142 at 200) is 1.4-2.5×
  its own `0.10` margin, so it could not have confirmed harm-free even in
  principle at either sample size — the failure on the 60-seed block is
  uninformative about whether a real effect exists, not evidence that one
  does. Read side by side, "one metric passed, one failed" is the wrong
  takeaway; the honest one is "one test could answer its question, the
  other could not."

  **No multiplicity correction applied** across the 2 metrics × 2 blocks
  above, unlike the evidence contract's own Holm-adjusted procedure for its
  full metric family. Naming it rather than silently borrowing only the
  favorable parts of the contract's machinery: a correction would only make
  the already-failing goals_total/official-block row harder to pass, not
  easier, so it does not change which claims are made above, but a reader
  should not read "two blocks, two metrics, mostly passing" as a formally
  adjusted family result.

  **Conclusion.** `save_rate` is non-inferior to the old formula against
  this repository's own preregistered margin, confirmed on two independent
  seed blocks — that claim is made without qualification. `goals_total` is
  **not** confirmed non-inferior on the official 60-seed block by itself;
  the honest statement there is "no shift detected at this sample size,
  and the sample is underpowered to detect a shift this mechanism's own
  measured rate could plausibly produce" — a power statement, not a
  null-effect statement, and not "not a regression." The mechanism itself
  (§ above) is real, structurally one-directional, and directly measured;
  whether it moves `goals_total` at population scale remains open at n=60
  and is not supported by the 200-seed block either. Nothing here was tuned
  to make any number look better.

  A second, smaller, mechanistically distinct effect exists: for a ball
  already in flight, the deleted formula and the live step function agree
  exactly on ballistic height *except* for the discretization gap between a
  continuous quadratic and the 60Hz semi-implicit Euler `ball_flight::step`
  actually runs (before any bounce), a `+0.5 * GRAVITY * dt * t` systematic
  *upward* bias in the live/predicted height relative to the old formula. It
  pushes in the opposite direction from the bounce mechanism above (some
  genuinely-in-flight shots read as *less* reachable, now correctly) and is
  folded into the same results above.

  **Control-arm recipe, for reproducing the paired rows above.** Bypass the
  predictor in `attempt_save` in place (uncommitted; restore before
  committing anything else): replace the `let Some(sample) = ...else {
  ...continue; }; let z_cross = sample.z;` block with
  `let z_cross = s.ball_z + s.ball_vz * tz - 0.5 * GRAVITY * tz * tz;` —
  i.e. the exact line this PR deletes, fed the same `tz` the real code
  computes. Run `outfield_ai_baseline::measure`/`headless::run_batch` with
  the fixture's declared options against both the bypassed tree and the real
  one, on the same seeds, and pair per seed. This is mechanically
  unambiguous from the diff (`crates/gc-sim/src/match.rs`'s `feat` commit
  shows exactly the line removed) even though the bypass itself is never
  committed — the classifier above is the committed, re-derivable half of
  this investigation; the paired significance table is the uncommitted half,
  reproducible from this recipe.

  Re-frozen as `baseline_version = 2`
  (`crates/gc-data/src/outfield_ai_baseline.rs`), measured with
  `gc_sim::outfield_ai_baseline::measure(&MeasureOpts { baseline_version:
  Some(2), ..MeasureOpts::default() })` and pasted via `::serialize` per
  that module's own re-freeze protocol (this repository still has no runner
  that drives the re-run automatically).

  **Landed on top of #487's identity-only re-pin, carrying both effects.**
  #487 (the tunable registry, merged as `ded59d2`) added `PASS_RANGE_MIN` to
  the shipped knob set and re-pinned `tuning_hash`/`fixture_hash`/`signature`
  for that reason alone — `baseline_version` stayed `1`, and every recorded
  stat reproduced bit-for-bit, because #487 changed no default's value.
  Rebasing this change onto that merged head conflicts on the same three
  identity fields `#487` touched, since both re-pins write `signature`.
  Re-running `measure`+`serialize` against the merged head (rather than
  hand-resolving the conflict toward either side) picks up both effects at
  once: `tuning_hash`/`fixture_hash` land on #487's post-registry values
  (`815f8929cfce068e` / `483fe1d1297befc1` — identical to what #487 itself
  recorded, confirmed by diffing this recording's stats block against the
  pre-rebase one: byte-for-byte identical except `signature`, which is
  derived from identity and so moves whenever identity does) *and* every
  stat reflects this change's real keeper-behavior shift. `baseline_version`
  moves straight from `1` to `2` — #487's re-pin never got its own recorded
  version bump, so there is no `1.5` to pass through.

- **2026-08-10 — a keeper's dive ends when it takes possession (#450).**
  `dive_timer` used to outlive the catch, so on the tick a keeper released the
  ball the off-ball dive branch took it back over: it was dragged toward a
  `dive_target` computed for a shot it had already caught, and `facing` — which
  `select_throw_target` uses as its aim cone — was written by that dive. In the
  pinned reference match this makes the keeper chase and re-catch its own throw
  three ticks after releasing it. Possession now ends the dive.

  The 30-seed tripwire moved fun 0.442 -> 0.508, goals 2.100 -> 2.400,
  shots_per_goal 23.725 -> 20.837, controlled_dribble_sprint_share
  0.274 -> 0.293, controlled_dribble_touches 85.19 -> 92.95,
  controlled_dribble_heavy_losses 0.450 -> 1.373 and ai_dribble_heavy_losses
  0.839 -> 0.532.

  **The 100-match validation says essentially all of that is small-sample
  noise.** Paired runs on the same tree, `love . --sim 100` before and after:
  fun 0.491 -> 0.481, goals 2.220 -> 2.160, shots_per_goal 23.147 -> 21.794,
  save_rate 0.856 -> 0.864, pass_completion 0.577 -> 0.578, turnovers
  4.054 -> 4.009, possession_balance 0.402 -> 0.396, longest_drought
  11.311 -> 11.223, decided_late 0.516 -> 0.493, ai_dribble_heavy_losses
  0.985 -> 1.013. Every one of those is inside a fraction of its own standard
  error; note in particular that `ai_dribble_heavy_losses`, which the 30 seeds
  showed *falling* by a third, does not move at all on 100. The largest
  survivor is `controlled_dribble_heavy_losses` 0.888 -> 1.316 (sd 2.2 / 2.7,
  so roughly 1.7 SE) — suggestive at most, and consistent with the ball
  reaching outfield play slightly more often now that a keeper's distribution
  is not intercepted by the keeper itself (`passes` 67.4 -> 68.3).

  So this refresh is the 30 pinned seeds playing out differently rather than
  the shape of the game moving. No band collapsed and no desirability changed
  materially; the known scoring-scarcity and keeper-wall weaknesses are
  unchanged in kind.

- **2026-07-31 — possessed ball kept inside the arena.** The touchline walls
  only ran on the loose-ball path, so a ball that ended up outside while owned
  was never pulled back: it stranded, the carrier could not walk out to it
  (players are clamped), and only a shot recovered it. An owned ball is now
  clamped to the arena — the pitch plus the two net boxes — with the outward
  pace reflected, the same as the loose walls do. The 30-seed tripwire moved
  fun 0.506 -> 0.442, goals 2.233 -> 2.100, shots_per_goal 21.602 -> 23.725,
  turnovers 3.916 -> 3.783, and ai_dribble_heavy_losses 0.609 -> 0.839.

  The 100-match validation says most of that was small-sample noise: fun 0.491,
  goals 2.220, save_rate 0.856, pass_completion 0.577, turnovers 4.054,
  possession_balance 0.402, decided_late 0.516 — all within noise of the old
  baseline. Two shifts survive the larger sample and are the intended ones:
  **shots_per_goal 21.6 -> 23.1** and **ai_dribble_heavy_losses 0.609 -> 0.985**.
  Both come from the same thing. The clamp binds on only 0.24% of possessed
  ticks (worst excursion 34 px), but on those ticks the boundary now takes the
  ball off a carrier who runs on, where previously the ball simply travelled
  outside the pitch alongside them. Running the ball out of play costing you
  possession is the behaviour we want; the touchline is no longer free space.

  Reflecting the outward pace rather than merely pinning the position is what
  keeps this proportionate — pinning alone cost roughly twice as much
  (ai_dribble_heavy_losses 0.992, goals 1.900, fun 0.401 on the same 30 seeds).
  No band collapsed; the known scoring-scarcity and keeper-wall weaknesses are
  unchanged in kind.

- **2026-07-08 — keeper dive/save sync fix.** Saves now resolve only at glove
  contact, dives launch timed to the ball's friction-true arrival and stop at
  the intercept point, and shots that die short release to the claim logic
  (previously they were vacuumed mid-air — the "invisible wall"). Keepers got
  honestly better: baseline goals 1.30 → 0.93, save_rate 0.775 → 0.82, fun
  0.35 → 0.25. Candidate A re-validated post-fix: **ΔFun +0.466 ± 0.069** on
  held-out seeds (unchanged), goals 3.37, all bands ≥ 0.65. The scoring
  drought at defaults is worse than first measured, which strengthens the
  case for shipping a candidate.

- **2026-07-10 — realistic-mechanics batch** (discrete kick-chase-kick dribble
  + carrier hook, keeper back-pass with feet reception, faster/closest-man
  passing, receiver full-pace trap). Baseline moved UP: fun 0.246 → 0.337,
  goals 0.93 → 1.28, save_rate 0.82 → 0.845, shots_per_goal ~19.5,
  pass_completion 0.51 (still knob-flat, still just below band). Candidates
  re-validated on held-out seeds 1001–1060 (paired):
  **A: ΔFun +0.428 ± 0.068 — still real** (fun 0.30 → 0.73, 2.9 goals,
  save_rate 0.675 in-band; the ~93 s short-match caveat remains).
  **B: ΔFun −0.008 ± 0.063 — DEAD.** The mechanics batch erased B's entire
  edge; AI_SHOOT_RANGE=300 alone no longer beats defaults. B's preset slot
  should be dropped or re-searched once the mechanics settle. Weak dimensions
  at defaults are unchanged in kind: scoring scarcity and keeper walls.

- **2026-07-10 — aerial reception and acrobatic finishing.** Added
  difficulty-resolved chest/leg control, jumping headers and volleys, contested
  high balls, and bicycle kicks. The 30-seed tripwire moved fun 0.431 -> 0.419,
  goals 1.467 -> 1.500, shots_per_goal 22.792 -> 20.386, pass completion
  0.556 -> 0.548, and decided_late 0.611 -> 0.662. The two flagged changes are
  intended consequences of midfielders controlling routine high balls instead
  of forcing every contact goalward. A 100-match validation produced fun 0.439,
  goals 1.69, shots_per_goal 20.93, save_rate 0.858, and pass completion 0.546;
  the known scoring-scarcity / keeper-wall weaknesses remain, with no new band
  collapse.

- **2026-07-10 — uninterferable keeper hand throws.** The aerial system had
  quietly made throws WORSE: a presser could jump-volley the old 34px-arc
  float mid-flight (scenario harness in `spec/sim/keeper_throw_spec.lua`
  measured 75–100% retention pre-fix, with mid-air volley steals). Hand
  throws are now PLANNED (`plan_throw`): a covered receiver gets the ball
  landed to their safe side, any opponent near the lane raises the arc above
  `aerial.MAX_TOUCH_Z`, and a designated receiver auto-runs onto the pass
  when the human gives no input. Scenario retention: 144/144 across every
  presser spot/aim/charge. Tripwire moved fun 0.419 -> 0.348, goals 1.500 ->
  1.300, shots_per_goal 20.386 -> 24.313, decided_late 0.662 -> 0.548 —
  intended: cheap goals off robbed keeper distributions no longer pad the
  scoring column. 100-match validation: fun 0.285, goals 1.29, save_rate
  0.870, pass completion 0.558 (in band for the first time). Candidate A
  re-validated: **ΔFun +0.345 ± 0.069** — smaller than before but still far
  and away real. Scoring scarcity remains the weak dimension and is now
  honest (it was partly keeper-robbery goals).

- **2026-07-10 — close control + standing-start inertia.** Two feel levers
  from Oscar's playtest: DRIBBLE_CLOSE (below ~1.05× move speed the ball is
  GLUED to the feet — knock-on kicks and their risk only at a sprint) and
  START_ACCEL (acceleration builds with momentum from 450 px/s² at rest to
  MOVE_ACCEL at speed; normalized by base speed so sprinting never weakens
  the push-off). Also fixed: the glue rode `run_vel` (intent), so a
  body-checked carrier's ball launched itself out of control — it now rides
  the REALIZED velocity. Tripwire moved fun 0.348 → 0.288 (30 seeds); the
  100-match run: fun 0.272, goals 1.14, turnovers 2.53, possession_balance
  0.31 (bot team holds less of the ball with heavier starts — worth watching
  at the band edge). Candidate A re-validated: **ΔFun +0.275 ± 0.071** —
  still real, margin shrinking as mechanics absorb what knobs used to buy.

- **2026-07-14 — AI dribble intent + proxy tool parity.** Team-AI carriers now
  sprint only with a real runway and juke only after a nearby defender has
  visibly committed; the proxy now covers reactive jukes, charged/lofted
  outlets, chips, and aerial strikes. A paired sweep selected
  `AI_SPRINT_SPACE=70`: 60 made sprint use more human-like but reduced held-out
  fun by 0.181 ± 0.097, while 70 was neutral (+0.002 ± 0.075). The final
  100-match signature is fun 0.383, goals 1.62, shots/goal 24.96, save rate
  0.879, pass completion 0.578, turnovers/min 3.56, and possession balance
  0.420. Controlled vs team-AI dribble was respectively: close-control share
  0.834 / 0.875, sprint share 0.284 / 0.121, juke-time share 0.027 / 0.037,
  and touches per carry-minute 89.2 / 81.7. AI still carries more cautiously
  than the proxy, but it now reaches and uses the risky knock-on branch without
  turning every possession into a sprint.

- **2026-07-17 — keeper back-pass interception.** Designated back-passes used
  the same distance-scaled predictive pursuit as loose-ball claims. On a pass
  travelling toward goal, that projected the ball behind the keeper and sent
  the keeper backward instead of out to meet it. Designated reception now
  steers to the current ball; predictive pursuit remains unchanged for loose
  claims and 1v1 rushes. A 14-scenario regression matrix (seven pass angles,
  pressure from both shoulders) now resolves 14/14 to the keeper with forward
  movement, no attacking steals, and no own goals. The 30-seed tripwire moved
  fun 0.290 → 0.344 and goals 1.467 → 1.633. The required 100-match audit was
  stable against the latest recorded signature: fun 0.383 → 0.392, goals
  1.62 → 1.68, shots/goal 24.96 → 24.10, save rate 0.879 → 0.874, pass
  completion 0.578 → 0.572, turnovers/min 3.56 → 3.45, and possession balance
  0.420 → 0.418. The larger sample shows no new systemic collapse or keeper
  overcommitment.

- **2026-07-22 — explicit keeper commitment, dynamic base depth, and chip
  counterplay.** The accepted base state is no longer a fixed legacy point.
  The keeper stays at the physical one-radius inset when play is far away, then
  advances from 12 px to at most 18 px near the claim edge
  (`12 + min(aggression * 0.15, 6) * approach`). The deliberately exaggerated
  ±40 px near-post bias replaces the legacy ±28 px band: it keeps the keeper's
  body inside the goal mouth while conceding the far corner to an attacker aiming
  away from the keeper. Base locomotion eases inside its final 18 px rather than
  oscillating across the cap. Contextual contain/advance uses the conservative
  centre ray at greater depth. Ground windups visibly set the keeper; a lob cue
  produces a real retreat to the goal line; a through-ball cue retreats an
  already committed keeper without relabelling an already-deep base keeper.
  Release-time movement consumes reaction time without changing physical reach
  or catch/parry RNG.

  AI chip visibility begins at twenty pixels of committed depth, explicitly
  beyond the neutral eighteen-pixel cap; trajectory feasibility still decides
  whether the attacker actually selects the chip.

  This hybrid is evidence-led rather than legacy preservation. A pure shallow
  centre-ray base made the keeper too central for the existing reach model:
  30-match candidates produced save rates from 0.914 to 0.926. Retaining the
  near-post concession while varying physical depth reduced the final 100-match
  save rate to 0.860, versus 0.872 for the rejected fixed-base candidate and
  0.874 before the milestone. The final audit measured fun 0.400, goals 1.750,
  shots/goal 26.484, pass completion 0.564, turnovers/min 3.330, and possession
  balance 0.416.

  Combined keeper-state occupancy was 215.637 s base, 0.074 s advance, 0.163 s
  contain, 7.593 s set, 1.640 s retreat, and 0.791 s recover per match; mean
  shot-release depth was 21.627 px. Across the sample, 39 chips were selected,
  32 were on target, and 12 scored. Mean saves were: base 0.010, advance 0,
  contain 0.020, set 5.680, retreat 0.180, recover 0.010, unclassified 6.610.
  Mean goals were: base 0, advance 0.010, contain 0, set 1.160, retreat 0.120,
  recover 0.020, unclassified 0.280. The unclassified bucket contains
  aerial/context-free outcomes rather than misreporting them as base.

  Chip type and launch remain locked at commit: AI declines infeasible chips,
  while a human-requested infeasible chip remains a deterministic poor chip
  instead of becoming a ground-shot decoy during retreat. After the 100-match
  audit, `love . --tripwire write` generated the new 30-seed guardrail. Key
  fixed-base → dynamic-base values were: fun 0.365731 → 0.431011, goals
  1.766667 → 1.800000, shots/goal 25.993210 → 24.882716, save rate 0.860101 →
  0.832823, pass completion 0.579522 → 0.566531, turnovers/min 3.724430 →
  3.493987, possession balance 0.429692 → 0.427794, drought 11.355556 →
  10.936111, and decided-late 0.651684 → 0.605511. The noisier 30-seed sample is
  retained exactly; the 100-match audit above is the calibration decision.

- **2026-07-25 — personal decision cadence and scored carrier choices.**
  AI outfielders now retain intent on a stat-derived 0.45–0.15 s cadence and
  select deterministic shoot/cross/pass/dribble candidates from distance,
  angle, coverage, space, progress, lane, and interception inputs. Live
  pressure/reception/loose-ball pursuit keeps its existing urgent refresh, and
  lower-composure sampling advances a serialized per-player decision stream
  rather than perturbing physical execution RNG.

  The pre-review 30-seed signature measured fun 0.500, goals 2.067,
  shots/goal 21.273, save rate 0.846, pass completion 0.585,
  turnovers/min 3.589, possession balance 0.411, drought 11.163 s, and
  decided-late 0.545. Controlled/team-AI sprint shares were 0.310/0.093,
  touches per carry-minute were 91.717/85.263, and heavy losses were
  0.592/0.992. The fixed-seed sample therefore catches the intended carrier
  and loose-ball behavior shift, but is too small to decide whether the
  apparent scoring gain is systemic.

  The pre-review 100-match audit measured fun 0.428, goals 1.850,
  shots/goal 23.381, save rate 0.871, pass completion 0.589,
  turnovers/min 3.743, possession balance 0.416, drought 11.550 s, and
  decided-late 0.575. Controlled/team-AI sprint shares were 0.283/0.103,
  touches per carry-minute were 89.194/80.790, and heavy losses were
  0.842/1.278. No target band collapsed; the larger sample keeps the prior
  overall envelope while showing the intended change from blindly continuing
  a carry to reconsidering scored alternatives.

  The refreshed 30-seed guardrail is deliberately selective. It accepts only
  the eight fields that breached tolerance and are directly coupled to the
  new behavior: fun, goals, shots-per-goal, decided-late, controlled
  sprint/heavy losses, and AI sprint/heavy losses. Unrelated or
  still-in-tolerance signatures retain their previous values so the refresh
  cannot hide other gameplay drift.

  Reviewer follow-up corrected carrier space from radial nearest-opponent
  distance to usable forward-corridor distance and tightened decision-state
  ownership boundaries. Its generated 30-seed signature measured fun 0.509,
  goals 2.067, shots/goal 21.691, save rate 0.841, pass completion 0.577,
  turnovers/min 3.828, possession balance 0.408, drought 10.093 s, and
  decided-late 0.594. Controlled/team-AI sprint shares were 0.283/0.098,
  touches per carry-minute were 92.565/76.299, and heavy losses were
  0/1.086.

  The repeated 100-match audit measured fun 0.460, goals 1.980,
  shots/goal 21.354, save rate 0.833, pass completion 0.580,
  turnovers/min 3.770, possession balance 0.404, drought 10.828 s, and
  decided-late 0.555. Controlled/team-AI sprint shares were 0.305/0.097,
  touches per carry-minute were 93.384/77.621, and heavy losses were
  0.321/1.422. All target bands remain intact.

  The follow-up refresh again remains selective: only the seven breached
  fields coupled to carrier route choice and ownership lifecycle moved
  (turnovers/min, drought, decided-late, controlled sprint/heavy losses, and
  AI touches/heavy losses). In-tolerance signatures retain their previous
  values.

  Exact-head review then found that multiple blockers shared a shrinking
  interpolation ceiling, making route clearance depend on player-list order.
  Using one immutable route ceiling and taking the independent minimum produced
  a final 30-seed signature of fun 0.490, goals 2.067, shots/goal 20.676, save
  rate 0.839, pass completion 0.577, turnovers/min 3.785, possession balance
  0.406, drought 10.117 s, and decided-late 0.563. Controlled/team-AI sprint
  shares were 0.281/0.099, touches per carry-minute were 93.090/77.195, and
  heavy losses were 0/1.086.

  The final 100-match audit measured fun 0.460, goals 1.990, shots/goal 20.787,
  save rate 0.832, pass completion 0.580, turnovers/min 3.772, possession
  balance 0.404, drought 10.836 s, and decided-late 0.546.
  Controlled/team-AI sprint shares were 0.305/0.097, touches per carry-minute
  were 93.526/77.937, and heavy losses were 0.321/1.422. All target bands
  remain intact. Only the three additional breached, route-coupled guardrails
  moved (shots/goal, possession balance, and decided-late); all other
  in-tolerance baselines remain pinned.

- **2026-07-25 — stable AI presser assignment and contain-or-commit defending.**
  AI-only defending now retains one team-owned primary presser with a 15%
  switch threshold. The default is a goal-side contain line; reachable
  challenges require one stable trigger reason, while a named low-discipline
  fallback preserves the prior dive-in failure mode. Cover keeps its existing
  interpose foundation and shadows the highest-scored pass-eligible lane.
  Human/fixed-slot players remain input-owned, and presser movement remains
  clamped by stat-derived speed.

  The 30-match signature measured fun 0.459, goals 1.867, shots/goal 24.063,
  save rate 0.853, pass completion 0.591, turnovers/min 3.689, possession
  balance 0.399, drought 11.459 s, and decided-late 0.512. Controlled/team-AI
  sprint shares were 0.271/0.072, touches per carry-minute were 83.641/73.439,
  and heavy losses were 1.113/0.206.

  The 100-match audit measured fun 0.557, goals 2.140, shots/goal 20.626, save
  rate 0.826, pass completion 0.576, turnovers/min 3.636, possession balance
  0.397, drought 10.789 s, and decided-late 0.536. Controlled/team-AI sprint
  shares were 0.290/0.085, touches per carry-minute were 87.032/74.587, and
  heavy losses were 1.023/0.660. The passing, turnover, possession, drought,
  goals, and decided-late target bands remain intact.

  The selective 30-seed baseline refresh accepts only the nine behavior-linked
  fields that crossed tolerance: fun, goals, shots/goal, drought, decided-late,
  controlled touches and heavy losses, and team-AI sprint share and heavy
  losses. Every in-tolerance field retains its previous pin.

- **2026-07-25 — technique-scaled AI kick execution.** AI-controlled
  outfield passes, crosses, and shots now rotate only their final horizontal
  release direction by one seeded normalized draw scaled by the existing
  technique-derived 0–12 degree maximum. Target, receiver lead, speed, loft,
  spin, action choice, and release timing remain unchanged. Maximum technique
  still consumes the shared draw at zero angle, while human, fixed-slot,
  keeper, aerial, selection, and cancelled-windup paths consume none.

  Before any baseline refresh, the 30-match signature moved from fun
  0.459 to 0.472, goals 1.867 to 2.033, shots/goal 24.063 to 24.188, save rate
  0.833 to 0.798, pass completion 0.567 to 0.566, turnovers/min 3.828 to
  3.893, possession balance 0.406 to 0.386, drought 11.459 s to 9.961 s,
  and decided-late 0.512 to 0.617. Controlled/team-AI sprint shares moved
  0.283/0.072 to 0.300/0.083, touches per carry-minute moved 83.641/76.299
  to 90.938/75.389, and heavy losses moved 1.113/0.206 to 0.701/1.281.

  The required 100-match audit compared with the stable-presser audit above
  measured fun 0.557 to 0.555, goals 2.140 to 2.120, shots/goal 20.626 to
  21.163, save rate 0.826 to 0.828, pass completion 0.576 to 0.568,
  turnovers/min 3.636 to 3.965, possession balance 0.397 to 0.392, drought
  10.789 s to 10.404 s, and decided-late 0.536 to 0.594. Controlled/team-AI
  sprint shares moved 0.290/0.085 to 0.295/0.088, touches per carry-minute
  moved 87.032/74.587 to 88.656/72.269, and heavy losses moved 1.023/0.660
  to 0.824/1.173. Every target band remains intact.

  The direction of drift matches the mechanic: imperfect AI releases create
  more AI heavy losses while preserving pass viability and the chosen action's
  physical parameters. Human release vectors remain exact; changes in the
  controlled proxy's carry metrics arise downstream from different AI ball
  trajectories and possession sequences. The selective 30-seed refresh
  accepts only the seven crossed, coupled fields: goals, drought, decided-late,
  controlled sprint/touches/heavy losses, and team-AI heavy losses. Every
  field still inside tolerance retains its prior pin.

- **2026-07-25 — role-gated off-ball runs and stable runner assignment.**
  AI-controlled outfield teammates can now retain one of two team-wide
  in-behind, come-short, or hold-width assignments for a fixed 1.8-second
  lifetime. Eligibility derives from the authored formation role and existing
  match stats; ordinary support, human/fixed-slot input ownership, press
  arbitration, and transition tactics retain their prior contracts.
  The conservative run-drive threshold is 0.55: it admits the authored
  0.56 profile while excluding the 0.54 and slower profiles. In a paired
  60-seed tactic audit, Press High won 20 matches (33.333%) and Counter Attack
  won 16 (26.667%), restoring a +6.667 percentage-point outcome lever versus
  the pre-tuning 0-point result.

  The generated 30-match signature measured fun 0.473, goals 2.033,
  shots/goal 24.655, save rate 0.869, pass completion 0.588,
  turnovers/min 3.836, possession balance 0.405, drought 10.866 s, and
  decided-late 0.551. Controlled/team-AI sprint shares were 0.269/0.089,
  touches per carry-minute were 85.698/75.283, and heavy losses were
  1.420/0.817.

  The 100-match audit measured fun 0.444, goals 1.900, shots/goal 22.947,
  save rate 0.851, pass completion 0.578, turnovers/min 3.780, possession
  balance 0.398, drought 10.596 s, and decided-late 0.594.
  Controlled/team-AI sprint shares were 0.286/0.090, touches per carry-minute
  were 86.278/75.418, and heavy losses were 1.027/0.940. Passing remains
  inside its 0.55–0.85 target band. Goals at 1.900 are 0.100 below the 2–5
  good band, though still above its catastrophic edge. The composite also
  fell to 0.444 from the preceding presser audit's 0.557. The threshold
  deliberately restores tactic liveness at that measured quality cost, so
  hands-on validation still owns the gameplay-quality decision.

  The selective 30-seed refresh accepts only the six behavior-linked fields
  that crossed tolerance: goals, drought, decided-late, controlled heavy
  losses, and team-AI sprint share and heavy losses. Every in-tolerance
  signature retains its previous pin.

- **2026-07-25 — exact-head off-ball run geometry review corrections.**
  Gameplay review found three invalid target cases in the issue-56 resolver:
  an in-behind target could sit behind an already advanced runner, a projected
  or marker-adjusted come-short target could move a nearby runner away from the
  carrier, and hold-width occupancy omitted the carrier. The correction
  requires 24 px of directional progress for in-behind and come-short runs and
  treats the carrier as occupying its current wide lane. Mirrored home/away
  regression scenarios pin each rule.

  Before refreshing the tripwire, the corrected 30-match signature measured
  fun 0.438, goals 1.900, shots/goal 25.256, save rate 0.875, pass completion
  0.579, turnovers/min 3.747, possession balance 0.405, drought 11.554 s, and
  decided-late 0.521. Controlled/team-AI sprint shares were 0.276/0.080,
  touches per carry-minute were 88.599/74.494, and heavy losses were
  1.479/0.471.

  The repeated 100-match audit measured fun 0.451, goals 1.900,
  shots/goal 23.733, save rate 0.855, pass completion 0.571,
  turnovers/min 3.768, possession balance 0.398, drought 10.824 s, and
  decided-late 0.581. Controlled/team-AI sprint shares were 0.276/0.085,
  touches per carry-minute were 85.933/71.322, and heavy losses were
  1.068/0.937. Pass completion, turnovers, possession, drought, and
  decided-late remain in their declared milestone bands. Goals remain 0.100
  below the 2–5 good band; high save rate and shots/goal remain the inherited
  quality exceptions already owned by hands-on review.

  A separate paired 60-seed audit kept the required tactic lever live:
  Press High won 45.0% at home versus Counter Attack's 33.3%, a +11.7
  percentage-point difference. Goals, save rate, and shots/goal had raw
  band-width deltas of +0.65, -0.64, and -4.04; all cleared the 0.5-band moved
  threshold. The separately measured formation comparison is explicitly
  disclosed at -21.7 percentage points, remains beyond its 20-point upper
  gate, and is not addressed by this target-validity correction.

  The selective refresh accepts only the six causally coupled fields that
  crossed tolerance: goals, save rate, drought, decided-late, controlled
  touches, and team-AI heavy losses. Every in-tolerance signature retains its
  previous pin.

- **2026-07-25 — rebased execution-error plus role-run combined head.**
  This audit starts from the merged execution-error baseline on `main`, then
  applies the final role-run implementation and target-validity corrections.
  AI pass, cross, and shot release error remains active at its established
  release seams; stable runner assignment, role gates, support behavior, and
  lead-pass behavior coexist with it. The audit did not reuse the role-run
  branch's stale pre-merge baseline.

  Before refreshing any pin, the combined 30-match signature measured fun
  0.545, goals 2.200, shots/goal 20.193, save rate 0.778, pass completion
  0.568, turnovers/min 3.666, possession balance 0.408, drought 10.398 s, and
  decided-late 0.578. Controlled/team-AI sprint shares were 0.301/0.083,
  touches per carry-minute were 88.783/78.332, and heavy losses were
  0.411/0.991. Relative to the merged execution-error baseline, seven fields
  crossed tolerance and are causally coupled to the new run opportunities:
  fun, goals, shots/goal, save rate, decided-late, controlled heavy losses,
  and team-AI heavy losses.

  The combined 100-match audit measured fun 0.531, goals 2.080, shots/goal
  21.574, save rate 0.827, pass completion 0.562, turnovers/min 3.714,
  possession balance 0.400, drought 10.603 s, and decided-late 0.557.
  Controlled/team-AI sprint shares were 0.286/0.086, touches per carry-minute
  were 87.631/73.229, and heavy losses were 0.745/1.141. Goals, passing,
  turnovers, possession, drought, and decided-late remain inside their target
  bands. Shots/goal and save rate remain the inherited quality exceptions
  owned by hands-on review; no catastrophic guardrail collapsed.

  In the paired 60-seed lever audit, Press High won 30.0% versus Counter
  Attack's 23.3%, a +6.7 percentage-point tactic outcome difference; goals,
  save rate, and shots/goal moved by +0.52, -0.64, and -3.05 respectively.
  The star-swap comparison won 23.3% with Zyro versus 6.7% with Mika, a +16.7
  percentage-point difference, and moved shots/goal by -1.83. Both comparisons
  pass their outcome and metric-movement gates. Balanced and Aggressive each
  won 23.3%, so formation outcome separation remains 0.0 percentage points;
  it is separately disclosed, remains outside the gate, and is not addressed
  by this integration.

  The selective refresh accepts only the seven crossed, behavior-linked
  fields named above. All other fields retain their merged execution-error
  pins, including values that shifted within tolerance.

- **2026-07-26 — tactic-shaped counter-press and counter-attack windows
  (issue 57).** A settled possession change now opens
  one counter-press window for the team that lost the ball and one
  counter-attack window for the team that won it, both decaying through
  `brain.phase` at the losing/winning tactic's authored seconds (Balanced
  2.5/2.5, Press High 3.0/2.5, Counter Attack 0.0/3.0). Inside a counter-press
  the losing side commits its two nearest eligible AI outfielders to the ball at
  full urgency instead of one presser plus a standing-off cover, while the rest
  hold their turnover position and shade the highest-valued outlet lane
  (defensive roles recover toward their anchors). Inside a counter-attack the
  winning side deepens support by formation role and may take one immediate
  in-behind run without the settled-carrier requirement.

  `sim/possession_transition.lua` requires ESTABLISHED possession — the same
  0.7 s hold `turnovers_per_min` counts — before a turnover opens a window. An
  earlier draft triggered on raw ownership flips; those run ~32/min here, so the
  windows refreshed forever and the ordinary attack/defend phases fell to ~3% of
  ticks each (fun 0.545 -> 0.386). With the settled rule the phase budget is
  roughly 6% counter-press / 6% counter-attack per team.

  The 30-seed tripwire moved fun 0.545 -> 0.496, goals 2.200 -> 2.133,
  shots/goal 20.193 -> 20.327, save rate 0.778 -> 0.803, pass completion
  0.567 -> 0.577, turnovers/min 3.828 -> 3.726, possession balance
  0.406 -> 0.410, drought 9.961 -> 10.269 s, and decided-late 0.578 -> 0.541.
  Controlled/team-AI heavy losses moved 0.411 -> 0.627 and 0.991 -> 0.609, and
  team-AI sprint share 0.072 -> 0.088. Five fields crossed tolerance: fun,
  decided-late, controlled heavy losses, team-AI sprint share, and team-AI heavy
  losses. Both metrics named as guardrails in the issue — turnovers/min and
  possession balance — stayed inside tolerance.

  The 100-match audit measured fun 0.477, goals 2.040, shots/goal 21.514, save
  rate 0.841, pass completion 0.576, turnovers/min 4.013, possession balance
  0.405, drought 10.926 s, and decided-late 0.550. Controlled/team-AI heavy
  losses were 0.712/0.679 and sprint shares 0.285/0.086. Goals, passing,
  turnovers, possession, drought, and decided-late remain inside their bands;
  shots/goal and save rate remain the inherited quality exceptions owned by
  hands-on review. No band collapsed.

  Reading of the crossed fields: two hunters closing the ball instead of one
  containing it costs the bot-driven controlled carrier the ball more often on a
  heavy touch, while AI carriers lose it LESS often because the counter-attack
  window gives them deeper support and an immediate in-behind option to pass
  into. The team-AI sprint share rises for the same reason a hunted carrier
  runs. Fun and decided-late follow those two.

  In the paired 60-seed lever audit the tactic lever got LIVELIER: Press High
  won 31.7% versus Counter Attack's 16.7%, a +15.0 percentage-point outcome
  difference (was +6.7 before this change), still inside the 3-20 point gate,
  with three banded metrics moved.

  `data/fun_baseline.lua` is refreshed with this entry, on the owner's explicit
  approval of the drift above. Band status was checked field by field first. Two
  metrics remain OUTSIDE their target bands -- shots/goal 21.514 (band 2.5-6) and
  save rate 0.841 (band 0.45-0.75) -- but both were already outside on the
  previous baseline (20.193 and 0.778); they are the inherited quality exceptions
  owned by hands-on review, and this change moves them a further +0.134 and
  +0.025 without hard-zeroing either (desirability 0.39 and 0.45). Every metric
  that was inside its band stays inside: goals 2.040 [2-5], pass completion
  0.576 [0.55-0.85], turnovers/min 4.013 [1-5], possession balance 0.405
  [0.35-0.65], drought 10.926 s [0-35], decided-late 0.550 [0.4-1.0]. No metric
  left a band it was previously inside, which is the condition that would have
  stopped the refresh. `goals_total` now sits nearest its lower edge
  (desirability 0.79) and is the field to watch if counter-pressing is
  calibrated further.

- **2026-07-28 — deterministic combat AI (#112). NO DRIFT; baseline NOT
  refreshed.** `gameplay_ai/combat/v1` gives AI outfielders an equipment-intent
  channel and the four combat families. That is a real gameplay-AI behaviour
  change, so the ritual was run rather than assumed.

  The 30-seed tripwire moved **every tracked metric by ±0.000**: fun 0.496,
  goals 2.133, shots/goal 20.327, save rate 0.803, pass completion 0.577,
  turnovers/min 3.726, possession balance 0.410, drought 10.269 s, decided-late
  0.541, and all ten dribble diagnostics unchanged to six decimal places.
  `data/fun_baseline.lua` is therefore left exactly as it was: refreshing an
  unmoved baseline would only destroy the evidence that it did not move.

  The reason is structural, not luck. `sim.tripwire` and `love . --sim 100` both
  run through `sim.headless`, which builds soccer-only matches: it never
  constructs a `CombatMatchState`. `match._ai_combat_inputs` is called only
  inside `if combat_state then`, so in a soccer-only match the combat AI does
  not run, allocates nothing, and consumes no RNG. The new per-decision seed is
  derived from the canonical combat tick rather than drawn from `s.rng`, so even
  an executed combat decision cannot perturb the soccer stream.

  The 100-match validation confirms it: fun 0.477, goals 2.040, shots/goal
  21.514, save rate 0.841, pass completion 0.576, turnovers/min 4.013,
  possession balance 0.405, drought 10.926 s, decided-late 0.550 — the same
  numbers as the previous entry, whose band status therefore still stands
  unchanged, including the two inherited quality exceptions.

  **What this baseline does NOT cover.** It measures combat-disabled play, so it
  says nothing about combat balance. The fun signature of combat-enabled matches
  is #149's calibration and #114's disposition; #59 freezes the combat-disabled
  policy this entry describes. When a combat-enabled fun signature is introduced,
  it needs its own baseline and its own tripwire entry — reading this one as
  evidence about combat would be reading it backwards.

- **2026-07-30 — no goal limit (#268). DRIFT; baseline refreshed.** The goal cap
  is gone: a match is decided on score at full time, in every mode
  (`docs/online/match_flow.md`, "A match has no goal limit, online or offline").
  `sim.tripwire` and `love . --sim 100` both run through `sim.headless`, which
  did not pass a `max_goals` of its own and so inherited the simulation default.
  Every batch match therefore used to stop at the third goal.

  The 30-seed tripwire moved six metrics past tolerance: shots/goal
  20.327 → 21.602, save rate 0.803 → 0.854, turnovers/min 3.726 → 3.916, drought
  10.269 → 11.171 s, decided-late 0.541 → 0.466, and controlled dribble
  touches/min 91.634 → 86.854. `fun` (0.496 → 0.506) and goals (2.133 → 2.233)
  stayed in band.

  One row in the refreshed baseline is worth pre-empting: **`ai_dribble_heavy_losses_per_min`
  is `0.609424` before and after, to all six decimal places** — the only one of
  the nineteen tracked metrics that did not move at all. That is a real
  coincidence, not an un-refreshed cell: it was reproduced independently, twice,
  in an isolated checkout with and without the capped-default revert. Heavy
  losses are rare per-match events on the AI side (mean well under one), so the
  handful of extra post-third-goal minutes happened to add none across the 30
  seeds. Do not read it as stale data.

  Every one of those is the same effect seen from a different angle: matches that
  used to be truncated by the cap now play out. The 100-match validation makes it
  literal — `duration` is 120.017 s at min **and** max across all 100, where the
  capped run ended some matches early, and `goals_total` now reaches a maximum of
  7, above the old ceiling of 3. Play after the third goal is disproportionately
  end-of-match play: the extra minutes are the ones with a settled scoreline, so
  they add shots that no longer need to be converted (shots/goal up), saves in
  low-danger situations (save rate up), scrappier possession (turnovers up), and
  longer goalless stretches (drought up). `decided_late` falls because the
  deciding goal is now measured as a fraction of a *full* match rather than of a
  match that ended shortly after it.

  100-match validation at the new rule: fun 0.470, goals 2.150, shots/goal
  22.327, save rate 0.864, pass completion 0.578, turnovers/min 4.109, possession
  balance 0.403, drought 11.333 s, decided-late 0.511. Band status is unchanged
  in kind from the previous entry — goals in band at 0.78, pass completion,
  turnovers, possession balance and drought healthy, and the two long-standing
  quality exceptions (shots/goal 0.35, save rate 0.43) still the weak dimensions.
  Nothing new collapsed, and nothing improved that would need explaining.

  **This is a rules change, not a tuning change**, which is why the baseline is
  refreshed rather than the drift investigated. The old baseline measured a game
  that ended matches at three goals; that game no longer exists. The frozen
  Outfield AI baseline is the opposite case and is deliberately *not* refreshed:
  it passes `max_goals = 3` explicitly (`sim/outfield_ai_baseline.lua:67`), so
  `love . --ai-baseline` still compares exactly and #148/#149 keep an intact
  control. Whether that control should itself be re-frozen under the no-limit
  rule is #149's calibration decision.
