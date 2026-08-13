# Design: playtest evidence — engineering rules

GOLISEO beta builds can record playtest sessions — inputs and match events — so we
can see how the game actually plays, calibrate opponent profiles against real
sessions, and evaluate builds.

**This file records only the rules that constrain code in this repository.**
Planning, decision records, and evaluation notes for the work are kept outside the
repository. If a rule here looks arbitrary, it is not: each one exists because
breaking it produces a conclusion the data does not support, or puts something in
this repo that should never be here.

Milestone 11 is unaffected. Its #128 evidence contract and #114 playtests are
upstream sources, and no model here becomes a dependency for them.

## 1. Data handling

- Beta builds ship terms of service explaining what is recorded and how it is
  used. Recording does not start unless they have been accepted, and one control
  turns recording off without affecting normal play — off by default in any build
  not deliberately running a session. ([#133][i133])
- **No direct identifiers anywhere in this repository.** No names, emails, account
  handles, IP addresses, hostnames, or raw filesystem paths in an input tape, a
  snapshot, an event row, a committed fixture, a filename, a commit message, or a
  CI artifact. The simulation has no use for a name, so this costs nothing — and
  Git history is permanent, which is the only reason it needs stating as a rule.
  Note `input_tape.copy_identity` currently validates its string fields only for
  non-emptiness; an allowlist is [#133][i133] / [#137][i137] work.
- Recordings live outside this repository. Derived tables and aggregate numbers
  are fine to discuss.
- Delete a tester's sessions on request: delete the session, then regenerate
  anything derived from it. Derived tables are always regenerated from their
  manifest, never hand-patched — which is a reproducibility rule anyway, and it is
  what makes deletion always possible.
- A deleted session still counts in the denominator. Report it as missing rather
  than quietly shrinking the sample.
- Free text and any voice, video, or screen capture are off by default, and stay
  out of training splits — free text is the one input a model could reproduce
  verbatim.
- If the terms are to permit training AI opponents that ship in the game, that
  line has to be present from the first recorded session. It cannot be added
  afterwards.

## 2. What an agent may observe

- A representative agent receives only what a player could perceive at that tick.
  No future ticks, no opponent same-tick input, no hidden RNG, no resolver
  outcomes, no post-action event labels.
- **Every field on a non-self observation must have a rendered analogue** —
  something a human could actually see on screen. `gc_sim::env_observation` pins
  the permitted set with a render citation per field, and a test walks real
  records against it. Adding a field requires a citation, not just a type.
- Privileged full-state observation is allowed only behind an explicit tag, and a
  policy that used it can never be described as a human proxy.
- Agents act only through the canonical input intents humans use. No teleporting,
  no setting ownership or outcome, no skipping cooldown or recovery, no sub-tick
  actions.

## 3. Don't fool yourself

- **No reward may be named `fun`.** `gc_sim::metrics`' geometric `fun_score` is a
  soccer-shape proxy, not a measure of enjoyment. Exports tag it as such, and it is
  never a training objective.
- **Never train an agent or predictor to maximise the metric it is later used to
  validate.** Win/soccer objectives, imitation objectives, and experience labels
  stay separate. Detection has to catch a renamed or derived near-duplicate, not
  just a literal name match.
- Engagement proxies are never a training signal for any population — session
  length, playtime, replay or return rate, action tempo, input rate, streak
  continuation, or any monotone transform or per-tick sum of those.
- Hold out testers *and* hold out builds and tuning configs. **The tester is the
  independent unit:** per-tick and per-frame rows are not independent samples, and
  treating them as such inflates confidence enormously.
- Keep populations distinct in every report: interpretable proxies, learned
  behaviour proxies, self-play competitors, adversarial exploiters, experience
  predictors. An adversarial agent is not evidence of typical human behaviour.
- A model records the split it trained on, so it can always say what data it used.
- Calibrate skill bands against held-out sessions. A slower reaction time is not a
  beginner, and a weak checkpoint is not a novice. Note `gc_sim::bot` has one
  profile and one tunable parameter, and does not yet satisfy §2 — it reads raw
  `MatchState`, including other players' internal timers.
- Don't collapse experience into one score. Predict several outcomes with
  calibration and uncertainty, and keep the underlying evidence.

## 4. Determinism

- The same identity, seed, and action tape must produce identical boundary hashes
  through direct sim, the environment, and replay.
- Speculative or rolled-back events are never observations, and never appear as
  confirmed evidence.
- The environment core stays pure Rust in `gc-sim` (`env.rs`, `env_config.rs`,
  `env_action.rs`, `env_observation.rs`, `env_reward.rs`) — no renderer, no file
  I/O, no networking, no learning framework. Bridge and batching code lives
  outside `gc-sim`. (See `AGENTS.md` §2.)

## Out of scope

Runtime ML in the shipped game, matchmaking, personalisation, retention
optimisation, monetisation, and any production telemetry backend.

[i133]: https://github.com/osobytes/goliseo/issues/133
[i137]: https://github.com/osobytes/goliseo/issues/137
