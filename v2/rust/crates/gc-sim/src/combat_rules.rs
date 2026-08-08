//! Port of `sim/combat_rules.lua`.
//!
//! Shared combat tuning constants used across the combat modules.

/// Maximum ticks a disable effect can hold a player for.
pub const MAX_DISABLE_TICKS: i64 = 30;
/// Post-hit immunity window, in ticks.
pub const IMMUNITY_TICKS: i64 = 45;
/// Minimum displacement, in pixels, that counts as a knockback.
pub const KNOCKBACK_THRESHOLD_PX: f64 = 12.0;
