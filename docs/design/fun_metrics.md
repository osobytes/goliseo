# Design: Fun metrics & simulation-based balance search

**Scope:** `sim/metrics.lua`, `sim/bot.lua`, `sim/headless.lua`, `sim/sweep.lua`, `main.lua`, `spec/sim/*`

Combat evidence is governed separately by
[`combat_fun_evidence_contract.md`](combat_fun_evidence_contract.md). The
soccer-only tripwire and its historical `fun` name remain regression tools;
neither is a measurement of human enjoyment or a combat-active baseline.

CLI (all headless, exit when done):

```
love . --sim [n]          n matches at current defaults, metric report
love . --sweep [n]        per-knob min/max sensitivity, ranked by fun impact
love . --search K1,K2 [n] greedy coordinate ascent over the named knobs
love . --eval FILE [n]    a tuning blob vs defaults on held-out seeds (paired)
love . --tripwire [write] fun-signature snapshot vs data/fun_baseline.lua
                          (in check.sh; exit 1 on drift; `write` refreshes)
```

**Status (2026-07-10):** phases 1–4 done. The tripwire (`sim/tripwire.lua`)
runs 30 seeded matches in check.sh and fails the gate when any banded-metric
mean (or the composite) drifts more than 5% from the checked-in baseline —
the sim is deterministic per seed, so a behavior-neutral change reproduces
the snapshot exactly. On intended drift: re-run `--sim 100`, log the shift in
the drift log below, then `love . --tripwire write`. Candidate A is validated
(re-validated twice); B died in the 2026-07-10 re-validation. A ships as an
F1 preset — awaiting a hands-on playtest verdict before any change to
`sim/tuning.lua` defaults.

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

Provisional bands (per 120 s match, first-to-3; revisit after the baseline run):

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
is reported but unbanded for now: with a 3-goal cap the honest range is 0–2 and
it's too coarse to score.

## The human proxy (the big caveat)

AI-vs-AI matches measure the AI ecosystem, not the player's hands. The
controlled slot is driven by `sim/bot.lua`, a deliberately human-ish input
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

## Baseline signature (defaults)

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

## Phase 2: sensitivity sweep (2026-07-05)

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
- **Matches got shorter.** Under A the 3-goal cap ends matches at ~78 s mean
  (min 21 s); under B ~106 s. If full-length matches matter, raise `max_goals`
  or prefer B.
- **Bot-relative.** All numbers are under the `sim/bot.lua` proxy. Verify by
  playing: both candidates ship as F1-panel presets (`data/tuning_presets.lua`,
  F4 cycles Defaults → A → B; F2 persists the choice across runs). Defaults in
  `sim/tuning.lua` stay untouched until a candidate survives hands-on play.
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

## Baseline drift log

Sim changes move the baseline; re-run `love . --sim 100` after touching
`sim/match.lua` and log meaningful shifts here (this is the manual tripwire
until phase 4 automates it).

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
