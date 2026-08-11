//! Shared test-support modules for `gc-netcode`'s integration tests.
//!
//! Each `tests/*.rs` file is compiled as its own crate root, so this is a
//! plain `mod.rs` re-export point rather than a crate: a consumer adds
//! `mod support;` and reaches these through `support::online_combat_phases`.

pub mod online_combat_phases;
