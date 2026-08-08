//! FNV-1a-64 hashing, used to detect rollback desyncs by hashing canonical
//! match snapshots.
//!
//! Numeric-type note: the Lua source carries the 64-bit hash as two unsigned
//! 32-bit limbs (`hi`/`lo`) with a hand-written nibble-XOR table, because
//! LuaJIT/love.js cannot rely on native 64-bit integers and every
//! intermediate has to stay an exactly representable `f64`. Rust's `u64` is a
//! real, portable 64-bit unsigned integer with well-defined wraparound
//! (`wrapping_mul`/`^` implement arithmetic mod 2^64 exactly), so the
//! two-limb emulation has no counterpart to preserve here — it was working
//! around a limitation `u64` does not have.
//!
//! This is not merely "close enough": `OFFSET_HI`/`OFFSET_LO` from the Lua
//! source (`3421674724`, `2216829733`) are `0xcbf29ce4` and `0x84222325`, the
//! two halves of the standard FNV-1a-64 offset basis `0xcbf29ce484222325`,
//! and `PRIME_LOW = 435` is the low bits of the standard prime
//! `2^40 + 435 = 0x100000001b3`. Both implementations are exact
//! realizations of the same published FNV-1a-64 spec, so `u64` arithmetic and
//! the Lua limb emulation are required to agree on every input — confirmed
//! against the published test vectors in `tests/fnv1a64.rs`, including
//! `hash("a") == "af63dc4c8601ec8c"`, a canonical FNV-1a-64 vector.

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
    /// for chaining, mirroring the Lua source's mutate-and-return `update`.
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
}

/// One-shot FNV-1a-64 hash of `bytes`, rendered as lowercase hex.
#[must_use]
pub fn hash(bytes: &[u8]) -> String {
    let mut state = Fnv1a64State::new();
    state.update(bytes);
    state.hex()
}
