//! Pins the coordinator's session-shaped output — transcript identity, wire
//! trace, and slot ownership — against golden values frozen once from this
//! game's original implementation before it was retired (see
//! `tools/lua_reference/README.md` for how this codebase captures and
//! treats this class of evidence elsewhere). Because [`crate::protocol`]'s
//! wire codec is meant to produce byte-identical output to that retired
//! reference, these golden constants (copied verbatim from it) are
//! themselves a differential check: if this module's [`verify`] reproduces
//! them, this crate's `coordinator`/`protocol`/`live_slot` combination
//! still agrees with that frozen reference on a full session, not just on
//! unit-level assertions. A disagreement here is a finding about this
//! crate, not a stale golden value to update.

use crate::coordinator::{self, TerminalReason};
use crate::coordinator_driver::{self as driver, Driver};
use crate::protocol;
use gc_core::fnv1a64;
use gc_sim::input_frame;

/// Golden values captured once from this game's retired original
/// implementation; see the module doc comment.
///
/// The four transcript ids below digest sessions built on
/// `protocol_fixture::manifest`, whose `max_goals` #268 took 5 -> 99 (no goal
/// limit), so the manifest id every session message carries moved and the
/// transcripts moved with it. `full_trace_digest`, the message count, and
/// every ownership golden are unchanged — the trace records only
/// "peer>peer kind" and seating never reads the goal limit — which is the
/// evidence that no coordinator behaviour moved.
///
/// #489 moved the four transcript ids a second time, for the same
/// mechanical reason as #268 rather than any coordinator behaviour change:
/// `protocol::validate_manifest` embeds `match_snapshot::COMBAT_VERSION` as
/// the manifest's `snapshot_version` field on every session, and #489 bumps
/// that constant 13 -> 14 for the new `action` field on every serialized
/// `MatchPlayer` (see `gc_sim::match_snapshot`'s module doc). This is the
/// same schema-coupled situation `tools/lua_reference/README.md`'s third
/// axis documents for `match_snapshot_case_a/b_lua_reference.txt`,
/// `match_driver_lua_reference.txt` and `input_tape_lua_reference.txt`
/// (all retired under #536) — a version word is on the wire, and the golden
/// hash of anything containing it moves with it, with no coordinator logic
/// change involved. This file's `GOLDEN` is a Rust-embedded copy of a
/// `*_lua_reference.txt`-class golden rather than a separate fixture, but
/// the same rule applies: only the four transcript ids below (which encode
/// the whole manifest) moved; `full_trace_digest`, `full_message_count` and
/// every `*_sources`/`*_owned` golden are unaffected, confirming no
/// coordinator behaviour moved this time either. Verified by reverting only
/// `gc-sim`/`gc-data` to `b53a5c0` (this branch's merge base, where this
/// file's test passes) inside this same worktree and reproducing the four
/// new values from a live `session()` call before writing them below.
///
/// Repinned again by #490 (`match_snapshot::COMBAT_VERSION` 14 -> 15,
/// `MatchPlayer::keeper_fatigue`) for exactly the same reason and with
/// exactly the same shape: the four transcript ids moved,
/// `full_trace_digest`, `full_message_count` and every `*_sources`/`*_owned`
/// golden did not. That asymmetry is the evidence -- a coordinator behaviour
/// change would have moved the trace and the seating, and neither did.
///
/// 2026-08-26: repinned a third time by the pass-reception rework
/// (`match_snapshot::COMBAT_VERSION` 15 -> 16, the new
/// `MatchPlayer::receive_target` and `MatchState::stick_latch` fields), same
/// mechanism and same shape as #268/#489/#490 above: only the four
/// transcript ids moved; `full_trace_digest`, `full_message_count` and every
/// `*_sources`/`*_owned` golden reproduced unchanged, confirming no
/// coordinator behaviour moved. Re-derived by calling `session(...)` (and
/// `sources`/`owned`) directly for all five cases from this build and
/// diffing against the values below.
pub struct Golden {
    /// [`Driver::transcript_id`] of the canonical 4v4 (3 guests) session.
    pub full_transcript_id: &'static str,
    /// `fnv1a64` digest of [`Driver::trace`] for the canonical session.
    pub full_trace_digest: &'static str,
    /// [`Driver::transcript`] length for the canonical session.
    pub full_message_count: usize,
    /// [`Driver::transcript_id`] of a bot-filled (2 guest) session.
    pub bot_transcript_id: &'static str,
    /// [`sources`] of the bot-filled session.
    pub bot_sources: &'static str,
    /// [`sources`] of a fully bot-filled (0 guest) session.
    pub solo_sources: &'static str,
    /// [`Driver::transcript_id`] of a full 1v1: two humans, four owned slots each.
    pub duo_transcript_id: &'static str,
    /// [`sources`] of the 1v1 session.
    pub duo_sources: &'static str,
    /// [`owned`] of the 1v1 session.
    pub duo_owned: &'static str,
    /// [`Driver::transcript_id`] of a full 2v2: four humans, two owned slots each.
    pub pair_transcript_id: &'static str,
    /// [`sources`] of the 2v2 session.
    pub pair_sources: &'static str,
    /// [`owned`] of the 2v2 session.
    pub pair_owned: &'static str,
    /// [`owned`] of the canonical 4v4 session.
    pub full_owned: &'static str,
}

/// The pinned golden values (see [`Golden`]).
pub const GOLDEN: Golden = Golden {
    full_transcript_id: "26675833586fe78d",
    full_trace_digest: "2fa7235df619b00a",
    full_message_count: 51,
    bot_transcript_id: "21f2ea240a71638a",
    bot_sources: "home_1=peer:host:nil|home_2=peer:guest.1:nil|home_3=peer:guest.2:nil|\
home_4=bot:bot.home_4:51677|away_1=bot:bot.away_1:59596|away_2=bot:bot.away_2:67515|\
away_3=bot:bot.away_3:75434|away_4=bot:bot.away_4:83353",
    solo_sources: "home_1=peer:host:nil|home_2=bot:bot.home_2:35839|home_3=bot:bot.home_3:43758|\
home_4=bot:bot.home_4:51677|away_1=bot:bot.away_1:59596|away_2=bot:bot.away_2:67515|\
away_3=bot:bot.away_3:75434|away_4=bot:bot.away_4:83353",
    duo_transcript_id: "1c30ed5b564c9b72",
    duo_sources: "home_1=peer:host:nil|home_2=peer:host:nil|home_3=peer:host:nil|\
home_4=peer:host:nil|away_1=peer:guest.1:nil|away_2=peer:guest.1:nil|\
away_3=peer:guest.1:nil|away_4=peer:guest.1:nil",
    duo_owned: "host=[home_1,home_2,home_3,home_4]>home_1|guest.1=[away_1,away_2,away_3,away_4]>away_1",
    pair_transcript_id: "c25cc5d317eea685",
    pair_sources: "home_1=peer:host:nil|home_2=peer:host:nil|home_3=peer:guest.1:nil|\
home_4=peer:guest.1:nil|away_1=peer:guest.2:nil|away_2=peer:guest.2:nil|\
away_3=peer:guest.3:nil|away_4=peer:guest.3:nil",
    pair_owned: "host=[home_1,home_2]>home_1|guest.1=[home_3,home_4]>home_3|\
guest.2=[away_1,away_2]>away_1|guest.3=[away_3,away_4]>away_3",
    full_owned: "host=[home_1]>home_1|guest.1=[home_2]>home_2|guest.2=[home_3]>home_3|\
guest.3=[home_4]>home_4|guest.4=[away_1]>away_1|guest.5=[away_2]>away_2|\
guest.6=[away_3]>away_3|guest.7=[away_4]>away_4",
};

/// A flat, reviewable rendering of who owns each canonical slot at freeze.
///
/// # Panics
///
/// Panics if `session` never froze.
#[must_use]
pub fn sources(session: &Driver) -> String {
    let freeze = session
        .host()
        .state
        .freeze
        .as_ref()
        .expect("the session never froze");
    let mut parts = Vec::with_capacity(input_frame::SLOT_COUNT as usize);
    for index in 1..=input_frame::SLOT_COUNT {
        let slot = input_frame::slot(index).expect("canonical slot index");
        let producer = coordinator::freeze_source(freeze, slot.id);
        let bot_seed = coordinator::producer_bot_seed(producer)
            .map_or_else(|| "nil".to_string(), |seed| seed.to_string());
        parts.push(format!(
            "{}={}:{}:{bot_seed}",
            protocol::slot_wire_id(slot.id),
            coordinator::producer_kind(producer).wire_str(),
            coordinator::producer_id(producer),
        ));
    }
    parts.join("|")
}

/// A flat rendering of every human's owned set and the slot that is live at
/// the first tick. In 4v4 each owned set has one member and the live slot is
/// it, which is exactly what makes switching inert there.
///
/// # Panics
///
/// Panics if `session` never froze.
#[must_use]
pub fn owned(session: &Driver) -> String {
    let freeze = session
        .host()
        .state
        .freeze
        .as_ref()
        .expect("the session never froze");
    let mut parts = Vec::new();
    for node in &session.nodes {
        if let Some(owned) = freeze.owned.get(&node.peer_id) {
            let owned_str = owned
                .iter()
                .map(|slot| protocol::slot_wire_id(*slot))
                .collect::<Vec<_>>()
                .join(",");
            let live = freeze.live.get(&node.peer_id).map_or_else(
                || "nil".to_string(),
                |slot| protocol::slot_wire_id(*slot).to_string(),
            );
            parts.push(format!("{}=[{owned_str}]>{live}", node.peer_id));
        }
    }
    parts.join("|")
}

/// A canonical driven session: connect, propose, assign, ready, countdown,
/// full simulation, completed result.
///
/// # Panics
///
/// Panics if the session never reaches its start boundary or never
/// completes — both would mean the coordinator itself regressed.
#[must_use]
pub fn session(guest_count: i64, mode: Option<protocol::MatchMode>) -> Driver {
    let mut session = Driver::new(driver::Options {
        guest_count: Some(guest_count),
        mode,
        ..Default::default()
    });
    session.reach_start(Some(3), Some(0));
    assert!(
        session.all_started(),
        "the canonical session never reached its start boundary"
    );
    session.play_out(Some(2), Some(1));
    assert!(
        session.all_terminal(Some(TerminalReason::Completed)),
        "the canonical session never completed"
    );
    session
}

/// The result of a successful [`verify`] run.
pub struct Report {
    /// [`Driver::transcript_id`] of the canonical 4v4 session.
    pub full_transcript_id: String,
    /// [`Driver::transcript_id`] of the bot-filled session.
    pub bot_transcript_id: String,
    /// [`Driver::transcript`] length of the canonical 4v4 session.
    pub message_count: usize,
}

/// Reproduce every [`GOLDEN`] value against a freshly driven session.
///
/// # Panics
///
/// Panics on the first golden value that disagrees with a freshly driven
/// session — meaning `coordinator`, `protocol`, or `live_slot` in this crate
/// no longer agrees with the frozen reference this module pins against (see
/// the module doc comment).
#[must_use]
pub fn verify() -> Report {
    let full = session(7, None);
    let full_transcript_id = full.transcript_id();
    assert_eq!(
        full_transcript_id, GOLDEN.full_transcript_id,
        "coordinator transcript golden changed: expected {}, got {full_transcript_id}",
        GOLDEN.full_transcript_id
    );
    let trace_digest = fnv1a64::hash(full.trace().as_bytes());
    assert_eq!(
        trace_digest, GOLDEN.full_trace_digest,
        "coordinator trace golden changed: expected {}, got {trace_digest}",
        GOLDEN.full_trace_digest
    );
    assert_eq!(
        full.transcript.len(),
        GOLDEN.full_message_count,
        "coordinator message count changed: expected {}, got {}",
        GOLDEN.full_message_count,
        full.transcript.len()
    );
    let full_sources = sources(&full);

    let bots = session(2, None);
    let bot_transcript_id = bots.transcript_id();
    assert_eq!(
        bot_transcript_id, GOLDEN.bot_transcript_id,
        "bot-filled transcript golden changed: expected {}, got {bot_transcript_id}",
        GOLDEN.bot_transcript_id
    );
    let bot_sources = sources(&bots);
    assert_eq!(
        bot_sources, GOLDEN.bot_sources,
        "bot-filled ownership golden changed: expected {}, got {bot_sources}",
        GOLDEN.bot_sources
    );

    let solo_sources = sources(&session(0, None));
    assert_eq!(
        solo_sources, GOLDEN.solo_sources,
        "solo ownership golden changed: expected {}, got {solo_sources}",
        GOLDEN.solo_sources
    );
    assert_ne!(
        full_sources, bot_sources,
        "human and bot-filled ownership must differ"
    );

    // The same eight canonical slots, shared three different ways. Pinning
    // all three keeps 4v4 a regression target while 1v1 and 2v2 gain
    // evidence of their own.
    let full_owned = owned(&full);
    assert_eq!(
        full_owned, GOLDEN.full_owned,
        "4v4 owned-set golden changed: expected {}, got {full_owned}",
        GOLDEN.full_owned
    );
    for (mode, guest_count, transcript_golden, sources_golden, owned_golden) in [
        (
            protocol::MatchMode::OneVOne,
            1,
            GOLDEN.duo_transcript_id,
            GOLDEN.duo_sources,
            GOLDEN.duo_owned,
        ),
        (
            protocol::MatchMode::TwoVTwo,
            3,
            GOLDEN.pair_transcript_id,
            GOLDEN.pair_sources,
            GOLDEN.pair_owned,
        ),
    ] {
        let case = session(guest_count, Some(mode));
        let mode_str = mode.wire_str();
        let transcript = case.transcript_id();
        assert_eq!(
            transcript, transcript_golden,
            "{mode_str} transcript golden changed: expected {transcript_golden}, got {transcript}"
        );
        let case_sources = sources(&case);
        assert_eq!(
            case_sources, sources_golden,
            "{mode_str} ownership golden changed: expected {sources_golden}, got {case_sources}"
        );
        let case_owned = owned(&case);
        assert_eq!(
            case_owned, owned_golden,
            "{mode_str} owned-set golden changed: expected {owned_golden}, got {case_owned}"
        );
    }

    Report {
        full_transcript_id,
        bot_transcript_id,
        message_count: full.transcript.len(),
    }
}

/// A stable, greppable one-line summary of a [`verify`] run.
#[must_use]
pub fn marker(report: &Report) -> String {
    format!(
        "GC_COORDINATOR|golden|schema=1|full_transcript_id={}|bot_transcript_id={}|messages={}",
        report.full_transcript_id, report.bot_transcript_id, report.message_count
    )
}
