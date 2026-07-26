# Design: Combat-soccer fun evidence contract

- **Status:** preregistration contract, pending multidisciplinary design review
- **Contract version:** `combat_fun_evidence_contract/v1`
- **Decision owner:** issue
  [#114](https://github.com/osobytes/goliseo/issues/114)
- **Instrumentation owner:** issue
  [#148](https://github.com/osobytes/goliseo/issues/148)
- **Calibration owner:** issue
  [#149](https://github.com/osobytes/goliseo/issues/149)
- **Human-study owner:** issue
  [#151](https://github.com/osobytes/goliseo/issues/151)
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

Combat snapshot version 1 and match snapshot version 11 serialize combat state
and events. Input tape version 2, replay, and rollback-confirmed event handling
provide deterministic identity and correction boundaries. Confirmed evidence
exports contain only events that survive canonical rollback confirmation.
Speculative presentation events and their correction/revocation records are
valid rollback diagnostics; they are retained in a separate diagnostic stream
and never mistaken for confirmed outcomes or lifecycle defects.

### 2.2 Known gaps and assigned owners

| Gap | Why the existing shape is insufficient | Owner |
| --- | --- | --- |
| Accepted/rejected equipment intent and typed reason | `prepare_inputs` consumes or ignores an edge without emitting an outcome | #148 |
| One terminal lifecycle outcome per accepted intent | A melee miss and several cancellation paths have no terminal event | #148 |
| Core authoritative telegraph start/end and pose projection | Landed #113 exposes the core presentation seam but does not export research rows | #113 seam; #148 deterministic join/export |
| Specialized cue/VFX state, occlusion, overlap, and response opportunity | Core telegraphs do not cover all specialized feedback | #147 hooks; #148 deterministic join/export |
| Runtime/render/device/network latency and crowded cue evidence | Wall-clock observations are not authoritative simulation fields | #150 |
| Stable AI decision reason and observable-information declaration | Combat AI is not implemented; generic brain helpers do not classify combat purpose | #112 |
| Off-ball purpose | Requires authoritative state; bot intent additionally needs stable #112 reasons | #112 emits bot reason; #148 records/reconciles deterministic context |
| Settled-possession and soccer-event source linkage | `MatchEvent` has no combat source sequence | #148 |
| Miss/expiry for every family, defended commitment costs, and rejected request outcomes | Only ranged expiry and contacts are explicit | #148 |
| Pass progression, retained space, lane denial, chance, and prevented-shot definitions | Not present as canonical event fields | #148 |
| Player-visible feedback for a rejected intent | Presentation behavior, not an event today | #113 core seam and #147 specialized feedback; #148 links |
| Versioned combat-active report and raw/config hashes | No combat metrics collector exists | #148 |
| Family calibration, exploit search, sensitivity, and untouched holdout execution | Evidence operation, not schema work | #149 |
| Human observation, device/accessibility strata, instruments, debrief, and participant evidence | Not simulation data | #151 |
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
Earlier non-participant machine evidence, preregistrations, manifests, decision
records, and amendment history remain immutable and labeled. Participant
payloads are not immutable: withdrawal tombstones the join, deletes the raw
payload, regenerates derived artifacts without it, and leaves only a
payload-free audit event.

### 3.2 Fixed experiment matrix

| ID | Fixture | Required comparison |
| --- | --- | --- |
| A | Same build, combat disabled, neutral loadouts | causal control and immutable soccer baseline |
| B | Default mixed family: keeper plus unarmed, guard, light melee, ranged | intended prototype |
| C | Mirrored matched-stat family/formation-slot rotations | family × canonical slot × formation × side without authored-player confounding |
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
holdout run. A and B use common random numbers.

Fixture C uses the canonical slot id
`<formation_id>/outfield/<outfield_index>` plus the exact normalized anchor
from `data/formations.lua`; it never treats non-isomorphic formation slots as
the same role. Its locked logical matrix is:

```text
formation_id ∈ {2-1-1, 1-2-1, 1-1-2}
outfield_index ∈ {1, 2, 3, 4}
family ∈ {unarmed, guard, light_melee, ranged}
side ∈ {home, away}
player_stat_profile ∈ {profile_1, profile_2, profile_3, profile_4}
opponent_formation ∈ {2-1-1, 1-2-1, 1-1-2}
```

Within each paired family contrast, controlled and opponent formation, slot
anchor, player/stat profile, opponent policy, seed, and side are held equal.
Across the complete matrix, a balanced Latin-square schedule rotates every
family and stat profile through all four slots, mirrors both sides, and crosses
all three controlled and opponent formations equally. Reports use canonical
slot id and separately report the formation's authored role label; they do not
collapse “defender”, “midfielder”, or “forward” across unequal anchors without
the formation interaction.

Family presentation swap E reuses the exact content/input/seed stream except
for presentation identity, which must not enter simulation identity.

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
  ticks, and corrected-away events from the confirmed stream;
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

### 4.0 Evidence identity, serialization, and vocabulary

`source_sequence` is unique only inside one combat companion and is never a
global event key. #148 must materialize these compound identities:

```text
run_id = sha256_hex(lp("combat-run/v1"), lp(contract_version),
                    lp(schema_version), lp(build_commit), lp(tape_hash),
                    lp(initial_boundary_hash), lp(fixture_hash),
                    lp(tuning_hash), lp(seed), lp(side_assignment),
                    lp(policy_id))

event_id = run_id / game_instance_id / canonical_tick / event_domain /
           event_kind / source_sequence_or_zero / same_kind_ordinal

funnel_row_id = run_id / game_instance_id / source_sequence /
                row_kind / row_ordinal
```

The slash notation denotes typed tuple fields, not delimiter concatenation.
Canonical textual ids concatenate `lp(field)` for every field in the displayed
order.

`sha256_hex` is SHA-256 rendered as exactly 64 lowercase hexadecimal
characters. `lp(value)` converts a schema-typed value to its canonical UTF-8
form, then emits ASCII decimal byte length, `:`, and those bytes. Integers use
canonical decimal; strings are Unicode NFC normalized and then UTF-8 encoded.
Concatenated length prefixes and the fixed field order make the tuple
unambiguous; delimiter-joined or implementation-default object hashing is
forbidden.

`game_instance_id` distinguishes restarts in one run. `event_domain` is one of
`input`, `soccer`, `combat`, or `lifecycle`. `same_kind_ordinal` is the
one-based position after canonical ordering by domain, kind, source sequence
(`nil` sorts as zero), source index, target index, then the original
authoritative array index. A funnel row stores `commit_event_id`,
`terminal_event_id`, optional `parent_funnel_row_id`, and
`consequence_event_id` foreign keys. Every funnel FK must share its `run_id`
and `game_instance_id`; cross-game references, missing keys, and non-unique
keys fail closed.

Canonical machine exports are UTF-8 JSON Lines with one schema header followed
by rows sorted first by numeric row rank, then lexicographically by the
materialized key below. Integers compare numerically; strings compare unsigned
UTF-8 bytes. No sort-key field is nullable.

| Rank / row type | Canonical key after rank |
| --- | --- |
| 0 schema header | the one literal `combat_active_signature/v1` |
| 10 event | `(run_id, game_instance_id, canonical_tick, domain_rank, event_kind, source_sequence_or_zero, same_kind_ordinal)` |
| 20 funnel | `(run_id, game_instance_id, root_commit_tick, source_sequence, row_kind_rank, row_ordinal)` |
| 30 per-player | `(run_id, game_instance_id, stable_player_index)` |
| 40 per-match | `(run_id, game_instance_id)` |
| 50 fixture | `(fixture_hash, seed, side_assignment, policy_id, run_id)` |
| 60 aggregate | `(metric_id, scope_rank, canonical_strata_json, window_ticks_or_zero)` |

`domain_rank`, `row_kind_rank`, and `scope_rank` are closed schema enums.
Optional sort integers materialize as `0`; optional sort strings materialize
as `""`; a real field that could equal that sentinel carries an explicit
presence bit in `canonical_strata_json`. Duplicate total keys fail closed.
Object keys use schema order; arrays preserve declared order; integers are
decimal without leading zero; finite non-integers use the shortest
round-trippable decimal, normalize negative zero to `0`, and reject
NaN/infinity. Newlines are LF and the final line ends in LF. Byte identity is
claimed only under the complete `run_id` tuple and this total ordering.

Wall-clock, frame time, device polling, network arrival, render presentation,
and platform/browser observations are **observational runtime evidence**. They
retain raw rows plus content/config hashes and must be protocol-repeatable;
they are not expected to be byte-identical across runs.

Vocabulary is closed:

- a physical equipment request is either `accepted` or `rejected`;
- only `accepted` creates an encounter/source sequence;
- an accepted encounter terminates exactly once as `miss`, `expire`,
  `guarded`, `immune`, `superseded`, `hit`, `interrupted`, `cancelled`, or
  `match_terminated`;
- `cancelled` means a previously accepted encounter ended under an allowed
  lifecycle rule; `superseded` means a legal contact lost same-target
  dominance; and
- `ignored` is presentation copy for a typed `rejected` request only. It is
  not an encounter terminal value and never enters a commitment-cost
  denominator.

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
| `intent_acceptance` | accepted legal equipment presses / legal requested presses; rate per player-match | physically malformed frames are retained as protocol-failure rows but are not legal-request denominator events; no legal requests → `NA` | higher; family/state/device; same tick | redefining “legal” after rejection inflates the rate | missing accepted/rejected event; #148 |
| `unexpected_rejection_rate` | legal requests rejected without a typed allowed reason / legal requests; rate | no legal requests → `NA` | hard target 0; family/state | broad catch-all reason hides defects | #148 typed outcome/reason |
| `unreadable_rejection_rate` | rejected requests without matching visible feedback state within 6 ticks / rejected requests; rate | no rejected requests → `NA` | hard target 0 for unexpected rejection; device/accessibility | flashing generic feedback for every input can game rate without comprehension | #147 feedback state, #148 linkage |
| `lifecycle_reconciliation` | accepted sequences with exactly one closed-enum terminal / accepted sequences; rate plus duplicate/orphan counts | unfinished match may use typed `match_terminated`; speculative revocation is diagnostic, not a terminal; no accepted sequences → `NA` | hard target 1.0; family/terminal kind | inventing `cancelled` at report time hides missing events | `CE.source_sequence` incomplete; #148 adds the terminal enum |
| `cooldown_use_share` | ticks cooldown is positive / eligible active ticks; share per action and player-match | no accepted action → `NA` | diagnostic; family/outcome | high cost can look like “depth”; low use can mean avoidance or irrelevance | `CS.cooldown_ticks`; #148 |

Allowed rejection reasons are versioned and closed: protected keeper/no
loadout, kickoff hold, soccer commitment wins, aerial state/recovery, forced
state, already committed, cooldown, missing press edge, and malformed input.
`unknown` is invalid. #112 uses the same outcomes as a human producer and
cannot bypass them.

### 4.3 P0 causal combat-to-soccer funnel

One encounter begins at an `accepted` commit and is keyed by its compound
`event_id` plus `source_sequence`. Attribution is eligible after terminals
`miss`, `expire`, `guarded`, `immune`, `superseded`, `hit`, `interrupted`, and
`cancelled`. `match_terminated` has no forward window. Rejected/“ignored”
requests never become encounters.

The **primary attribution window (`D`) is 180 ticks / 3.0 seconds**, beginning
at the terminal combat event's canonical tick. Sensitivity reports repeat the
attribution at 90 ticks (1.5 s) and 300 ticks (5.0 s). The primary window may
change only from blinded instrumentation-pilot evidence before freeze; all
three windows remain reported.

#148 implements this deterministic parent propagation:

1. `commit` is the root. Projectile, contact, forced, recoil, and spill events
   inherit its source sequence and store the root `event_id`.
2. A `hit` maps the source actor, target actor, target pre/post positions, and,
   for a spill, the ball. Guarded/immune/superseded contacts map both actors;
   miss/expire/interrupted/cancelled map the source actor and the paid
   commitment state.
3. Ball provenance begins only at a linked spill. It survives loose-ball ticks
   until a mapped source/target touch, a settled possession, or an independent
   ball event. Actor provenance survives while that actor remains in forced or
   action recovery.
4. A candidate soccer event inherits the encounter only when its actor is in
   the mapped actor set or its ball is carrying that encounter's provenance,
   and the exact predicate below passes.
5. A consequence may have one parent. When several encounters qualify, choose
   the greatest terminal tick, then greatest source sequence, then lowest
   `event_id`. Non-winning candidates remain in the sensitivity audit.

Within a tick, #148 processes: combat terminal and outcomes in canonical event
order; owner/touch transitions; soccer `MatchEvent` array order; score
transition; then stoppage. A direct mapped soccer event is considered before a
later same-tick score/stoppage. A score inherits only from an already attributed
shot/chance in its chain.

At each later event, attribution stops using this precedence:

1. full time, goal restart, or other stoppage after any same-tick attributable
   consequence;
2. first settled possession by a team unrelated to mapped spill provenance;
3. first independent ball event (`pass`, `shot`, `touch`, `tackle`, `claim`,
   `catch`, `parry`, `block`, `header`, `volley`, `bicycle`, or `reception`)
   whose actor and ball have no mapped provenance;
4. first independent accepted combat terminal touching the same mapped actor
   or ball; then
5. attribution-window expiry.

Current `MatchEvent` fields cannot encode these parents. #148 must add the
join/provenance rows and tests above; it may not infer a more favorable rule
from calibration results.

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
| `combat_to_soccer_conversion` | eligible encounters yielding one attributable settled possession, progressive pass, retained-space event, passing-lane control, receiver-lane denial, chance, shot, prevented shot, or goal / eligible terminal encounters | no encounters → `NA`; consequence types also reported separately | higher only with safety/soccer gates; 1.5/3/5 s | one loose composite hides low-quality or spam-created outcomes | CE→MS parent linkage; #148 |
| `time_to_soccer_consequence_s` | ticks from terminal combat result to first attributed soccer consequence / 60; distribution per converted encounter | no consequence → right-censored at window and reported | lower can indicate immediacy, not quality; consequence type | stopping only successful cases hides failures | #148 causal rows |
| `commitment_cost_conversion` | missed/expired/guarded/immune/superseded/interrupted/cancelled encounters with opponent opportunity, lost space, or possession risk / encounters in those terminals | zero denominator → `NA` | higher supports counterplay; family/response | rejected requests paid no commitment and are excluded | #148, definitions below |

For this contract:

- **settled possession** uses the existing 0.7 s `metrics.SETTLE_HOLD`;
- **attack-axis progress** is
  `team_sign * (end_x - start_x)`, with home `team_sign=+1` and away
  `team_sign=-1`;
- **clear segment** means no opponent collision circle intersects the closed
  segment after expanding each circle by `BALL_RADIUS=6`; exact tangency is
  blocked, and ties use stable player index;
- **progressive pass** advances the ball at least 48 px on the attack axis and
  is retained by the passer's team for 0.7 s;
- **retained space** freezes a radius-36 px zone at the linked target's
  pre-terminal position. The acting team must own the ball at the terminal;
  before entry, the target's expanded collision circle must become disjoint
  from that zone (tangency still blocks) and remain disjoint through credit.
  Only then may the carrier or ball enter that fixed zone, gain at least 36 px
  attack-axis progress from its terminal position, and remain team-settled for
  0.7 s;
- **open passing lane** is a clear segment of at least 48 px from the carrier
  to a non-keeper teammate's current position;
- **open shot lane/chance** requires an eligible carrier within 180 px of the
  opponent goal line and a clear segment to at least one of three goal points:
  center or 12 px inside either post;
- **likely shooter** is the current owner when that owner has an open shot
  lane; otherwise it is the non-keeper attacker with an open shot lane and the
  smallest squared distance to the ball, tied by stable player index;
- **passing-lane control** freezes every carrier-to-non-keeper-teammate segment
  of at least 48 px immediately before the terminal. A candidate was
  `blocked_solo_by_target` only when the linked target's expanded collision
  circle intersected it and, after removing that target alone, no other
  opponent circle intersected it. Credit requires that exact frozen endpoint
  segment to become clear of every opponent for 12 consecutive ticks;
- **receiver-lane denial** is separate: a linked opposing potential receiver
  must have had a clear carrier-to-receiver segment immediately before the
  terminal, and that exact segment must remain blocked for 12 consecutive
  ticks by the source actor or linked ball outcome. It is never counted as
  passing-lane control;
- **shot denial** requires the linked target to be the frozen likely shooter
  and the terminal to remove every open shot lane for at least 12 ticks;
- **shot** and **goal** use existing match events/score;
- **prevented shot** requires a pre-terminal open shot lane, the linked target
  to be the frozen likely shooter, and no independent event before the
  12-tick denial is satisfied; and
- **lost space/opponent opportunity** requires the source team to lose settled
  possession or lose at least 36 px attack-axis ball progress while the source
  is still in commitment/recovery, with the opponent settled for 0.7 s.

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
| possession / loose ball (two-sided equivalence) | owned ticks and loose ticks / ball-in-play ticks | no ball-in-play → retained protocol-failure row that blocks the pair; never an exclusion | 90% interval paired home-share change within `[-0.04,+0.04]`; absolute home share `[0.1,0.9]`; loose-share change within `[-0.08,+0.08]` | side/skill asymmetry; MS, loose share added #148 |
| formation displacement (upper NI) | player-to-current-authored-anchor distance summed / active outfield-player ticks; px plus share above 120 px | no active outfield ticks → protocol failure | 95% upper bound mean-distance B-A `<+12 px`; share-above-120 change `<+0.05` | pressing/tactic legitimately moves anchors; pair within identical formation/tactic; MS, #148 |
| progressive possessions / attacking-zone entries (lower NI) | possessions gaining >=96 attack-axis px before loss; settled entries across attacking-third line / active minute | no active minute → `NA`; zero events remains zero | 95% lower bound B/A `>0.85` for each; B absolute complete count reported | combat-created transition spam can inflate entries; MS provenance, #148 |
| chance creation / shot quality (lower NI) | open-shot-lane chances / active minute; on-target outfield shots / shots | no shots → on-target `NA` plus zero-shot count | chance-rate B/A lower bound `>0.85`; on-target-share B-A lower bound `>-0.08` | close low-quality shots can game volume; exact chance predicate in 4.3, #148 |
| drought (upper NI) / decided late (lower NI) | longest shot-or-goal drought; deciding-goal tick / match ticks | goalless decided-late remains 1 and is reported with zero-goal share | 95% upper bound drought B-A `<+3 s`, B absolute `<80 s`; 95% lower bound decided-late B-A `>-0.05`, B absolute `>0.05` | goalless matches inflate decided-late; existing metrics |
| dribble carry (equivalence) / heavy losses (upper NI) | carry ticks; close/sprint/juke shares; losses / carry minute | no carry → `NA` for shares | 90% interval carry-time ratio `[0.85,1.15]`; 95% upper bound heavy-loss rate B-A `<+0.50/min` | role/policy mix; existing metrics |
| dribble progression / retention (lower NI) | carries gaining >=48 attack-axis px; carries retained for >=0.7 s after that gain / carry opportunities | no carry opportunities → `NA` plus count | progression-rate B/A lower bound `>0.80`; retention-share B-A lower bound `>-0.08` | one long safe carry can dominate totals; per-carry rows, #148 |
| ball in play (one-sided NI) | active non-stoppage ticks / match ticks | no match ticks → retained protocol-failure row that blocks the pair; never an exclusion | 95% lower bound B-A `>-0.05`; B absolute `>=0.75` | early finish and stoppage definitions; #148 |
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

Timing has two non-overlapping clocks:

- `viable_input_window_ticks` is pure: latest canonical input tick that still
  produces the declared counter minus earliest legal counter-input tick plus
  one. It includes no wall-clock or latency subtraction.
- `observed_effective_window_ms` begins when the cue is actually presented on
  the measured device and ends at the final device-sample wall time whose
  mapped input arrives soon enough to occupy the last viable canonical tick.

The observed row records render-queue delay, device sample time, local producer
tick, network send/arrival ticks, prediction use, correction tick, confirmed
tick, cue state (`speculative`, `confirmed`, or `revoked`), display timestamp,
and tick-clock mapping error. Components are recorded once and never subtracted
twice. A revoked speculative cue is a valid diagnostic; only a confirmed cue
can support a comprehension claim.

Counters are labeled `reactive` (input after cue display),
`prediction_or_preposition` (movement/guard began before cue), or
`post_whiff_punishment` (response begins after miss/defense). Only `reactive`
supports the human-actionable cue-window gate.

Normal-context scenario ids freeze every family at center and edge of its legal
arc/range in carrier contest, carrier protection, loose-ball contest, and
off-ball lane/shot contexts, with each legal movement, juke, guard, or spacing
response. They run at `960x540` and `1280x720`, 30/60/120 Hz presentation,
keyboard and standard gamepad, native LÖVE 11.5 plus the recorded Chrome and
Firefox versions, and network profiles `clean`, `omp0_parity`, and `playable`.
The `stress` profile is adversarial, not a normal-context support claim.

The burden pilot derives `human_actionable_floor_ticks` as the ceiling of the
worst supported stratum's p95 cue-to-valid-input latency plus p95 absolute
tick-clock mapping error, converted to 60 Hz ticks. The numeric value and raw
pilot hash must be inserted by a pre-freeze amendment and re-reviewed; until
then counterplay remains blocked. Every normal-context
`viable_input_window_ticks` minimum and observed p05 reactive window must meet
that floor. A merely positive or frame-perfect window fails.

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `counter_coverage` | threats with >=1 response meeting the canonical and pilot-derived floors / incoming threats; rate | no threats → `NA` | hard target 1 in frozen normal contexts; family/response/device/network | declaring a frame-perfect response viable | #113 core cue, #147 specialization, #148 canonical matrix |
| `counter_attempt_rate` | valid response attempts / threats with viable response; rate | no viable threats → `NA` | diagnostic; family/skill/accessibility | players may not perceive the cue | input linkage; #148 |
| `counter_success` | avoid/defend/reverse outcomes / valid attempts; rate | no attempts → `NA` | no universal threshold; family-response matrix | tuning attacks weak to inflate success | CE + input linkage; #148 |
| `viable_input_window_ticks` | inclusive earliest-to-latest successful canonical inputs; ticks per scenario | no legal successful input → 0/hard failure | minimum >= pilot floor; scenario/family/response | sim-only window does not prove presentation | #148 |
| `observed_effective_window_ms` | cue-display wall time to last effective sampled input wall time; ms per threat | missing mapping/component → structural missing row and blocked stratum | p05 and minimum converted ticks >= pilot floor; device/runtime/network/cue state | averaging or double-subtracting latency hides bad tails | #150 runtime/network evidence; #151 participant/device strata |
| `tick_clock_mapping_error_ms` | absolute difference between observed timestamp mapping and canonical boundary time; ms | missing clock anchor → protocol failure | lower; p50/p95/max by runtime | correcting timestamps after outcome can flatter window | #150 |
| `attacker_punishment` | defended/missed actions followed by recovery exposure, lost space, possession risk, or opponent opportunity / defended/missed actions | no defended/missed action → `NA` | higher supports tradeoff; family/response; 3 s | vague opportunity labels manufacture punishment | CE→MS; #148 |
| `unseen_impact_rate` | impacts the participant could not identify before contact / impacts probed; rate | unprobed impact kept as missing | lower; family/presentation/accessibility | asking leading questions improves apparent recognition | replay probe; #151 |
| `occluded_or_masked_cue_rate` | threats whose cue is geometrically occluded, audio-masked, or HUD/ball-masked beyond threshold / threats | missing render evidence → `NA` | lower; viewport/frame rate/accessibility | telemetry can disagree with human perception | presentation evidence; #147/#148 |
| `concurrent_cue_count` / `cue_overlap_share` | active cues per tick; ticks with >=2 cues / threat ticks | no threat ticks → `NA` | lower tails, diagnostic | suppressing needed cues improves metric but harms comprehension | #147/#148 |
| `false_defensive_reaction_rate` | defensive responses to no authoritative threat / defensive response opportunities | no responses → `NA` | lower; presentation/device | cautious play or ordinary juke can be mislabeled | input + cue linkage; #148 |
| `ball_hud_occlusion_share` | threat ticks where cue geometry masks ball or required HUD target / threat ticks | no threat ticks → `NA` | hard review trigger above 0; viewport | pixel overlap is only a proxy for readability | presentation capture; #147/#148 |
| `accepted_causal_identification_accuracy` | correctly identified source, family, direction, target, and available-response components / 30 applicable components from exactly six accepted B rows; rate per participant plus five component rates with denominator 6 each | `unsure` or missing response is incorrect but retained; fewer than six unique accepted rows → structural missingness and blocked coverage; rejected rows and A are absent | confirmatory simultaneous lower bound `>=0.70` overall; component and accepted-terminal priority-stratum rules in 6.3 | guessing, leading labels, memory delay, accommodated exposure | neutral B replay probe; #151 |
| `rejected_request_feedback_accuracy` | correctly identified perceived rejection state, typed rejection reason, and feedback meaning / 6 applicable components from exactly two rejected B rows; rate per participant plus three component rates with denominator 2 each | telemetry `missing_feedback` has a scored no-feedback answer; `unsure` or missing response is incorrect but retained; fewer than two unique rejected rows → structural missingness and blocked coverage; accepted rows and A are absent | confirmatory simultaneous lower bound `>=0.70` overall; component rules in 6.3 | generic feedback, guessing, memory delay, accommodated exposure | neutral B replay probe; #151 |
| `causal_identification_time_s` | probe onset to final answer; seconds per probe | timeout is right-censored at declared limit | lower diagnostic; same strata | faster guesses are not better comprehension | #151 |

Freeze-probe tasks are separate from ordinary matches. Concurrent think-aloud
is prohibited during high-action play because observer presence/type can alter
performance, motivation, anxiety, and reported experience in some contexts
([E][observer-effects]). Replay-cued commentary is subjective comprehension
evidence anchored to telemetry, not authoritative proof of causality
([E: small qualitative method][gow]).

### 4.6 P0 off-ball purpose and horizontal balance

#148 evaluates the following eligibility predicates from the authoritative
decision-tick snapshot for every human and bot opportunity:

1. `carrier_contest`: the linked target is the opposing current owner;
2. `carrier_protection`: source team owns the ball, the target is within 48 px
   of that owner, and is the nearest opposing outfielder to the owner (stable
   index tie);
3. `loose_ball_contest`: owner is nil and both source and target are within
   96 px of the ball;
4. `passing_lane_or_shot_denial`: the target is the sole blocker of a frozen
   candidate passing segment, is the frozen likely shooter, or controls an
   open shot lane under section 4.3's exact predicates; or
5. `recovery_punish`: the target is in an accepted equipment recovery phase
   **and** at least one of predicates 1–4 is also true for that source/target.

All true predicates are retained as an eligibility bitset. Zero true predicates
on a committed action produces the closed outcome `unattributed_off_ball`;
more than one becomes `multi_context` and is reported in every applicable
stratum rather than coerced into a favorable purpose. Recovery without a ball,
carrier, or lane/shot predicate is `recovery_only_diagnostic`, never an
eligibility bit or attributed purpose. `formation_risk_tradeoff` is not a
purpose. It is a separate cost flag when committing increases
source-to-anchor distance by at least 36 px or leaves the source more than
120 px from its current authored anchor.

`family_commit_feasibility/v1` is a pure temporal predicate shared by the
family-neutral envelope and commit reconciliation. Its only dynamic input is
`combat_sim_observation/v1` at the decision tick plus a searched legal
source-movement/facing tape, a frozen target identity, a catalog family id, and
the frozen catalog/static-collision version. It does not read confirmed future
inputs, rollback-only or hidden state, an unresolved outcome, presentation, or
eventual contact. Public motion is projected deterministically: a public
soccer/combat accepted or forced phase uses its catalogued public pose path
anchored at the observed state; otherwise current public position and velocity
advance under neutral input at 60 Hz with the frozen pitch/collision rules. The
source follows the searched tape until its candidate commit tick. From that
commit through the end of the family proof horizon, it repeats the witness
tape's last legal movement vector and facing input (neutral movement and
observed facing when the tape is empty) through the ordinary family movement
multiplier, pitch, and collision transition. An exact catalogued
committed-motion rule supersedes that repetition when present. No later input
is searched or read. This no-response projection is a feasibility
counterfactual, not a claim that the target will stand still or that a hit will
occur.

The frozen mechanics rows below come directly from the family catalog. Timing
uses the ordinary `sim.combat` transition order; `hit` is the unguarded
`interruption ticks / displacement px / ball spill` tuple.

| Family | Activation and committed schedule | Threat geometry / travel | Actual recovery, cooldown, and hit |
| --- | --- | --- | --- |
| `unarmed` | press edge; windup 6 ticks, active 4 ticks | melee reach 30 px, front arc 100°, movement multiplier 0.80 | recovery 12, cooldown 24, hit `10/8/true` |
| `light_melee` | press edge; windup 12 ticks, active 5 ticks | melee reach 42 px, front arc 75°, movement multiplier 0.50 | recovery 21, cooldown 42, hit `18/18/true` |
| `ranged` | held-release; windup 18 ticks, active/spawn 1 tick | front arc 20°, movement multiplier 0.40, projectile 300 px/s for at most 60 ticks | recovery 27, cooldown 60, hit `12/10/true` |
| `guard` | held; windup 6 ticks, then active while held | self-only guard arc 120°, movement multiplier 0.55 | recovery 9 after release, cooldown 0, no unguarded hit |

The predicate evaluates an executable family-specific tape and horizon:

- `unarmed`: commit with the canonical press edge, apply its 6-tick windup and
  4 active ticks, and require the catalogued swept melee threat to intersect
  the frozen target's projected collision volume on at least one
  contact-legal active tick;
- `light_melee`: commit with the canonical press edge, apply its 12-tick
  windup and 5 active ticks, and require the catalogued swept melee threat to
  intersect the frozen target's projected collision volume on at least one
  contact-legal active tick;
- `ranged`: commit with `pressed=true, held=true, released=false`; on the next
  canonical tick emit the frozen early-release
  `held=false, released=true` edge, which latches through the rest of the
  18-tick windup. A same-tick release is also legal and produces the same
  post-windup spawn; v1 uses the next-tick edge as its one canonical witness.
  At the resulting 1-tick active/projectile-spawn transition, require a legal
  aim and clear line against projected public blockers. Freeze direction at
  spawn, then advance the projectile at 300 px/s in canonical collision order
  for no more than its 60-tick lifetime or until public field exit, whichever
  comes first. Feasibility requires an intersection with the frozen target's
  projected collision volume inside that travel horizon; and
- `guard`: commit with `pressed=true, held=true, released=false`, hold through
  the 6-tick windup and subsequent guard phase, and release on the tick after
  the first witnessed defensive intersection. Its relevant threat set contains
  only finite paths already proved by the public observation:
  (1) a hostile melee `windup` or `active` row through its remaining public
  windup/active path; (2) an already in-flight hostile projectile through that
  row's public `horizon_ticks`; or (3) a hostile ranged row whose accepted
  source sequence and public `release_latched=true` state fix its spawn, through
  remaining public windup-to-spawn ticks plus the canonical spawned
  projectile's `min(lifetime, field-exit)` path. Projected latch-resolved ranged
  telegraphs therefore participate; held or aimed ranged state without a
  public release latch does not.

For each relevant threat, the path is reconstructed from its public phase,
remaining phase ticks, telegraph start/end, projected geometry,
source-sequence id, and, for ranged, release-latch and projected-spawn fields.
The last relative path tick is remaining windup plus catalog active ticks for
melee windup, remaining active ticks for melee active, projectile
`horizon_ticks` for an in-flight row, or remaining ticks to ranged spawn plus
the spawned projectile horizon for latch-resolved ranged. The guard
intersection cap is the maximum of those finite last ticks and the 6-tick guard
raise needed to test an active intersection. No relevant public path makes
guard infeasible. The guard must intersect the frozen hostile source's melee or
projectile/contact path inside its catalogued arc on an active guard tick at or
before that cap; it then releases on the next tick. No intersection is
infeasible, and the release tick cannot create one.

Guard never extends, truncates, or invents a threat path using future confirmed
input, hidden state, RNG, or eventual outcome. It is self-only, so neither
feasibility nor reconciliation asks the guard geometry to contain the hostile
player's body.

All four families contribute their temporal relation. Family feasibility
ignores which family is actually equipped and ignores actual cooldown,
recovery, commitment, request acceptance, resource cost, and hit outcome.
Those catalog fields remain actual-family availability/cost/outcome facts and
are still measured for four-family balance. Feasibility remains true after a
subsequent miss, defense, supersession, interruption, expiry, or other
terminal: eventual hit/miss/contact is not part of this predicate.

`intervention_candidate/v2` is the family-neutral feasibility envelope.
Starting from the decision-tick boundary, a pure reachability search varies
only the source's canonical legal movement/facing inputs for at most 30 ticks.
Every non-source trajectory is the public no-response projection above, never
a future confirmed tape. A `(target, context)` pair enters the envelope only
when its purpose predicate is true and at least one searched source pose makes
`family_commit_feasibility/v1` true for at least one of the four catalogued
families toward that same frozen target/threat-source identity. A commit may
start within the 30-tick search window; its family-specific windup, active,
release, projectile-travel, or guard horizon may end later.

#148 freezes the input alphabet/search order, public projection rule, family
catalog version, and static collision version; deduplicates equal states by
canonical state hash; and records first feasible commit tick, witness-tape
hash, carried post-commit movement/facing input, ranged release/spawn/travel
ticks, guard hold/intersection/release ticks, family-feasibility bitset,
projection-input digest, and catalog ids. The stored envelope is offline
evidence and never becomes an extra AI observation. The same pure helper is
lawful for #112 to recompute because every dynamic field it reads is already
in `combat_sim_observation/v1`; #148 independently recomputes rather than
trusting the AI result. A true purpose predicate outside the envelope is
`context_only_remote`: retained as a diagnostic, never an opportunity or
dominance denominator.

#112 emits one stable bot decision reason from the five purpose ids or
`decline`, using only its observable allowlist. #148 independently records the
eligibility bitset, intervention envelope, and risk flag. An accepted commit
reason reconciles only when its frozen `(target, context)` is in that envelope
**and** `family_commit_feasibility/v1`, recomputed from that commit's public
decision snapshot, its one-tick actually materialized movement/facing input,
and the actually equipped family, is true for the same frozen identity. The
helper carries that input after commit under the frozen rule above; it reads no
later confirmed input. For unarmed, light melee, and ranged the identity is the
projected target; for guard it is the incoming hostile threat source and the
defensive-intersection relation above applies. Reconciliation never requires
eventual contact or a favorable terminal. `decline` is valid only when the
episode closes without an action; it can never label a commit. A committed
zero-bitset or infeasible-target action is
`unattributed_off_ball`; for a representative policy it additionally raises
the hard
`representative_policy_context_violation` schema error. Human stated intent is
a separate replay-debrief field, never substituted for authoritative context.
An AI reason is an intent claim, not proof of value.

A **family-neutral opportunity set** contains every `(target, context)` pair
inside `intervention_candidate/v2`, without consulting the equipped family or
its actual readiness. One source-player episode starts when that set becomes
nonempty, stores its sorted pairs and canonical formation slot, and ends when
the set changes, an action commits, or 30 ticks elapse. Thus every family
receives the same feasible intervention denominator without dilution by remote
soccer context.

Each episode closes with exactly one outcome:

- `acted` when an accepted commit occurred, with its target and reason;
- `declined` when at least one tick was action-ready but no commit occurred; or
- `unavailable:<reason>` when no tick was action-ready, using the closed
  reason with the most unavailable ticks and rejection-enum order as the tie
  break.

Per-reason unavailable tick counts are always retained. Allowed unavailability
reasons are `no_loadout`, `soccer_commitment`, `aerial_or_recovery`, `forced`,
`already_committed`, `cooldown`, and `family_commit_feasibility`; malformed
input is a protocol failure and missing press edge is a decline, not
unavailability. For this classifier, `action-ready` means a legal request
could reach at least one envelope pair under the equipped family's current
`family_commit_feasibility/v1` relation and non-input state; the physical press
edge itself is not a readiness requirement. Every episode separately records
equipped-family feasibility-available ticks, temporal geometry/line failures,
ready ticks, cooldown/recovery/commitment ticks, and accepted cadence; none can
change the common envelope.

Episode benefits are linked only from an accepted action and counted once per
group: at most one possession gain (`+1.0`), one of progressive pass, retained
space, passing-lane control, or receiver-lane denial (`+0.5`), one of chance,
shot, or prevented shot (`+1.0`), and one goal (`+3.0`). Costs cap at one
settled-possession loss (`-1.0`) and one lost-space/opponent-chance outcome
(`-0.5`), while duration costs integrate continuously. `interrupted` is a paid
commitment terminal.

Formation cost samples the half-open confirmed-tick interval
`[episode_start, cost_end)`, where `cost_end` is episode end without an action
or the first action-ready tick after the linked terminal/recovery. With `d0`
the source-to-current-authored-anchor distance at episode start,
`formation_displacement_px_s = sum(max(0, d(t)-d0) / 60)`. If recovery
extension overlaps another episode, each source tick belongs to the earliest
episode start (then lowest episode id); it is never charged twice.
Multi-context pairs share the same source episode. `net_soccer_utility` is
reported both per opportunity and per active player-minute:

```text
+1.0 settled possession gained
+0.5 progressive pass, retained space, passing-lane control, or receiver-lane denial
+1.0 chance, shot, or prevented shot
+3.0 goal
-1.0 settled possession lost
-0.5 lost space or opponent chance
-commitment_seconds / 3
-recovery_seconds / 3
-cooldown_unavailable_seconds / 6
-formation_displacement_px_s / 120
```

Raw components are always reported; the weighted utility is a declared project
comparison, not human fun.

| Metric | Numerator / denominator; unit and grain | Exclusions and zero/missing behavior | Direction; strata and window | Confounds and Goodhart failure | Source / owner |
| --- | --- | --- | --- | --- | --- |
| `off_ball_context_share` | opportunities/actions in each eligibility bit / all opportunities/actions; shares | no opportunities → `NA`; multi-context rows remain in every true bit | diagnostic; family/slot/phase | selected actions hide declines and availability | MS/CE; #148 |
| `ai_reason_reconciliation` | bot reasons present in the independent eligibility bitset / bot opportunity decisions | no opportunities → `NA`; missing reason is schema failure | hard target 1.0 | AI labels cannot define their own ground truth | `R112` plus MS/CE; #112/#148 |
| `unattributed_off_ball_share` | unattributed off-ball actions / off-ball actions; rate | no off-ball actions → `NA` | review trigger, not automatic harassment; player/policy | forcing every action into a named bucket hides abuse | MS/CE and `R112`; #148 |
| `reason_value_conversion` | actions producing the context-specific soccer consequence / actions in that context | zero bucket → `NA`; 1.5/3/5 s | higher only with guardrails | circularly defining outcome from reason | MS/CE plus R112 diagnostics; #148 |
| `family_context_utility` | net utility and each raw benefit/cost / matched opportunity and active player-minute | absent cell → missing, never zero; declines stay in denominator | SESOI `+0.10` net utility/opportunity; family×slot×phase×skill | selected-action conversion hides availability and cost | C rotation; #149 |
| `strict_upgrade_cells` | contexts where one family exceeds every alternative by `+0.10` utility/opportunity and is no worse on safety/soccer/cost components / evaluable contexts | insufficient matched opportunity precision → inconclusive | hard target: no family wins every priority context | aggregate weights can hide component harm | C/D matrices; #149 |
| `machine_unarmed_viability` | unarmed player's touches, passes, progressive involvements, option episodes, settled-possession contributions, and net soccer utility / active player-minute and opportunities | missing role/slot cell → inconclusive | all section 4.4 team gates pass; each per-slot involvement ratio lower bound `>0.80`; utility difference lower bound `>-0.10/opportunity` | team success can hide one sidelined player | #149 |
| `human_unarmed_viability` | unarmed player's enjoyment, partial-PXI autonomy/ease, and replay-cued agency versus matched armed role | incomplete follow-up retained as missing/harm disposition | enjoyment lower bound `>-0.50` points; other outcomes diagnostic unless powered | occasional selection is not agency | #151 evidence, #114 decision |

Priority contexts are exactly the five ids above: carrier contest, carrier
protection, loose-ball contest, lane/shot denial, and recovery punish. They are
crossed with every canonical
`<formation_id>/outfield/<outfield_index>` slot and both sides.
`formation_risk_tradeoff` remains only a cost flag and can never become a sixth
context or reason. Results report authored player/stat, formation, score state,
skill, and policy. A family is a strict upgrade only when its
matched-opportunity uncertainty interval clears the context SESOI in every
priority context and no raw cost/safety guard is worse. Absence of evidence is
`inconclusive`, not balance. #149 can close from the machine gate without
waiting for #151; the later human gate remains required for #114.

### 4.7 Human-proxy and adversarial policy roles

`combat_sim_observation/v1` is the only schema available inside `sim/` and to
shipped gameplay AI. It contains authoritative public simulation state only:

- self: stable player/slot/team id, family and public source-sequence id,
  public keeper/outfielder role, soccer action/commitment state,
  accepted-action/forced/immunity phase and remaining phase ticks, position,
  velocity, facing, own ready/not-ready cooldown, and own materialized input
  history through the observed tick;
- teammates: one row for every other same-team player with stable
  player/slot/team id, public keeper/outfielder role, soccer
  action/commitment state, family, public source-sequence id, accepted-action/
  forced/immunity/guard phase and remaining public phase ticks, telegraph
  start/end and projected action geometry, ranged `release_latched`, and
  `projected_spawn_tick` only when that latch is true (both nil for other
  families), position, velocity, and facing;
- opponents: one row for every opposing player with the same authoritative
  public identity, keeper/outfielder role, soccer action/commitment, family,
  source-sequence, action/forced/immunity/guard phase, remaining phase ticks,
  telegraph, projected geometry, ranged release-latch/spawn, position,
  velocity, and facing fields as teammate rows;
- in-flight projectiles: stable `projectile_id`, source player/team id,
  source-sequence id, family id, position, unit direction and velocity in
  px/s, public `in_flight` phase, remaining lifetime ticks, and
  `horizon_ticks = min(remaining_lifetime_ticks, field_exit_ticks)` for legal
  guard and spacing. `projectile_id` is the collision-free canonical encoding
  of `(source_player_id, source_sequence)` because the catalog emits at most
  one projectile per source sequence;
- ball and match: ball position/velocity/owner, score, canonical remaining
  ticks, formation-slot anchors, public stoppage state, and the public
  pitch/static-collision catalog version; and
- identity: schema, policy id, producer/observed canonical tick, canonical
  player-index order, projectile order, family/catalog versions, and digest.

It excludes pixels, cue visibility/occlusion, render timing, theme,
presentation id, viewport, frame rate, unresolved outcomes, hidden collision
results, future input, and RNG state. Teammate and opponent rows are ordered by
canonical player index. Projectile rows are ordered by
`(source_sequence, source_player_id, projectile_id)`; all scalar/vector fields
use the section 4.0 canonical encoding. The allowlist and digest cover the
schema tag, row counts, row order, every public field above, and their catalog
versions, and reject a missing, duplicate, reordered, or undeclared field.

`human_proxy_observation/v1` is evidence-only and is built outside `sim/`. It
joins a simulation observation to recorded cue visible/occluded state,
presented cue geometry, viewport/theme/presentation identity, render/device
timestamps, and accessibility presentation. The harness then applies a
12-tick reaction delay, 12-tick cadence, seeded aim/position noise, 4 px
position and 16 px/s velocity quantization, and 16-sector facing quantization.
The proxy can act only by materializing an ordinary input tape that passes the
same producer and legality checks as human input; it has no direct simulation
action channel.

Each schema has a separate canonical digest and allowlist test. #112 emits the
simulation digest with its reason; the evidence harness emits the human-proxy
digest. Gameplay-AI decision inputs, ordered public rows, digests, reasons, and
materialized input tapes remain invariant across presentation/theme/viewport
swaps. Those presentation identities are retained only outside the policy
input in fixture E, which must reproduce the identical observation digest,
gameplay-AI decisions, and input tapes.

Policy ids are distinct:

- `gameplay_ai/combat/v1` — deterministic shipped gameplay AI using only
  `combat_sim_observation/v1` at its declared gameplay cadence;
- `human_proxy/combat/v1` — the representative delayed/quantized/noisy
  evidence population using `human_proxy_observation/v1`;
- `adversarial_observable/combat/v1` — exploit search constrained to
  `combat_sim_observation/v1`; and
- `adversarial_privileged/combat/v1` — explicitly diagnostic search that may
  read hidden state but still emits only legal materialized inputs.

The adversarial population searches chain extension, permanent guard, ranged
lane denial, safe zones, cooldown loops, repeated-family dominance, and
off-ball harassment. Privileged state is allowed only in fixtures explicitly
tagged `adversarial_privileged`; those results may find counterexamples but
never support representative-player, comprehension, or fun claims. Reports
never pool policy ids.

A privileged search may falsify a deterministic safety invariant when its
materialized input tape is legal and reproduces from an ordinary public
replay. A dominance or harassment claim must additionally reproduce either
under `adversarial_observable/combat/v1` or as a fixed feasible open-loop input
strategy, and its paired lower interval must clear the preregistered
`+0.10 net utility/opportunity` margin across the declared context population.
Otherwise it is a diagnostic hypothesis, not a population dominance claim.

## 5. Human player-experience protocol

### 5.1 Instrument stack and reuse constraints

| Purpose | Instrument | Locked use |
| --- | --- | --- |
| Primary repeated endpoint | Three-item PXI enjoyment add-on | after A and B; independently supported as a separate factor, not one of the original ten PXI constructs |
| Mechanism diagnostics | Unchanged PXI autonomy, mastery, challenge, ease-of-control, goals/rules, and progress-feedback subscales | partial-PXI diagnostics only; no claim that selected subscales are a full validated PXI administration or benchmark |
| Exploratory deep-dive mechanisms | 18-item BANGS particular-session variant | separate formative deep-dive wave only; six three-item satisfaction/frustration subscales; never collapse frustration into satisfaction |
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

PXI was not validated as a responsiveness instrument for this two-period
combat crossover. This is a project application, not a claim of validated
within-person change sensitivity. Before freeze, the burden pilot must reach
the complete three-item enjoyment rule in at least 90% of administered
condition blocks. Condition-wise omega and any within-person contrast
reliability/generalizability decomposition are explicitly **diagnostic**:
their estimator, variance components, script/environment hash, and uncertainty
are reported, but no numeric reliability cutoff gates the study or supports a
responsiveness claim. Confirmatory usability is decided by the complete-score
rule and the exact-model power, missingness sensitivity, and interval-precision
requirements in section 6. Item-level ordinal and plausible
single-item-missing sensitivity analyses test whether the condition contrast
depends on the complete-score rule.

BANGS uses the particular-session wording, all 18 items, randomized order, and
the validated labeled `1..7` response scale in a separate formative deep-dive
wave of at least 16 participants, with order balanced and the instrument
administered after each condition. Its six subscales are scored separately.
BANGS is exploratory: it has no confirmatory harm margin, multiplicity family,
or `proceed` gate in Milestone 11. The article is CC BY 4.0; the accompanying
guide/materials are CC BY-SA 4.0, so #151 records exact source/version,
attribution, and share-alike obligations ([E and guide][bangs],
[BANGS guide][bangs-guide]).

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

### 5.3 Replay-probe sampling and scoring

Comprehension is measured only for combat-active condition B. #151 builds two
disjoint confirmed pools:

- `accepted_encounter_pool`: one `accepted_encounter` probe row per accepted
  source sequence with its terminal and linked presentation evidence; and
- `rejected_request_feedback_pool`: one `rejected_request` probe row per
  confirmed rejected physical request, its typed rejection reason, and its
  optional feedback-event FK or typed `missing_feedback` result.

A rejected-request row has no source sequence and never becomes an encounter.
The eligible pool is the tagged union of those two row types. Reports publish
each pool's raw count, selected count, applicable component denominators, and
unique row ids. A fixed debrief seed hashes the pseudonymous session id,
literal `condition_b`, and protocol version, then samples exactly eight unique
rows without replacement:

- two `hit` events, including a spill/forced event when available;
- two defended contacts from `guarded`, `immune`, or `superseded`;
- two `miss`, `expire`, `interrupted`, or `cancelled` terminals; and
- two rejected requests, prioritizing `unexpected_rejection` and
  `missing_feedback`.

Within the accepted pool, a missing terminal stratum is filled from the next
listed accepted terminal stratum in cyclic order; within the rejected pool, a
missing priority is filled by another rejected-request row. Cross-pool
substitution is forbidden. Fewer than six unique accepted rows, two unique
rejected rows, or eight unique rows in the union is `blocked_coverage`: no
sampling with replacement, silent reuse, or facilitator-selected clip is
allowed. Sampling balances families within row type before repeating a family.

`replay_probe_component/v2` freezes this ordered component vector and exact
row-type applicability masks (`1` means scored, `0` means absent):

| Row type | source | family | direction | target | available response | perceived rejection state | typed rejection reason | feedback comprehension |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `accepted_encounter` | 1 | 1 | 1 | 1 | 1 | 0 | 0 | 0 |
| `rejected_request` | 0 | 0 | 0 | 0 | 0 | 1 | 1 | 1 |

Accepted truth comes from the linked accepted sequence, canonical legal-response
matrix, and presented evidence. For a rejected row, perceived rejection state
is correct only when the participant identifies `rejected`; typed reason is
correct only when the selected closed-enum reason equals the confirmed
request's telemetry reason. The feedback codebook maps each feedback-event
enum to one neutral plain-language meaning. When the row has a feedback FK,
feedback comprehension is correct only for that meaning; when telemetry is
typed `missing_feedback`, it is correct only for
`no_feedback_presented`. `unsure` is a distinct answer for every component and
is retained as incorrect. A blank or timed-out component is retained as
`missing_response` and incorrect; it is never confused with telemetry
`missing_feedback`.

The accepted endpoint is correct applicable components divided by exactly 30
(six rows × five components), with each accepted component denominator exactly
6. The rejected-feedback endpoint is correct applicable components divided by
exactly 6 (two rows × three components), with each rejected component
denominator exactly 2. These are separate confirmatory claims: no
accepted/rejected pooled accuracy, discretionary weighting, or substitution is
permitted. A pooled summary may not be reported as a claim, even if labeled
exploratory. The masks, selected row ids, component responses, correctness, and
denominators are exported. Condition A has no combat probe and cannot enter
either comprehension denominator or power model. Milestone 11 does not
administer an A guessing/no-threat control; a later control must use a
separately named soccer/no-threat diagnostic pool and may not be pooled with B
accuracy.

The probe begins within five minutes of B. It first plays once at full speed
without overlay, then permits exactly one rewind and up to two
participant-requested pauses. Accessibility accommodations may allow additional
rewinds, pauses, slower replay, or modality alternatives; the exact exposure is
recorded as `accommodated_exposure` and reported as its own stratum. The
participant answers before any source, family, target, direction, terminal, or
response annotation is revealed. Neutral prompts ask what was noticed,
intended, caused, and still possible. Primary comprehension retains
accommodated participants and includes a strict-exposure sensitivity; a
disposition change makes the gate unresolved rather than denying the
accommodation.

The legal-response rubric is generated from the frozen canonical scenario
matrix and lists every response as legal/illegal, reactive/predictive/
post-whiff, its viable tick interval, and actual outcome. The facilitator sees
opaque condition ids and not the intended hypothesis. A separate scorer,
blinded to condition id, assignment order, aggregate results, and product
preference, scores source/family/direction/target/response components.
Disagreement receives a second blinded score and adjudication; raw answers and
both scores remain retained. Replay recall is subjective comprehension
evidence, not authoritative causality.

### 5.4 Accessibility

Recruitment records functional configuration, not diagnosis. The planned 48
complete pairs must cover at least eight participants in each domain below
(overlap is allowed), with at least four keyboard and four gamepad users where
the function affects input:

| Functional domain | Supported configuration under test |
| --- | --- |
| Vision/color/contrast | 200% text, high contrast, non-color cue channel, supported corrective lenses |
| Deaf/Hard of Hearing | captions/text or visual equivalents for every research/gameplay audio cue |
| Motor/dexterity/hold-tap | remapping where available, keyboard/gamepad alternatives, no simultaneous hold requirement in research UI |
| Cognitive/processing | plain language, practice/repetition, participant-paced research screens and breaks |
| Motion/photosensitivity | reduced motion/flash setting, no essential information carried by shake/flash |

Each participant-domain uses one frozen checklist. With the supported
configuration active, the participant must:

1. navigate instructions, practice, replay, and instrument screens;
2. submit one response, deliberately skip one optional response, and recover
   from one validation message;
3. operate replay play/pause/rewind and answer controls;
4. identify ball/possession state and one combat telegraph through the claimed
   modality; and
5. execute one mapped soccer action and one viable defensive response in the
   canonical practice scenario.

The binary endpoint is 1 only when all applicable items complete without the
operator performing an input or revealing an answer; otherwise it is 0.
Allowed assistance is setup, device positioning, remapping, verbatim
read-aloud/repetition, participant-chosen time, rest, and the replay
accommodations in section 5.3. Assistance and every checklist item are
recorded.

The numerator is completed participant-domain checklists. The denominator is
every participant who consents to and begins that domain's configuration.
Unavailable modality, structural inability after setup begins, assistance
beyond the allowed list, or access/adverse stop is a noncompletion and harm
disposition. Withdrawal before setup is structural missingness in the assigned
count; withdrawal/deletion that removes a begun result is also structural
missingness and is treated as failure in the mandatory worst-case
sensitivity. Neither can improve the rate.

For each domain, report assigned, begun, completed, structural-missing, and
independent-participant counts plus the simultaneous interval in section 6.3;
planned two-sided precision is half-width `<=0.35`. Falling below
count/precision is an explicit coverage limitation and prevents a claim for
that domain. Any access failure, adverse response, or access-related dropout is
retained as a harm disposition regardless of rate and is never an
outcome-blind exclusion.

Research UI acceptance requires:

- complete keyboard and gamepad focus order, no focus trap, and visible focus;
- text reflow without clipping at 200% scale;
- text contrast at least 4.5:1, large text/non-text/focus contrast at least
  3:1, and no color-only meaning;
- pointer/touch targets at least 44×44 px with 8 px spacing;
- programmatic and visible labels, plain instructions, error identification,
  and status confirmation;
- participant-controlled time, pause/resume, breaks, and no expiring response;
  and
- text/non-text and audio/visual modality alternatives.

A normal supported gameplay configuration with no viable cue/counter is a hard
failure. If the visual Affective Slider itself is inaccessible, record
structural missingness and offer a separately named custom accessible
valence/arousal item for participant care. That custom result is not a
validated equivalent and is never pooled with Affective Slider scores or used
to exclude the participant.

### 5.5 Privacy, consent, and custody for Milestone 11

Milestone 11 collects only what the declared questions require:

- deterministic gameplay trace/events;
- device/performance diagnostics;
- instrument responses and structured missingness;
- optional replay annotations/free text; and
- optional audio/video/screen media, disabled by default.

Each is a separate consent scope with separate allowed internal-analysis and
de-identified-public-aggregate uses. Declining an optional scope does not
change eligibility, compensation, core participation, or access to the game.
Compensation is not contingent on completing the session or answering every
item. A participant may stop, skip, or withdraw without penalty.

Recruitment is adult-only (18+) unless a separately approved guardian-consent
and appropriate ethics path exists; no such path is approved by this contract.
Consent is plain-language and available in text plus a participant-requested
read-aloud/accessible modality. The operator offers an accommodation path
before practice and stops/escalates on pain, distress, photosensitivity,
motion sickness, fatigue, or other adverse burden.

Names, emails, account handles, IPs, hostnames, raw paths, direct identifiers,
consent prose, and re-identification keys never enter gameplay artifacts,
filenames, Git, issues, CI artifacts, cloud sync, or public reports.
Participant/session ids are non-guessable random values. External packages use
rotated publication ids that cannot join to the internal longitudinal id. The
recontact/withdrawal map is stored separately from research payloads.

All participant data, including trace and device diagnostics, lives on an
encrypted local research volume. It is never placed in the repository, GitHub
issues/PRs, CI artifacts, ordinary cloud drives, or unapproved remote services.
Least-privilege roles are:

- collection operator: write-only access to the isolated intake quarantine;
- study owner: consent validation and separately encrypted withdrawal map;
- research lead: approved pseudonymous payload and deletion authority;
- analyst/scorer: only the minimum pseudonymous scopes needed for analysis;
  and
- reviewer: aggregate tables and redacted exemplars only.

Access, export, quarantine, redaction, tombstone, deletion, and backup-expiry
actions append timestamp, role, participant/session id, scope, and action to an
encrypted audit log; the log contains no response, trace, free text, media, or
withdrawn payload.

There is one encrypted offline backup of approved pseudonymous payloads and no
backup of the re-identification map beyond its separately encrypted primary
copy. A withdrawal tombstone is applied immediately; primary and backup
payload deletion is verified by hash/manifest within 14 days, and any expired
backup copy is destroyed no later than 30 days. Derived tables are rebuilt
from the tombstoned manifest, never patched by hand.

The Milestone 11 manual custody decision (`D`) is: raw optional media is deleted
within 30 days of validated transcription/debrief or 90 days after collection,
whichever comes first; pseudonymous trace/responses are retained for at most
one year; the withdrawal map is deleted with the last retained session.
Withdrawal requests are actioned within 14 days across raw and derived local
artifacts. A de-identified aggregate already published may be non-retractable,
which is disclosed before consent.

Every intake begins in an isolated quarantine. Consent/scope and trace identity
must validate within 72 hours. Unexpected identifiers or out-of-scope content
stay isolated and are redacted with a logged second-person check or deleted
within seven days; quarantine is never analyzed or backed up. Collection beyond
consent, lost custody, or inability to propagate a requested deletion blocks
use and is recorded as an incident.

M11 does not infer disability, protected traits, identity, emotion, mental
state, or biometrics from traces, device data, text, voice, video, or inferred
behavior. Self-described functional settings are used only for the declared
access analysis. #133 owns a future engineered consent/storage/deletion system;
this contract does not pretend the manual controls implement that milestone.

## 6. Statistical and decision contract

### 6.1 Independent unit, endpoint, and estimand

The independent unit is the participant. Contacts, ticks, and matches nested
within a participant are not independent observations.

The primary endpoint is the mean of the three unchanged PXI enjoyment add-on
items after each core condition. The primary estimand is the adjusted
within-participant mean difference `B - A` in all randomized/assigned
participants, regardless of match result, compliance, access configuration, or
whether the participant later supplies both scores. The complete-pair estimator
requires all three enjoyment items in both conditions; partial items and
assigned participants without a pair remain in the analysis manifest,
missingness analyses, harm dispositions, and sensitivity population.

The exhaustive outcome-blind exclusions are:

1. consent was absent or invalid before assignment;
2. a duplicate enrollment was identified before outcomes, in which case the
   first assignment is retained;
3. the wrong build, assignment manifest, or consent scope was detected before
   the first condition began; or
4. a total device/storage failure occurred before any condition stimulus,
   gameplay input, or outcome was observed.

The same event discovered after a condition begins is a retained protocol
deviation, not an exclusion. Poor play, noncompliance, an outlier, a technical
failure during play, withdrawal, inaccessible presentation, adverse burden,
and condition-correlated missingness are never analysis exclusions. A
withdrawal may require payload deletion; its assignment and payload-free
disposition remain in the audit count.

The smallest effect worth acting on (`SESOI`, `D`) is **+0.50 PXI points** on
the -3..+3 response scale. The unacceptable enjoyment degradation is
**-0.33 points**. These are product decisions representing persistent response
movements, not generic standardized-effect labels. Partial-PXI, Affective
Slider, and BANGS outcomes are diagnostic or exploratory in Milestone 11 and
have no confirmatory harm margin. The blinded burden/variance pilot may amend a
margin before freeze only with anchor evidence and a new multidisciplinary
review disposition.

### 6.2 Sample size and power

The planning target is **48 complete paired participants**, recruiting up to
54 to allow approximately 11% incomplete sessions. This number is not frozen
merely because it appears here. Before confirmatory collection, #151 simulates
the exact section 6.3 model and decision algorithm using only blinded-pilot
estimates of residual variance, participant variance, within-pair correlation,
period/order effects, incomplete-item and condition attrition by assigned
label and access stratum, and carryover. Simulation uses the actual response
bounds, allocation, fixed seed/opponent blocks, missingness rule, and interval
method.

For each candidate even complete-pair count, at least 100,000 Monte Carlo
replicates evaluate true mean differences
`{-0.58,-0.33,0,+0.49,+0.50,+0.75}`. The report publishes the probability of
every section 6.4 disposition at every effect, Monte Carlo uncertainty, and
the generating parameters. Every replicate applies section 6.4's exact harm
classifier before assigning its disposition. The selected count must meet all
of:

- `P(proceed) >= 0.80` at `+0.75` (SESOI plus 0.25);
- `P(proceed) <= 0.05` at `+0.50`, `+0.49`, `0`, `-0.33`, and `-0.58`;
- `P(stop for confirmed enjoyment harm) >= 0.80` at `-0.58`; and
- `P(stop for confirmed enjoyment harm) <= 0.05` at the exact `-0.33`
  boundary.

The boundary cases are deliberate: a bound equal to a margin does not pass a
strict inequality. The smallest count that satisfies all rules and the
planned missingness allowance is frozen; 48 is only the starting candidate.
The count may increase before unblinding, but may not decrease. An unmet rule
requires a reviewed increase or makes population-level `proceed` unavailable.

Every stochastic blocking guardrail receives the same treatment at its own
null boundary and at a frozen meaningful buffer beyond that boundary:
`P(false pass at the boundary) <= 0.05` and `P(pass at the favorable buffered
effect) >= 0.80`. The buffer is 25% of the corresponding NI/equivalence margin
unless section 4 gives a stricter value.

Human-family favorable rates are frozen instead of using that generic buffer.
Comprehension simulation generates the exact clustered B vector for every
participant: six accepted rows with the five-component accepted mask and two
rejected rows with the three-component rejected mask. Masked components are
structurally absent, not generated successes. For each row type separately,
boundary rates are 0.70 overall and 0.60 for every applicable component;
favorable rates are 0.85 overall and 0.75 for every applicable component. The
three accepted-terminal priority strata (`hit`, defended, and other terminal)
use boundary 0.55 and favorable 0.80. The rejected-feedback endpoint is not
pooled with those strata.

Because each row-type overall is the arithmetic mean of its component rates,
the simulation never assigns incompatible overall and component marginals. For
a row type with `k` applicable components, its overall-boundary and
overall-favorable configurations set all `k` component rates to 0.70 and 0.85,
respectively. For component `j`, its boundary/favorable configurations set
that rate to 0.60/0.75 and every other component rate to
`(0.85*k-rate_j)/(k-1)`, keeping overall at the favorable 0.85. Thus the common
other-component rates are 0.9125/0.875 for an accepted boundary/favorable
component and 0.975/0.90 for a rejected boundary/favorable component. Accepted
priority-stratum configurations set all five rates to 0.55/0.80.

The comprehension simulation uses blinded-pilot within-participant
correlation, component correlation, row-type-specific `missing_response`,
`unsure`, and accommodated-exposure rates, plus the exact masks, component
denominators, max-T vector, and structural-coverage rules in sections 5.3 and
6.3. It reports false-pass probability `<=0.05` for every boundary
configuration and pass probability `>=0.80` for every corresponding favorable
configuration. `missing_feedback` is generated as rejected-row truth under its
blinded-pilot rate and scored by the frozen no-feedback rule.

Functional access uses the participant-domain binary checklist: boundary
completion is 0.65 and favorable completion is 0.90 in every domain. Its
simulation uses blinded-pilot within-domain correlation, structural
missingness, the exact simultaneous-interval method in 6.3, and required
strata. It likewise must have false-pass probability `<=0.05` with all true
rates at their boundaries and pass probability `>=0.80` with all true rates
favorable.

If blinded pilot data cannot support a required operating characteristic, the
measure is either reclassified as diagnostic before freeze or its
independent-unit count is increased. Exact deterministic invariants and
full-cell schema reconciliation are zero-tolerance checks, not powered claims.

Formative stopping uses theme saturation plus deliberate negative-case search,
not a statistical claim. It cannot satisfy the confirmatory count.

### 6.3 Model, missingness, and multiplicity

The primary linear mixed model is:

```text
enjoyment ~ condition + period + sequence + side + canonical_slot
          + prior_experience_z + device + accessibility_configuration
          + seed_block + opponent_policy
          + (1 | participant)
```

`seed_block` and `opponent_policy` are fixed blocking factors, not sampled
populations. Match result is post-treatment and is absent from the primary
model. Winner/loser estimates and a model adding result plus
`condition:result` are labeled heterogeneity/sensitivity analyses only.
Robust-residual, participant-paired randomization, and ordinal item-level
models accompany but do not replace the primary estimate. Later
human-vs-human work adds dyad/party and preregisters a new model.

A two-period/two-sequence crossover cannot separately identify an unrestricted
carryover effect alongside treatment, period, and sequence. This contract does
not pretend otherwise. It estimates the condition contrast separately in
period one (`delta_1`) and period two (`delta_2`) and reports the main model's
sequence coefficient. Crossover validity requires both the 90% interval for
`delta_2 - delta_1` and the 90% interval for the sequence coefficient to lie
wholly inside `[-0.33,+0.33]`. This is a conservative order/learning/carryover
sensitivity, not a uniquely causal carryover estimate.

If either interval fails equivalence, the two-period estimate is not primary.
The frozen fallback is the period-one parallel-group contrast with the same
fixed blocks. It may decide only if its own prefreeze simulation meets section
6.2 and its 95% interval half-width is `<=0.50`; otherwise the result is
`inconclusive` and the study must be redesigned. Crossover, both
period-specific estimates, their difference, sequence, and period-one fallback
are all reported.

No mean/zero imputation is allowed. The primary complete-pair estimate is
always accompanied by assignment counts and typed missingness by condition,
sequence, period, device, functional configuration, access failure, adverse
event, and withdrawal. A delta/tipping-point sensitivity is mandatory at every
missingness rate: missing condition scores use their blinded-predictive mean
shifted by delta from `-3` through `+3` in 0.25-point steps and truncated to
the response bounds, separately by condition and adverse/access disposition.
It also reports the worst allowed assignment-consistent case (`B=-3`, `A=+3`)
and the smallest delta that changes the decision. Multiple imputation or
inverse weighting may be added only as a labeled sensitivity with explicit
assumptions; neither can erase the delta result. An access-related or adverse
withdrawal independently remains a harm disposition even if its deleted
payload cannot be imputed.

Comprehension simultaneous intervals use a studentized
participant-cluster max-T bootstrap with 50,000 resamples within
sequence×device strata. The random generator is PCG64DXSM seeded from the first
128 bits of SHA-256 over
`combat_fun_evidence_contract/v1/comprehension-max-t/v2`; the frozen analysis
records generator, library, script, and environment hashes. The max-T vector
contains exactly ten base claims: accepted overall plus its five
components, and rejected-feedback overall plus its three components. It also
contains accepted overall for each of the three accepted-terminal priority
strata, and each applicable row-type overall for every frozen device,
experience-tertile, functional-domain, and strict/accommodated-exposure
stratum. Each statistic divides only by its exported applicability-mask
denominator. The one-sided 95% familywise critical value supplies lower pass
bounds; the two-sided 95% max-absolute-T critical value supplies precision
half-widths. A zero denominator, zero standard error, or failed resample is
unresolved, never a pass.

Functional-access simultaneous intervals use conservative
Bonferroni-Wilson score bounds across the five binary domain endpoints:
one-sided 99% lower bounds for passing and two-sided 99% intervals for
half-width. This controls familywise alpha at no more than 0.05 without
assuming independent domains. Structural-missing sensitivity must also pass.

The confirmatory families are exhaustive:

| Family | Members and directional null | Adjustment and interval | Unit, strata, and minimum coverage |
| --- | --- | --- | --- |
| Primary enjoyment | one member, `B-A <= +0.50`; harm boundary `B-A <= -0.33` is reported from the same contrast | unadjusted two-sided 95% interval; strict bounds | randomized participant; frozen powered count; sequence, period, device, experience, functional configuration, and winner/loser sensitivity |
| Soccer integrity | goals, completion, shots, shots/goal, save rate, pass volume, pass completion, turnovers, possession, loose ball, formation mean displacement, formation share above 120 px, progressive possessions, zone entries, chance rate, on-target share, drought, decided-late share, carry time, close/sprint/juke carry shares, heavy-loss rate, carry progression, carry retention, ball-in-play share, and soccer cadence; each null is the harm side of its section 4.4 NI or equivalence margin | Holm-adjusted one-sided 95% harm bounds; equivalence members use both Holm-adjusted TOST tests and familywise 90% intervals; absolute catastrophe floors also must pass | paired common seed; at least 60 pairs for each A/B fixture contrast, every declared C cell present, and at least 4 pairs/cell; aggregate interval width no greater than twice its full allowed margin, otherwise `inconclusive` |
| Machine family balance | each of the four family net utilities in each of the five eligibility contexts; each raw benefit/cost NI check; strict-upgrade cell; unarmed touches, passes, progressive involvements, option episodes, settled-possession contributions, and net utility | Holm-adjusted one-sided 95% bounds within balance; a dominance claim requires every context lower bound `>+0.10` and every cost bound to pass | paired common seed across every family×formation×canonical-slot×side×profile×opponent-formation cell; no missing cell; at least 4 pairs/cell and 60 pairs in each collapsed claim |
| Human causal comprehension | two B-only claims: accepted overall plus source, family, direction, target, and available-response accuracy; rejected-feedback overall plus perceived rejection state, typed rejection reason, and feedback comprehension; each row-type null is overall `<0.70` and component `<0.60` | one participant-cluster max-T family with one-sided 95% simultaneous lower bounds and two-sided 95% max-T precision intervals; applicability masks forbid cross-row pooling | frozen powered participants; exactly six accepted plus two rejected probes each; accepted component denominator 6 and rejected component denominator 2; keyboard and gamepad each `>=16`, each experience tertile `>=12`, each supported functional domain `>=8`; each accepted priority-stratum lower bound `>=0.55` and simultaneous half-width `<=0.35`; A absent |
| Functional access | binary checklist completion for vision, Deaf/Hard-of-Hearing, motor, cognitive, and motion/photosensitivity; null `<0.65` | Bonferroni-Wilson one-sided 99% lower and two-sided 99% precision intervals; any adverse/access failure retained separately | participant-domain; `>=8` per overlapping domain and `>=4` per applicable device; half-width `<=0.35`; no diagnosis strata |

The exact lifecycle, safety, counter-matrix, policy-allowlist, and reconciliation
rules are full-enumeration hard gates. A normal-context runtime/counter window
is a gate only after its pilot-derived floor, planned threat count, quantile
method, and one-sided 95% quantile bound meet section 6.2; until then it is
diagnostic. Human unarmed viability is likewise diagnostic until #151 freezes
an independently powered matched-role sample. Partial-PXI, Affective Slider,
BANGS, custom items, unpowered strata, entropy, pacing, and rematch/loadout
choice are diagnostics/exploration and never silently become blockers.

### 6.4 Interval-based decision quadrants

All primary value and harm claims use two-sided 95% intervals unless a
one-sided safety/NI rule above is stricter. Let `L` and `U` be the primary
enjoyment interval, and let each other gate be `pass`, `confirmed failure`, or
`unresolved`.

Enjoyment harm uses that same interval and exactly three states:

- `confirmed_harm` when `U <= -0.33`;
- `harm_excluded` when `L > -0.33`; and
- `harm_unresolved` when `L <= -0.33 < U`.

Thus equality at the upper harm boundary confirms harm, while equality at the
lower boundary does not exclude it. Section 6.2 applies these predicates in
every simulation replicate.

Evidence integrity is decided first. A failed A/A, missing/duplicate key,
reconciliation error, broken assignment, underpowered plan, absent required
cell, failed carryover fallback, or missingness sensitivity that changes the
disposition is `inconclusive`; collection/inspection stops, the pipeline or
plan is repaired, and fresh untouched evidence is required. It is not a
product `revise`. A reproducible product safety invariant or inaccessible
supported normal configuration is a `confirmed failure`, not an integrity
problem.

For the first valid confirmatory cycle, the quadrants are exhaustive:

| Value interval and confirmatory gates | Decision |
| --- | --- |
| `L > +0.50`, `harm_excluded`, and every hard, soccer, balance, comprehension, and access gate passes | `proceed` |
| `U <= +0.50`, including equality, regardless of otherwise favorable diagnostics | `stop`; the first-cycle interval rules out the required value |
| `confirmed_harm` or any confirmed safety/access/product-rule failure, regardless of value | `stop`; a threshold may not be weakened |
| `L <= +0.50 < U`, `harm_excluded`, all hard gates pass, and a frozen bounded change has a causal hypothesis | `revise` once |
| `U > +0.50` with `harm_unresolved`, all hard gates pass, and the frozen bounded change specifically addresses the plausible harm | `revise` once |
| `U > +0.50` with `harm_unresolved` but no bounded harm hypothesis | `inconclusive`; do not guess away possible harm |
| `L > +0.50`, but a non-hard soccer/balance/comprehension/access gate is unresolved or fails in a bounded remediable way without confirmed harm | `revise` once |
| value or a non-hard gate is unresolved but no bounded causal revision is specified | `inconclusive`; do not improvise a change after seeing outcomes |

Strict inequalities settle equality: `L == +0.50` does not pass; it is
revision-eligible only when `U > +0.50`, while `U == +0.50` stops. Favorable
enjoyment cannot overrule soccer, safety, comprehension, or access harm, and a
telemetry alert cannot be converted into human harm without its declared
evidence.

The one allowed `revise` must name one bounded product/presentation hypothesis,
freeze a versioned contract and new untouched holdout, and keep pilot, first
cycle, and revised estimates separate. On that revised cycle, all gates passing
with `L > +0.50` and `harm_excluded` yields `proceed`; `harm_unresolved` or any
other valid result that does not pass yields `stop`. Only a new
evidence-integrity failure remains `inconclusive` and may be rerun without
changing the product. Contradictory gate results therefore never default to the
favorable outcome or authorize a second revision.

## 7. Blinded A/A instrumentation and reconciliation gate

#148 runs machine A/A before A/B; #151 later runs protocol A/A before
comparative participant collection. In both cases identical builds, rules,
fixtures, inputs/assignments, and seeds receive two opaque labels generated
after artifact creation.

Machine A/A uses seeds `18001..18030`, mirrors label order, and must produce:

- byte-identical **confirmed** canonical event/funnel rows after removing only
  the opaque label, under the complete section 4.0 identity tuple;
- identical source-sequence counts, snapshot boundaries, final hashes, and
  missing-denominator reasons;
- zero corrected-away events in the confirmed export;
- no duplicate/orphan confirmed lifecycle;
- a controlled rollback sub-suite containing at least one predicted encounter,
  one prediction that becomes confirmed, and one prediction that is revoked,
  with equal speculative-created, speculative-confirmed, speculative-revoked,
  and confirmed-survivor counts under both labels; and
- every deterministic condition contrast exactly zero, with intervals centered
  on zero for resampled reporting.

Speculative/revoked rows live in a separate diagnostic stream and are expected
in that sub-suite; a revocation never mutates a prior confirmed row. Runtime
observations are not compared byte-for-byte. Their schema/config hashes,
component counts, monotonic timestamp invariants, cue-state transitions, and
declared repeatability tolerances must instead reconcile under both labels.

Protocol A/A uses a **separate participant pool** and counterbalances identical
condition blocks under opaque labels. Its planning count is 32 complete pairs,
never pooled with the A/B pilot or confirmatory population. Before collection,
#151 independently simulates
`enjoyment ~ opaque_label + period + sequence + (1 | participant)` with its own
blinded A/A variance, correlation, attrition, and section 6.3 delta-missingness
rule. The frozen count must give `P(equivalence pass) >=0.90` when the true
label effect is zero and `P(equivalence pass) <=0.05` at either `-0.50` or
`+0.50`; otherwise it increases. Its 90% interval must lie wholly inside
`[-0.50,+0.50]` under TOST alpha 0.05. Assignment, timing, item order/scoring,
joins, missingness, access/adverse disposition, and observer behavior are
inspected before A/B. Passing A/A does not prove the instrument valid for the
product; it only fails to manufacture a practical label effect.

Failure dispositions are locked:

| Failure | Disposition |
| --- | --- |
| hash/event/source mismatch | block; fix deterministic export/reconciliation and rerun from fresh artifacts |
| non-zero deterministic metric difference | block; locate label leakage or asymmetric pipeline path |
| orphan/duplicate confirmed event or corrected-away row in confirmed export | block; fix event lifecycle or confirmation filtering |
| no predicted/confirmed/revoked rollback coverage, or diagnostic counts fail to reconcile | block; repair the speculative diagnostic path and rerun the controlled sub-suite |
| runtime hash/invariant/repeatability mismatch | block runtime claim; preserve raw observations and investigate clock/presentation path |
| missingness/assignment differs by opaque label | block protocol; inspect randomization, UI, timing, joins |
| A/A interval not equivalent within ±0.50 | block A/B; investigate scoring, period, observer, and carryover |
| only formatting/order differs | fix canonical serialization; do not normalize it away after comparison |

No comparative calibration or holdout inspection begins until all blocking A/A
findings are resolved and the disposition artifact is reviewed.

## 8. Evidence artifacts and reproducibility

`combat_active_signature/v1` contains:

- confirmed event rows keyed by compound `event_id`, funnel rows keyed by
  `funnel_row_id`, and validated commit/terminal/parent/consequence foreign
  keys from section 4.0;
- separate speculative-created/confirmed/revoked diagnostics, never folded
  into the confirmed outcome stream;
- per-player, per-match, fixture, seed, policy, family/role, and aggregate rows;
- all raw numerators, denominators, `NA` reasons, exclusions, and sample counts;
- schema/ruleset/contract, commit/build, content, tuning, config, fixture, seed,
  assignment and bot-policy identities/hashes;
- exact command, runtime/platform, canonical artifact hash, and replay result;
- raw observational runtime rows, their schema/config/content hashes, clock
  anchors, and protocol-repeatability result, explicitly outside canonical
  byte-identity claims;
- separate calibration, adversarial, sensitivity, A/A, and untouched-holdout
  manifests/reports;
- replay/clip references for hard failures and negative counterexamples; and
- amendments, reviewer dispositions, contradictions, and final #114 decision.

Survey instrument/version/timing and pseudonymous research-session identity are
stored outside gameplay identity and joined by an allowed session-to-tape
reference. Raw independent-unit counts are always published beside derived
rows. On withdrawal, participant payloads and joinable derived rows are
deleted, aggregate artifacts are regenerated from the tombstoned manifest, and
only a payload-free audit event remains. Rebuilding the canonical machine
export under the complete section 4.0 identity tuple must be byte-identical;
runtime and participant observations make no such claim.

## 9. Multidisciplinary design-review log

Internal review is a **multidisciplinary design review**, not academic peer
review. Each perspective reviews construct definitions, instrument/reuse
terms, event schema, fixture matrix, analysis/sample plan, accessibility,
ethical boundaries, and decision rules. Allowed final statuses are `approve`,
`approve with changes`, or `block`. A material objection requires an explicit
disposition and evidence link.

Pass 1 reviewed exact head
`08795355f174269cbe2cf705011953efe6f4a1dc` on
`2026-07-25T05:51:51Z`. All nine perspectives returned `block / request
changes` with high confidence. The author response below is implemented in the
current PR revision, but no row is approved: every status remains **initial
block; exact-head re-review pending**. The review record is preserved rather
than overwritten by the response.

| Perspective | Reviewer | Status | Objections / required changes | Disposition / evidence | Contract version |
| --- | --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | block (pass 1); re-review pending | balance non-isomorphic formation slots; add positioning, progression, chance quality, and dribble guards; use opportunity/cost dominance denominators; split machine/human unarmed evidence | author response: 3.2, 4.3, 4.4, 4.6, and 6.3; acceptance pending | v1 response 1 |
| Competitive combat/fighting counterplay | gameplay/AI council | block (pass 1); re-review pending | make attribution, stop precedence, geometry, legal counters, and human-actionable timing executable | author response: 4.0, 4.3, and 4.5; acceptance pending | v1 response 1 |
| Games user research and psychometrics | GUR/stats/accessibility council | block (pass 1); re-review pending | bound PXI crossover use and reliability; freeze unbiased replay probes, carryover, and exploratory BANGS scope | author response: 5.1, 5.3, 6.1, and 6.3; acceptance pending | v1 response 1 |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | block (pass 1); re-review pending | remove post-treatment adjustment; simulate exact decisions; freeze population/exclusions, MNAR, confirmatory families, coverage, and exhaustive quadrants | author response: 6.1–6.4 and 7; acceptance pending | v1 response 1 |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | block (pass 1); re-review pending | collision-free keys/FKs, deterministic attribution, confirmed-versus-revoked streams, closed vocabulary, and correctly scoped byte identity | author response: 4.0, 4.2, 4.3, 7, and 8; acceptance pending | v1 response 1 |
| AI human-proxy and adversarial behavior | gameplay/AI council | block (pass 1); re-review pending | freeze independent context predicates and observable-information digest; separate gameplay/proxy/search policies; constrain privileged evidence | author response: 4.6–4.7 and 6.3; acceptance pending | v1 response 1 |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | block (pass 1); re-review pending | operationalize functional coverage, uncertainty, accessible research UI, structural missingness, and access harm | author response: 5.4, 6.1, 6.3, and 6.4; acceptance pending | v1 response 1 |
| Netcode, performance, and latency | data/netcode/privacy council | block (pass 1); re-review pending | separate pure ticks from observed clocks and prediction lifecycle; freeze runtime/device/network strata and ownership | author response: 2.2, 4.5, 6.3, and 7; acceptance pending | v1 response 1 |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | block (pass 1); re-review pending | reconcile immutable evidence with deletion; strengthen encrypted custody, roles, consent scopes, quarantine, adult participation, compensation, pseudonyms, and non-inference | author response: 3.1, 5.4–5.5, 6.1, and 8; acceptance pending | v1 response 1 |

Pass 2 reviewed exact head
`34dd810451d2e62b53b7830393594b98ce636090` on 2026-07-25. It produced
seven `block` and two `approve with changes` dispositions as recorded below.
These are pass-2 statuses, not final approvals. The author response is awaiting
a pass-3 review at its new exact head.

| Perspective | Reviewer | Pass-2 status | Remaining objection / required change | Author response for pass 3 |
| --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | block | repair retained-space/lane predicates; use family-neutral episodes, capped utility, five contexts, and canonical slots | 4.3 and 4.6; pending |
| Competitive combat/fighting counterplay | gameplay/AI council | block | make vacated space, lane branches, recovery context, and interrupted commitment cost executable | 4.3 and 4.6; pending |
| AI human-proxy and adversarial behavior | gameplay/AI council | block | separate simulation-public gameplay observations from presentation-aware evidence proxy; close zero-context action semantics | 4.6–4.7; pending |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | block | add game instance to funnel identity; pin hash encoding and total canonical row order; remove stale vocabulary | 2.2 and 4.0; pending |
| Netcode, performance, and latency | data/netcode/privacy council | approve with changes | clock/prediction split accepted; preserve runtime/canonical boundary while applying telemetry cleanup | 4.0, 4.5, and 7; pass-3 confirmation pending |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | approve with changes | custody/deletion/consent response accepted; verify later edits do not expand participant scope | 3.1, 5.4–5.5, and 8; pass-3 confirmation pending |
| Games user research and psychometrics | GUR/stats/accessibility council | block | make comprehension B-only and make PXI contrast reliability explicitly diagnostic or executable | 5.1 and 5.3; pending |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | block | freeze harm classifier, favorable operating points, and reproducible simultaneous intervals | 6.2–6.4; pending |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | block | define the participant-domain task, assistance, endpoint, denominator, missingness, and replay accommodations | 5.3–5.4 and 6.2–6.3; pending |

Pass 3 reviewed exact head
`d23b5bfdab2c389bb1bab4a5b878399531755fa8` on 2026-07-25. Gameplay/AI
returned three `block` dispositions for one shared intervention-feasibility
defect; GUR returned `block` for the contradictory probe pool; statistics,
accessibility, and telemetry returned `approve`; and netcode and privacy
returned `approve with changes`. These are pass-3 statuses only. Both author
responses require a pass-4 audit at the new exact head.

| Perspective | Reviewer | Pass-3 status | Remaining objection / accepted disposition | Author response for pass 4 |
| --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | block | remote soccer context can dilute matched opportunity denominators without feasible source intervention | 4.6 `intervention_candidate/v1`; pending |
| Competitive combat/fighting counterplay | gameplay/AI council | block | a far-range commit can reconcile to a target outside feasible threat geometry | 4.6 envelope plus commit-tick geometry reconciliation; pending |
| AI human-proxy and adversarial behavior | gameplay/AI council | block | purpose reasons lack a shared family-neutral feasibility boundary while actual family reach/cadence is unseparated | 4.6 common envelope and separate availability/cost fields; pending |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | approve | pass-2 identity, hashing, ordering, and vocabulary fixes accepted | no new change; pass-4 exact-head audit pending |
| Netcode, performance, and latency | data/netcode/privacy council | approve with changes | clock/prediction boundary remains accepted subject to exact-head regression audit | no new change; pass-4 exact-head audit pending |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | approve with changes | custody/consent boundary remains accepted subject to exact-head scope audit | no new change; pass-4 exact-head audit pending |
| Games user research and psychometrics | GUR/stats/accessibility council | block | B probe pool claimed encounter-only eligibility while requiring rejected requests that cannot be encounters | 5.3 disjoint tagged pools, explicit denominators, and blocked shortfall; pending |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | approve | pass-2 harm, operating-characteristic, and simultaneous-interval fixes accepted | no new change; pass-4 exact-head audit pending |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | approve | pass-2 checklist, assistance, missingness, and replay-accommodation fixes accepted | no new change; pass-4 exact-head audit pending |

Pass 4 reviewed exact head
`3df27a62c1321c4905d589769527fec06ffdd915` on 2026-07-25. Gameplay/AI
returned three `block` dispositions for one shared family-temporal feasibility
defect; GUR and statistics returned `block` for the missing rejected-row
construct and nondiscretionary analysis; accessibility and telemetry returned
`approve`; and netcode and privacy returned `approve with changes`. These are
pass-4 statuses only. The author responses below require a pass-5 audit at the
new exact head.

| Perspective | Reviewer | Pass-4 status | Remaining objection / accepted disposition | Author response for pass 5 |
| --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | block | the common union uses static commit-tick target geometry instead of family-temporal intervention feasibility | 4.6 `family_commit_feasibility/v1` and `intervention_candidate/v2`; pending |
| Competitive combat/fighting counterplay | gameplay/AI council | block | melee, ranged, and self-only guard require different temporal commit relations, distinct from eventual contact and actual availability | 4.6 family-specific temporal rules and shared reconciliation; pending |
| AI human-proxy and adversarial behavior | gameplay/AI council | block | the envelope holds future confirmed non-source inputs that representative AI cannot observe or lawfully predict | 4.6 public no-response projection and 4.7 observation allowlist; pending |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | approve | pass-3 identity, hashing, ordering, and vocabulary dispositions remain accepted | no new change; pass-5 exact-head audit pending |
| Netcode, performance, and latency | data/netcode/privacy council | approve with changes | canonical/runtime and public-prediction boundaries accepted subject to the family-temporal exact-head audit | 4.6 deterministic public projection; pass-5 audit pending |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | approve with changes | custody/consent scope remains accepted; verify the new observation helper reads no hidden or participant state | 4.6–4.7 allowlists; pass-5 audit pending |
| Games user research and psychometrics | GUR/stats/accessibility council | block | rejected rows have no frozen scored construct for perceived rejection, typed reason, or feedback comprehension | 4.5 and 5.3 separate rejected-feedback construct, exact mask, and `missing_feedback`/`unsure` coding; pending |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | block | accepted/rejected aggregation, component denominators, and power do not implement the exact six-plus-two probe vector | 6.2–6.3 separate endpoints, fixed denominators, exact simulation, and ten-claim max-T vector; pending |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | approve | pass-3 accommodations, exposure strata, assistance, and structural-missingness rules remain accepted | no new change; pass-5 exact-head audit pending |

Pass 5 reviewed exact head
`df74d77a98087e4ad588443c75005504fc843b73` on 2026-07-25. Gameplay/AI
returned three `block` dispositions for one shared mechanics-alignment and
lawful-observation defect. GUR, statistics, accessibility, and telemetry
returned `approve`; netcode and privacy returned `approve with changes`.
These are pass-5 statuses only. The gameplay/AI author response and the two
cross-cutting changes require a pass-6 audit at the new exact head.

| Perspective | Reviewer | Pass-5 status | Remaining objection / accepted disposition | Author response for pass 6 |
| --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | block | excluding implemented unarmed from the common union breaks four-family opportunity balance, while missing teammate rows prevents lawful carrier-protection and lane reasoning | 4.6 four-family union/readiness and 4.7 authoritative teammate rows; pending |
| Competitive combat/fighting counterplay | gameplay/AI council | block | unarmed is a real 6/4 press melee family, and post-search movement, ranged release/travel, and guard hold/release horizons are not executable | 4.6 catalog mechanics table and frozen family tapes/horizons; pending |
| AI human-proxy and adversarial behavior | gameplay/AI council | block | the sole lawful observation omits teammates and in-flight projectiles needed for the declared reason/guard predicates | 4.6 no-future-input helper and 4.7 ordered public teammate/projectile rows; pending |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | approve | pass-4 telemetry identity and replay dispositions accepted | 4.7 reuses collision-free ids, canonical encoding, and explicit order for the public observation expansion; pass-6 audit pending |
| Netcode, performance, and latency | data/netcode/privacy council | approve with changes | public projectile horizon and post-commit schedule need exact ordering/digest semantics without crossing the canonical/runtime boundary | 4.6 frozen witnesses and 4.7 row order/digest/allowlist; pass-6 audit pending |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | approve with changes | expanded policy observation must remain public-only and exclude hidden outcome, RNG, future input, and presentation state | 4.7 explicit public fields and exclusions; pass-6 audit pending |
| Games user research and psychometrics | GUR/stats/accessibility council | approve | separate accepted/rejected constructs, masks, coding, and nondiscretionary denominators accepted | no new change; pass-6 exact-head regression audit pending |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | approve | exact six-plus-two simulation, compatible operating points, and ten-claim max-T family accepted | no new change; pass-6 exact-head regression audit pending |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | approve | accommodations, exposure strata, assistance, and structural-missingness rules remain accepted | no new change; pass-6 exact-head regression audit pending |

Pass 6 reviewed exact head
`0aad9b58f97df42609423f3e8ba8fd0aeb5dabcd` on 2026-07-25. Gameplay/AI
returned three `block` dispositions for one shared fixed-guard-horizon defect.
GUR, statistics, accessibility, and telemetry returned `approve`; netcode and
privacy returned `approve with changes`. These are pass-6 statuses only. The
guard-horizon author response and the two cross-cutting changes require a
pass-7 audit at the new exact head.

| Perspective | Reviewer | Pass-6 status | Remaining objection / accepted disposition | Author response for pass 7 |
| --- | --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | block | a fixed 66-tick guard proof can create or remove matched opportunities after the actual public hostile path ends | 4.6 threat-derived finite guard cap; pending |
| Competitive combat/fighting counterplay | gameplay/AI council | block | the fixed cap truncates some latch-resolved ranged paths and extends short melee/projectile paths instead of following their executable remaining threat | 4.6 melee, in-flight, and latch-resolved ranged path rules; pending |
| AI human-proxy and adversarial behavior | gameplay/AI council | block | guard may reason beyond the public finite threat path rather than solely from phase, remaining ticks, telegraph/geometry, source sequence, and projectile horizon | 4.6 public-path proof and 4.7 public release-latch/spawn fields; pending |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | approve | ordered teammate/projectile rows, collision-free identity, digest, and allowlist accepted | 4.7 preserves canonical row identity/digest while exposing only the public latch-resolved spawn fields; pass-7 audit pending |
| Netcode, performance, and latency | data/netcode/privacy council | approve with changes | guard cap must derive from canonical public remaining paths and preserve deterministic schedule/order | 4.6 exact per-threat path lengths and max cap; pass-7 audit pending |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | approve with changes | guard proof must not infer an unlatch-resolved release, future input, hidden outcome, or RNG | 4.6 public-path exclusions and 4.7 public latch fields; pass-7 audit pending |
| Games user research and psychometrics | GUR/stats/accessibility council | approve | pass-5 accepted/rejected constructs and evidence protocol remain accepted | no new change; pass-7 exact-head regression audit pending |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | approve | pass-5 exact simulation, operating points, and max-T family remain accepted | no new change; pass-7 exact-head regression audit pending |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | approve | pass-5 accommodations, exposure, and structural-missingness rules remain accepted | no new change; pass-7 exact-head regression audit pending |

Pass 7 reviewed exact head
`7fc68b680e8b065621eafabca61c70238c1c3dc5` on
`2026-07-25T07:39:39Z`. All prior blocker dispositions were accepted. Seven
perspectives returned `approve`; netcode and privacy returned
`approve with changes` for nonblocking downstream ownership. There is no
unresolved dissent or blocker.

| Perspective | Reviewer | Final status | Accepted disposition / final finding |
| --- | --- | --- | --- |
| Soccer tactics and arcade-sports design | gameplay/AI council | approve | four-family matched opportunities and the threat-derived finite guard cap preserve soccer-purpose denominators |
| Competitive combat/fighting counterplay | gameplay/AI council | approve | unarmed, melee, ranged, and guard schedules, geometry, reconciliation, availability, and terminals are executable |
| AI human-proxy and adversarial behavior | gameplay/AI council | approve | reasons and family feasibility use only the ordered public observation; no future input, hidden state, RNG, outcome, or presentation field enters representative AI |
| Telemetry/data engineering and deterministic replay | data/netcode/privacy council | approve | collision-free identity, ordering, canonical encoding, allowlists, digests, and independent reconciliation are complete |
| Games user research and psychometrics | GUR/stats/accessibility council | approve | B-only accepted/rejected constructs, masks, scoring, accommodations, and evidence boundaries are complete |
| Experimental statistics and reproducibility | GUR/stats/accessibility council | approve | exact simulation, compatible operating points, max-T family, missingness, and decision rules are complete |
| Accessibility, readability, and inclusive participant design | GUR/stats/accessibility council | approve | functional coverage, assistance, accessible replay, exposure strata, and structural-missingness rules are complete |
| Netcode, performance, and latency | data/netcode/privacy council | approve with changes | v1 is approved; the issue refresh must add the nonblocking runtime-cost and participant/device imports below before downstream implementation |
| Privacy, responsible engagement, and player advocacy | data/netcode/privacy council | approve with changes | v1 is approved; the issue refresh must add the nonblocking publication-disclosure controls below before publication |

The two `approve with changes` dispositions do not block #128 or PR #155.
They are owned by the recommended downstream issue refresh before
implementation or publication:

- refresh #150 with cue/device/network/tick-clock response windows and runtime
  cost evidence for observation production/hashing, reachability search, and
  guard projection; import the participant/device strata into #151; and
- refresh #151 with publication disclosure controls: a frozen minimum cell
  size and suppression of rare crossed access strata, review/redaction of
  distinctive replay exemplars, and no participant-level external trace
  packages.

**Final council disposition:** approve
`combat_fun_evidence_contract/v1` at reviewed head
`7fc68b680e8b065621eafabca61c70238c1c3dc5`. This log-only response does not
change the approved contract. Its exact commit hash is posted in the PR
comment, not self-referenced here, and receives a pass-8 log-integrity audit.

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
response owner, response commit, unresolved dissent, and final disposition.
The PR comment records the response's exact head so this document need not
self-reference its own commit. #149 cannot begin comparative calibration while
a row is pending or blocked.

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
[i131]: https://github.com/osobytes/goliseo/issues/131
[i135]: https://github.com/osobytes/goliseo/issues/135
