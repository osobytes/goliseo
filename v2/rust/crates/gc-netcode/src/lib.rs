//! Deterministic online: rollback scheduling, input frames, state hashing.
//!
//! Every module is declared up front, including unported ones, so agents working
//! on different parts of `game/online/` never contend on this file.
#![deny(missing_docs)]

pub mod coordinator;
pub mod coordinator_conformance;
pub mod coordinator_driver;
pub mod coordinator_fixture;
pub mod desync_package;
pub mod diagnostics_schema;
pub mod fake_relay;
pub mod fake_star;
pub mod fault_harness;
pub mod fault_scenarios;
pub mod fault_transport;
pub mod input_protocol;
pub mod input_protocol_conformance;
pub mod input_protocol_fixture;
pub mod live_slot;
pub mod match_driver;
pub mod match_driver_fixture;
pub mod match_manifest;
pub mod match_session;
pub mod protocol;
pub mod protocol_conformance;
pub mod protocol_fixture;
