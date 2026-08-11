//! Tests for `gc_sim::env_config`.
//!
//! `env_config` is not itself on the wire/resim path (it validates and
//! resolves episode identity before a match starts), so no differential
//! coverage against the reference Lua is required — see
//! `tools/lua_reference/README.md`'s "When it is worth doing".

use gc_sim::env_config::{
    self, EnvConfigErrorCode, EnvObservationProfile, EnvSlotSourceKind, RawEnvConfig, RawFieldSize,
    RawSlotSource,
};
use gc_sim::env_reward::{EnvRewardChannelId, EnvSide};
use gc_sim::input_frame;

fn raw() -> RawEnvConfig {
    RawEnvConfig {
        build: Some("spec-build".to_string()),
        content: Some("showcase-content-v1".to_string()),
        seed: Some(5.0),
        duration: Some(4.0),
        ..Default::default()
    }
}

fn slot_source(kind: &str, seed: Option<f64>, policy_id: Option<&str>) -> RawSlotSource {
    RawSlotSource {
        kind: kind.to_string(),
        seed,
        policy_id: policy_id.map(str::to_string),
    }
}

#[test]
fn env_config_normalize_stamps_the_version_and_fills_documented_defaults() {
    let config = env_config::normalize(&raw()).unwrap();
    assert_eq!(config.version, env_config::VERSION);
    assert_eq!(
        config.source, "spec-build",
        "source defaults to the build label"
    );
    assert_eq!(config.max_goals, env_config::DEFAULT_MAX_GOALS);
    assert_eq!(config.field.w, env_config::DEFAULT_FIELD.w);
    assert_eq!(config.home_team_id, env_config::DEFAULT_HOME_TEAM_ID);
    assert_eq!(config.home_tactic_id, env_config::DEFAULT_TACTIC_ID);
    assert!(!config.combat);
    assert_eq!(
        config.observation_profile,
        EnvObservationProfile::Representative
    );
    assert_eq!(config.reward_team, EnvSide::Home);
    assert_eq!(config.slot_sources.len(), input_frame::SLOT_COUNT as usize);
    assert_eq!(config.slot_sources[0].kind, EnvSlotSourceKind::Policy);
    assert_eq!(config.slot_sources[1].kind, EnvSlotSourceKind::Neutral);
    assert_eq!(
        config.objective_channels[0],
        EnvRewardChannelId::MatchOutcome
    );
    assert_eq!(config.shaping_channels.len(), 0);
    assert_eq!(config.max_episode_ticks, None);
    assert_eq!(
        config.fixture,
        "nebula-v-orion;2-1-1-v-1-1-2;balanced-v-balanced;combat=false"
    );
}

#[test]
fn env_config_normalize_copies_rather_than_aliases_the_callers_tables() {
    let mut sources = env_config::default_slot_sources();
    let field = RawFieldSize {
        w: Some(800.0),
        h: Some(480.0),
        has_unknown_field: false,
    };
    let config = env_config::normalize(&RawEnvConfig {
        slot_sources: Some(sources.clone()),
        field: Some(field),
        ..raw()
    })
    .unwrap();
    // `normalize` takes `&RawEnvConfig` and builds a fresh owned `EnvConfig`;
    // Rust's ownership model makes aliasing the caller's `sources`/`field`
    // impossible by construction. Mutating them here after the fact, to
    // prove `normalize` doesn't alias, is therefore a regression guard
    // against ever changing that shape, not a live risk.
    sources[0].kind = "neutral".to_string();
    assert_eq!(config.slot_sources[0].kind, EnvSlotSourceKind::Policy);
    assert_eq!(config.field.w, 800.0);
}

#[test]
fn env_config_normalize_rejects_unknown_fields_and_unsupported_versions() {
    let unknown = env_config::normalize(&RawEnvConfig {
        has_unknown_field: true,
        ..raw()
    })
    .expect_err("an unknown field must be rejected");
    assert_eq!(unknown.code, EnvConfigErrorCode::Malformed);

    let versioned = env_config::normalize(&RawEnvConfig {
        version: Some(99),
        ..raw()
    })
    .expect_err("an unsupported version must be rejected");
    assert_eq!(versioned.code, EnvConfigErrorCode::UnsupportedVersion);
}

#[test]
fn env_config_normalize_rejects_missing_provenance_and_bad_scalars() {
    let no_build = env_config::normalize(&RawEnvConfig {
        seed: Some(1.0),
        ..Default::default()
    })
    .expect_err("a config without provenance is rejected");
    assert_eq!(no_build.code, EnvConfigErrorCode::Malformed);

    let no_seed = env_config::normalize(&RawEnvConfig {
        build: Some("spec-build".to_string()),
        ..Default::default()
    })
    .expect_err("a config without a seed is rejected");
    assert_eq!(no_seed.code, EnvConfigErrorCode::Malformed);

    let cases: Vec<RawEnvConfig> = vec![
        RawEnvConfig {
            build: Some(String::new()),
            ..raw()
        },
        RawEnvConfig {
            seed: Some(1.5),
            ..raw()
        },
        RawEnvConfig {
            duration: Some(0.0),
            ..raw()
        },
        RawEnvConfig {
            max_goals: Some(0.0),
            ..raw()
        },
        RawEnvConfig {
            field: Some(RawFieldSize {
                w: Some(0.0),
                h: Some(10.0),
                has_unknown_field: false,
            }),
            ..raw()
        },
        RawEnvConfig {
            field: Some(RawFieldSize {
                w: Some(10.0),
                h: Some(10.0),
                has_unknown_field: true,
            }),
            ..raw()
        },
        RawEnvConfig {
            max_episode_ticks: Some(0.0),
            ..raw()
        },
        RawEnvConfig {
            reward_team: Some("both".to_string()),
            ..raw()
        },
        RawEnvConfig {
            observation_profile: Some("oracle".to_string()),
            ..raw()
        },
    ];
    for config in cases {
        let err = env_config::normalize(&config).expect_err("config must be rejected");
        assert_eq!(err.code, EnvConfigErrorCode::Malformed);
    }
}

#[test]
fn env_config_normalize_rejects_unknown_authored_content() {
    let cases: Vec<RawEnvConfig> = vec![
        RawEnvConfig {
            home_team_id: Some("void".to_string()),
            ..raw()
        },
        RawEnvConfig {
            away_team_id: Some("void".to_string()),
            ..raw()
        },
        RawEnvConfig {
            home_formation: Some("9-9-9".to_string()),
            ..raw()
        },
        RawEnvConfig {
            away_formation: Some("9-9-9".to_string()),
            ..raw()
        },
        RawEnvConfig {
            home_tactic_id: Some("chaos".to_string()),
            ..raw()
        },
        RawEnvConfig {
            away_tactic_id: Some("chaos".to_string()),
            ..raw()
        },
    ];
    for config in cases {
        let err = env_config::normalize(&config).expect_err("config must be rejected");
        assert_eq!(err.code, EnvConfigErrorCode::UnknownContent);
    }
}

#[test]
fn env_config_normalize_validates_slot_sources_strictly() {
    fn code_for(mutate: impl FnOnce(&mut Vec<RawSlotSource>)) -> EnvConfigErrorCode {
        let mut sources = env_config::default_slot_sources();
        mutate(&mut sources);
        env_config::normalize(&RawEnvConfig {
            slot_sources: Some(sources),
            ..raw()
        })
        .expect_err("must be rejected")
        .code
    }

    assert_eq!(
        code_for(|sources| sources[1] = slot_source("bot", None, None)),
        EnvConfigErrorCode::Malformed,
        "a bot slot needs a seed"
    );
    assert_eq!(
        code_for(|sources| sources[1] = slot_source("neutral", Some(3.0), None)),
        EnvConfigErrorCode::Malformed,
        "only bot slots carry a seed"
    );
    assert_eq!(
        code_for(|sources| sources[1] = slot_source("neutral", None, Some("x"))),
        EnvConfigErrorCode::Malformed,
        "only policy slots carry a policy id"
    );
    assert_eq!(
        code_for(|sources| sources[1] = slot_source("oracle", None, None)),
        EnvConfigErrorCode::Malformed,
        "unknown kinds are rejected"
    );
    assert_eq!(
        code_for(|sources| {
            sources.remove(7);
        }),
        EnvConfigErrorCode::Malformed,
        "every canonical slot must be declared"
    );
    assert_eq!(
        code_for(|sources| sources[0] = slot_source("neutral", None, None)),
        EnvConfigErrorCode::EmptySelection,
        "an environment with no policy slot is useless"
    );
}

#[test]
fn env_config_normalize_keeps_the_representative_profile_to_exactly_one_controlled_slot() {
    let mut sources = env_config::default_slot_sources();
    sources[4] = slot_source("policy", None, None);

    let mismatch = env_config::normalize(&RawEnvConfig {
        slot_sources: Some(sources.clone()),
        observation_profile: Some("representative".to_string()),
        ..raw()
    })
    .expect_err("two policy slots cannot be representative");
    assert_eq!(mismatch.code, EnvConfigErrorCode::ProfileMismatch);

    let team = env_config::normalize(&RawEnvConfig {
        slot_sources: Some(sources),
        observation_profile: Some("team".to_string()),
        ..raw()
    })
    .unwrap();
    assert_eq!(env_config::controlled_slots(&team).len(), 2);
}

#[test]
fn env_config_normalize_refuses_diagnostic_metrics_as_an_optimization_target() {
    let objective_err = env_config::normalize(&RawEnvConfig {
        objective_channels: Some(vec!["experience_proxy_metrics".to_string()]),
        ..raw()
    })
    .expect_err("a diagnostic channel cannot be an objective");
    assert!(objective_err.message.contains("diagnostic"));

    let shaping = env_config::normalize(&RawEnvConfig {
        shaping_channels: Some(vec!["experience_proxy_metrics".to_string()]),
        ..raw()
    });
    assert!(shaping.is_err(), "a diagnostic channel cannot be shaping");

    let misplaced = env_config::normalize(&RawEnvConfig {
        objective_channels: Some(vec!["possession_gain".to_string()]),
        ..raw()
    });
    assert!(misplaced.is_err(), "a shaping channel is not an objective");
}

#[test]
fn env_config_normalize_classifies_tape_and_policy_slots() {
    let mut sources = env_config::default_slot_sources();
    sources[4] = slot_source("tape", None, None);
    sources[5] = slot_source("tape", None, None);
    let config = env_config::normalize(&RawEnvConfig {
        slot_sources: Some(sources),
        ..raw()
    })
    .unwrap();
    assert_eq!(env_config::controlled_slots(&config).len(), 1);
    assert_eq!(env_config::tape_slots(&config).len(), 2);
    assert_eq!(env_config::tape_slots(&config)[1], 6);
}

#[test]
fn env_config_digest_is_stable_for_equal_configs_and_sensitive_to_every_knob() {
    let base = env_config::normalize(&raw()).unwrap();
    assert_eq!(
        env_config::digest(&base),
        env_config::digest(&env_config::normalize(&raw()).unwrap())
    );

    let variants: Vec<RawEnvConfig> = vec![
        RawEnvConfig {
            duration: Some(5.0),
            ..raw()
        },
        RawEnvConfig {
            max_goals: Some(2.0),
            ..raw()
        },
        RawEnvConfig {
            field: Some(RawFieldSize {
                w: Some(800.0),
                h: Some(540.0),
                has_unknown_field: false,
            }),
            ..raw()
        },
        RawEnvConfig {
            away_team_id: Some("nebula".to_string()),
            ..raw()
        },
        RawEnvConfig {
            home_tactic_id: Some("press_high".to_string()),
            ..raw()
        },
        RawEnvConfig {
            combat: Some(true),
            ..raw()
        },
        RawEnvConfig {
            observation_profile: Some("privileged".to_string()),
            ..raw()
        },
        RawEnvConfig {
            reward_team: Some("away".to_string()),
            ..raw()
        },
        RawEnvConfig {
            max_episode_ticks: Some(10.0),
            ..raw()
        },
        RawEnvConfig {
            shaping_channels: Some(vec!["possession_gain".to_string()]),
            ..raw()
        },
    ];
    for variant in variants {
        let other = env_config::normalize(&variant).unwrap();
        assert_ne!(
            env_config::digest(&other),
            env_config::digest(&base),
            "the digest must react to a changed knob"
        );
    }
}

#[test]
fn env_config_digest_records_per_slot_ownership_and_policy_ids() {
    let mut sources = env_config::default_slot_sources();
    sources[0] = slot_source("policy", None, Some("ppo-42"));
    sources[2] = slot_source("bot", Some(7.0), None);
    let config = env_config::normalize(&RawEnvConfig {
        slot_sources: Some(sources),
        ..raw()
    })
    .unwrap();
    let digest = env_config::digest(&config);
    assert!(digest.contains("1:policy#ppo-42"));
    assert!(digest.contains("3:bot@7"));
}

#[test]
fn env_config_resolve_resolves_authored_ids_into_the_fixture_tables_the_simulation_consumes() {
    let config = env_config::normalize(&raw()).unwrap();
    let fixture = env_config::resolve(&config);
    assert_eq!(fixture.home.id, "nebula");
    assert_eq!(fixture.away.id, "orion");
    assert_eq!(fixture.home_tactic.id, "balanced");
    assert!(fixture.players_by_id.contains_key("zyro_vex"));
}
