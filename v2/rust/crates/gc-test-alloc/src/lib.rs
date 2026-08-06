//! A counting `#[global_allocator]`, wrapping `std::alloc::System`, for
//! measuring per-call allocation from a `cargo test` integration-test
//! binary.
//!
//! This exists only because that measurement needs `unsafe impl
//! GlobalAlloc`, which the workspace forbids per-package everywhere except
//! here (see this crate's `Cargo.toml`). It contains the entire `unsafe`
//! surface this measurement needs and nothing else: `gc-sim` and every
//! other product crate keep `unsafe_code = "forbid"` in force.
//!
//! # What this measures, exactly
//!
//! [`CountingAllocator::allocated`] reports **gross bytes requested from the
//! allocator** since the last [`CountingAllocator::start`] — every `alloc`
//! call's `Layout::size`, plus the growth portion of every `realloc` call
//! that grows. It does **not** subtract bytes freed by `dealloc`, and a
//! `realloc` that shrinks is not credited back. That is deliberately the
//! same shape as the Lua original's measurement technique
//! (`collectgarbage("count")` deltas with the collector stopped, so nothing
//! is reclaimed mid-measurement, `spec/sim/env_budget_spec.lua`): both count
//! what was asked for, not net memory pressure at the end.
//!
//! # Not thread-scoped
//!
//! The counter is one process-wide [`AtomicUsize`]. That is fine for a
//! single-threaded `cargo test` binary measuring one call at a time, and
//! actively wrong if anything else in the same process allocates
//! concurrently with the code under measurement — there is no per-thread or
//! per-scope isolation here, honestly, on purpose: this is a small
//! measurement helper, not a profiler.
//!
//! # Usage
//!
//! ```ignore
//! use gc_test_alloc::CountingAllocator;
//!
//! #[global_allocator]
//! static ALLOC: CountingAllocator = CountingAllocator;
//!
//! #[test]
//! fn stays_within_budget() {
//!     ALLOC.start();
//!     let _ = do_the_thing();
//!     let bytes = ALLOC.allocated();
//!     assert!(bytes < some_ceiling, "allocated {bytes} bytes");
//! }
//! ```
//!
//! A `#[global_allocator]` declared inside an integration-test file (a file
//! directly under `tests/`) applies to that test binary alone — each file
//! under `tests/` compiles to its own binary by default — so this never
//! touches `gc-sim`'s library build or any other test binary.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

/// A [`GlobalAlloc`] that forwards every call to [`System`] unchanged and
/// additionally counts bytes requested. See the module doc comment for
/// exactly what is and is not counted.
#[derive(Clone, Copy, Debug, Default)]
pub struct CountingAllocator;

// SAFETY: Every method below forwards to `System`'s implementation of the
// same method, with the same pointer/layout/size arguments the caller gave
// us, unmodified. The only addition is a non-negative counter update
// alongside the forwarded call. This introduces no new unsafety: it never
// fabricates a pointer, never reads or writes memory itself, and never
// changes what is passed to the underlying allocator. `System` is `std`'s
// own `GlobalAlloc` implementation, so delegating to it entirely preserves
// whatever safety guarantees it already upholds.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: forwards to `System::alloc` with the exact `Layout` the
        // caller gave `Self::alloc`; upholds the same preconditions.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwards to `System::dealloc` with the exact pointer and
        // `Layout` the caller gave `Self::dealloc`; upholds the same
        // preconditions (in particular, `ptr` must have come from a prior
        // `alloc`/`realloc` with this same layout, which is the caller's
        // obligation under `GlobalAlloc`, not this wrapper's).
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            ALLOCATED.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        // SAFETY: forwards to `System::realloc` with the exact pointer,
        // `Layout`, and new size the caller gave `Self::realloc`; upholds
        // the same preconditions.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

impl CountingAllocator {
    /// Resets the counter to zero. Call immediately before the code under
    /// measurement, after any warm-up allocations you don't want counted.
    pub fn start(&self) {
        ALLOCATED.store(0, Ordering::SeqCst);
    }

    /// Gross bytes requested from the allocator since the last
    /// [`Self::start`]. See the module doc comment for exactly what counts.
    #[must_use]
    pub fn allocated(&self) -> usize {
        ALLOCATED.load(Ordering::SeqCst)
    }
}
