//! Port of `spec/sim/outfield_ai_baseline_spec.lua`.

use gc_data::outfield_ai_baseline as frozen;
use gc_sim::outfield_ai_baseline as sut;
use gc_sim::outfield_ai_policy;
use gc_sim::tripwire;

/// The frozen record is shared module state; never hand a mutable copy of
/// it to a comparison test. `From` already gives us an owned copy, so this
/// is just a readable alias for that conversion at each call site.
fn frozen_record() -> sut::OutfieldAiBaselineRecord {
    sut::OutfieldAiBaselineRecord::from(&frozen::RECORD)
}

#[test]
fn outfield_ai_baseline_declares_an_explicit_contiguous_seed_set() {
    let seeds = sut::seeds();
    assert_eq!(seeds.len() as i64, sut::SEED_COUNT);
    assert_eq!(seeds[0], sut::SEED_FIRST);
    assert_eq!(
        *seeds.last().unwrap(),
        sut::SEED_FIRST + sut::SEED_COUNT - 1
    );
    for i in 1..seeds.len() {
        assert_eq!(
            seeds[i],
            seeds[i - 1] + 1,
            "the declared block is contiguous"
        );
    }
}

#[test]
fn outfield_ai_baseline_keeps_its_seeds_clear_of_every_other_locked_block() {
    // Locked in docs/design/combat_fun_evidence_contract.md §3.3. The
    // holdout blocks matter most: a control that quietly overlapped
    // 30001..30060 would spend the untouched confirmatory set.
    let reserved: &[(&str, i64, i64)] = &[
        ("adversarial", 21001, 21060),
        ("untouched holdout", 30001, 30060),
        ("replacement holdout", 31001, 31060),
    ];
    for seed in sut::seeds() {
        assert!(
            seed > tripwire::DEFAULT_N,
            "seed {seed} collides with the tripwire"
        );
        assert!(
            !(1001..=1060).contains(&seed),
            "seed {seed} is a spent evaluation seed"
        );
        for &(name, first, last) in reserved {
            assert!(
                seed < first || seed > last,
                "seed {seed} collides with the {name} block"
            );
        }
    }
}

#[test]
fn outfield_ai_baseline_freezes_an_identity_that_still_describes_this_build() {
    let live = sut::identity(None);
    let frozen_id = frozen_record().identity;
    for &field in sut::IDENTITY_FIELDS {
        assert_eq!(
            identity_field_string(&frozen_id, field),
            identity_field_string(&live, field),
            "frozen identity field {field} no longer matches this build"
        );
    }
    assert_eq!(frozen::RECORD.identity.policy_id, outfield_ai_policy::id());
}

// A local, test-only accessor: production code never needs to read an
// identity field by string name outside `compare`/`report`/`signature`,
// which live in `sut` and already have their own private version of this.
fn identity_field_string(identity: &sut::OutfieldAiBaselineIdentity, field: &str) -> String {
    match field {
        "schema" => identity.schema.clone(),
        "schema_version" => identity.schema_version.to_string(),
        "policy_id" => identity.policy_id.clone(),
        "fixture" => identity.fixture.clone(),
        "config" => identity.config.clone(),
        "config_hash" => identity.config_hash.clone(),
        "content_hash" => identity.content_hash.clone(),
        "tuning_hash" => identity.tuning_hash.clone(),
        "snapshot_version" => identity.snapshot_version.to_string(),
        "input_version" => identity.input_version.to_string(),
        "tick_rate" => identity.tick_rate.to_string(),
        "seed_first" => identity.seed_first.to_string(),
        "seed_count" => identity.seed_count.to_string(),
        "seed_hash" => identity.seed_hash.clone(),
        "fixture_hash" => identity.fixture_hash.clone(),
        other => panic!("unknown identity field: {other}"),
    }
}

#[test]
fn outfield_ai_baseline_records_every_tracked_metric_with_usable_variance() {
    for &key in sut::TRACKED {
        let stat = stat_by_key(&frozen::RECORD.stats, key);
        assert!(stat.n >= 1, "{key} has no contributing matches");
        assert!(
            stat.n <= frozen::RECORD.identity.seed_count,
            "{key} counts more than it ran"
        );
        assert!(stat.mean.is_finite(), "{key}.mean is not NaN/inf");
        assert!(stat.sd.is_finite(), "{key}.sd is not NaN/inf");
        assert!(stat.min.is_finite(), "{key}.min is not NaN/inf");
        assert!(stat.max.is_finite(), "{key}.max is not NaN/inf");
        assert!(stat.min <= stat.mean, "{key} min <= mean");
        assert!(stat.mean <= stat.max, "{key} mean <= max");
        assert!(stat.sd >= 0.0, "{key} has non-negative sd");
    }
}

fn stat_by_key(
    stats: &gc_data::outfield_ai_baseline::OutfieldAiBaselineStats,
    key: &str,
) -> gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
    match key {
        "fun" => stats.fun,
        "goals_total" => stats.goals_total,
        "goals_home" => stats.goals_home,
        "goals_away" => stats.goals_away,
        "shots" => stats.shots,
        "shots_per_goal" => stats.shots_per_goal,
        "save_rate" => stats.save_rate,
        "passes" => stats.passes,
        "pass_completion" => stats.pass_completion,
        "turnovers_per_min" => stats.turnovers_per_min,
        "possession_balance" => stats.possession_balance,
        "longest_drought_s" => stats.longest_drought_s,
        "decided_late" => stats.decided_late,
        "lead_changes" => stats.lead_changes,
        "margin" => stats.margin,
        "duration" => stats.duration,
        "ai_dribble_carry_s" => stats.ai_dribble_carry_s,
        "ai_dribble_close_share" => stats.ai_dribble_close_share,
        "ai_dribble_sprint_share" => stats.ai_dribble_sprint_share,
        "ai_dribble_juke_share" => stats.ai_dribble_juke_share,
        "ai_dribble_touches_per_min" => stats.ai_dribble_touches_per_min,
        "ai_dribble_heavy_losses_per_min" => stats.ai_dribble_heavy_losses_per_min,
        "ai_jukes" => stats.ai_jukes,
        other => panic!("unknown tracked metric: {other}"),
    }
}

#[test]
fn outfield_ai_baseline_carries_a_self_consistent_signature() {
    // Recomputing over the file's own contents catches a hand-edited number
    // that never went through the write path.
    let record = frozen_record();
    assert_eq!(sut::signature(&record), frozen::RECORD.signature);
}

#[test]
fn outfield_ai_baseline_keeps_baseline_version_out_of_the_signature() {
    // The freeze counter must not enter the evidence hash, or a re-freeze
    // that changes nothing would look like a changed recording.
    let mut bumped = frozen_record();
    bumped.baseline_version += 7;
    assert_eq!(sut::signature(&bumped), frozen::RECORD.signature);
}

#[test]
fn outfield_ai_baseline_passes_when_a_record_is_compared_against_itself() {
    let comparison = sut::compare(&frozen_record(), &frozen_record());
    assert!(comparison.ok);
    assert!(comparison.metrics_ok);
    assert!(comparison.identity_ok);
    assert!(comparison.signature_ok);
    assert!(!comparison.stale);
    assert_eq!(comparison.rows.len(), sut::TRACKED.len());
}

#[test]
fn outfield_ai_baseline_flags_a_moved_metric_instead_of_absorbing_it() {
    let mut current = frozen_record();
    current.stats.pass_completion.mean += 1e-9;
    current.signature = sut::signature(&current);
    let comparison = sut::compare(&frozen_record(), &current);
    assert!(!comparison.ok, "an exact baseline must not absorb drift");
    assert!(comparison.identity_ok, "the fixture itself is unchanged");
    assert!(!comparison.signature_ok);
    let moved: Vec<&str> = comparison
        .rows
        .iter()
        .filter(|r| !r.ok)
        .map(|r| {
            assert_eq!(r.moved, vec!["mean"], "and names the moved field");
            r.key
        })
        .collect();
    assert_eq!(moved.len(), 1, "only the moved metric is flagged");
    assert_eq!(moved[0], "pass_completion");
}

#[test]
fn outfield_ai_baseline_warns_about_a_stale_identity_without_blocking_unrelated_work() {
    // Identity covers all 40 knob defaults and every authored roster, so it
    // moves for edits that provably cannot change combat-disabled play.
    // Those must stay visible without taxing an unrelated branch with the
    // re-freeze ceremony.
    let mut current = frozen_record();
    current.identity.tuning_hash = "deadbeefdeadbeef".to_string();
    current.signature = sut::signature(&current);
    let comparison = sut::compare(&frozen_record(), &current);
    assert!(
        comparison.ok,
        "identical play must not fail the shared gate"
    );
    assert!(
        comparison.stale,
        "but the stale identity is still called out"
    );
    assert!(!comparison.identity_ok);
    assert!(!comparison.signature_ok);
    for row in &comparison.rows {
        assert!(row.ok, "no metric moved");
    }
    let report = sut::report(&comparison, &frozen_record(), &current);
    assert!(
        report.contains("AI BASELINE STALE"),
        "the report names staleness"
    );
    assert!(
        report.contains("tuning_hash"),
        "and which identity field moved"
    );
    assert!(
        report.contains("drift-log entry"),
        "and still demands the drift log"
    );
    assert!(
        !report.contains("AI BASELINE MOVED"),
        "without claiming a failure"
    );
}

#[test]
fn outfield_ai_baseline_still_fails_when_a_changed_policy_actually_moves_a_metric() {
    let mut current = frozen_record();
    current.identity.policy_id =
        "outfield_ai_policy/v1/combat_disabled/deadbeefdeadbeef".to_string();
    current.stats.goals_total.mean += 0.25;
    current.signature = sut::signature(&current);
    let comparison = sut::compare(&frozen_record(), &current);
    assert!(
        !comparison.ok,
        "moved play under a new policy is a hard failure"
    );
    assert!(
        !comparison.stale,
        "which is a failure, not a staleness warning"
    );
    assert!(!comparison.identity_ok);
}

#[test]
fn outfield_ai_baseline_tells_the_reader_not_to_refresh_a_failure_away() {
    let mut current = frozen_record();
    current.stats.goals_total.mean += 0.5;
    current.signature = sut::signature(&current);
    let comparison = sut::compare(&frozen_record(), &current);
    let report = sut::report(&comparison, &frozen_record(), &current);
    assert!(
        report.contains("AI BASELINE MOVED"),
        "the report names the failure"
    );
    assert!(
        report.contains("deletes the evidence"),
        "and the reason not to"
    );
    assert!(report.contains("--refreeze-ack"), "and the deliberate path");
}

#[test]
fn outfield_ai_baseline_serializes_a_loadable_record_that_round_trips_exactly() {
    // Exact comparison is only sound if the on-disk decimal form loses
    // nothing, so this pins the serializer's precision, not its layout.
    let record = frozen_record();
    let chunk = sut::serialize(&record);
    let loaded = parse_serialized(&chunk);
    assert_eq!(loaded.baseline_version, record.baseline_version);
    assert_eq!(loaded.signature, record.signature);
    for &field in sut::IDENTITY_FIELDS {
        assert_eq!(
            identity_field_string(&loaded.identity, field),
            identity_field_string(&record.identity, field),
            "{field}"
        );
    }
    for &key in sut::TRACKED {
        let want = stat_by_key(&record.stats, key);
        let got = stat_by_key(&loaded.stats, key);
        assert_eq!(got.n, want.n, "{key}.n round-trips bit-for-bit");
        assert!(
            got.mean.to_bits() == want.mean.to_bits(),
            "{key}.mean round-trips bit-for-bit"
        );
        assert!(
            got.sd.to_bits() == want.sd.to_bits(),
            "{key}.sd round-trips bit-for-bit"
        );
        assert!(
            got.min.to_bits() == want.min.to_bits(),
            "{key}.min round-trips bit-for-bit"
        );
        assert!(
            got.max.to_bits() == want.max.to_bits(),
            "{key}.max round-trips bit-for-bit"
        );
    }
    assert_eq!(sut::signature(&loaded), record.signature);
}

/// A minimal parser for [`sut::serialize`]'s own output shape — this test
/// file's counterpart to `sim/main.lua`'s `loadstring`, which Rust has no
/// equivalent of. Good enough to round-trip exactly what `serialize` emits;
/// not a general Lua-table parser.
fn parse_serialized(chunk: &str) -> sut::OutfieldAiBaselineRecord {
    let mut baseline_version = 0i64;
    let mut identity = sut::OutfieldAiBaselineIdentity {
        schema: String::new(),
        schema_version: 0,
        policy_id: String::new(),
        fixture: String::new(),
        fixture_hash: String::new(),
        config: String::new(),
        config_hash: String::new(),
        content_hash: String::new(),
        tuning_hash: String::new(),
        snapshot_version: 0,
        input_version: 0,
        tick_rate: 0,
        seed_first: 0,
        seed_count: 0,
        seed_hash: String::new(),
    };
    let mut stats = gc_data::outfield_ai_baseline::OutfieldAiBaselineStats {
        fun: zero_stat(),
        goals_total: zero_stat(),
        goals_home: zero_stat(),
        goals_away: zero_stat(),
        shots: zero_stat(),
        shots_per_goal: zero_stat(),
        save_rate: zero_stat(),
        passes: zero_stat(),
        pass_completion: zero_stat(),
        turnovers_per_min: zero_stat(),
        possession_balance: zero_stat(),
        longest_drought_s: zero_stat(),
        decided_late: zero_stat(),
        lead_changes: zero_stat(),
        margin: zero_stat(),
        duration: zero_stat(),
        ai_dribble_carry_s: zero_stat(),
        ai_dribble_close_share: zero_stat(),
        ai_dribble_sprint_share: zero_stat(),
        ai_dribble_juke_share: zero_stat(),
        ai_dribble_touches_per_min: zero_stat(),
        ai_dribble_heavy_losses_per_min: zero_stat(),
        ai_jukes: zero_stat(),
    };
    let mut signature = String::new();

    // A small explicit-stack scope tracker: `return { ... }` nests
    // `identity = { ... }` and `stats = { fun = { ... }, ... }`, and a
    // bare `fixture_hash`/`fixture` etc. key means something different
    // depending which scope it's read in (`stats` has no such keys, but
    // being explicit here means a future field name collision fails loudly
    // instead of silently reading the wrong scope).
    let mut scope: Vec<String> = Vec::new();
    let mut metric_n = 0i64;
    let mut metric_mean = 0.0;
    let mut metric_sd = 0.0;
    let mut metric_min = 0.0;
    let mut metric_max = 0.0;

    for raw_line in chunk.lines() {
        let line = raw_line.trim();
        if line == "return {" {
            scope.push("root".to_string());
            continue;
        }
        if let Some(key) = line.strip_suffix(" = {") {
            scope.push(key.trim().to_string());
            continue;
        }
        if line == "}," || line == "}" {
            if let Some(closed) = scope.pop()
                && scope.last().map(String::as_str) == Some("stats")
            {
                let stat = gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
                    n: metric_n,
                    mean: metric_mean,
                    sd: metric_sd,
                    min: metric_min,
                    max: metric_max,
                };
                set_stat(&mut stats, &closed, stat);
            }
            continue;
        }
        let line = line.trim_end_matches(',');
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let unquoted = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'));
        let in_identity = scope.last().map(String::as_str) == Some("identity");
        let in_metric = scope.len() >= 2 && scope[scope.len() - 2] == "stats";
        match key {
            "baseline_version" => baseline_version = value.parse().unwrap(),
            "schema" if in_identity => identity.schema = unquoted.unwrap().to_string(),
            "schema_version" if in_identity => {
                identity.schema_version = value.parse().unwrap();
            }
            "policy_id" if in_identity => identity.policy_id = unquoted.unwrap().to_string(),
            "fixture" if in_identity => identity.fixture = unquoted.unwrap().to_string(),
            "fixture_hash" if in_identity => {
                identity.fixture_hash = unquoted.unwrap().to_string();
            }
            "config" if in_identity => identity.config = unquoted.unwrap().to_string(),
            "config_hash" if in_identity => {
                identity.config_hash = unquoted.unwrap().to_string();
            }
            "content_hash" if in_identity => {
                identity.content_hash = unquoted.unwrap().to_string();
            }
            "tuning_hash" if in_identity => {
                identity.tuning_hash = unquoted.unwrap().to_string();
            }
            "snapshot_version" if in_identity => {
                identity.snapshot_version = value.parse().unwrap();
            }
            "input_version" if in_identity => {
                identity.input_version = value.parse().unwrap();
            }
            "tick_rate" if in_identity => identity.tick_rate = value.parse().unwrap(),
            "seed_first" if in_identity => identity.seed_first = value.parse().unwrap(),
            "seed_count" if in_identity => identity.seed_count = value.parse().unwrap(),
            "seed_hash" if in_identity => identity.seed_hash = unquoted.unwrap().to_string(),
            "signature" => signature = unquoted.unwrap().to_string(),
            "n" if in_metric => metric_n = value.parse().unwrap(),
            "mean" if in_metric => metric_mean = value.parse().unwrap(),
            "sd" if in_metric => metric_sd = value.parse().unwrap(),
            "min" if in_metric => metric_min = value.parse().unwrap(),
            "max" if in_metric => metric_max = value.parse().unwrap(),
            _ => {}
        }
    }

    sut::OutfieldAiBaselineRecord {
        baseline_version,
        identity,
        stats,
        signature,
    }
}

fn zero_stat() -> gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
    gc_data::outfield_ai_baseline::OutfieldAiBaselineStat {
        n: 0,
        mean: 0.0,
        sd: 0.0,
        min: 0.0,
        max: 0.0,
    }
}

fn set_stat(
    stats: &mut gc_data::outfield_ai_baseline::OutfieldAiBaselineStats,
    key: &str,
    stat: gc_data::outfield_ai_baseline::OutfieldAiBaselineStat,
) {
    match key {
        "fun" => stats.fun = stat,
        "goals_total" => stats.goals_total = stat,
        "goals_home" => stats.goals_home = stat,
        "goals_away" => stats.goals_away = stat,
        "shots" => stats.shots = stat,
        "shots_per_goal" => stats.shots_per_goal = stat,
        "save_rate" => stats.save_rate = stat,
        "passes" => stats.passes = stat,
        "pass_completion" => stats.pass_completion = stat,
        "turnovers_per_min" => stats.turnovers_per_min = stat,
        "possession_balance" => stats.possession_balance = stat,
        "longest_drought_s" => stats.longest_drought_s = stat,
        "decided_late" => stats.decided_late = stat,
        "lead_changes" => stats.lead_changes = stat,
        "margin" => stats.margin = stat,
        "duration" => stats.duration = stat,
        "ai_dribble_carry_s" => stats.ai_dribble_carry_s = stat,
        "ai_dribble_close_share" => stats.ai_dribble_close_share = stat,
        "ai_dribble_sprint_share" => stats.ai_dribble_sprint_share = stat,
        "ai_dribble_juke_share" => stats.ai_dribble_juke_share = stat,
        "ai_dribble_touches_per_min" => stats.ai_dribble_touches_per_min = stat,
        "ai_dribble_heavy_losses_per_min" => stats.ai_dribble_heavy_losses_per_min = stat,
        "ai_jukes" => stats.ai_jukes = stat,
        _ => {}
    }
}

#[test]
fn outfield_ai_baseline_reproduces_a_fresh_run_of_the_fixture_exactly() {
    // A two-seed probe of the real code path: the full 60-seed verification
    // is `outfield_ai_baseline_reproduces_the_frozen_fixture_exactly`
    // below, but exact comparison is only defensible if measurement is
    // reproducible in the first place.
    let seeds = [sut::SEED_FIRST, sut::SEED_FIRST + 1];
    let first = sut::measure(&sut::MeasureOpts {
        baseline_version: None,
        seeds: Some(&seeds),
    });
    let second = sut::measure(&sut::MeasureOpts {
        baseline_version: None,
        seeds: Some(&seeds),
    });
    let comparison = sut::compare(&first, &second);
    assert!(
        comparison.ok,
        "the same seeds must produce the same recording"
    );
    assert_eq!(first.signature, second.signature);
}

#[test]
#[ignore = "Lua mutates the global tuning knob registry's default value at \
            runtime (`tuning.by_key[\"AI_SHOOT_RANGE\"].default = knob.max`) to \
            prove a policy change moves the recording. crate::tuning::KNOBS is a \
            `pub static &'static [Knob]` — compile-time-immutable by design (see \
            tuning.rs's module doc: the Lua global singleton was deliberately \
            replaced with an explicit owned Tuning value, AGENTS.md §3 forbids \
            stray mutable globals). There is no equivalent mutation point in this \
            port: a knob DEFAULT is baked into the static registry, not a \
            runtime Tuning value, so this specific test has no Rust-shaped \
            equivalent short of adding mutable global state the rest of the port \
            deliberately removed."]
fn outfield_ai_baseline_detects_a_real_policy_change_by_re_running_the_fixture() {
    unimplemented!(
        "needs a way to mutate crate::tuning::KNOBS's default at runtime, which \
         this port's architecture does not provide (see #[ignore] reason)"
    );
}

#[test]
fn outfield_ai_baseline_cannot_mistake_a_probe_run_for_the_frozen_freeze() {
    let seeds = [sut::SEED_FIRST];
    let probe = sut::measure(&sut::MeasureOpts {
        baseline_version: None,
        seeds: Some(&seeds),
    });
    assert_eq!(probe.identity.seed_count, 1);
    assert_ne!(
        probe.identity.seed_hash,
        frozen::RECORD.identity.seed_hash,
        "different seed set"
    );
    assert_ne!(
        probe.identity.fixture_hash,
        frozen::RECORD.identity.fixture_hash,
        "different fixture"
    );
    let comparison = sut::compare(&frozen_record(), &probe);
    assert!(
        !comparison.identity_ok,
        "a truncated run can never satisfy the freeze"
    );
    assert_ne!(
        probe.identity.seed_count,
        frozen::RECORD.identity.seed_count,
        "the recorded seed count is what actually ran"
    );
}

/// The determinism-path question this whole module exists to answer: does a
/// full, real 60-seed run of the frozen fixture reproduce
/// `gc_data::outfield_ai_baseline::RECORD` exactly? This is NOT a port of a
/// `spec/sim/outfield_ai_baseline_spec.lua` assertion — that file only checks
/// a 2-seed *self*-reproducibility (see
/// `outfield_ai_baseline_reproduces_a_fresh_run_of_the_fixture_exactly`
/// above) and never asserts the full 60-seed run equals the frozen record;
/// that comparison is a separate build-time check
/// (`love . --ai-baseline`/`scripts/check.sh`), not a `busted` spec case.
///
/// This test used to be `#[ignore]`d: a full 60-seed run of the already-
/// ported simulation did not reproduce the frozen fixture bit-for-bit, even
/// though this module's own identity/content/tuning hashes and
/// self-reproducibility were exact (proof the divergence was upstream, in
/// `sim::r#match`/`sim::ai`/`sim::bot`/`sim::keeper` — none of which are
/// this module's files). Root cause, found via `match_differential`
/// extended from 600 to the full 7200-tick match length: `sim/match.lua`
/// has an `and`/`or` operator-precedence bug in three places (`toward_goal`
/// in the off-ball keeper positioning, `inbound` in the queued-dive resolve,
/// and `toward` in `attempt_save`) — `(team == "home") and X < 0 or X > 0`
/// parses as `((team == "home") and (X < 0)) or (X > 0)`, not the intended
/// per-team ternary, so for the home team it is true for ANY nonzero
/// `ball_vel.x`. The Rust port had translated each occurrence into the
/// obviously-intended `if team == home { X < 0 } else { X > 0 }`, which
/// reads correctly but does not match the reference's actual (buggy)
/// runtime behavior — divergent home-keeper save/positioning decisions that
/// first showed up as a position mismatch at tick 831 on one seed and tick
/// 4527 on another. Fixed in `crates/gc-sim/src/match.rs` by reproducing
/// Lua's actual parse verbatim. All 60 baseline seeds now reproduce the
/// frozen fixture exactly over the full 7200-tick match, so this test is
/// back in the gate.
#[test]
fn outfield_ai_baseline_reproduces_the_frozen_fixture_exactly() {
    let current = sut::measure(&sut::MeasureOpts::default());
    assert_eq!(
        current.identity.policy_id,
        frozen::RECORD.identity.policy_id,
        "policy id must match the frozen fixture"
    );
    let comparison = sut::compare(&frozen_record(), &current);
    assert!(
        comparison.ok,
        "a fresh 60-seed run must reproduce the frozen baseline exactly; report:\n{}",
        sut::report(&comparison, &frozen_record(), &current)
    );
    assert!(comparison.identity_ok, "identity must also match exactly");
    assert_eq!(current.signature, frozen::RECORD.signature);
}
