//! RenderFrame producer: the wasm side of the render boundary.
//!
//! Every module is declared up front, so agents working on different parts
//! of `render/` never contend on this file.
#![deny(missing_docs)]

pub mod frame;
pub mod frame_buffer;
pub mod identity;
pub mod player_pose;
