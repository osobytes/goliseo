//! The simulation. Pure, deterministic, no I/O.
//!
//! Every module is declared here up front, so contributors adding new
//! modules in parallel never contend on this file.
#![deny(missing_docs)]

pub mod aerial;
pub mod ai;
pub mod ai_driven_evidence;
pub mod bot;
pub mod brain;
pub mod combat;
pub mod combat_feasibility;
pub mod combat_identity;
pub mod combat_intent;
pub mod combat_observation;
pub mod combat_policy;
pub mod combat_rules;
pub mod combat_snapshot;
pub mod content_validation;
pub mod determinism_evidence;
pub mod env;
pub mod env_action;
pub mod env_config;
pub mod env_observation;
pub mod env_reward;
pub mod fixed_clock;
pub mod headless;
pub mod input_frame;
pub mod input_tape;
pub mod keeper;
pub mod lever_metrics;
// `match` is a Rust keyword, so the module needs a raw identifier. Refer to
// it as `gc_sim::r#match` or bring it into scope with
// `use gc_sim::r#match as sim_match;`.
pub mod r#match;
pub mod match_snapshot;
pub mod metrics;
pub mod network_conditions;
pub mod offball_runs;
pub mod outfield_ai_baseline;
pub mod outfield_ai_policy;
pub mod outfield_decision;
pub mod outfield_press;
pub mod passing;
pub mod placement;
pub mod possession_transition;
pub mod rating;
pub mod rating_validation;
pub mod replay;
pub mod research_dataset;
pub mod research_features;
pub mod research_response;
pub mod research_schema;
pub mod research_session;
pub mod research_timeline;
pub mod research_trace;
pub mod rollback_events;
pub mod rollback_input_history;
pub mod rollback_lab;
pub mod rollback_playable_lab;
pub mod rollback_session;
pub mod rollback_snapshot_history;
pub mod rollback_validation;
pub mod slot_input;
pub mod snapshot_headroom;
pub mod species;
pub mod stats;
pub mod sweep;
pub mod tripwire;
pub mod tuning;
