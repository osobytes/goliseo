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
//! 1. `render_frame_build(handle)` — builds this tick's [`RenderFrame`] for
//!    the session named by `handle` (see [`crate::registry`]) and encodes
//!    it into a reused, thread-local buffer. Returns `1` on success, `0` if
//!    `handle` names no live session.
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
//! [`RenderFrame`]: gc_render::frame::RenderFrame

use std::cell::RefCell;

use gc_render::{frame, frame_buffer};

use crate::registry;
use crate::session::frame_options;

thread_local! {
    static FRAME_BUFFER: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

/// Build the current render frame for the session named by `handle` into
/// the shared thread-local buffer. Returns `1` on success, `0` if `handle`
/// names no live session (a stale or already-freed [`crate::session::Session`]).
#[unsafe(no_mangle)]
pub extern "C" fn render_frame_build(handle: u32) -> u32 {
    let built = registry::with_entry(handle, |entry| {
        let options = frame_options(entry);
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
