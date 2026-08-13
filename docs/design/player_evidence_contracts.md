# Design: player evidence data contracts

> **Partly pre-port (LÖVE/Lua).** The contract this document describes is
> current, but it still names the Lua tree that commit `2c0d449` (#467) deleted.
> Read `sim/foo.lua` as `rust/crates/gc-sim/src/foo.rs`, `data/foo.lua` as
> `rust/crates/gc-data/src/foo.rs`, `sim.foo` as `gc_sim::foo`, and `game/**` /
> `spec/**` as `ts/packages/**`. Any `love .` command, `love.*` API, or
> `file.lua:LINE` citation is **pre-port evidence**, not something you can run
> or open. The live tree is described by `ARCHITECTURE.md`.

Versioned data contracts that connect deterministic gameplay to playtest
feedback without mixing participant data into simulation identity.

**Scope.** This is a small internal playtest. The game is set to open as a beta
behind a terms-of-service agreement, and what gets recorded is game-movement
telemetry plus an optional survey. Testers accept the terms in-game while
recording is active. Recorded data is handled entirely outside this repository —
this repo is public, so no recorded payload, issue comment, or CI artifact ever
carries one, and results stay internal.

What these contracts are *for* is validity: if the owner concludes "combat made
this more fun", the data has to actually support that. So the parts that stop a
wrong conclusion — separately named instrument constructs, leakage
classification, participant- and build-held-out splits, the pseudo-replication
guard, and the boundary-hash invariance proof — are strict. The parts that would
be compliance theatre at this scale are deliberately absent.

This document is the data dictionary and compatibility policy for the
`sim/research_*.lua` modules. Study design and analysis decisions live in
[`combat_fun_evidence_contract.md`](combat_fun_evidence_contract.md). Recorder,
storage, UI, transport, and model code are out of scope; this is the schema layer
those tools must satisfy.

## 1. Where the contracts live

| Layer | Contents | Rule |
| --- | --- | --- |
| `sim/research_*.lua` | shapes, strict validators, canonical serialization, content hashing | pure; no `love`, no file I/O |
| `data/research_*.lua` | instrument register, feature-register source data | pure data tables, no functions |
| `game/` or tooling | recorder, intake quarantine, storage, exports, UI | owns every side effect |

| Module | Owns |
| --- | --- |
| `sim/research_schema.lua` | field kinds, strict validation, canonical `lp` serialization, decode, digests, version gates |
| `sim/research_trace.lua` | `gameplay_trace_manifest/v1` and the simulation/annotation field partition |
| `sim/research_timeline.lua` | `research_event_stream/v1`, `research_annotation_set/v1`, boundary mapping |
| `sim/research_session.lua` | `research_session_envelope/v1`, `research_withdrawal_tombstone/v1` |
| `sim/research_response.lua` | `research_response_set/v1`, instrument register validation, construct scoring |
| `sim/research_features.lua` | `research_feature/v1` register expansion and invariants |
| `sim/research_dataset.lua` | `research_dataset_manifest/v1`, split and lineage rules |
| `data/research_instruments.lua` | instrument structure and provenance (never item text) |
| `data/research_features.lua` | per-instrument and per-construct feature metadata, behavioral features |

## 2. Identity chain

Each arrow is a content-addressed reference. Nothing downstream can alter
anything upstream.

```text
InputTape (sim/input_tape.lua)          authoritative gameplay spine
  │  tape_content_hash            = run scope id
  ├─────────────► research_event_stream/v1     (confirmed events only)
  │                     │ stream_hash
  └─────────────► gameplay_trace_manifest/v1   (references both hashes)
                        │ trace_id = H(simulation identity, game_instance_id)
                        ├──────── research_session_envelope/v1 ── trace_links
                        │                     ├── research_response_set/v1
                        │                     └── research_withdrawal_tombstone/v1
                        └──────── research_annotation_set/v1 (many per run scope)
                                              │
research_feature/v1 register ─────────────────┴──► research_dataset_manifest/v1
```

Ordering matters. The event stream is scoped by **tape content**, which is
knowable before any recorder diagnostics exist, so the trace manifest can
reference the stream hash without a circular dependency.

`trace_id` is derived, not assigned: it is
`H("gameplay-trace/v1", simulation_identity_hash, game_instance_id)`. A
hand-edited simulation field therefore invalidates the id and the manifest fails
validation. `game_instance_id` distinguishes match restarts inside one run.

## 3. Canonical serialization and hashing

`sim/research_schema.lua` implements the length-prefixed discipline from section
4.0 of the evidence contract. For every value, `lp(bytes)` emits the ASCII
decimal byte length, `:`, the bytes, then `;`.

| Kind | Wire form |
| --- | --- |
| absent optional | `n;` |
| boolean | `b1;` / `b0;` |
| integer | `i` + lp(canonical decimal) |
| number | `d` + lp(`match_snapshot.number_bytes`) — exact frexp form, no NaN/inf |
| string / id / text / hash | `s` + lp(bytes) |
| enum | `e` + lp(member) |
| array | `a` + lp(count) + each element in declared order |
| map | `m` + lp(count) + sorted `k` + lp(key) + value |
| record | `r` + lp(field count) + `k` + lp(name) + value, in declared field order |

Every payload is prefixed `GCRS<serialization_version>;` + lp(shape name), so
two contracts can never collide and a foreign payload is rejected rather than
misread. Field names are on the wire, so reordering a record is detectable.

- Digest: `fnv1a64/v1`, 16 lowercase hex characters, from `core/fnv1a64.lua`.
  The evidence contract specifies SHA-256 for machine exports; `sim/` has no
  SHA-256 and must stay dependency-free and love.js-safe, so the digest is a
  **named field** (`digest`) rather than an assumption. A tooling-layer
  `sha256/v1` can be added later as a new digest name without reinterpreting
  stored hashes.
- Maps hash independently of Lua table insertion order; records hash in declared
  field order. Delimiter-joined or table-order hashing is forbidden.
- `research_schema.tuple_hash(label, parts)` covers compound ids where there is
  no record shape. Parts are type-tagged, so `1` and `"1"` cannot collide.
- Byte identity is claimed only for the same shape at the same serialization
  version. Frozen vectors live in `spec/fixtures/research/canonical_vectors.lua`.

### Strict reader rules

Every public reader returns `nil, err_string` (§7 of `AGENTS.md`) and fails
closed on: unknown fields, missing required fields, non-members of a closed
enum, non-finite or out-of-range numbers, sparse or non-array "arrays",
malformed digests, control characters in machine strings, non-canonical integers,
truncated or trailing wire bytes, unsorted map keys, orphan joins, duplicate
canonical keys, and any payload whose declared `schema_version` this reader does
not support. There is no "ignore what you do not know" path.

Authored content (the instrument register, the feature register) asserts instead:
a broken register is a programmer error, not external input.

## 4. Data dictionary

Types below are the `ResearchFieldKind` values from `sim/research_schema.lua`.
`id` is a bounded lowercase slug (`[a-z0-9][a-z0-9_\-.]*`) used as a join key.
`/` and `:` are **not** in the charset, so a pasted filesystem path or URL cannot
satisfy the grammar at all — `home/oscar/matches/save1.json` and
`c:/users/oscar/save.json` are rejected on the separator, not on a substring
blacklist they could slip past. `@` and `..` are additionally forbidden as
substrings. `text` is bounded free text and is never a join key.

### 4.1 `gameplay_trace_manifest/v1`

Envelope: `schema_version`, `manifest_kind`, `digest`, `trace_id` (hash,
derived), `game_instance_id` (id).

`simulation` — the only group inside `simulation_identity_hash`:

| Field | Type | Meaning |
| --- | --- | --- |
| `tape_version`, `input_version`, `snapshot_version` | integer | versions copied from `InputTapeIdentity` |
| `ruleset_version`, `event_schema_version` | integer | recorder-declared rule and event-schema versions |
| `combat_identity` | string? | required for a combat tape, forbidden for a soccer tape |
| `build`, `source`, `content`, `tuning`, `config`, `fixture` | string | build/commit and content/tuning/config/fixture identity (`tuning` may be empty: no override) |
| `seed` | integer | simulation seed |
| `tick_rate` | integer | must equal `fixed_clock.TICK_RATE` |
| `first_boundary_tick`, `last_boundary_tick`, `frame_count` | integer | tick range; `last = first + frame_count` |
| `tape_content_hash` | hash | identity + canonical initial snapshot + every canonical `InputFrame` wire + every boundary hash |
| `initial_boundary_hash`, `final_boundary_hash` | hash | copied from `tape.boundary_hashes` |
| `confirmed_event_stream_hash` | hash | `stream_hash` of the confirmed event stream |
| `completion` | enum | `completed`, `incomplete_interrupted`, `incomplete_abandoned`, `incomplete_process_exit` |
| `producers` | array(8) | per canonical slot: `slot`, `team`, `player_id`, `producer_kind` (`human`/`bot`/`replay`), `producer_policy_id` (required for machines, forbidden for humans) |
| `divergence` | record? | replay divergence artifact: boundary tick, expected/actual hash, state path, causal input tick |

`runtime` — observational, protocol-repeatable, never authoritative: `platform`,
`renderer`, `render_hz`, `render_hz_mode`, `input_device`, `mean_frame_ms`,
`p99_frame_ms`, `dropped_frame_count`, `pause_count`, `goal_replay_count`,
`rollback_count`, `max_rollback_ticks`, `raw_device_event_policy`
(`not_collected` / `minimized_diagnostic` / `full_diagnostic`),
`raw_device_event_clock` (`none` / `wall_clock_monotonic`).

`research_links` — append-only `{ link_kind, target_id, target_hash }` rows
(`research_session`, `annotation_set`, `response_set`, `derived_dataset`).
Duplicates fail closed.

### 4.2 `research_event_stream/v1`

`run_scope_id` (= `tape_content_hash`), `game_instance_id`,
`confirmed_through_tick`, `confirmed_boundary` (= tick + 1), `rows`,
`stream_hash`.

Row: `canonical_tick`, `domain` (`input`/`soccer`/`combat`/`lifecycle`),
`domain_rank` (1/2/3/4, part of the sort key), `event_kind`, `source_sequence`
(0 when the source has none), `same_kind_ordinal`, `event_id` (tuple hash of run
scope, instance, tick, domain, kind, sequence, ordinal), `rollback_event_id` (the
`sim/rollback_events.lua` id, for tracing back to the sim layer), `payload_hash`.

Canonical order: tick, domain rank, kind, source sequence, ordinal, event id.
Duplicate total keys fail closed. `stream_hash` covers the ordered rows plus the
run scope and confirmed cursor.

### 4.3 `research_annotation_set/v1`

`annotation_set_id`, `run_scope_id`, `session_id`, `author_role`
(`participant`/`researcher`/`tool`), `agreement_version`,
`coding_scheme_version`, `annotations`.

Annotation: `annotation_id`, `canonical_tick`, `boundary_source`
(`canonical_tick`/`wall_clock_mapped`/`render_frame_mapped`),
`mapping_error_ms` (signed; must be 0 for a canonical cue), `wall_clock_ms`
(required for a mapped cue), `event_id?` (must join to a confirmed row),
`code_id`, `confidence?`, `disagreement_group?`, `free_text?` (only from a
`participant` or `researcher` author, never a `tool`).

`research_timeline.join_annotations` allows **many sets per run** and rejects
sets from another run, duplicate ids, cues past the confirmed boundary, and
orphan event joins. Disagreement between coders is expressed by sharing a
`disagreement_group`, never by overwriting a code.

### 4.4 `research_session_envelope/v1`

Ids: `session_id`, `participant_id` (pseudonymous, at least 16 bytes),
`study_id`, `protocol_version`, `cohort_id`, `recruitment_channel`, `block_id`,
`trial_id`, `condition_id`, `condition_order`, `sequence_label`,
`counterbalancing_cell`. `session_id` must differ from `participant_id`.

`assignment`: `build_id`, `tuning_config_id`, `seed`, `side`, `role_id`,
`loadout_id?`, `opponent_population`, `opponent_policy_id`, `practice_block`.

`environment`: `device_class`, `control_device`, `control_mapping_id`,
`viewport_w/h`, `render_hz`, `input_latency_ms`, `language_tag`,
`readability_settings?` (a map, recorded because it changes what the participant
could see, which is a cue-readability validity concern).

`lifecycle`: `status` (`in_progress`, `completed`, `interrupted`, `abandoned`,
`withdrawn`, `excluded`), `started_wall_clock_ms`, `ended_wall_clock_ms?`
(required for every terminal status), `observer_mode`, `assistance`,
`interruptions[]`, `exclusions[]`, `missingness[]`. A `completed` session may not
carry an `operator_stop` or `process_exit` interruption; an `interrupted` session
must record at least one interruption; an `excluded` session must record at least
one exclusion. Missingness rows are `{ target_id, target_kind, reason_code }`
with a closed reason set.

`experience`: continuous and ordinal source measures
(`play_hours_per_week`, `years_playing`, three 1–5 ordinals) plus an optional
`derived_label` that must name `calibration_id` and `calibration_version`. There
is no field for a hand-entered "beginner/intermediate/experienced" fact.

`agreement`: `{ agreement_version, accepted, accepted_wall_clock_ms,
model_use_covered }`. One recorded agreement, not a permission matrix.
`accepted` must be true and `accepted_wall_clock_ms` must not be later than
`lifecycle.started_wall_clock_ms`, so the envelope proves the participant agreed
*before* recording started. Agreement prose and direct identifiers are
structurally absent — only the version id is stored.

`model_use_covered` records whether the accepted agreement version covered
training and shipping a model. It is a property of what the participant accepted,
so it can never be granted retroactively; `research_session.allows_model_use`
reads it and nothing else.

`trace_links[]`: `{ trace_id, game_instance_id, role }` where role is
`condition_block`, `practice`, or `replay_probe`. A `practice` link requires
`assignment.practice_block`.

### 4.5 `research_withdrawal_tombstone/v1`

`tombstone_id`, `session_id`, `participant_id`, `request_wall_clock_ms`,
`revoked_payload_hashes[]` (must be non-empty), `rebuild_required` (must be
`true`).

There is exactly **one** withdrawal path: the session is deleted and anything
derived from it is regenerated. Partial scopes (`media_only`, `free_text_only`,
`responses_only`) were removed because nothing enforced them, and an unenforced
promise is worse than an honest smaller one.

Everything that carries a `session_id` has a withdrawal path:
`research_response.validate_against_session`,
`research_timeline.validate_against_session`, and
`research_dataset.validate_against_sessions` all reject a payload whose envelope
is `withdrawn`, and `research_dataset.validate_against_tombstones` rejects a
dataset that still names a withdrawn participant, session, or payload hash.

A `withdrawn` envelope must name a `tombstone_id`, must record
`participant_withdrew` missingness, and must retain **no** trace links. Derived
datasets are rebuilt from the tombstoned manifest, never hand-patched — that is
why `rebuild_required` cannot be false. `research_dataset.validate_against_tombstones`
revokes by participant id, session id, *and* payload hash, so a dataset cannot
retain a withdrawn session even if its pinned hashes are stale.

### 4.6 `research_response_set/v1`

`response_set_id`, `session_id`, `participant_id`, `condition_id`,
`instrument_id`, `instrument_version`, `scoring_key_version`, `analysis_role`,
`validated_instrument`, `partial_administration`, `locale`
(`language_tag`, `translation_provenance`, `translation_id?`), `timing`
(`relative_to_play`, `offset_ms`, `canonical_boundary_tick?`, `trace_id?`),
`presentation_order[]`, `randomized_presentation`, `responses[]`, `scores[]`.

Response: `item_id`, `raw_response?`, `missing_reason?`, `response_ms?`. Exactly
one of raw response or missingness reason, one row per administered item, and the
raw value must sit on the registered scale (range and step).

`instrument_version`, `scoring_key_version`, `analysis_role`,
`validated_instrument`, and `partial_administration` must match
`data/research_instruments.lua` exactly. A drifted scoring key is a stop, not a
reinterpretation.

Scores are recomputed and compared: a construct is scored only when every item
of that construct is answered, an `item_only` instrument may carry **no**
construct score at all, and a hand-edited score fails closed. This is how PXI
enjoyment, partial-PXI mechanisms, BANGS satisfaction/frustration, Affective
Slider valence/arousal, and the custom exploratory items stay separate.

The register records structure and provenance only: `item_text_included` must be
`false` for every instrument, and license/reuse terms are named per instrument.
No instrument wording or asset enters this repository.

### 4.7 `research_feature/v1`

Per feature: `id`, `version`, `description`, `grain`, `source_schemas[]`,
`source_fields[]`, `extraction_module`, `extraction_config_id`, `numerator`,
`denominator?`, `unit`, `exclusions[]`, `missing_value_behavior`,
`causal_window` (`{ kind, ticks? }`), `normalization`, `leakage_risk`,
`observability`, `evidence_tier`, `outcome_role`, `aggregation_levels[]`,
`pseudo_replication_guard`, `confounds[]`, `goodhart_failure`,
`prohibited_uses[]`, `human_fun_claim`.

`extraction_commit` is deliberately **not** a register field. The register
declares which module and config own an extraction; the commit that actually ran
it is recorded per dataset in `research_dataset_manifest.extraction`, where it is
a fact rather than a promise.

Register invariants (asserted at load):

- `human_fun_claim` implies `evidence_tier = human_experience` **and**
  `outcome_role = primary_outcome`. Exactly one feature qualifies:
  `pxi_enjoyment_addon.enjoyment`.
- a `primary_outcome` requires human-experience evidence and `leakage_risk =
  none`;
- a `soccer_shape_proxy` must list `human_fun_claim` in `prohibited_uses` and can
  never claim human fun;
- a windowed causal window must state its ticks, and a non-windowed one may not;
- `aggregation_levels` must include the feature's own grain;
- a `tick`, `decision`, `possession`, `encounter`, `match`, or `player_match`
  grain may not declare `independent_unit_participant`;
- every instrument construct expands to exactly one feature, and every
  instrument in the register has feature defaults.

`metrics.fun_score` is registered as **`soccer_shape_proxy_score`**: a geometric
mean of banded `MatchMetrics` desirabilities, evidence tier
`soccer_shape_proxy`, `human_fun_claim = false`, and `human_fun_claim`,
`primary_outcome`, `proceed_gate`, and `participant_facing_report` all
prohibited. It is a soccer-shape proxy over simulated statistics and is never
human fun.

### 4.8 `research_dataset_manifest/v1`

`dataset_id`, `dataset_version`, `parent_dataset_hash?`,
`created_wall_clock_ms`, `purpose` (`analysis`/`model_training`/`qa`),
`usage` (`agreement_versions[]`, `model_use_covered`),
`extraction` (`extraction_commit`, `extraction_config_id`,
`feature_registry_hash`), `feature_versions[]`
(`feature_id`, `version`, `aggregation_level`), `sources[]`,
`transformations[]`, `splits[]`, `dataset_hash`.

Source row: `trace_id`, `trace_manifest_hash`, `session_id`, `participant_id`,
`condition_id`, `build_id`, `tuning_config_id`, `excluded`, `exclusion_reason?`,
`agreement_version`, `model_use_covered`.

Split: `split_id`, `grouping` (`participant`, `build`, `participant_and_build`),
`folds[]` of `{ fold_id, role, participant_ids[], build_ids[] }`.

Split rules — all fail closed:

- participants may not overlap across folds when the grouping includes
  participants; builds may not overlap when it includes builds;
- coverage is checked in both directions: every included participant (or build)
  must appear in exactly one fold, and every fold member must have a source row;
- a participant-only split may not declare builds, and vice versa;
- every split needs a `train` fold and a `test` or `holdout` fold;
- a fully excluded participant may never appear in a fold;
- `model_training` requires at least one split.

Lineage rules: `feature_registry_hash` must equal the live
`research_features.registry_hash()`; each pinned feature version must match the
register and be defined at the requested aggregation level; a manifest with
transformations must name its parent dataset hash; a transformation whose input
and output hashes are equal is rejected; every source row's `agreement_version`
must be declared in `usage.agreement_versions`; `usage.model_use_covered` is
accepted only when **every** included source row carries it; a `model_training`
dataset requires both a split manifest and agreement-covered model use; and
`dataset_hash` covers everything else, so any hand edit is detected.

Those rules are *internal* consistency only: a source row self-declares
`agreement_version` and `model_use_covered`.
`research_dataset.validate_against_sessions(manifest, envelopes)` is the check
against the actual source of truth. For every source row it resolves the session
envelope by `session_id` (an unresolvable row is an orphan join) and requires the
row's participant, condition, build, tuning configuration, and pinned trace to
match that envelope, the row's `agreement_version` to equal the version the
participant actually accepted, and the row's `model_use_covered` to equal
`research_session.allows_model_use(envelope)`. A withdrawn session fails outright.

So "model-use permission cannot be granted at the dataset level" holds per row,
not merely in aggregate: a dataset that claims coverage for a participant whose
accepted agreement did not grant it fails this join. Run it before treating a
`model_training` dataset as usable — the manifest alone cannot prove the claim,
because self-declaration never can.

`research_dataset.validate_against_tombstones` rejects a dataset that still
retains a withdrawn participant, session, or revoked payload hash.

`trace_manifest_hash` in a source row is a point-in-time snapshot: a dataset
extracted before further research links were appended pins a different manifest
hash than one extracted after. Because withdrawal is always full and also revokes
by `participant_id` and `session_id`, that staleness cannot let a withdrawn
session survive; it only means hash-only matching is not sufficient on its own.

## 5. Grains: raw, canonical, and derived

Three different grains are routinely confused. They are kept distinct:

| Grain | Authority | Collected? | Clock |
| --- | --- | --- | --- |
| canonical `InputFrame` (per fixed tick) | authoritative; drives simulation | always, inside the tape | fixed tick |
| raw device events (key/pad transitions) | diagnostic only | optional, per `raw_device_event_policy` | wall-clock monotonic |
| render frames (30/60/120+ Hz) | never authoritative | only as aggregate runtime stats | wall clock |

Raw device events are a minimization decision, not a default: the policy and the
clock are recorded in the manifest so an export can state which grain it holds.
They are never a substitute for canonical frames and never a join key.

Wall-clock and render-time cues (participant annotations, debrief markers) are
mapped to the **nearest canonical boundary** by
`research_timeline.map_to_boundary`, which subtracts non-simulated wall time
(pause, goal replay, menus), records the signed `mapping_error_ms`, and fails
closed for a cue outside the trace window. Nothing is silently snapped.

Pseudo-replication rule: tick, decision, possession, encounter, match, and
player-match rows are **not** independent participants. The register enforces a
clustering guard at those grains, dataset splits group by participant (and by
build), and the independent unit for confirmatory human evidence is the
participant.

## 6. Confirmed versus speculative events

Rollback means "what happened" is a function of confirmation, not of what a
screen showed. `sim/rollback_events.lua` owns the speculative window and reports
`added` / `replaced` / `revoked`. `sim/research_timeline.lua` is the research
projection of that:

1. `observe_diff` records every correction. A revoked id is remembered; revoking
   or replacing an **already-confirmed** id is a contract violation, not a
   correction.
2. `confirm` accepts only contiguous confirmed steps from
   `rollback_events.confirm`, and refuses a step that still carries a revoked or
   already-confirmed event.
3. `export` refuses to emit a row for any revoked or still-speculative event, so
   a predicted-then-corrected event can never appear as participant evidence.

Time and tick semantics therefore survive 30/60/120+ Hz rendering, pause, goal
replay, rollback and resimulation, and restart (a new `game_instance_id`). An
abrupt process exit yields `incomplete_process_exit` with whatever boundary was
flushed; the manifest states the tick range it actually covers.

## 7. Proof obligations and where they are tested

| Obligation | Test |
| --- | --- |
| Participant/research metadata cannot change a simulation boundary hash | `spec/sim/research_trace_spec.lua` — "research metadata cannot move a simulation boundary hash" |
| `InputTapeIdentity` rejects participant fields | same spec, via `input_tape.copy_identity` |
| One tape joins many research annotations without altering the tape | `research_trace_spec.lua` (links) and `research_package_spec.lua` (two annotation sets) |
| Corrected-away events are absent from exports | `spec/sim/research_timeline_spec.lua`, `research_package_spec.lua` |
| Every cross-contract join rejects an orphan reference | `research_trace.validate_against_stream`, `research_timeline.join_annotations`, `research_session.validate_against_traces`, `research_response.validate_against_trace` / `validate_against_session`, `research_dataset.validate_against_tombstones` — see `research_package_spec.lua` "rejects a session or response set that names a trace nobody holds" |
| A session that says an instrument was not administered cannot also carry its response set | `research_package_spec.lua` — "refuses a response set for an instrument the session says was skipped" |
| Free-text annotations have the same withdrawal path as survey answers | `research_timeline.validate_against_session`; `research_timeline_spec.lua` — "gives free-text annotations the same withdrawal path as survey answers" |
| Each dataset source row's agreement and model-use claim matches its real session envelope | `research_dataset.validate_against_sessions`; `research_dataset_spec.lua` — "verifies every source row against the session envelope it came from" |
| Model use is only claimable when the accepted agreement covered it, per row | `research_session_spec.lua`, `research_dataset_spec.lua` |
| Ids reject pasted filesystem paths and URLs | `research_schema_spec.lua` — "rejects direct identifiers and raw paths in join keys" |
| Every `observability` category has a worked feature example | `research_features_spec.lua` |
| Canonical serialization, round-trip, malformed, unknown-field, cross-version | `research_schema_spec.lua`, `research_canonical_spec.lua`, plus per-contract specs |
| Frozen hash/byte vectors | `spec/fixtures/research/canonical_vectors.lua` + `research_canonical_spec.lua` |
| Participant-grouped and build-grouped splits fail closed on overlap | `spec/sim/research_dataset_spec.lua` |
| `metrics.fun_score` is tagged as a soccer-shape proxy, never human fun | `spec/sim/research_features_spec.lua` |
| Constructs are never silently combined | `spec/sim/research_response_spec.lua` |
| Example package: completed, interrupted, missing items, withdrawal tombstone | `spec/sim/research_package_spec.lua` |

## 8. Compatibility policy

### Version fields

Three independent version axes:

1. `research_schema.SERIALIZATION_VERSION` — the wire grammar. Changing it
   changes every preimage and therefore every hash.
2. per-contract `schema_version` (each module's `VERSION` /
   `SUPPORTED_VERSIONS`) — the field set and semantics of one contract.
3. content versions carried *inside* payloads: instrument version, scoring key
   version, feature version, calibration version, ruleset/event-schema version,
   agreement version, dataset version.

**These rules bind from the first merged version.** Until a contract has merged
and something outside its own pull request consumes it, renaming or removing a
field is pre-release iteration, not a breaking change, and `VERSION` legitimately
stays at 1 while the shape settles. After the first merge that stops being true:
from then on, "`VERSION` unchanged" means "nothing breaking happened", and every
change below needs its bump. Do not read an unchanged `VERSION` in the history of
an unmerged contract as evidence that its shape never moved.

### Bump rules

| Change | Action |
| --- | --- |
| add an optional field with a safe absent meaning | bump the contract `VERSION`, keep the previous version in `SUPPORTED_VERSIONS`, and state the absent-value semantics here |
| add a required field, remove a field, rename a field, change a field's type or units | bump the contract `VERSION` and register an explicit migration; the old version stays supported only while a migration exists |
| add an enum member | bump the contract `VERSION`: old readers must reject the new member rather than guess |
| change canonical ordering, a domain rank, a digest, or the wire grammar | bump `SERIALIZATION_VERSION` and re-freeze the canonical vectors |
| change how a value is computed (scoring key, feature formula, calibration) | bump the *content* version (scoring key, feature, calibration) and keep the old id retrievable |
| fix a validator without changing accepted payloads | no bump; extend the spec |

Two rules are absolute: **never widen a validator to accept an existing invalid
payload**, and **never reinterpret an old field under a new meaning**. Add a
migration or stop with a diagnostic.

### Reader behaviour

A reader accepts only versions in its `SUPPORTED_VERSIONS`. Anything else stops
with a diagnostic that names the version and the missing migration
(`research_schema.accepts_version`). Unknown fields are rejected even when they
look harmless, because an unknown field means the writer knew something this
reader does not.

### Migrations

A migration is a pure function from version N to N+1 plus a spec fixture at both
versions. There are no migrations yet: every contract is at version 1, so the
only cross-version behaviour to test is "stop with a diagnostic", which the specs
cover. When the first migration lands, register it beside its contract and list
it here with its from/to versions and its field-by-field rules.

### Deprecation

1. Announce here with the replacement and the reason.
2. Keep the field or version accepted for at least one release while both are
   populated; readers must tolerate both, writers must write both.
3. Stop writing the old form; keep reading it.
4. Remove it in a contract `VERSION` bump, with a migration if any stored payload
   still uses it. Nothing is removed silently, and no id is ever reused for a new
   meaning.

Immutability rules that survive every bump: raw responses and gameplay artifacts
are never edited in place — corrections are new derived datasets with lineage —
and withdrawal is a tombstone plus a rebuild, never a hand patch.

### Hashes are integrity checks, not an audit trail

`dataset_hash`, `trace_manifest_hash`, `stream_hash`, and the content hashes
generally detect **accidental** corruption, drift, and hand-edits: change a field
and validation fails. They are not tamper-evident. Anyone who can rewrite a
manifest can also recompute its hash, and `fnv1a64/v1` is a 64-bit non-cryptographic
digest chosen for portability inside `sim/` (see §3). Do not read a matching hash
as proof that nobody altered a payload on purpose.

## 9. What these contracts keep out of the repository

This repo is public and participant data lives entirely outside it. The schemas
are shaped so that a payload which accidentally reached the repo would still
carry nothing sensitive, and so that an operator cannot paste something personal
into a field by mistake:

- **No direct identifier field exists anywhere**: no name, email, handle, IP,
  hostname, raw path, or agreement prose. The `id` charset excludes `/` and `:`
  and forbids `@` and `..`, so a pasted path or address is rejected outright
  (§4).
- **Participant and session ids are join keys, not secrets.** What is actually
  enforced is: the `id` charset, a 16-byte minimum length, uniqueness inside a
  payload, and `session_id ~= participant_id`. This validator does **not** measure
  entropy or guessability — generating unguessable ids belongs to the recorder.
  Treat the minimum length as a floor against obviously-derived ids, nothing more.
- **One agreement, recorded by version.** The envelope stores which agreement
  version was accepted and when, never its text, and proves acceptance preceded
  recording.
- **Free text is bounded, author-attributed, and never a join key.**
- **No inference about people.** Readability settings are recorded because they
  change what was visible on screen. The feature register contains a
  `protected_sensitive` example and an explicitly `prohibited` example
  (`inferred_participant_skill_trait`) precisely so that any pipeline reaching for
  a trait score fails the use gate instead of quietly computing one.
- **Withdrawal needs no retained payload**: the tombstone names hashes, ids, and
  a rebuild requirement, and the withdrawn envelope keeps only structural fields.

**Known residual, accepted at this scale.** `environment.readability_settings` is
an open `map<id, string>`: the keys must be slugs, but the *values* are only
length-bounded, so it is the one field where an operator could type something
identifying and meet no structural resistance. This is deliberate — the setting
names and values a future build reports are not knowable here, and inventing a
closed value list would either block real settings or be quietly widened later.
At this scale the blast radius is one map in one local file, so the
recorder is responsible for what it writes into it. If this ever carries
operator-typed text rather than machine-emitted setting values, close it with a
closed key/value vocabulary rather than a validator that guesses.

Retention is simple by design: keep what the current question needs, delete on
request. There is no retention timer, cell-size floor, suppression rule, or
rotating publication id in these contracts, because there is no publication.

## 10. Not covered here

Deliberately out of scope for this layer, and tracked separately:

- recorder, local storage, deletion tooling, and the in-game agreement screen
  (this document only defines what they must produce);
- JSON Lines export layout, SHA-256 digests, and the `run_id` tuple form used by
  machine exports in the evidence contract;
- learning-environment observation and action contracts (issue #138);
- the playtest lab record owned by issue #131;
- statistical models, margins, multiplicity, and decision gates
  ([`combat_fun_evidence_contract.md`](combat_fun_evidence_contract.md)).
