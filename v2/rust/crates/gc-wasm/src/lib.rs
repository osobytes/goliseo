//! wasm32-unknown-unknown bindings for the Galactic Cup simulation.
//!
//! This crate is glue, not simulation: it owns zero gameplay math. Its only
//! job is to get bits across the wasm boundary faithfully, in both
//! directions, at the right granularity for each direction's call
//! frequency.
//!
//! ## Two binding strategies, deliberately different
//!
//! - [`session`], [`determinism`] and [`protocol_bridge`] are the CONTROL
//!   SURFACE: session lifecycle, the determinism acceptance check, and lobby
//!   wire helpers. These are called rarely (session setup, once per lobby
//!   message, once at startup) so they use `wasm-bindgen`, trading a little
//!   per-call overhead for ergonomic, type-checked JS bindings generated
//!   automatically.
//!
//! - [`render_export`] is the PER-FRAME HOT PATH: up to eight resimulated
//!   ticks folded into one rendered frame, every rendered frame, for the
//!   lifetime of a match. `wasm-bindgen` glue allocates and marshals on
//!   every call, which is the wrong cost to pay here (`gc-render`'s own
//!   `frame_buffer` module already exists specifically to cross this
//!   boundary ONCE per frame as a flat block, and wasm-bindgen glue would
//!   undo that). So this module exports raw `extern "C"` functions instead:
//!   no `wasm-bindgen` macro, no per-call JS shim. JS reads the result
//!   directly out of linear memory as a `Float64Array` view, using the
//!   pointer and length the export hands back. See that module's doc for
//!   the exact contract.
//!
//! ## Why gc-netcode is a dependency, and what of it is actually bound
//!
//! `gc-netcode`'s deterministic reducer types (`coordinator::CoordinatorState`,
//! `coordinator::Event`, `protocol::Value`) carry no `serde` impls — and
//! this crate does not add any, because `crates/gc-netcode/**` is not a file
//! this task owns (see the worktree's per-wave file-ownership split).
//! Hand-writing a field-by-field `JsValue` bridge for every variant of that
//! reducer is a substantial task in its own right, not a two-line binding,
//! and it is not what this wave's acceptance test needs. So the lobby/
//! coordinator surface bound here ([`protocol_bridge`]) is deliberately
//! narrow: the pieces of `gc_netcode::protocol` that are ALREADY textual
//! (wire encode/decode, the vocabulary digest) and therefore bind cleanly
//! with no new serialization code. Wrapping the full coordinator reducer is
//! flagged as follow-up in this crate's own report, not silently skipped.
#![deny(missing_docs)]

pub mod determinism;
pub mod protocol_bridge;
pub mod registry;
pub mod render_export;
pub mod session;
