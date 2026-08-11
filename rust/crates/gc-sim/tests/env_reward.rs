//! Tests for `gc_sim::env_reward`.
//!
//! `env_reward` is not on the determinism path: reward shaping doesn't cross
//! the network or feed a resim, so no differential coverage is required here
//! (ARCHITECTURE.md §3 rule 7 only requires it on the determinism path) —
//! the assertions below are the whole contract.

use gc_sim::env_reward::{
    self, CombatContactResult, EnvRewardChannelId, EnvRewardErrorCode, EnvRewardEvent,
    EnvRewardRole, EnvRewardScore, EnvRewardSelection, EnvRewardTransition, EnvSide,
    RawChannelIdEntry, RawChannelIds, RawSelection, RawSelectionTable,
};

fn base_transition() -> EnvRewardTransition {
    EnvRewardTransition {
        team: EnvSide::Home,
        score_before: EnvRewardScore { home: 0, away: 0 },
        score_after: EnvRewardScore { home: 0, away: 0 },
        owner_team_before: None,
        owner_team_after: None,
        events: Vec::new(),
        terminated: false,
    }
}

fn selection(
    objectives: &[EnvRewardChannelId],
    shaping: &[EnvRewardChannelId],
) -> EnvRewardSelection {
    EnvRewardSelection {
        objectives: objectives.to_vec(),
        shaping: shaping.to_vec(),
    }
}

fn ids(names: &[&str]) -> RawChannelIds {
    RawChannelIds::List(
        names
            .iter()
            .map(|name| RawChannelIdEntry::Str((*name).to_string()))
            .collect(),
    )
}

fn event(kind: &str, team: Option<EnvSide>, result: Option<CombatContactResult>) -> EnvRewardEvent {
    EnvRewardEvent {
        kind: kind.to_string(),
        team,
        result,
    }
}

#[test]
fn env_reward_registry_has_no_channel_named_fun_and_no_optimizable_diagnostic() {
    for channel in env_reward::CHANNELS {
        let name = env_reward::channel_id_name(channel.id);
        assert_ne!(name, "fun", "no reward channel may be named fun");
        assert!(!name.contains("fun"), "no channel id may mention fun");
        if channel.role == EnvRewardRole::Diagnostic {
            assert!(!channel.optimizable);
        }
        assert!(
            !channel.description.is_empty(),
            "every channel documents itself"
        );
    }
    let diagnostic = env_reward::channel(EnvRewardChannelId::ExperienceProxyMetrics);
    assert_eq!(diagnostic.role, EnvRewardRole::Diagnostic);
    assert_eq!(
        env_reward::DIAGNOSTIC_METRIC_CHANNEL,
        EnvRewardChannelId::ExperienceProxyMetrics
    );
}

#[test]
fn env_reward_registry_defaults_to_the_sparse_match_outcome_only() {
    assert_eq!(env_reward::DEFAULT_OBJECTIVES.len(), 1);
    assert_eq!(
        env_reward::DEFAULT_OBJECTIVES[0],
        EnvRewardChannelId::MatchOutcome
    );
}

#[test]
fn env_reward_validate_selection_accepts_channels_of_the_requested_role() {
    let objectives =
        env_reward::validate_selection(&ids(&["goal_scored"]), EnvRewardRole::Objective).unwrap();
    assert_eq!(objectives[0], EnvRewardChannelId::GoalScored);
    let empty = env_reward::validate_selection(&ids(&[]), EnvRewardRole::Shaping).unwrap();
    assert_eq!(empty.len(), 0);
}

#[test]
fn env_reward_validate_selection_rejects_unknown_misplaced_duplicated_and_malformed_selections() {
    let cases: Vec<(RawChannelIds, EnvRewardRole, EnvRewardErrorCode)> = vec![
        (
            ids(&["score_more"]),
            EnvRewardRole::Objective,
            EnvRewardErrorCode::UnknownChannel,
        ),
        (
            ids(&["possession_gain"]),
            EnvRewardRole::Objective,
            EnvRewardErrorCode::WrongRole,
        ),
        (
            ids(&["match_outcome"]),
            EnvRewardRole::Shaping,
            EnvRewardErrorCode::WrongRole,
        ),
        (
            ids(&["experience_proxy_metrics"]),
            EnvRewardRole::Objective,
            EnvRewardErrorCode::WrongRole,
        ),
        (
            ids(&["goal_scored", "goal_scored"]),
            EnvRewardRole::Objective,
            EnvRewardErrorCode::DuplicateChannel,
        ),
        (
            RawChannelIds::List(vec![RawChannelIdEntry::Other]),
            EnvRewardRole::Objective,
            EnvRewardErrorCode::Malformed,
        ),
        (
            RawChannelIds::Other,
            EnvRewardRole::Objective,
            EnvRewardErrorCode::Malformed,
        ),
    ];
    for (raw_ids, role, expected_code) in cases {
        let err = env_reward::validate_selection(&raw_ids, role)
            .expect_err("the selection must be rejected");
        assert_eq!(err.code, expected_code);
        assert!(!err.message.is_empty(), "a reason is always supplied");
    }
}

#[test]
fn env_reward_validate_selection_rejects_a_selection_table_with_unknown_fields() {
    let raw = RawSelection::Table(RawSelectionTable {
        objectives: Some(RawChannelIds::List(Vec::new())),
        shaping: None,
        has_unknown_field: true,
    });
    let err = env_reward::validate(&raw).expect_err("must be rejected");
    assert_eq!(err.code, EnvRewardErrorCode::Malformed);
}

#[test]
fn env_reward_evaluate_pays_the_sparse_outcome_only_when_the_match_ends() {
    let sel = selection(&[EnvRewardChannelId::MatchOutcome], &[]);

    let running = env_reward::evaluate(
        &EnvRewardTransition {
            score_after: EnvRewardScore { home: 2, away: 0 },
            ..base_transition()
        },
        &sel,
    );
    assert_eq!(
        *running
            .objectives
            .get(&EnvRewardChannelId::MatchOutcome)
            .unwrap(),
        0.0
    );

    let won = env_reward::evaluate(
        &EnvRewardTransition {
            score_after: EnvRewardScore { home: 2, away: 0 },
            terminated: true,
            ..base_transition()
        },
        &sel,
    );
    assert_eq!(
        *won.objectives
            .get(&EnvRewardChannelId::MatchOutcome)
            .unwrap(),
        1.0
    );

    let lost = env_reward::evaluate(
        &EnvRewardTransition {
            score_after: EnvRewardScore { home: 0, away: 2 },
            terminated: true,
            ..base_transition()
        },
        &sel,
    );
    assert_eq!(
        *lost
            .objectives
            .get(&EnvRewardChannelId::MatchOutcome)
            .unwrap(),
        -1.0
    );

    let drawn = env_reward::evaluate(
        &EnvRewardTransition {
            score_after: EnvRewardScore { home: 1, away: 1 },
            terminated: true,
            ..base_transition()
        },
        &sel,
    );
    assert_eq!(
        *drawn
            .objectives
            .get(&EnvRewardChannelId::MatchOutcome)
            .unwrap(),
        0.0
    );
}

#[test]
fn env_reward_evaluate_expresses_goal_channels_from_the_configured_perspective() {
    let sel = selection(
        &[
            EnvRewardChannelId::GoalScored,
            EnvRewardChannelId::GoalConceded,
            EnvRewardChannelId::GoalDifferenceDelta,
        ],
        &[],
    );

    let moved = EnvRewardTransition {
        score_before: EnvRewardScore { home: 1, away: 1 },
        score_after: EnvRewardScore { home: 2, away: 1 },
        ..base_transition()
    };
    let home = env_reward::evaluate(&moved, &sel);
    assert_eq!(
        *home
            .objectives
            .get(&EnvRewardChannelId::GoalScored)
            .unwrap(),
        1.0
    );
    assert_eq!(
        *home
            .objectives
            .get(&EnvRewardChannelId::GoalConceded)
            .unwrap(),
        0.0
    );
    assert_eq!(
        *home
            .objectives
            .get(&EnvRewardChannelId::GoalDifferenceDelta)
            .unwrap(),
        1.0
    );

    let away_view = EnvRewardTransition {
        team: EnvSide::Away,
        score_before: EnvRewardScore { home: 1, away: 1 },
        score_after: EnvRewardScore { home: 2, away: 1 },
        ..base_transition()
    };
    let away = env_reward::evaluate(&away_view, &sel);
    assert_eq!(
        *away
            .objectives
            .get(&EnvRewardChannelId::GoalScored)
            .unwrap(),
        0.0
    );
    assert_eq!(
        *away
            .objectives
            .get(&EnvRewardChannelId::GoalConceded)
            .unwrap(),
        -1.0
    );
    assert_eq!(
        *away
            .objectives
            .get(&EnvRewardChannelId::GoalDifferenceDelta)
            .unwrap(),
        -1.0
    );
    assert_eq!(away.team, EnvSide::Away);
}

#[test]
fn env_reward_evaluate_scores_shaping_channels_from_confirmed_events_and_ownership() {
    let gained = env_reward::evaluate(
        &EnvRewardTransition {
            owner_team_before: Some(EnvSide::Away),
            owner_team_after: Some(EnvSide::Home),
            ..base_transition()
        },
        &selection(&[], &[EnvRewardChannelId::PossessionGain]),
    );
    assert_eq!(
        *gained
            .shaping
            .get(&EnvRewardChannelId::PossessionGain)
            .unwrap(),
        1.0
    );

    let lost = env_reward::evaluate(
        &EnvRewardTransition {
            owner_team_before: Some(EnvSide::Home),
            owner_team_after: None,
            ..base_transition()
        },
        &selection(&[], &[EnvRewardChannelId::PossessionGain]),
    );
    assert_eq!(
        *lost
            .shaping
            .get(&EnvRewardChannelId::PossessionGain)
            .unwrap(),
        -1.0
    );

    let shots = env_reward::evaluate(
        &EnvRewardTransition {
            events: vec![
                event("shot", Some(EnvSide::Home), None),
                event("header", Some(EnvSide::Home), None),
                event("shot", Some(EnvSide::Away), None),
                event("pass", Some(EnvSide::Home), None),
            ],
            ..base_transition()
        },
        &selection(&[], &[EnvRewardChannelId::ShotAttempt]),
    );
    assert_eq!(
        *shots.shaping.get(&EnvRewardChannelId::ShotAttempt).unwrap(),
        2.0
    );

    let contacts = env_reward::evaluate(
        &EnvRewardTransition {
            events: vec![
                event(
                    "contact",
                    Some(EnvSide::Home),
                    Some(CombatContactResult::Hit),
                ),
                event(
                    "contact",
                    Some(EnvSide::Home),
                    Some(CombatContactResult::Guarded),
                ),
                event(
                    "contact",
                    Some(EnvSide::Away),
                    Some(CombatContactResult::Hit),
                ),
            ],
            ..base_transition()
        },
        &selection(&[], &[EnvRewardChannelId::EquipmentContact]),
    );
    assert_eq!(
        *contacts
            .shaping
            .get(&EnvRewardChannelId::EquipmentContact)
            .unwrap(),
        1.0
    );
}

#[test]
fn env_reward_evaluate_keeps_shaping_separable_so_an_ablation_is_a_subtraction() {
    let moved = EnvRewardTransition {
        score_after: EnvRewardScore { home: 1, away: 0 },
        owner_team_before: None,
        owner_team_after: Some(EnvSide::Home),
        events: vec![event("shot", Some(EnvSide::Home), None)],
        ..base_transition()
    };
    let both = env_reward::evaluate(
        &moved,
        &selection(
            &[EnvRewardChannelId::GoalScored],
            &[
                EnvRewardChannelId::PossessionGain,
                EnvRewardChannelId::ShotAttempt,
            ],
        ),
    );
    assert_eq!(both.objective_total, 1.0);
    assert_eq!(both.shaping_total, 2.0);
    assert_eq!(both.total, 3.0);

    let ablated = env_reward::evaluate(&moved, &selection(&[EnvRewardChannelId::GoalScored], &[]));
    assert_eq!(ablated.objective_total, both.objective_total);
    assert_eq!(ablated.shaping_total, 0.0);
    assert_eq!(ablated.total, 1.0);
    assert!(
        ablated.shaping.is_empty(),
        "an ablated run reports no shaping terms at all"
    );
}

#[test]
fn env_reward_evaluate_never_invents_a_channel_the_selection_did_not_ask_for() {
    let result = env_reward::evaluate(
        &EnvRewardTransition {
            score_after: EnvRewardScore { home: 3, away: 0 },
            terminated: true,
            ..base_transition()
        },
        &selection(&[EnvRewardChannelId::MatchOutcome], &[]),
    );
    assert!(
        result
            .objectives
            .get(&EnvRewardChannelId::GoalScored)
            .is_none()
    );
    assert!(
        result
            .objectives
            .get(&EnvRewardChannelId::ExperienceProxyMetrics)
            .is_none()
    );
    assert_eq!(result.version, env_reward::VERSION);
}
