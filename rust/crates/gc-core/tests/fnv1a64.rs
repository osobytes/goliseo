//! Tests FNV-1a-64 against published test vectors, incremental byte updates,
//! and agreement between bulk and byte-at-a-time updates.

use gc_core::fnv1a64::{Fnv1a64State, hash};

#[test]
fn core_fnv1a64_matches_published_fnv1a64_vectors_without_native_integer_support() {
    assert_eq!(hash(b""), "cbf29ce484222325");
    assert_eq!(hash(b"a"), "af63dc4c8601ec8c");
    assert_eq!(hash(b"foobar"), "85944171f73967e8");
    assert_eq!(hash(b"hello"), "a430d84680aabd0b");
}

#[test]
fn core_fnv1a64_supports_incremental_byte_updates() {
    let mut state = Fnv1a64State::new();
    state.update(b"foo");
    state.update(b"bar");
    assert_eq!(state.hex(), hash(b"foobar"));
}

#[test]
fn core_fnv1a64_keeps_bulk_and_byte_at_a_time_updates_identical_across_every_byte() {
    let mut bytes: Vec<u8> = Vec::new();
    let mut incremental = Fnv1a64State::new();
    for byte in 0..=255_u8 {
        bytes.push(byte);
        incremental.update_byte(byte);
    }
    assert_eq!(hash(&bytes), "4242dc5249c33625");
    assert_eq!(incremental.hex(), "4242dc5249c33625");
}

/// `finish()` is the zero-allocation escape hatch for a caller that only
/// wants the `u64` (a cache key, a fingerprint compared with `==`) --
/// `gc_sim::ball_prediction::ball_fingerprint` is exactly that caller. It
/// must report the same bits `hex()` renders, for every published vector,
/// so the two never silently drift into disagreement.
#[test]
fn core_fnv1a64_finish_agrees_with_hex_on_every_published_vector() {
    for input in [b"".as_slice(), b"a", b"foobar", b"hello"] {
        let mut state = Fnv1a64State::new();
        state.update(input);
        assert_eq!(
            format!("{:016x}", state.finish()),
            state.hex(),
            "finish() disagreed with hex() for input {input:?}"
        );
    }
}
