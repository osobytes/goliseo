//! Frozen conformance vectors for the input-protocol wire format.
//!
//! The wire literals embed `manifest_id`, a hash over the session manifest,
//! which carries the snapshot and combat schema versions. Ideally this
//! module would check `match_snapshot::VERSION`/`match_snapshot::COMBAT_VERSION`
//! against [`Golden::snapshot_version`]/[`Golden::combat_version`] before
//! trusting the golden literals, so a stale golden reports which version it
//! was built for instead of an opaque byte mismatch. [`verify`] does not
//! currently perform that cross-check; the pinned `snapshot_version`/
//! `combat_version` fields are carried for parity and documentation only.
//! Everything else — the wire literals, the digests, the maximal packet's
//! measured size and margin — is fully checked.

use crate::input_protocol::{self, MAX_HOST_ROWS, MAX_WIRE_BYTES, MIN_WIRE_MARGIN_BYTES};
use crate::input_protocol_fixture as fixture;
use gc_core::fnv1a64;
use gc_sim::input_frame;

/// Pinned native and love.js conformance vectors. See the module doc comment
/// for why the version fields are not cross-checked against a live
/// `match_snapshot` here.
pub struct Golden {
    /// `sim.match_snapshot.VERSION` this golden was generated against.
    pub snapshot_version: i64,
    /// `sim.match_snapshot.COMBAT_VERSION` this golden was generated against.
    pub combat_version: i64,
    /// Canonical wire bytes for [`fixture::guest`].
    pub guest_wire: &'static [u8],
    /// `fnv1a64` digest of [`Golden::guest_wire`].
    pub guest_digest: &'static str,
    /// Canonical wire bytes for [`fixture::host`].
    pub host_wire: &'static [u8],
    /// `fnv1a64` digest of [`Golden::host_wire`].
    pub host_digest: &'static str,
    /// Encoded byte length of [`fixture::maximal`].
    pub maximal_wire_bytes: usize,
    /// Spare bytes [`fixture::maximal`] leaves under [`MAX_WIRE_BYTES`].
    pub maximal_wire_margin: usize,
}

/// The pinned golden vectors.
pub const GOLDEN: Golden = Golden {
    snapshot_version: 14,
    combat_version: 15,
    // The embedded manifest id moved with #268's `max_goals` 5 -> 99 (no goal
    // limit). The packet payloads either side of it are byte-identical, and
    // `maximal_wire_bytes` does not move: `fixture::maximal` carries a
    // manifest id of its own.
    guest_wire: b"GCIP;1;G;2;eb59f113614c35b2;7;f6f6f9dbe278dccb;12;0;4;3;7;\
AAAAAAJ/fwAAAAAAAQIA/gUJAAAAAgJ/f4AgAAAAAwJ/f4AAAAAABAJ/fwBAAAAABQJ/f38f\
AAAABgL+ABIW",
    guest_digest: "a099c86d6520d6bc",
    host_wire: b"GCIP;1;H;2;eb59f113614c35b2;13;65c65955c65cc80a;15;0;5;3;16;\
AAAABQGwTgAAAAAABQKvTwEBAAAABQOuUAICAAAABQStUQMDAAAABQWsUgQEAAAABQarUwUF\
AAAABQeqVAYGAAAABQipVYAgAAAABgG6RAAAAAAABgK5RQEBAAAABgO4RgICAAAABgS3RwMD\
AAAABgW2SAQEAAAABga1SQUFAAAABge0SgYGAAAABgizS4Ag",
    host_digest: "fb46b26858818ead",
    maximal_wire_bytes: 958,
    maximal_wire_margin: 66,
};

/// The measured output of [`verify`].
pub struct Report {
    /// `fnv1a64` digest of the re-encoded guest fixture.
    pub guest_digest: String,
    /// `fnv1a64` digest of the re-encoded host fixture.
    pub host_digest: String,
    /// Encoded byte length of the re-encoded maximal fixture.
    pub maximal_wire_bytes: usize,
    /// Spare bytes the maximal fixture leaves under [`MAX_WIRE_BYTES`].
    pub maximal_wire_margin: usize,
    /// Number of pinned wire vectors checked.
    pub vector_count: i64,
}

/// Re-derive the guest, host, and maximal fixtures, check them byte-for-byte
/// against [`GOLDEN`], and report the measurements.
///
/// # Panics
///
/// Panics if any fixture no longer encodes to its pinned golden: a stale
/// golden is a build-time invariant violation, not caller input, so it
/// asserts rather than returning an error.
#[must_use]
pub fn verify() -> Report {
    let guest = fixture::guest();
    let guest_wire = input_protocol::encode(&guest).expect("fixture guest packet must encode");
    assert!(
        guest_wire == GOLDEN.guest_wire,
        "guest input packet golden changed"
    );
    let guest_digest = fnv1a64::hash(&guest_wire);
    assert!(
        guest_digest == GOLDEN.guest_digest,
        "guest input packet digest changed"
    );
    let decoded_guest = input_protocol::decode(
        GOLDEN.guest_wire,
        &input_protocol::DecodeContext {
            session_id: guest.session_id.clone(),
            manifest_id: guest.manifest_id.clone(),
            sender_id: guest.sender_id.clone(),
        },
    )
    .expect("golden guest wire must decode");
    assert!(
        input_protocol::encode(&decoded_guest).as_deref() == Ok(GOLDEN.guest_wire),
        "guest input literal no longer round-trips"
    );

    let host = fixture::host();
    let host_wire = input_protocol::encode(&host).expect("fixture host packet must encode");
    assert!(
        host_wire == GOLDEN.host_wire,
        "host input packet golden changed"
    );
    let host_digest = fnv1a64::hash(&host_wire);
    assert!(
        host_digest == GOLDEN.host_digest,
        "host input packet digest changed"
    );
    let decoded_host = input_protocol::decode(
        GOLDEN.host_wire,
        &input_protocol::DecodeContext {
            session_id: host.session_id.clone(),
            manifest_id: host.manifest_id.clone(),
            sender_id: host.sender_id.clone(),
        },
    )
    .expect("golden host wire must decode");
    assert!(
        input_protocol::encode(&decoded_host).as_deref() == Ok(GOLDEN.host_wire),
        "host input literal no longer round-trips"
    );

    let maximal = fixture::maximal();
    let maximal_wire =
        input_protocol::encode(&maximal).expect("fixture maximal packet must encode");
    assert!(
        maximal.rows.len() as i64 == MAX_HOST_ROWS,
        "maximal input row count changed"
    );
    assert!(
        maximal_wire.len() == GOLDEN.maximal_wire_bytes,
        "maximal input packet size changed"
    );
    assert!(
        maximal_wire.len() <= MAX_WIRE_BYTES,
        "maximal input packet exceeds its transport bound"
    );
    // The margin is pinned, not merely non-negative. #243 sized `MAX_HOST_ROWS`
    // against this budget, and a bound sized to the exact edge is a bound the
    // next additive header field silently breaks. This says how much slack the
    // sizing deliberately left, and fails with the number if it is spent.
    let margin = MAX_WIRE_BYTES - maximal_wire.len();
    assert!(
        margin >= MIN_WIRE_MARGIN_BYTES,
        "maximal input packet leaves {margin} spare bytes, below the declared {MIN_WIRE_MARGIN_BYTES}-byte margin"
    );
    assert!(
        margin == GOLDEN.maximal_wire_margin,
        "maximal input packet wire margin changed"
    );
    assert!(
        maximal.input_version == input_frame::VERSION,
        "input conformance fixture uses the wrong sample version"
    );
    Report {
        guest_digest,
        host_digest,
        maximal_wire_bytes: maximal_wire.len(),
        maximal_wire_margin: margin,
        vector_count: 2,
    }
}

/// A single-line marker string summarizing a [`Report`], suitable for a CI
/// log line.
#[must_use]
pub fn marker(report: &Report) -> String {
    format!(
        "GC_INPUT_PROTOCOL|golden|schema={}|input={}|history={}|delay={}|vectors={}\
|guest={}|host={}|host_rows={}|max_bytes={}|margin={}",
        input_protocol::VERSION,
        input_frame::VERSION,
        input_protocol::HISTORY_ROWS,
        input_protocol::FAIRNESS_DELAY_TICKS,
        report.vector_count,
        report.guest_digest,
        report.host_digest,
        MAX_HOST_ROWS,
        report.maximal_wire_bytes,
        report.maximal_wire_margin,
    )
}
