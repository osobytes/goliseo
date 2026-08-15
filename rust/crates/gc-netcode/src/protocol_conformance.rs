//! Golden evidence pinning `crate::protocol_fixture`'s wire, manifest, and
//! transcript digests — frozen cross-language determinism evidence (see
//! `tools/lua_reference/README.md` for provenance). These are the same
//! numbers `tests/protocol.rs`'s differential fixture is checked against.

use gc_core::fnv1a64;

use crate::protocol::{self, MessageKind};
use crate::protocol_fixture as fixture;

/// `SessionProtocolGolden`.
pub struct Golden {
    /// [`protocol::vocabulary_id`], pinned. The vocabulary digest rides in
    /// `build_id`, so a kind, a body field, or a phase rule cannot change
    /// without changing which builds will play each other.
    pub vocabulary_id: &'static str,
    /// [`protocol::manifest_id`] of `fixture::manifest(None)`, pinned.
    pub manifest_id: &'static str,
    /// [`protocol::transcript_id`] of `fixture::messages()`, pinned.
    pub transcript_id: &'static str,
    /// The kind whose complete wire is pinned byte for byte.
    pub complete_kind: MessageKind,
    /// The exact pinned wire for `complete_kind`.
    pub complete_wire: &'static str,
    /// Per-kind `fnv1a64` digest of that kind's encoded fixture wire.
    pub wire_digests: &'static [(MessageKind, &'static str)],
}

/// Repinned by #268: the fixture manifest's `max_goals` moved 5 -> 99 (no
/// goal limit), so `manifest_id` moves and every wire that embeds it moves
/// with it. The digests below that do not carry a manifest id are
/// unchanged, which is the evidence nothing else moved.
///
/// Repinned again by #489, same mechanism as the #268 note above and as
/// `coordinator_conformance::Golden` (retired in the same PR, see that
/// constant's doc comment for the root cause): `match_snapshot::COMBAT_VERSION`
/// bumps 13 -> 14, `manifest_id` and `transcript_id` move, and every wire
/// digest for a kind whose required fields include `manifest_id`
/// (`ManifestProposal`, `ManifestAccept`, `SlotAssignment`, `Ready`,
/// `Countdown`, `Start`, `PairPreference`, `PairPreferenceResult`) moves with
/// it. `Handshake`, `PeerAssignment`, `MatchPhase`, `HashReport`,
/// `ResultAck`, `Abort` and `Disconnect` do not carry a manifest id and are
/// confirmed unchanged below — the same evidence-of-nothing-else-moved shape
/// as #268. Recorded by printing every value from a throwaway probe calling
/// `protocol::manifest_id`/`transcript_id`/`encode` directly against
/// `fixture::manifest(None)`/`fixture::messages()`, the same fixture this
/// golden already pins.
pub const GOLDEN: Golden = Golden {
    vocabulary_id: "e13e3647001a0a7e",
    manifest_id: "572bbff19cdfc603",
    transcript_id: "a83439d3fe39fa52",
    complete_kind: MessageKind::ManifestAccept,
    complete_wire: "GCOP;1;t7:s4:bodyt1:s11:manifest_ids16:572bbff19cdfc603s4:kinds15:\
manifest_accepts10:message_ids32:GCMI;1;13:session_alpha4:host1:2s7:peer_ids4:\
hosts8:sequencei1:2s10:session_ids13:session_alphas7:versioni1:1",
    wire_digests: &[
        (MessageKind::Handshake, "2722abf054051350"),
        (MessageKind::ManifestProposal, "a16e66840b834469"),
        (MessageKind::ManifestAccept, "61cf18d63e6076e9"),
        (MessageKind::PeerAssignment, "fa48b31571dfe543"),
        (MessageKind::SlotAssignment, "956e42e74b995ad9"),
        (MessageKind::Ready, "ab1187cad581ee86"),
        (MessageKind::PairPreference, "6ca49f34edf71a88"),
        (MessageKind::PairPreferenceResult, "c0de33146372fa77"),
        (MessageKind::Countdown, "12b32574af080ab9"),
        (MessageKind::Start, "c1d8945760e1deae"),
        (MessageKind::MatchPhase, "1671940891b78f1f"),
        (MessageKind::HashReport, "4405d9323b1e5b0f"),
        (MessageKind::ResultAck, "5f466e6740c6d4cf"),
        (MessageKind::Abort, "9db9c05e9728c4c1"),
        (MessageKind::Disconnect, "a7599b154bb86cec"),
    ],
};

/// `SessionProtocolConformanceReport`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// [`protocol::manifest_id`] of the fixture manifest.
    pub manifest_id: String,
    /// [`protocol::transcript_id`] of the fixture messages.
    pub transcript_id: String,
    /// How many fixture messages were checked.
    pub message_count: usize,
}

/// `conformance.verify`: checks every pinned golden digest against a fresh
/// encode of the fixture data.
///
/// # Panics
///
/// Panics (via `assert!`) on any mismatch — this is a conformance gate, not
/// a caller-recoverable operation.
#[must_use]
pub fn verify() -> Report {
    let vocabulary_id = protocol::vocabulary_id();
    assert_eq!(
        vocabulary_id, GOLDEN.vocabulary_id,
        "protocol vocabulary golden changed"
    );

    let manifest_id = protocol::manifest_id(&fixture::manifest(None));
    assert_eq!(
        manifest_id, GOLDEN.manifest_id,
        "protocol manifest golden changed"
    );

    let messages = fixture::messages();
    let mut seen: Vec<MessageKind> = Vec::new();
    for message in &messages {
        assert!(
            !seen.contains(&message.kind),
            "protocol fixture repeats a message kind"
        );
        seen.push(message.kind);
        let expected = GOLDEN
            .wire_digests
            .iter()
            .find(|(kind, _)| *kind == message.kind)
            .map(|(_, digest)| *digest)
            .expect("protocol fixture has an unpinned message kind");
        let wire = protocol::encode(message).expect("fixture message must encode");
        let actual = fnv1a64::hash(wire.as_bytes());
        assert_eq!(
            actual,
            expected,
            "{} protocol wire golden changed: expected {expected}, got {actual}",
            message.kind.wire_str()
        );
        if message.kind == GOLDEN.complete_kind {
            assert_eq!(
                wire, GOLDEN.complete_wire,
                "complete protocol wire golden changed"
            );
        }
    }
    let golden_count = GOLDEN.wire_digests.len();
    for (kind, _) in GOLDEN.wire_digests {
        assert!(
            seen.contains(kind),
            "protocol golden has no conformance message for {}",
            kind.wire_str()
        );
    }
    assert_eq!(
        golden_count,
        messages.len(),
        "protocol message/golden count differs"
    );

    let decoded = protocol::decode(GOLDEN.complete_wire).expect("complete wire must decode");
    assert_eq!(
        decoded.kind, GOLDEN.complete_kind,
        "complete wire kind changed"
    );
    assert_eq!(
        protocol::encode(&decoded).expect("decoded complete wire must re-encode"),
        GOLDEN.complete_wire,
        "complete wire no longer decodes canonically"
    );

    let transcript_id = protocol::transcript_id(&messages);
    assert_eq!(
        transcript_id, GOLDEN.transcript_id,
        "protocol transcript golden changed"
    );

    Report {
        manifest_id,
        transcript_id,
        message_count: messages.len(),
    }
}

/// `conformance.marker`: the browser evidence parser compares this marker's
/// field set exactly, so the vocabulary pin deliberately stays out of it.
#[must_use]
pub fn marker(report: &Report) -> String {
    format!(
        "GC_PROTOCOL|golden|schema=1|manifest_id={}|transcript_id={}|messages={}",
        report.manifest_id, report.transcript_id, report.message_count
    )
}
