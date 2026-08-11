//! Content tables: players, teams, formations, tactics, traits, arenas.
//!
//! Every module here is data, not logic (AGENTS.md §8: data is content, code
//! is mechanism). The one exception is [`omp1_determinism`], a golden
//! determinism-evidence fixture converted mechanically to JSON; see its
//! module doc for details.

pub mod action_families;
pub mod arenas;
pub mod character_presentations;
pub mod cosmetic_variants;
pub mod equipment_presentations;
pub mod formations;
pub mod fun_baseline;
pub mod loadouts;
pub mod network_profiles;
pub mod omp1_determinism;
pub mod omp2_rollback_validation;
pub mod outfield_ai_baseline;
pub mod players;
pub mod research_features;
pub mod research_instruments;
pub mod showcase_player_compatibility;
pub mod species;
pub mod tactics;
pub mod teams;
pub mod tuning_presets;
