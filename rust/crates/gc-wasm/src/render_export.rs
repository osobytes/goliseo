//! The per-frame render path: RAW `extern "C"` exports, deliberately with
//! no `wasm-bindgen` involved.
//!
//! `gc_render::frame_buffer`'s own doc explains why the render boundary
//! must cross once per rendered frame, in batch, as one flat `Vec<f64>`
//! block: rollback can resimulate up to eight ticks inside a single
//! rendered frame, so a per-field or per-entity crossing here is exactly
//! the cost that block format exists to avoid. Routing that block through
//! `wasm-bindgen` would reintroduce a version of the same problem one
//! level up — JS-glue allocation and copy on every call — so this module
//! exports three plain `u32`-in/`u32`-out functions instead, and JS reads
//! the result straight out of linear memory.
//!
//! ## The three-call contract
//!
//! 1. `render_frame_build(handle, kick_follow_slots)` — builds this tick's
//!    [`RenderFrame`] for the session named by `handle` (see
//!    [`crate::registry`]) and encodes it into a reused, thread-local
//!    buffer. Returns `1` on success, `0` if `handle` names no live session.
//!    `kick_follow_slots` is the renderer's own release follow-through
//!    window, as a roster-slot bitmask (`0` for "nothing is following
//!    through"). A plain scalar, which is exactly why it may ride this path
//!    at all — see [`crate::session::kick_follow_ids`] for why the ids
//!    themselves do not cross.
//! 2. `render_frame_ptr()` — byte offset into this module's exported
//!    `memory` where the block built by the last `render_frame_build` call
//!    starts.
//! 3. `render_frame_len()` — the block's length in `f64` elements (not
//!    bytes) — pass both to `new Float64Array(memory.buffer, ptr, len)` on
//!    the JS side.
//!
//! Call `render_frame_build` again before every read; the pointer is only
//! valid for the block most recently built (the buffer is reused, and can
//! move if it has to grow). Decode the block with the same field order and
//! version numbers `gc_render::frame_buffer::decode` documents — this
//! module does not re-describe that layout, it only gets the bytes there.
//!
//! ## The `MatchDriverBridge` half
//!
//! [`crate::match_driver_bridge::MatchDriverBridge`] is not
//! `crate::registry`-backed the way [`crate::session::Session`] is — it owns
//! its `gc_netcode::match_driver::MatchDriver` (and therefore the live
//! `MatchState` a render frame needs) directly as a struct field, not via a
//! registry-issued `u32` handle. A raw `extern "C"` export can only take
//! FFI-safe scalars, so there is no way to hand one "which bridge" the way
//! [`render_frame_build`] is handed "which session": reinterpreting a
//! `wasm-bindgen`-managed pointer as a raw `*mut MatchDriverBridge` would
//! work in practice but relies on that crate's internal object
//! representation rather than a contract it publishes, and this crate's
//! `unsafe_code` opt-out (`Cargo.toml`) is scoped to the two
//! `#[unsafe(no_mangle)]` attributes below, not to new raw-pointer code — see
//! that file's comment for why widening it is off the table.
//!
//! [`crate::match_driver_bridge::MatchDriverBridge::render_frame_build`]
//! resolves this the other way: it is an ordinary bound `wasm-bindgen`
//! method (`&mut self`, returning a bare `u32`) rather than a free function.
//! That costs nothing extra on the per-frame path — `wasm-bindgen` passes an
//! exported class's own pointer to a method call the same cheap way this
//! module's `handle: u32` is passed to a free function; the expensive thing
//! this module's whole design avoids is `wasm-bindgen` marshalling the
//! ENCODED FRAME BLOCK itself (a `Vec<f64>` boxed and copied element by
//! element into a JS array), not the O(1), scalar-only trigger call. So the
//! actual per-frame payload still crosses raw memory, unmarshalled, exactly
//! like the `Session` path: [`MatchDriverBridge::render_frame_build`] encodes
//! into [`DRIVER_FRAME_BUFFER`] below (a second, separate reused buffer —
//! kept apart from [`FRAME_BUFFER`] so a caller building both a `Session`'s
//! and a driver's frame in the same tick cannot have one silently overwrite
//! the other before it is read), and [`driver_render_frame_ptr`]/
//! [`driver_render_frame_len`] are the same kind of raw, `wasm-bindgen`-free
//! `extern "C"` reads [`render_frame_ptr`]/[`render_frame_len`] already are.
//!
//! [`MatchDriverBridge::render_frame_build`]: crate::match_driver_bridge::MatchDriverBridge::render_frame_build
//! [`RenderFrame`]: gc_render::frame::RenderFrame

use std::cell::RefCell;

use gc_render::{frame, frame_buffer};

use crate::registry;
use crate::session::frame_options;

thread_local! {
    static FRAME_BUFFER: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    /// [`MatchDriverBridge::render_frame_build`]'s own reused buffer — see
    /// the module doc's "The `MatchDriverBridge` half" section for why this
    /// is a second buffer rather than sharing [`FRAME_BUFFER`].
    ///
    /// [`MatchDriverBridge::render_frame_build`]: crate::match_driver_bridge::MatchDriverBridge::render_frame_build
    static DRIVER_FRAME_BUFFER: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

/// Build the current render frame for the session named by `handle` into
/// the shared thread-local buffer. Returns `1` on success, `0` if `handle`
/// names no live session (a stale or already-freed [`crate::session::Session`]).
///
/// `kick_follow_slots`: the renderer's release follow-through window as a
/// roster-slot bitmask — see the module doc's three-call contract and
/// [`crate::session::kick_follow_ids`]. Pass `0` when no window is open.
#[unsafe(no_mangle)]
pub extern "C" fn render_frame_build(handle: u32, kick_follow_slots: u32) -> u32 {
    let built = registry::with_entry(handle, |entry| {
        let options = frame_options(entry, kick_follow_slots);
        let render_frame = frame::build(&entry.state, &options);
        FRAME_BUFFER.with(|buf| {
            let mut buf = buf.borrow_mut();
            buf.clear();
            frame_buffer::encode(&render_frame, &mut buf);
        });
    });
    u32::from(built.is_some())
}

/// Byte offset into this module's linear memory where the block built by
/// the last [`render_frame_build`] call starts. `0` (a real but always-empty
/// address in a fresh module, since nothing meaningful is ever placed at
/// offset zero this early) if no block has been built yet.
#[unsafe(no_mangle)]
pub extern "C" fn render_frame_ptr() -> u32 {
    FRAME_BUFFER.with(|buf| buf.borrow().as_ptr() as u32)
}

/// Length, in `f64` elements, of the block built by the last
/// [`render_frame_build`] call.
#[unsafe(no_mangle)]
pub extern "C" fn render_frame_len() -> u32 {
    FRAME_BUFFER.with(|buf| buf.borrow().len() as u32)
}

/// Encodes `render_frame` into [`DRIVER_FRAME_BUFFER`], for
/// [`crate::match_driver_bridge::MatchDriverBridge::render_frame_build`] —
/// see the module doc's "The `MatchDriverBridge` half" section. Not `pub`:
/// the bridge is this buffer's only writer.
pub(crate) fn build_driver_frame(render_frame: &gc_render::frame::RenderFrame) {
    DRIVER_FRAME_BUFFER.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        frame_buffer::encode(render_frame, &mut buf);
    });
}

/// Byte offset into this module's linear memory where the block built by the
/// last [`crate::match_driver_bridge::MatchDriverBridge::render_frame_build`]
/// call starts. `0` (see [`render_frame_ptr`]'s identical note) if no block
/// has been built yet.
#[unsafe(no_mangle)]
pub extern "C" fn driver_render_frame_ptr() -> u32 {
    DRIVER_FRAME_BUFFER.with(|buf| buf.borrow().as_ptr() as u32)
}

/// Length, in `f64` elements, of the block built by the last
/// [`crate::match_driver_bridge::MatchDriverBridge::render_frame_build`]
/// call.
#[unsafe(no_mangle)]
pub extern "C" fn driver_render_frame_len() -> u32 {
    DRIVER_FRAME_BUFFER.with(|buf| buf.borrow().len() as u32)
}
