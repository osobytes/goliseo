# Design: Combat-soccer fun evidence contract

- **Status:** preregistration contract, pending multidisciplinary design review
- **Contract version:** `combat_fun_evidence_contract/v1`
- **Decision owner:** issue
  [#114](https://github.com/osobytes/galactic-cup/issues/114)
- **Instrumentation owner:** issue
  [#148](https://github.com/osobytes/galactic-cup/issues/148)
- **Calibration owner:** issue
  [#149](https://github.com/osobytes/galactic-cup/issues/149)
- **Human-study owner:** issue
  [#151](https://github.com/osobytes/galactic-cup/issues/151)
- **Related contracts:** [combat interaction](combat_interaction_contract.md),
  [soccer-shape metrics](fun_metrics.md), [snapshot/replay](../online/snapshot_replay.md)

This document decides what evidence can support the claim that fixed-loadout
combat improves GOLISEO. It does not implement telemetry, tune combat, run a
study, or decide whether the prototype proceeds.

## 1. Evidence labels and criterion hierarchy

Every material claim in this contract has one of three authorities:

- **Evidence (`E`)** — a finding or measurement rule supported by a cited
  peer-reviewed source. A citation does not turn a project threshold into a
  scientific fact.
- **Decision (`D`)** — a falsifiable GOLISEO product or analysis choice. These
  choices are preregistered here so they cannot move after results are known.
- **Hypothesis (`H`)** — an exploratory prediction. Hypotheses may explain or
  motivate follow-up work, but cannot waive a failed gate.

The criterion-source order is:

1. hard deterministic safety and simulation invariants;
2. human enjoyment, agency, comprehension, and negative-experience evidence;
3. soccer-integrity outcomes;
4. combat-to-soccer, counterplay, and horizontal-balance diagnostics;
5. AI-proxy and adversarial evidence.

No lower source can replace a higher one. In particular, AI-vs-AI behavior,
action volume, retention, playtime, close scores, and the existing
`MatchMetrics.fun` soccer-shape proxy are not measurements of human fun
([E: telemetry supplements GUR][drachen], [E: playtime is not a sufficient
experience proxy][vuorre]).

### 1.1 Product construct and non-goals

**Fun (`D`)** is a voluntarily desired, positively experienced match in which
players understand their options, can improve, and want to explore another
plan.

The primary question is whether the default mixed-family ruleset improves
post-condition enjoyment by a practically worthwhile amount over the same
build with combat disabled. Mechanism constructs are autonomy, mastery,
appropriate challenge, ease of control, progress feedback, goals/rules
clarity, counterplay comprehension, and causal comprehension. Guardrails are
soccer primacy, control availability, frustration, overload, fairness,
accessibility, and subgroup harm.

The following are explicitly not the construct: attack frequency, hit rate,
spectacle, arousal, combat time, win rate, score closeness, match length,
rematch rate, retention, monetization, compulsion, NPS, or a new composite of
unvalidated combat measures (`D`).

### 1.2 Falsifiers

The prototype cannot receive `proceed` when any of these is reproducible:

- continuous combat disable exceeds 30 ticks or recovery immunity is shorter
  than 45 ticks;
- a normal-context threat has neither a perceivable authoritative cue nor an
  actionable response;
- a keeper is targeted, AI uses privileged information for a representative
  claim, or presentation changes simulation truth;
- a family or loop is materially superior in every priority context;
- off-ball harassment is a dominant winning policy;
- combat improves raw goals or turnovers while enjoyment, agency,
  readability, or soccer primacy suffers material harm;
- impactful source, family, direction, or response comprehension is below the
  locked floor;
- deterministic event, replay, or attribution reconciliation fails; or
- the allowed bounded revision still rules out the smallest worthwhile
  enjoyment gain.

### 1.3 Exploratory hypotheses

- `H1`: combat improves enjoyment through autonomy, mastery, and readable
  soccer consequences, not through action volume alone.
- `H2`: guarded/missed commitments create punishable space without making
  unarmed play nonviable.
- `H3`: family tradeoffs transfer across presentation swaps when cue grammar is
  preserved.
- `H4`: unattributed off-ball actions are more common in adversarial policies
  than in the observable human-proxy population.

Failure to support a hypothesis is retained; it does not silently redefine a
gate.

## 2. Current implementation truth and telemetry ownership

This inventory is authoritative for the commit at which version 1 is accepted.
Future schema versions must update the map rather than silently claiming that a
field already exists.

### 2.1 Available authoritative inputs

`sim/combat.lua` currently supplies:

- `CombatEvent.kind`: `commit`, `projectile_spawn`, `projectile_expire`,
  `contact`, `ball_spill`, `forced`, and `guard_recoil`;
- canonical `tick`, `family_id`, source/target player indices,
  `source_sequence`, contact result (`hit`, `extended`, `guarded`, `immune`,
  `superseded`), position, interruption ticks, and displacement;
- per-player family, action phase, phase/cooldown counters, forced state,
  forced/chain/immunity counters, and source sequence; and
- projectile source, position, direction, and remaining lifetime.

`sim/match.lua` currently supplies score, time, ball/owner state, all player
positions and soccer commitments, and one-tick `MatchEvent`s for shots, passes,
touches, tackles, saves, blocks, claims, aerial actions, and jukes.
`sim/metrics.lua` derives the existing soccer-only `MatchMetrics`, and
`sim/tripwire.lua` compares its checked-in 30-seed signature.

Combat snapshot version 1 and match snapshot version 6 serialize combat state
and events. Input tape version 2, replay, and rollback-confirmed event handling
provide deterministic identity and correction boundaries. Only confirmed
events may enter evidence.

### 2.2 Known gaps and assigned owners

| Gap | Why the existing shape is insufficient | Owner |
| --- | --- | --- |
| Accepted/rejected equipment intent and typed reason | `prepare_inputs` consumes or ignores an edge without emitting an outcome | #148 |
| One terminal lifecycle outcome per accepted intent | A melee miss and several cancellation paths have no terminal event | #148 |
| Perceivable cue start/end, occlusion, overlap, and response opportunity | Not authoritative simulation state and not in presentation evidence | #148 with #147 presentation hooks |
| Stable AI decision reason and observable-information declaration | Combat AI is not implemented; generic brain helpers do not classify combat purpose | #112 |
| Off-ball purpose | Requires authoritative state; bot intent additionally needs stable #112 reasons | #112 emits bot reason; #148 records/reconciles deterministic context |
| Settled-possession and soccer-event source linkage | `MatchEvent` has no combat source sequence | #148 |
| Miss/expiry for every family and defended/ignored commitment costs | Only ranged expiry and contacts are explicit | #148 |
| Pass progression, retained space, lane denial, chance, and prevented-shot definitions | Not present as canonical event fields | #148 |
| Player-visible feedback for a rejected intent | Presentation behavior, not an event today | #147 implements; #148 records feedback state |
| Versioned combat-active report and raw/config hashes | No combat metrics collector exists | #148 |
| Family calibration, exploit search, sensitivity, and untouched holdout execution | Evidence operation, not schema work | #149 |
| Human observation, instruments, debrief, and participant evidence | Not simulation data | #151 |
| Final contradiction resolution and product disposition | Must integrate all evidence without retuning | #114 |

No participant, survey, presentation, or research-session field may be added to
`MatchState`, `CombatMatchState`, a snapshot boundary hash, or input-tape
identity. Milestone 12 issues [#131][i131]–[#135][i135] may later automate
governance, data contracts, consent/storage/deletion, and protocol operation;
Milestone 11 uses a documented local/manual protocol and does not duplicate
those implementations.

## 3. Freeze, fixtures, seeds, and identity

### 3.1 Freeze states

Evidence progresses through four states:

1. **Design review:** this version is reviewed with no comparative result
   visible.
2. **Instrumentation/burden pilot:** A/A and formative-only sessions may
   estimate variance, missingness, comprehension, burden, and practical
   margins. Pilot data cannot enter confirmatory estimates.
3. **Freeze:** contract version, schema/ruleset, primary endpoint, numeric
   margins, models, exclusions, seed manifests, and decision rules are hashed
   and committed.
4. **Evaluation:** calibration is read first; the holdout is opened exactly
   once at the declared checkpoint.

Any post-freeze change creates `combat_fun_evidence_contract/v2` (or later),
records author, timestamp, evidence already inspected, rationale, affected
outcomes, and reviewer dispositions, and allocates a new untouched holdout.
Earlier evidence remains retained and labeled. No file is overwritten.

### 3.2 Fixed experiment matrix

| ID | Fixture | Required comparison |
| --- | --- | --- |
| A | Same build, combat disabled, neutral loadouts | causal control and immutable soccer baseline |
| B | Default mixed family: keeper plus unarmed, guard, light melee, ranged | intended prototype |
| C | Mirrored matched-stat family/role rotations | family × role × side without authored-player confounding |
| D | Unarmed-only and opt-in repeated-family lineups | viability and exploit discovery; human use only after safety review |
| E | Identical-family presentation/theme swaps | identical hashes/events; human readability transfer |
| F | Human-proxy versus adversarial policy populations | representative behavior versus exploit discovery |

Every machine fixture records commit/build, schema, ruleset, content, tuning,
fixture and bot-policy hashes; seed, side, role, player stats, loadout,
runtime/platform, command, and raw artifact hash.

### 3.3 Locked machine seed sets

These integer seeds are manifest identities, not invitations to inspect a
later set early:

- instrumentation A/A: `18001..18030`;
- formative instrumentation/sensitivity pilot: `19001..19030`;
- paired calibration/common-seed matrix: `20001..20060`;
- adversarial scenarios/search: `21001..21060`;
- untouched confirmatory machine holdout: `30001..30060`; and
- replacement holdout after an accepted post-holdout amendment:
  `31001..31060`.

The checked-in soccer tripwire remains seeds `1..30`. Historical soccer
evaluation seeds, including `1001..1060`, are already known and cannot become a
combat holdout.

Calibration, adversarial, and holdout manifests are committed before any
holdout run. A and B use common random numbers. C mirrors side and rotates each
family through every outfield role and matched-stat player. Family presentation
swap E reuses the exact content/input/seed stream except for presentation
identity, which must not enter simulation identity.

Human seed blocks are generated from a separately committed assignment
manifest. Within a participant, A/B order, side, role, and seed block are
balanced, and a recognizable match is not repeated. Assignment labels are
opaque to analysis (`condition_x`, `condition_y`) until the primary table is
locked.

## 4. Metric dictionary rules

All P0 metrics use confirmed canonical ticks at 60 Hz. Reports contain the raw
numerator and denominator, not only a rate. `NA` is distinct from zero; a zero
denominator produces `NA` plus a typed missing-denominator reason and remains
visible in sample counts. No non-finite value is accepted.

Unless a row narrows them, all metrics:

- exclude loading, pause, kickoff hold, goal stoppage/replay, post-finish
  ticks, and invalid/reconciled-away sessions;
- report player, match, fixture, family, role, side, possession phase,
  score state, skill/experience, human/bot policy, input device, frame-rate/
  latency, viewport, and accessibility strata when applicable;
- report independent player/match counts, distribution, interval, p50, p95,
  p99, and max for tail-sensitive measures; and
- are diagnostics unless explicitly named as a safety, value, or
  non-inferiority gate.

The `source/owner` column distinguishes existing fields from required work.
`CE` means `CombatEvent`, `CS` means `CombatMatchState`, `MS` means
`MatchState`/`MatchEvent`, and `R112` means the stable observable AI-reason
code required from #112.

### 4.1 P0 control and safety

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `involuntary_disable_share` | ticks with `forced_ticks > 0` / eligible active-play ticks; share per player-match | common exclusions; no eligible ticks → `NA: no_active_play` | lower; player/family/matchup, entire match | disconnect or voluntary commitment mislabeled as disable; shortening matches to lower counts | `CS.forced_ticks`; #148 collector |
| `max_continuous_disable_s` | longest contiguous forced run / 60; seconds per player-match and scenario | chain split only by a tick with action available; no forced tick → 0 | hard max `<=0.5 s`; family/matchup; all play | averaging hides one breach; reset/stoppage can hide a chain | `CS.forced_ticks/chain_ticks`; #148 invariant reconciliation |
| `repeat_interruptions_before_recovery` | contacts that hit/extend from chain start until first empirically actionable tick; count per chain | guarded/immune/superseded do not increment; no chains → 0 with zero chains reported | lower; attacker/target family and same-victim streak | treating nominal immunity as recovery when another soccer state still blocks action | `CE.contact`, `CS`, `MS`; #148 derives actionable tick |
| `chain_extension_rate` | `contact.result=extended` / contacts against targets already forced; rate per player-match | zero eligible contacts → `NA: no_forced_target_contacts` | lower; family/matchup; current chain | attackers avoiding forced targets can look healthy while targeting one victim immediately after immunity | `CE.contact`; #148 |
| `immunity_contact_rate` | `contact.result=immune` / contacts received during recovery immunity; rate | zero immunity contacts → `NA` | diagnostic; target/family; immunity interval | high value may show readable counterplay or harassment, not success | `CE.contact`, `CS.immunity_ticks`; #148 |
| `recovery_immunity_s` | contiguous normal-action ticks after forced state before immunity expires / 60; seconds per chain | stoppage truncation reported, never counted as a pass; no chains → `NA` | hard minimum `>=0.75 s` for an untruncated chain | using nominal timer while another combat outcome still disables control | `CS.immunity_ticks`; #148 invariant test |
| `same_victim_targeting_streak` | consecutive accepted commits targeting/contacting the same opponent before another opponent or 3 s idle; count per attacker-victim run | targetless misses retain the prior victim only when a recorded opportunity names that victim; absent target context → `NA` | lower tail; attacker/target/family/score | target selection is not currently encoded; contacts alone miss harassment attempts | CE plus deterministic opportunity; `R112` for bots; #148 fold |
| `time_under_threat_share` | eligible ticks inside at least one perceivable viable threat window / eligible active ticks; share per player-match | no eligible play → `NA`; overlapping threats count once | lower tails; target/family/cue/accessibility | invisible cues make telemetry falsely low; spawning harmless cues inflates denominator | presentation cue + viability; #147 hook, #148 schema |
| `voluntary_commitment_share` | player-chosen windup/aim/guard/active/recovery ticks / eligible active ticks; share | forced ticks excluded from numerator even if phase remains; no play → `NA` | diagnostic, not “disable”; family/role | mashing can look like engagement; shorter recoveries can inflate action volume | `CS.phase`; #148 |
| `reduced_movement_share` | voluntary committed ticks with multiplier `0 < m < 1` / eligible active ticks; share | forced zero-movement ticks excluded; no play → `NA` | diagnostic; phase/family | cannot be called loss of control; stationary player intent confounds realized speed | `CS.phase` plus family catalog; #148 |

The 30-tick disable and 45-tick immunity rules are scenario/property
invariants. No confidence interval, low population frequency, or mean can waive
one reproducible violation.

### 4.2 P0 intent and lifecycle reliability

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `intent_acceptance` | accepted legal equipment presses / legal requested presses; rate per player-match | physically malformed frames excluded as protocol failures; no legal requests → `NA` | higher; family/state/device; same tick | redefining “legal” after rejection inflates the rate | missing accepted/rejected event; #148 |
| `unexpected_rejection_rate` | legal requests rejected without a typed allowed reason / legal requests; rate | no legal requests → `NA` | hard target 0; family/state | broad catch-all reason hides defects | #148 typed outcome/reason |
| `unreadable_rejection_rate` | rejected requests without matching visible feedback state within 6 ticks / rejected requests; rate | no rejected requests → `NA` | hard target 0 for unexpected rejection; device/accessibility | flashing generic feedback for every input can game rate without comprehension | #147 feedback state, #148 linkage |
| `lifecycle_reconciliation` | accepted sequences with exactly one terminal resolution / accepted sequences; rate plus duplicate/orphan counts | unfinished match may use typed `match_terminated`; no accepted sequences → `NA` | hard target 1.0; family/terminal kind | inventing “cancelled” at report time hides missing events | `CE.source_sequence` incomplete; #148 adds miss/expire/interrupted/cancelled |
| `cooldown_use_share` | ticks cooldown is positive / eligible active ticks; share per action and player-match | no accepted action → `NA` | diagnostic; family/outcome | high cost can look like “depth”; low use can mean avoidance or irrelevance | `CS.cooldown_ticks`; #148 |

Allowed rejection reasons are versioned and closed: protected keeper/no
loadout, kickoff hold, soccer commitment wins, aerial state/recovery, forced
state, already committed, cooldown, missing press edge, and malformed input.
`unknown` is invalid. #112 uses the same outcomes as a human producer and
cannot bypass them.

### 4.3 P0 causal combat-to-soccer funnel

One encounter begins at an accepted `commit` and is keyed by
`source_sequence`. Its terminal combat branch is exactly one of miss/expire,
guarded, immune, superseded, unguarded hit, interrupted, or cancelled.

The **primary attribution window (`D`) is 180 ticks / 3.0 seconds**, beginning
at terminal combat resolution. Attribution stops earlier at a goal stoppage,
other stoppage, a new unrelated opponent possession, or the first independent
soccer event whose source cannot descend from the encounter. Sensitivity
reports repeat the attribution at 90 ticks (1.5 s) and 300 ticks (5.0 s).
The primary window may change only from blinded instrumentation-pilot evidence
before freeze; all three windows remain reported.

When several encounters could claim one consequence, use the latest eligible
causal encounter affecting the player/ball involved; exact-tick ties choose the
lowest source sequence. A soccer consequence belongs to at most one encounter.
Prevented shots require an observable shot opportunity in the frozen
counterfactual heuristic; they are never inferred merely because no shot
occurred.

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `attempts_per_player_min` | accepted commits / eligible player-minutes; rate per player-match | no active minutes → `NA` | diagnostic; family/role/phase | spam masquerades as value | `CE.commit`; #148 |
| `terminal_branch_rate` | encounters in each terminal branch / resolved encounters; multinomial rates | unresolved at termination reported separately; no encounters → `NA` | diagnostic by branch; family/role | collapsing guarded/immune/superseded loses counterplay | missing terminals; #148 |
| `contact_rate` | contacts / resolved encounters; rate | no encounters → `NA` | diagnostic; family/phase | melee can miss without an event today | `CE.contact`; #148 adds misses |
| `unguarded_hit_rate` | `contact.result=hit or extended` / contacts; rate | no contacts → `NA` | diagnostic, not success; family/target | rewarding hits encourages spam and weak defense | `CE.contact`; #148 |
| `spill_rate` | encounters with `ball_spill` / resolved encounters; rate | no encounters → `NA` | diagnostic; possession/family | spills without opponent recovery have little soccer value | `CE.ball_spill`; #148 |
| `forced_state_rate` | encounters with `forced` / resolved encounters; rate | no encounters → `NA` | diagnostic; family/target | more disable can reduce fun | `CE.forced`; #148 |
| `displacement_px` | authoritative displacement sum and distribution per resolved encounter; px | no displacement → 0; no encounters → `NA` | diagnostic; family/context | large displacement can be harmful or meaningless | `CE.displacement_px`; #148 |
| `projectile_result` | spawned, contact, expire counts / ranged commits; rates and travel ticks | no ranged commits → `NA` | diagnostic; role/lane | projectile volume is not lane value | `CE.projectile_*`, contact; #148 |
| `recovery_punished_rate` | encounters where opponent gains a registered soccer consequence while attacker remains in recovery / missed or defended encounters | no missed/defended encounters → `NA` | higher supports commitment cost; family/response | proximity alone can falsely claim punishment | `CS.phase`, linked MS consequence; #148 |
| `spill_to_opponent_settle` | opposing settled possessions attributable to a spill / spills; rate | no spills → `NA` | bounded higher; family/phase; 1.5/3/5 s | rewarding any turnover encourages combat replacing passing | `CE.ball_spill`, `MS.owner`; #148 |
| `combat_to_soccer_conversion` | encounters yielding one attributable settled possession, progressive pass, retained-space event, chance, shot, prevented shot, or goal / resolved encounters | no encounters → `NA`; consequence types also reported separately | higher only with safety/soccer gates; 1.5/3/5 s | one loose composite hides low-quality or spam-created outcomes | CE→MS linkage missing; #148 |
| `time_to_soccer_consequence_s` | ticks from terminal combat result to first attributed soccer consequence / 60; distribution per converted encounter | no consequence → right-censored at window and reported | lower can indicate immediacy, not quality; consequence type | stopping only successful cases hides failures | #148 causal rows |
| `commitment_cost_conversion` | ignored/defended/missed encounters with opponent opportunity, lost space, or possession risk / ignored/defended/missed encounters | zero denominator → `NA` | higher supports counterplay; family/response | broad “lost space” can manufacture a cost | #148, definitions below |

For this contract:

- **settled possession** uses the existing 0.7 s `metrics.SETTLE_HOLD`;
- **progressive pass** advances the ball at least 48 px toward the opposing goal
  and is retained by the passer's team for 0.7 s;
- **retained space** means the acting team keeps possession and advances the
  ball or carrier at least 36 px into space vacated by the affected opponent,
  with no independent intervening action;
- **chance** is a possession inside 180 px of the target goal with a
  line-of-shot opening under the frozen geometric rule;
- **shot** and **goal** use existing match events/score;
- **prevented shot** requires a pre-action registered chance and the affected
  opponent being the likely shooter under the frozen observable rule; and
- **lost space/opponent opportunity** is the symmetric version for a defended
  or missed attacker.

These distances are project decisions, not validated human thresholds. #148
must pin them in the schema and #149 must report ±25% sensitivity without
choosing the most favorable definition.

### 4.4 P0 soccer integrity

The existing soccer tripwire and `data/fun_baseline.lua` remain combat-disabled
and are never refreshed from a combat fixture. Combat evidence is a separate
`combat_active_signature/v1` report. `MatchMetrics.fun` retains its historical
name in the game code but is labeled **soccer-shape proxy** in every research
export.

The latest documented 100-match control audit (2026-07-22) already misses
provisional bands for goals (`1.750 < 2`), shots per goal
(`26.484`, beyond the provisional zero edge of `25`), and save rate
(`0.860 > 0.75`). Pass completion (`0.564`), turnovers/minute (`3.330`), and
possession balance (`0.416`) are in band. A fresh same-build fixture A remains
mandatory; historical values do not substitute for its paired control.

| Metric family and claim | Numerator / denominator; unit and grain | Missing behavior and direction | Locked A-vs-B margin and catastrophe floor | Confounds / source |
| --- | --- | --- | --- | --- |
| goals (one-sided NI) / complete matches (one-sided NI) | goals; completed matches / started matches; count and share per match | always defined; goals have bounded higher preference | 95% lower bound: goals B-A `>-0.10`; completion B-A `>-0.02`; B mean goals `>0.50`, completion `>=0.95` | early 3-goal finish; `MS.score/finished`, #148 |
| shots (two-sided equivalence) / shots per goal (one-sided NI) | outfield goalward strikes; shots / goals | zero goals → shots-per-goal `NA` plus zero-goal count; excessive ratio is harm | 90% interval for shots ratio within `[0.90,1.10]`; 95% upper bound shots/goal B-A `<+1.25`; B absolute `<35` | spam and low-quality shots; existing `MatchEvent`, #148 |
| save rate (one-sided NI) | saves / on-target shots | no on-target shots → `NA`; excessive rate is harm | 95% upper bound B-A `<+0.04`; B absolute `<0.95` | shot quality and keeper state; existing metrics |
| passing (volume equivalence; completion NI) | passes and completed / attempted | no passes → completion `NA` and hard failure for match | 90% interval volume ratio within `[0.90,1.10]`; 95% lower bound completion B-A `>-0.03`; B absolute `>0.25` | short safe passes game completion; existing metrics |
| settled turnovers (two-sided equivalence) | settled team changes / active minute | always count-defined | 90% interval B/A within `[0.80,1.20]`; B absolute `[0.3,10]` per minute | raw ownership flicker; existing metrics |
| possession / loose ball (two-sided equivalence) | owned ticks and loose ticks / ball-in-play ticks | no ball-in-play → invalid match | 90% interval paired home-share change within `[-0.04,+0.04]`; absolute home share `[0.1,0.9]`; loose-share change within `[-0.08,+0.08]` | side/skill asymmetry; MS, loose share added #148 |
| drought (upper NI) / decided late (lower NI) | longest shot-or-goal drought; deciding-goal tick / match ticks | goalless decided-late remains 1 and is reported with zero-goal share | 95% upper bound drought B-A `<+3 s`, B absolute `<80 s`; 95% lower bound decided-late B-A `>-0.05`, B absolute `>0.05` | goalless matches inflate decided-late; existing metrics |
| dribble carry (equivalence) / heavy losses (upper NI) | carry ticks; close/sprint/juke shares; losses / carry minute | no carry → `NA` for shares | 90% interval carry-time ratio `[0.85,1.15]`; 95% upper bound heavy-loss rate B-A `<+0.50/min` | role/policy mix; existing metrics |
| ball in play (one-sided NI) | active non-stoppage ticks / match ticks | no match ticks → invalid | 95% lower bound B-A `>-0.05`; B absolute `>=0.75` | early finish and stoppage definitions; #148 |
| soccer cadence (equivalence) / equipment cadence (diagnostic) | accepted soccer or equipment decisions / active minute | no active minute → `NA`; report separately | 90% interval soccer B/A `[0.90,1.10]`; no minimum combat cadence | equating buttons with decisions; #148 |

Margins above are product tolerances (`D`), not literature-derived universal
values. The shots-per-goal absolute floor is deliberately looser than the
legacy zero edge because the current control already crosses that edge; its
paired NI margin still prevents combat from exploiting the known failure.
The instrumentation pilot may demonstrate that a margin is below measurement
resolution; any change must occur before freeze, cite that blinded evidence,
and receive a new review disposition.

One-sided NI uses alpha `0.05` and the harm-side 95% bound. Two-sided
equivalence uses TOST alpha `0.05` and a 90% interval wholly inside both
margins. “Not significant” is not evidence of non-inferiority or equivalence
([E][lakens-equivalence]). Multiplicity is governed by section 6.3.

### 4.5 P0 counterplay and readability

A threat starts at the first perceivable authoritative cue and ends at impact
or expiry. A response is **legal** under the simulation and **viable** when a
frozen scenario test demonstrates that correctly timed human-available input
can avoid, defend, or reverse the outcome. Nominal legality without enough
effective time is not viability.

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `counter_coverage` | threats with >=1 legal viable response / incoming threats; rate | no threats → `NA` | hard target 1 in normal contexts; family/response/device/latency | declaring an impractical response viable | cue + sim response matrix; #147/#148 |
| `counter_attempt_rate` | valid response attempts / threats with viable response; rate | no viable threats → `NA` | diagnostic; family/skill/accessibility | players may not perceive the cue | input linkage; #148 |
| `counter_success` | avoid/defend/reverse outcomes / valid attempts; rate | no attempts → `NA` | no universal threshold; family-response matrix | tuning attacks weak to inflate success | CE + input linkage; #148 |
| `effective_response_window_ms` | impact boundary minus cue-visible boundary minus measured input/system latency; ms per threat | missing cue/latency → invalid row, not zero | positive tail floor; family/device/runtime | average hides impossible p1 windows | presentation/runtime + canonical tick; #148 |
| `attacker_punishment` | defended/missed actions followed by recovery exposure, lost space, possession risk, or opponent opportunity / defended/missed actions | no defended/missed action → `NA` | higher supports tradeoff; family/response; 3 s | vague opportunity labels manufacture punishment | CE→MS; #148 |
| `unseen_impact_rate` | impacts the participant could not identify before contact / impacts probed; rate | unprobed impact kept as missing | lower; family/presentation/accessibility | asking leading questions improves apparent recognition | replay probe; #151 |
| `occluded_or_masked_cue_rate` | threats whose cue is geometrically occluded, audio-masked, or HUD/ball-masked beyond threshold / threats | missing render evidence → `NA` | lower; viewport/frame rate/accessibility | telemetry can disagree with human perception | presentation evidence; #147/#148 |
| `concurrent_cue_count` / `cue_overlap_share` | active cues per tick; ticks with >=2 cues / threat ticks | no threat ticks → `NA` | lower tails, diagnostic | suppressing needed cues improves metric but harms comprehension | #147/#148 |
| `false_defensive_reaction_rate` | defensive responses to no authoritative threat / defensive response opportunities | no responses → `NA` | lower; presentation/device | cautious play or ordinary juke can be mislabeled | input + cue linkage; #148 |
| `ball_hud_occlusion_share` | threat ticks where cue geometry masks ball or required HUD target / threat ticks | no threat ticks → `NA` | hard review trigger above 0; viewport | pixel overlap is only a proxy for readability | presentation capture; #147/#148 |
| `causal_identification_accuracy` | correctly identified source, family, direction, target, and available response components / presented components; rate per participant-condition | “unsure” is incorrect but retained; no probe → missing | primary comprehension floor: lower 95% interval `>=0.70` overall and no priority stratum point estimate `<0.55` | guessing, leading labels, memory delay | neutral replay probe; #151 |
| `causal_identification_time_s` | probe onset to final answer; seconds per probe | timeout is right-censored at declared limit | lower diagnostic; same strata | faster guesses are not better comprehension | #151 |

Freeze-probe tasks are separate from ordinary matches. Concurrent think-aloud
is prohibited during high-action play because observer presence/type can alter
performance, motivation, anxiety, and reported experience in some contexts
([E][observer-effects]). Replay-cued commentary is subjective comprehension
evidence anchored to telemetry, not authoritative proof of causality
([E: small qualitative method][gow]).

### 4.6 P0 off-ball purpose and horizontal balance

#112 must emit exactly one stable reason at decision time for bot actions,
chosen by this precedence:

1. `carrier_contest`;
2. `carrier_protection`;
3. `loose_ball_contest`;
4. `passing_lane_or_shot_denial`;
5. `recovery_punish`;
6. `formation_risk_tradeoff`; or
7. `unattributed_off_ball`.

The reason uses only player-observable state. #148 independently records the
authoritative geometric/possession context for every bot and human action.
Human stated intent is a separate replay-debrief field, never substituted for
authoritative context. An AI reason is an intent claim, not proof of value.

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `off_ball_context_share` | actions in each authoritative taxonomy context / accepted actions; multinomial share | no actions → `NA`; ambiguous context becomes unattributed | diagnostic; family/role/phase | post-hoc priority can favor desired bucket | MS/CE; #148 |
| `ai_reason_reconciliation` | bot reasons compatible with recorded authoritative context / bot actions | no bot actions → `NA`; missing reason is schema failure | hard target 1.0 | plausible labels can hide privileged state | `R112` plus MS/CE; #112/#148 |
| `unattributed_off_ball_share` | unattributed off-ball actions / off-ball actions; rate | no off-ball actions → `NA` | review trigger, not automatic harassment; player/policy | forcing every action into a named bucket hides abuse | MS/CE and `R112`; #148 |
| `reason_value_conversion` | actions producing the context-specific soccer consequence / actions in that context | zero bucket → `NA`; 1.5/3/5 s | higher only with guardrails | circularly defining outcome from reason | MS/CE plus R112 diagnostics; #148 |
| `family_context_utility` | attributable reason-value conversions / actions in a priority context; share and raw paired difference versus matched unarmed/control | absent cell → missing, never zero | context SESOI is `+0.10` conversion share; family×role×phase×skill | usage and win rate confounded by selection | C rotation; #149 |
| `strict_upgrade_cells` | priority contexts where one family exceeds every alternative by `+0.10` conversion share without a soccer-integrity loss / evaluable contexts | insufficient precision → inconclusive | hard target: no family wins every priority context | choosing contexts after results | C/D matrices; #149 |
| `unarmed_viability` | paired soccer-integrity outcomes and enjoyment for unarmed versus family alternatives | missing matched cell → inconclusive | NI: every section 4.4 raw soccer margin passes and human enjoyment lower bound `>-0.50` PXI points | occasional use is not viability | C/D + human follow-up; #149/#151 |

Priority contexts are fixed as carrier contest, carrier protection, loose ball,
lane/shot denial, recovery punish, and formation-risk tradeoff, crossed with
four outfield roles and both sides. Results report authored player/stat,
formation, score state, skill, and policy. A family is a strict upgrade only
when its uncertainty interval clears the context SESOI in every priority
context; absence of evidence is `inconclusive`, not balance.

### 4.7 Human-proxy and adversarial policy roles

The human-proxy population is interpretable, seeded, and limited to
player-observable cues. It uses the same input contract, action eligibility,
reaction cadence, and uncertainty available to a human, and emits the stable
#112 reason plus an observable-input digest. It cannot read a future tick,
resolver result, hidden target, opponent input, outcome label, or presentation-
only identity.

The adversarial population searches chain extension, permanent guard, ranged
lane denial, safe zones, cooldown loops, repeated-family dominance, and
off-ball harassment. Privileged state is allowed only in fixtures explicitly
tagged `adversarial_privileged`; those results may find counterexamples but
never support representative-player, comprehension, or fun claims. Reports
never pool the two populations. A reproducible invariant-breaking or dominant
loop blocks the prototype even when its estimated population frequency is
zero.

## 5. Human player-experience protocol

### 5.1 Instrument stack and reuse constraints

| Purpose | Instrument | Locked use |
| --- | --- | --- |
| Primary repeated endpoint | Three-item PXI enjoyment add-on | after A and B; independently supported as a separate factor, not one of the original ten PXI constructs |
| Mechanism diagnostics | Unchanged PXI autonomy, mastery, challenge, ease-of-control, goals/rules, and progress-feedback subscales | partial-PXI diagnostics only; no claim that selected subscales are a full validated PXI administration or benchmark |
| Deep-dive mechanisms | 18-item BANGS particular-session variant | six separate three-item satisfaction/frustration subscales; never collapse frustration into satisfaction |
| Momentary diagnostic | Affective Slider valence and arousal | constrained dual visual instrument at natural breaks, not two Likert questions |
| Custom exploration | soccer primacy, fairness, suspense, counterplay readability, overload, frustration, and desire to explore | item-by-item only; never called a validated scale |
| Explanation | replay-cued retrospective interview | neutral prompts around commits, contacts, guards, misses, spills, turnovers, and unexplained failures |

The original PXI validation covers ten constructs/30 items. The enjoyment
add-on is not an original PXI construct; the independent validation supports
the three enjoyment items as a separate factor ([E][pxi-independent]).
The maintainers describe PXI as open-access/free to use, but require unchanged
items, the seven labeled responses from `-3` to `+3`, and published scoring;
rewording, relabeling, or removing constructs loses the full-instrument
validation and benchmark claim ([reuse guidance][pxi-guide]).

For this study, exact source/version, item text, labels, item randomization,
post-condition timing, scoring, and missingness rules are frozen before
collection. The enjoyment score is the arithmetic mean only when all three
items are answered. Each multi-item diagnostic reports omega reliability by
condition; no dropped-item score is computed. Selected PXI mechanism
subscales retain exact wording and scoring but are explicitly **partial-PXI
diagnostics**.

BANGS uses the particular-session wording, all 18 items, randomized order, and
the validated labeled `1..7` response scale. Its six three-item subscales are
scored separately. The article is CC BY 4.0; the accompanying guide/materials
are CC BY-SA 4.0, so #151 records the exact source/version, attribution, and
share-alike obligations ([E and guide][bangs], [BANGS guide][bangs-guide]).

Affective Slider presents simultaneous horizontal grayscale valence/arousal
sliders, centered initially at `0.5`, on continuous `0..1` scales with step
`<=0.01`, the prescribed intensity/emoticon cues, randomized vertical order,
and interaction recorded. Its assets/code are CC BY-SA 4.0 and require
attribution/share-alike ([E][affective], [assets/reuse][affective-assets]).

No instrument text or asset is copied into this repository until its exact
reuse terms and version are recorded. Translations require an existing
validated version or a separately labeled translation/adaptation process. A
custom translation cannot inherit the original validation claim.

miniPXI is not a shortcut for these repeated mechanism comparisons; its
validation/reliability limitations are recorded and multi-item PXI is
preferred where burden permits ([E][minipxi], [test-retest caution][minipxi-tt]).
The original GEQ is not the default because a published review found no
support for its proposed seven-factor structure and identified reporting/
citation problems; this is not a blanket claim that every GEQ-derived item is
invalid ([E][geq]).

### 5.2 Participant coverage and procedure

Formative sessions seek construct/interaction defects and negative cases; they
are never pooled into confirmatory evidence. Confirmatory recruitment
purposively covers football-first, action/fighting-first, mixed/casual,
novice-through-expert experience, keyboard and gamepad, and relevant
accessibility/readability needs. Experience is recorded continuously/ordinally,
not forced into brittle player types.

The core procedure is:

1. accessible consent and separate collection scopes;
2. device, controls, display/audio, and accessibility setup;
3. standardized neutral instructions and an unanalyzed practice block;
4. randomized, counterbalanced A/B blocks with balanced side, seed, role, and
   order;
5. natural break, fatigue check, and post-condition instruments;
6. optional, unrewarded rematch/loadout choice marked secondary;
7. replay-cued debrief with a neutral facilitator where possible; and
8. withdrawal reminder and data-scope confirmation.

Winner and loser evidence is separate. The analysis records novelty, learning,
fatigue, period/order, win/loss, opponent policy, device, runtime performance,
and accessibility settings. No high-action concurrent think-aloud is used.
Observer mode and relationship to the project are recorded because no protocol
can eliminate observer effects simply by choosing one facilitator
([E: Dominic Kao][observer-effects]).

### 5.3 Accessibility

The protocol supports keyboard-only and gamepad navigation, scalable text,
high contrast, no color-only meaning, remappable controls where available,
captions/text equivalents for audio cues, safe pause/resume, breaks, and extra
practice without silently excluding the participant. Relevant settings and
functional readability needs are voluntarily self-described and minimized;
diagnosis or protected-trait disclosure is not required.

Accessibility configurations are analysis strata, not reasons to discard a
session. A normal supported configuration with no viable cue/counter is a hard
failure. Where runtime limits prevent a requested accommodation, the limitation
and affected population are recorded; the project does not generalize beyond
tested coverage.

### 5.4 Privacy, consent, and custody for Milestone 11

Milestone 11 collects only what the declared questions require:

- pseudonymous participant/session ids;
- gameplay tape/events and device/performance diagnostics;
- instrument responses and structured missingness;
- optional replay annotations/free text; and
- audio/video only under a separate opt-in scope, disabled by default.

Names, emails, account handles, IPs, hostnames, raw paths, direct identifiers,
consent prose, and re-identification keys never enter gameplay artifacts,
filenames, Git, issues, CI artifacts, or public reports. The recontact/
withdrawal map is stored separately on an encrypted local volume with access
limited to the study owner. Gameplay, survey, free text, and media are separate
consent scopes.

The Milestone 11 manual custody decision (`D`) is: raw optional media is deleted
within 30 days of validated transcription/debrief or 90 days after collection,
whichever comes first; pseudonymous trace/responses are retained for at most
one year; the withdrawal map is deleted with the last retained session.
Withdrawal requests are actioned within 14 days across raw and derived local
artifacts. A de-identified aggregate already published may be non-retractable,
which is disclosed before consent.

Unexpected identifiers quarantine the session. Collection beyond consent,
lost custody, or inability to propagate a requested deletion blocks use and is
recorded as an incident. #133 owns a future engineered consent/storage/
deletion system; this contract does not pretend the manual controls implement
that milestone.

## 6. Statistical and decision contract

### 6.1 Independent unit, endpoint, and estimand

The independent unit is the participant. Contacts, ticks, and matches nested
within a participant are not independent observations.

The primary endpoint is the mean of the three unchanged PXI enjoyment add-on
items after each core condition. The primary estimand is the adjusted
within-participant mean difference `B - A` for the intended participant
coverage, regardless of match result. A retained primary record requires all
three enjoyment items in both conditions; partial items remain visible and are
not imputed as neutral.

The smallest effect worth acting on (`SESOI`, `D`) is **+0.50 PXI points** on
the -3..+3 response scale. The unacceptable enjoyment degradation is
**-0.33 points**. Mechanism harm margins are **-0.50 points** for each partial
PXI subscale and BANGS satisfaction, and **+0.50 points** for BANGS
frustration. These are product decisions representing a persistent
half-category movement, not generic standardized-effect labels. The blinded
burden/variance pilot may amend them before freeze only with anchor evidence
and review.

### 6.2 Sample size and power

The planning target is **48 complete paired participants**, recruiting up to
54 to allow approximately 11% incomplete sessions. Before confirmatory
collection, #151 runs simulation-based power for the exact mixed model using
blinded pilot variance and within-player correlation. The frozen design must
provide at least 90% power for a +0.50 primary effect with two-sided alpha
0.05 and at least 80% power for the -0.33 harm boundary. If 48 is
insufficient, recruitment increases; reducing the target or accepting lower
precision requires a versioned amendment and makes population-level
`proceed` unavailable without new review.

Formative stopping uses theme saturation plus deliberate negative-case search,
not a statistical claim. It cannot satisfy the confirmatory count.

### 6.3 Model, missingness, and multiplicity

The primary linear mixed model is:

```text
enjoyment ~ condition + period + order + side + role + win_loss
          + prior_experience_z + device + accessibility_setup
          + condition:prior_experience_z
          + (1 + condition | participant)
          + (1 | seed_block)
          + (1 | opponent_policy)
```

If the random-effect correlation is singular, use an uncorrelated participant
intercept/slope. If the slope variance remains singular, retain participant
intercept only and report both fits. The fallback is fixed before outcome
unblinding. Later human-vs-human work adds dyad/party as an independent random
effect and does not reuse this model unchanged.

No mean/zero imputation is allowed. Primary complete-pair analysis is
accompanied by missingness reasons and condition/order comparison. When primary
missingness exceeds 5%, report a prespecified multiple-imputation or inverse-
probability sensitivity analysis if its assumptions are defensible; otherwise
the result is `inconclusive`. Attrition, protocol deviations, outliers, and all
planned subgroup estimates remain visible. Robust residual and ordinal
sensitivity models accompany, but do not replace, the primary model.

There is one primary endpoint and no multiplicity adjustment for it. The six
partial-PXI mechanism contrasts form one Holm-adjusted family. Safety/
readability human guardrails form a second Holm-adjusted family. BANGS
satisfaction/frustration subscales form a third. Soccer NI/equivalence
contrasts use Holm-adjusted p-values and simultaneous interval reporting within
the declared soccer family. All custom items, subgroups, entropy, and pacing
analyses are exploratory with intervals and no confirmatory language.

### 6.4 Interval-based decision quadrants

All primary value and harm claims use two-sided 95% intervals unless a
one-sided safety/NI rule above is stricter.

| Primary interval and other gates | Decision |
| --- | --- |
| lower bound `>+0.50`, harm lower bound `>-0.33`, all hard/soccer/human gates pass | `proceed` |
| interval overlaps `+0.50` while not wholly below it, or a bounded mechanism/readability/soccer harm is present without hard failure | `revise` once under a versioned bounded retest |
| after the one allowed revision, upper bound `<+0.50`; or harm upper bound `<=-0.33`; or an invariant/product rule would need weakening | `stop` |
| interval spans both material benefit and material harm, power/coverage/missingness is inadequate, A/A or reconciliation fails, or gates contradict without a locked disposition | `inconclusive` |

`Proceed` also requires attributable soccer value and no priority subgroup with
unresolved practical harm. A hard failure overrides a favorable enjoyment
mean. A telemetry alert without validated human harm/value triggers
investigation, not a silent threshold change. The only allowed `revise` cycle
freezes a new contract and untouched holdout; pilot and prior confirmatory data
remain separate from the new confirmatory estimate.

## 7. Blinded A/A instrumentation and reconciliation gate

#148 runs machine A/A before A/B; #151 later runs protocol A/A before
comparative participant collection. In both cases identical builds, rules,
fixtures, inputs/assignments, and seeds receive two opaque labels generated
after artifact creation.

Machine A/A uses seeds `18001..18030`, mirrors label order, and must produce:

- byte-identical canonical event/funnel rows after removing only the opaque
  label;
- identical source-sequence counts, snapshot boundaries, final hashes, and
  missing-denominator reasons;
- zero corrected-away events;
- no duplicate/orphan lifecycle; and
- every deterministic condition contrast exactly zero, with intervals centered
  on zero for resampled reporting.

Protocol A/A counterbalances identical condition blocks under opaque labels.
Its 90% interval must lie wholly inside `[-0.50,+0.50]` under TOST alpha 0.05;
assignment, timing, item order/scoring, joins, missingness, and observer
behavior are inspected before A/B. Passing A/A does not prove the instrument
valid for the product; it only fails to manufacture a practical label effect.

Failure dispositions are locked:

| Failure | Disposition |
| --- | --- |
| hash/event/source mismatch | block; fix deterministic export/reconciliation and rerun from fresh artifacts |
| non-zero deterministic metric difference | block; locate label leakage or asymmetric pipeline path |
| orphan/duplicate/corrected-away event | block; fix event lifecycle or rollback confirmation |
| missingness/assignment differs by opaque label | block protocol; inspect randomization, UI, timing, joins |
| A/A interval not equivalent within ±0.50 | block A/B; investigate scoring, period, observer, and carryover |
| only formatting/order differs | fix canonical serialization; do not normalize it away after comparison |

No comparative calibration or holdout inspection begins until all blocking A/A
findings are resolved and the disposition artifact is reviewed.

## 8. Evidence artifacts and reproducibility

`combat_active_signature/v1` contains:

- per-event/funnel rows keyed by source sequence and canonical tick;
- per-player, per-match, fixture, seed, policy, family/role, and aggregate rows;
- all raw numerators, denominators, `NA` reasons, exclusions, and sample counts;
- schema/ruleset/contract, commit/build, content, tuning, config, fixture, seed,
  assignment and bot-policy identities/hashes;
- exact command, runtime/platform, canonical artifact hash, and replay result;
- separate calibration, adversarial, sensitivity, A/A, and untouched-holdout
  manifests/reports;
- replay/clip references for hard failures and negative counterexamples; and
- amendments, reviewer dispositions, contradictions, and final #114 decision.

Survey instrument/version/timing and pseudonymous research-session identity are
stored outside gameplay identity and joined by an allowed session-to-tape
reference. Raw independent-unit counts are always published beside derived
rows. Rebuilding from the same commit/config/seeds must be byte-identical for
machine evidence.

## 9. Multidisciplinary design-review log

Internal review is a **multidisciplinary design review**, not academic peer
review. Each perspective reviews construct definitions, instrument/reuse
terms, event schema, fixture matrix, analysis/sample plan, accessibility,
ethical boundaries, and decision rules. Allowed final statuses are `approve`,
`approve with changes`, or `block`. A material objection requires an explicit
disposition and evidence link.

Initial entries are intentionally pending; this document does not fabricate
review or consensus.

| Perspective | Reviewer | Status | Objections / required changes | Disposition / evidence | Contract version |
| --- | --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | pending assignment | pending | pending review | pending | v1 |
| Competitive combat/fighting counterplay | pending assignment | pending | pending review | pending | v1 |
| Games user research and psychometrics | pending assignment | pending | pending review | pending | v1 |
| Experimental statistics and reproducibility | pending assignment | pending | pending review | pending | v1 |
| Telemetry/data engineering and deterministic replay | pending assignment | pending | pending review | pending | v1 |
| AI human-proxy and adversarial behavior | pending assignment | pending | pending review | pending | v1 |
| Accessibility, readability, and inclusive participant design | pending assignment | pending | pending review | pending | v1 |
| Netcode, performance, and latency | pending assignment | pending | pending review | pending | v1 |
| Privacy, responsible engagement, and player advocacy | pending assignment | pending | pending review | pending | v1 |

Each reviewer record uses this category template:

| Category | Status | Objection | Required change | Disposition/evidence |
| --- | --- | --- | --- | --- |
| Construct definitions | pending | pending | pending | pending |
| Instrument and reuse terms | pending | pending | pending | pending |
| Event/telemetry schema | pending | pending | pending | pending |
| Fixture/seed/holdout matrix | pending | pending | pending | pending |
| Sample and analysis plan | pending | pending | pending | pending |
| Accessibility coverage | pending | pending | pending | pending |
| Ethical/privacy boundaries | pending | pending | pending | pending |
| Decision/amendment rules | pending | pending | pending | pending |

The review artifact also records reviewer identity/role, date, reviewed commit,
response owner, change commit, unresolved dissent, and final disposition.
#149 cannot begin comparative calibration while a row is pending or blocked.

## 10. Explicit boundaries and downstream imports

This issue makes no combat, AI, presentation, telemetry, tuning, collection,
storage-system, or model change. It adds no economy, progression, family,
keeper combat, learned model, retention optimization, or art. The showcase
release scope remains unchanged.

- #112 imports the observable-information rule and off-ball reason taxonomy.
- #148 imports the metric dictionary, event/lifecycle gaps, identity, A/A,
  causal attribution, and reconciliation rules.
- #149 imports fixture rotations, seed manifests, margins, sensitivity, and
  exploit dispositions.
- #150 imports crowded-fixture runtime strata but owns performance evidence.
- #151 imports instruments, participant coverage, accessibility/privacy,
  counterbalancing, power, analysis, and debrief rules.
- #114 imports the interval-based decision and amendment rules.
- Milestone 12 imports requirements from this contract but remains responsible
  for laboratory governance (#131), canonical research schemas (#132),
  engineered consent/storage/deletion (#133), and protocol automation (#135).

## 11. Source register

The research sources below support constructs or methods; numeric GOLISEO
thresholds remain project decisions.

- [PXI development and validation][pxi] (`E`)
- [Independent PXI validation][pxi-independent] (`E`)
- [miniPXI validation and reliability caveats][minipxi] (`E`)
- [BANGS satisfaction/frustration validation][bangs] (`E`)
- [Affective Slider validation][affective] (`E`)
- [Need satisfaction and game motivation][ryan] (`E`)
- [Close outcomes, suspense, and enjoyment][suspense] and
  [FPS suspense evidence][klimmt] (`E`; neither makes closeness a fun proxy)
- [Player- and data-driven balance evidence][pfau] (`E`)
- [Telemetry as a GUR supplement][drachen] (`E`)
- [Replay-cued post-game commentary][gow] (`E`)
- [Observer effects (Dominic Kao)][observer-effects] (`E`)
- [GEQ validation caution][geq] (`E`)
- [Sample-size and SESOI justification][lakens-sample] (`E`)
- [Equivalence testing][lakens-equivalence] (`E`)
- [Playtime and wellbeing/experience limits][vuorre] (`E`)

[pxi]: https://doi.org/10.1016/j.ijhcs.2019.102370
[pxi-independent]: https://doi.org/10.1145/3613904.3642270
[minipxi]: https://doi.org/10.1145/3549507
[minipxi-tt]: https://arxiv.org/abs/2407.19516
[bangs]: https://doi.org/10.1016/j.ijhcs.2024.103289
[bangs-guide]: https://nickballou.com/docs/bangs/userguide/
[affective]: https://doi.org/10.1371/journal.pone.0148037
[affective-assets]: https://github.com/albertobeta/AffectiveSlider
[ryan]: https://doi.org/10.1007/s11031-006-9051-8
[suspense]: https://doi.org/10.1007/s11031-014-9425-2
[klimmt]: https://doi.org/10.1089/cpb.2008.0060
[pfau]: https://doi.org/10.1145/3675807
[drachen]: https://doi.org/10.1007/978-1-4471-4769-5_14
[gow]: https://research.monash.edu/en/publications/capturing-player-experience-with-post-game-commentaries
[observer-effects]: https://doi.org/10.1080/0144929X.2021.1906321
[geq]: https://doi.org/10.1145/3242671.3242683
[lakens-sample]: https://doi.org/10.1525/collabra.33267
[lakens-equivalence]: https://doi.org/10.1177/1948550617697177
[vuorre]: https://doi.org/10.1098/rsos.220411
[pxi-guide]: https://playerexperienceinventory.org/docs
[i131]: https://github.com/osobytes/galactic-cup/issues/131
[i135]: https://github.com/osobytes/galactic-cup/issues/135
