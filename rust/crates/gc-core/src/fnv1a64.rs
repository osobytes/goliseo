//! FNV-1a-64 hashing, used to detect rollback desyncs by hashing canonical
//! match snapshots.
//!
//! `OFFSET_BASIS` (`0xcbf29ce484222325`) and `PRIME` (`2^40 + 435`, i.e.
//! `0x100000001b3`) are the standard FNV-1a-64 offset basis and prime.
//! Rust's `u64` has well-defined wraparound (`wrapping_mul`/`^` implement
//! arithmetic mod 2^64 exactly), so this is an exact realization of the
//! published FNV-1a-64 spec — confirmed against the published test vectors
//! in `tests/fnv1a64.rs`, including `hash("a") == "af63dc4c8601ec8c"`, a
//! canonical FNV-1a-64 vector. The TypeScript implementation
//! (`packages/online/src/diagnostics_schema.ts`) must agree with this one
//! bit-for-bit: a match snapshot hash mismatch between host and guest is
//! exactly what desync detection is looking for.

/// The standard FNV-1a-64 offset basis.
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// The standard FNV-1a-64 prime (`2^40 + 435`).
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Incremental FNV-1a-64 hash state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fnv1a64State(u64);

impl Default for Fnv1a64State {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a64State {
    /// A fresh hasher, seeded at the FNV-1a-64 offset basis.
    #[must_use]
    pub fn new() -> Self {
        Fnv1a64State(OFFSET_BASIS)
    }

    /// Fold a single byte into the state.
    pub fn update_byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(PRIME);
    }

    /// Fold a byte slice into the state. Mutates in place and returns `self`
    /// for chaining.
    pub fn update(&mut self, bytes: &[u8]) -> &mut Self {
        for &byte in bytes {
            self.update_byte(byte);
        }
        self
    }

    /// Render the state as a lowercase, zero-padded 16-hex-digit string.
    #[must_use]
    pub fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }

    /// The raw accumulated state, with no string round trip.
    ///
    /// For a caller that wants the `u64` and nothing else -- a cache key, a
    /// fingerprint compared with `==` -- `hex()` followed by
    /// `u64::from_str_radix` is a `String` allocation and a parse to recover
    /// a value this type already holds. `finish()` is that shortcut: same
    /// bits `hex()` would render, zero allocation, named after
    /// `std::hash::Hasher::finish` for the same reason that method exists.
    #[must_use]
    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// One-shot FNV-1a-64 hash of `bytes`, rendered as lowercase hex.
#[must_use]
pub fn hash(bytes: &[u8]) -> String {
    let mut state = Fnv1a64State::new();
    state.update(bytes);
    state.hex()
}
