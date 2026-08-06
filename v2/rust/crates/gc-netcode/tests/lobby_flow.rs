//! Port of `spec/game/online_lobby_flow_spec.lua`.
//!
//! Every one of this spec's 35 `t.it` cases is `#[ignore]`d here, and that is
//! the correct outcome rather than a shortfall — see the reasoning below
//! before assuming a case was overlooked.
//!
//! Per `v2/README.md` §2's file-mapping table, this spec spans a boundary,
//! not a single crate:
//!
//! - `game/online/coordinator.lua`, `protocol.lua`, `match_manifest.lua` are
//!   this crate (`gc-netcode`).
//! - `game/online/lobby_link.lua` is TypeScript `@gc/online`, already ported
//!   to `v2/ts/packages/online/src/lobby_link.ts`. No Rust port exists or is
//!   planned.
//! - `game/screens/lobby_model.lua` is TypeScript `@gc/screens`, already
//!   ported to `v2/ts/packages/screens/src/lobby_model.ts`. No Rust port
//!   exists or is planned.
//!
//! The spec's own `LobbyTestDriver` (the `Driver` local at the top of the Lua
//! file, above the first `t.describe`) makes the module under test explicit:
//! `Driver:send` calls `lobby_model.command(peer.model, command)` and routes
//! every resulting effect through `peer.link` (`lobby_link:apply`) over a
//! fake transport star. Every `t.it` in every `t.describe` block drives peers
//! through that `Driver`, and every assertion reads either `lobby_model.view`
//! projections (`view(peer).phase`, `.slots`, `.preference`, `.terminal_text`,
//! `.departure_text`, `.identity`, ...) or `lobby_model`'s own text tables
//! (`PREFERENCE_TEXT`, `DEPARTURE_TEXT`, `TERMINAL_TEXT`). The one exception
//! — the `"lobby control framing"` block — drives `lobby_link.frame` /
//! `lobby_link.absorb` directly instead, with no `lobby_model` involved at
//! all. Neither module has a Rust home, so neither the driver harness nor a
//! single assertion pulled from it can be exercised from this crate.
//!
//! **`"lobby build skew"` looked portable and is worth explaining why it is
//! not.** Its coordinator-level mechanics genuinely do exist in this crate:
//! `coordinator::expectation_difference` (compared against a guest's
//! `ManifestExpectation` in `apply_manifest_proposal`) produces exactly the
//! `TerminalReason::BuildMismatch` / `"local identity differs at
//! manifest.build_id"` detail the Lua case pins, and `coordinator::Departure`
//! (set by `drop_guest` from `apply_abort`) mirrors the Lua `coordinator`
//! module's own "a drop is not terminal, the lobby stands" record field for
//! field (`peer_id`, `reason`, `code`, `detail`). A driver-level test for
//! that alone would belong in `tests/coordinator_driver.rs`, not here. But
//! every case in this block *also* asserts on `lobby_model`-only surface in
//! the same `t.it` — `build_row` reads `view(peer).identity` (a `lobby_model`
//! projection), and every terminal/departure check is paired with a
//! `view(...).terminal_text` / `.departure_text` sentence pulled from
//! `lobby_model.TERMINAL_TEXT` / `lobby_model.DEPARTURE_TEXT`. Dropping those
//! assertions to keep only the coordinator-shaped remainder would silently
//! narrow what the Lua case claims to prove — exactly what the porting rule
//! says not to do — so the honest move is to `#[ignore]` the case whole
//! rather than half-port it under the original's name. See each case's
//! reason below for the specific split.
//!
//! No case in this file invents lobby_model/lobby_link logic in Rust to
//! force a pass; see `v2/README.md` and this agent's brief for why that would
//! misrepresent coverage rather than establish it.

// ---------------------------------------------------------------------------
// "lobby control framing"
// ---------------------------------------------------------------------------
// Drives `lobby_link.frame` / `lobby_link.absorb` directly — no `lobby_model`
// here at all. `lobby_link` is TypeScript `@gc/online`
// (v2/ts/packages/online/src/lobby_link.ts); it has no Rust port and should
// not get one (see v2/README.md §2's table: lobby/signalling/coordination is
// TS, only wire encoding and the replicated state machine are Rust). These
// six cases pin `lobby_link`'s own chunking/reassembly invariants, not
// anything `protocol` or `coordinator` compute.

#[test]
#[ignore = "requires @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists. Would assert: \
lobby_link.frame() splits a 2500-byte wire into exactly 3 frames, and \
lobby_link.absorb() rebuilds the original wire only once the final frame \
lands (nil, nil, then the wire on the third)."]
fn splits_and_rebuilds_a_wire_that_exceeds_one_transport_payload() {}

#[test]
#[ignore = "requires @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists. Would assert: \
every frame lobby_link.frame() produces for a protocol::MAX_WIRE_BYTES-sized \
wire stays within lobby_link's own 1024-byte transport payload bound."]
fn keeps_every_frame_inside_the_transport_payload_bound() {}

#[test]
#[ignore = "requires @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists. Would assert: \
lobby_link.absorb() refuses a stream that starts mid-wire (a non-first frame \
fed to a fresh buffer) and refuses a frame replayed out of order, returning an \
error in both cases rather than silently reassembling garbage."]
fn refuses_a_stream_that_starts_mid_wire_or_reorders() {}

#[test]
#[ignore = "requires @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists. Would assert: \
a wire built entirely out of lobby_link's own framing delimiters round-trips \
through frame()/absorb() unchanged -- pinning that chunking uses string:sub \
offsets and an anchored header pattern, never a delimiter scan, so a payload \
can never be corrupted by containing what looks like a separator."]
fn carries_payloads_full_of_its_own_delimiters_unchanged() {}

#[test]
#[ignore = "requires @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists. The wire under \
test is built with protocol.new/protocol.encode (protocol is ported, in \
gc_netcode::protocol), but the behavior this case pins is lobby_link's: that \
framing an already-encoded control wire and reassembling it through \
frame()/absorb() reproduces the exact bytes protocol.decode can read back as \
an `abort` message. The framing round trip, not the encode/decode step, is \
the module under test."]
fn carries_a_real_control_wire_whose_body_holds_delimiters() {}

#[test]
#[ignore = "requires @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists. Would assert: \
lobby_link.frame() refuses a wire one byte past protocol::MAX_WIRE_BYTES with \
an error rather than framing it -- lobby_link's own bound check, even though \
the bound value itself comes from protocol (already ported)."]
fn refuses_a_wire_beyond_the_protocol_bound() {}

// ---------------------------------------------------------------------------
// "manual lobby handshake"
// ---------------------------------------------------------------------------
// Every case drives LobbyTestDriver, so both lobby_model (peer.model,
// `view()`) and lobby_link (peer.link, opened the moment a peer is added) are
// exercised. Neither has a Rust port.

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: after a scripted 1v1 host/guest connect via \
LobbyTestDriver:connect (invite, copy, paste, copy, paste -- no console \
commands), lobby_model.view(...).connected reads 2 on both peers and the \
host's pending_link clears."]
fn completes_offer_and_answer_exchange_without_console_commands() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: after a completed handshake, lobby_model clears its \
internal `outgoing` field (never retains the pasted blob) while \
view(host).imported records the answer's direction, byte count, and an \
8-character fingerprint."]
fn never_retains_the_pasted_blob_after_it_is_used() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: pasting an empty string sets view(host).error \
without moving view(host).phase off \"handshake\", and a subsequent invite \
clears the error -- lobby_model's own recovery behavior."]
fn reports_a_malformed_paste_without_ending_the_session() {}

// ---------------------------------------------------------------------------
// "lobby match modes"
// ---------------------------------------------------------------------------
// Slot ownership from a role/mode/lock sequence is computed by
// lobby_model.command, not by anything this crate's coordinator does on its
// own (coordinator::plan_assignments exists, but these cases assert on
// lobby_model's `view(...).slots` projection, not on a coordinator
// Assignments value). Reimplementing that projection here to pass these
// cases would duplicate lobby_model's logic in a different language, which
// is exactly what this porting task says not to do.

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: in a locked 1v1 lobby, view(host).slots reports the \
host owning all 4 home slots (home_1..home_4) and the guest owning all 4 away \
slots, on both the host's and the guest's own view."]
fn seats_a_1v1_human_on_a_whole_outfield_line() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: within the host's 4 owned slots in a locked 1v1 \
lobby, exactly 1 has driver == \"human\" and 3 have driver == \"ai\" with \
owner_kind == \"peer\" -- lobby_model's AI-fill-inside-a-human's-set \
presentation."]
fn shows_ai_driven_slots_inside_a_humans_owned_set() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a locked 2v2 lobby seats the host on home_1,home_2 \
and guest_1 on home_3,home_4; readiness can be set; and a host `swap` \
repartitions ownership (host <-> guest_1) and clears readiness -- all read \
off view(...).slots and view(...).ready."]
fn seats_a_2v2_human_on_a_chosen_pair_and_repartitions_on_a_swap() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: view(host).required reads 4 after selecting 2v2 \
(vs 2 after re-selecting 1v1), and `lock` with too few connected peers sets \
view(host).error without advancing view(host).phase off \"handshake\" -- all \
lobby_model view fields."]
fn gates_the_required_peer_count_on_the_mode() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a solo host in 4v4 with `bot_fill` then `lock` \
seats the host on 1 slot and fills the remaining 7 with owner_kind == \"bot\" \
/ driver == \"ai\" -- lobby_model's bot-fill-on-approval path, read off \
view(...).slots."]
fn fills_empty_seats_with_ai_only_when_the_host_approves_it() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: in every mode (1v1, 2v2, 4v4), view(host).keepers \
has exactly 2 entries and no row in view(host).slots names a keeper's \
player_id -- lobby_model's own keeper-protection invariant over its slot/\
keeper projections."]
fn keeps_both_keepers_protected_and_slotless_in_every_mode() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: once `lock` proposes a 1v1 manifest, \
view(host).mode_locked is true and a subsequent `mode` command to 4v4 sets \
view(host).error while view(host).mode stays \"1v1\" -- lobby_model's own \
mode-locking behavior, not coordinator's manifest immutability check (which \
this crate already covers separately in tests/coordinator.rs's \
propose_manifest_is_immutable_after_the_first_proposal)."]
fn locks_the_mode_once_the_manifest_is_proposed() {}

// ---------------------------------------------------------------------------
// "lobby readiness and countdown"
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. The freeze fields it reads at the end (match_mode, manifest_id, \
owned, live) do mirror coordinator::Freeze, and this crate's \
coordinator_driver.rs already covers the underlying reach-start-to-freeze \
path directly (e.g. runs_a_host_plus_seven_humans_from_connect_to_acknowledged_result). \
But getting there in this case goes through lobby_model commands (`lock`, \
`ready`, `start`) and view fields (view(...).error, .phase, .can_start, \
.countdown) at every step in between, not through coordinator::Event, so the \
case as written cannot be driven without lobby_model."]
fn reaches_a_synchronized_start_only_after_every_peer_is_ready() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: in a locked, all-ready 2v2 lobby, a host `swap` \
moves view(host).phase from \"ready\" back to \"assigned\" and \
view(host).ready_count to 0 -- lobby_model's readiness-clears-on-repartition \
rule."]
fn clears_readiness_when_ownership_is_republished() {}

// ---------------------------------------------------------------------------
// "lobby pair selection"
// ---------------------------------------------------------------------------
// Pair-request status/text and slot ownership are entirely lobby_model
// state (`Preference`, `view(...).preference`, `PREFERENCE_TEXT`). The
// coordinator-side `PreferenceVerdict`/`Preference` types exist in this
// crate and coordinator::PREFERENCE_TIMEOUT_TICKS is real, but every
// assertion below reads lobby_model's client-side projection and/or its
// human-readable text table, not the coordinator record itself.

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a 2v2 guest can offer_pair on an unclaimed slot, a \
`pair` command shows an immediate `pending` view(...).preference with the \
lobby_model.PREFERENCE_TEXT.pending sentence, and after the host processes it \
the preference reads `granted` with ownership repartitioned and readiness \
cleared."]
fn lets_a_guest_choose_its_pair_and_shows_the_request_through_to_the_grant() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a granted pair survives a subsequent round of \
ordinary `ready` traffic -- the host's publish path must not quietly restore \
the original seating plan over a granted pair -- read off view(...).slots on \
both host and guest."]
fn keeps_a_granted_pair_through_the_traffic_that_follows_it() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a pair request for a slot another guest already \
claimed comes back `rejected` with reason `already_taken` and the \
lobby_model.PREFERENCE_TEXT.already_taken sentence, and ownership is left \
exactly where it was -- all lobby_model view/text state."]
fn shows_the_typed_reason_when_the_host_refuses() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: when a roster change (`leave`) reseats humans, a \
pair that still fits stays `granted` while one that no longer fits reads \
`rejected` with reason `reseated` and the lobby_model.PREFERENCE_TEXT.reseated \
sentence -- plus a partition-completeness check (assert_partition, a spec-\
local helper) over view(...).slots/.seats on every peer."]
fn keeps_the_pair_a_roster_change_still_fits_and_says_why_the_other_went() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a host `swap` that moves a guest off a pair it was \
granted reads back `rejected` with reason `reseated` (read off the ownership, \
not the cause) and the lobby_model.PREFERENCE_TEXT.reseated sentence -- \
lobby_model's own reason-derivation rule."]
fn tells_a_guest_when_the_hosts_swap_took_its_pair_back() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: in 1v1 and 4v4, no row in view(...).slots sets \
can_prefer, and a `pair` command sets view(host).error without ever \
populating view(host).preference -- lobby_model's mode-gating of the pair \
control."]
fn offers_nothing_to_choose_in_1v1_or_4v4() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: with the host peer removed from the driver's peer \
list (so its link stays open but is never serviced), a guest's pending pair \
request times out after coordinator::PREFERENCE_TIMEOUT_TICKS and \
view(...).preference reads `rejected`/`no_response` with the \
lobby_model.PREFERENCE_TEXT.no_response sentence, while view(...).terminal \
stays nil (a silent host does not end the session). The timeout constant is \
real (gc_netcode::coordinator::PREFERENCE_TIMEOUT_TICKS), but the read-back is \
entirely lobby_model's client-side wait-and-give-up state."]
fn stops_waiting_in_plain_language_when_the_host_never_answers() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) -- no Rust port exists. Uses no \
LobbyTestDriver at all; it is a pure completeness check that \
lobby_model.PREFERENCE_TEXT has a non-empty string for every status/reason \
protocol.PREFERENCE_STATUSES / protocol.PREFERENCE_REJECTIONS can produce, \
and nothing extra. protocol's tables are real (gc_netcode::protocol), but the \
table under test -- and the table this case would have to duplicate to pass \
in Rust -- is lobby_model's own, which this crate does not and should not \
own."]
fn has_plain_language_for_every_outcome_a_request_can_end_on() {}

// ---------------------------------------------------------------------------
// "lobby build skew"
// ---------------------------------------------------------------------------
// See the module doc comment for the full reasoning: the coordinator-level
// mechanics these cases exercise (expectation_difference -> BuildMismatch,
// drop_guest -> Departure) are real and already reachable in this crate, but
// every case also asserts on lobby_model's `view(...).identity` /
// `.terminal_text` / `.departure_text`, which this crate cannot produce
// without duplicating lobby_model's text tables.

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. The coordinator-level half is real and already covered by this \
crate: coordinator::expectation_difference against a guest's \
ManifestExpectation yields TerminalReason::BuildMismatch with detail \"local \
identity differs at manifest.build_id\" (see apply_manifest_proposal), and \
the host's response -- dropping just that guest via drop_guest, recording a \
coordinator::Departure, and putting a `disconnect`/`protocol_error` wire on \
the link rather than the local `build_mismatch` reason -- matches this case's \
assertions on host_state/guest state exactly. But the case also reads \
build_row(peer) (view(...).identity) and view(...).terminal_text / \
.departure_text, which are lobby_model projections and text this crate has no \
way to produce without reimplementing lobby_model in Rust -- so the case as a \
whole cannot be ported without dropping assertions the Lua original makes."]
fn ends_a_same_version_different_vocabulary_session_at_the_manifest_check() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. The coordinator half is real: a guest `leave` before the freeze \
produces coordinator::Departure { reason: GuestLeft, .. } on the host via \
drop_guest (apply_disconnect's \"peer_left\" path), matching this case's \
`departure.reason == \"guest_left\"`. But the case also asserts \
view(host).departure_text == \"A guest left the lobby.\", a lobby_model.\
DEPARTURE_TEXT sentence this crate has no way to produce."]
fn says_only_that_a_guest_went_when_the_two_builds_agree() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) -- no Rust port exists. Uses no \
LobbyTestDriver at all; it is a pure completeness check that \
lobby_model.DEPARTURE_TEXT/TERMINAL_TEXT have a sentence for every \
coordinator.DISCONNECT_REASONS code and every departure reason, and that \
DEPARTURE_TEXT.build_mismatch specifically exists. coordinator's own \
disconnect-code table is real (this crate's equivalent is the \
`disconnect_reason` mapping in src/coordinator.rs), but the tables under test \
are lobby_model's, which this crate does not and should not own."]
fn has_host_side_language_for_every_reason_a_drop_can_carry() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. This is the build-skew block's no-false-positive case: matching \
builds must reach the freeze exactly as any other healthy 1v1 session would \
(already covered generally by tests/coordinator_driver.rs's reach_start-based \
cases), plus view(host).departure/.departure_text must both stay nil -- the \
latter is a lobby_model field this crate cannot produce."]
fn leaves_peers_on_the_same_vocabulary_exactly_as_compatible_as_before() {}

// ---------------------------------------------------------------------------
// "online lobby screen shell"
// ---------------------------------------------------------------------------
// Drives the actual mounted product screen, not the bare Driver.

#[test]
#[ignore = "requires @gc/screens' online_lobby and lobby (TypeScript, \
v2/ts/packages/screens/src/online_lobby.ts and lobby.ts, which wrap \
lobby_model.ts) and @gc/online's lobby_link (TypeScript, \
v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists for any of \
them. Would assert: two mounted OnlineLobby screens, driven only through \
`dispatch` (role, mode, invite, copy, paste_request, lock, ready, start) and \
a scripted clock, both reach lobby_model.view(...).started == true -- the \
full product flow, one layer above the bare LobbyTestDriver every other case \
in this file uses."]
fn carries_a_full_1v1_session_through_two_mounted_screens() {}

#[test]
#[ignore = "requires @gc/app (TypeScript, v2/ts/packages/app/src/app.ts), \
@gc/ui's hit-testing (v2/ts/packages/ui), and @gc/screens' menu/online_lobby \
routing -- no Rust port exists for any of them, and none of this crate \
(gc-netcode) is involved at all. Would assert: clicking the title menu's \
\"online_lobby\" widget navigates App to route \"lobby\", and pressing escape \
returns to route \"title\" -- pure UI navigation, unrelated to coordinator/\
protocol/match_manifest."]
fn is_reachable_from_the_title_and_returns_to_it() {}

// ---------------------------------------------------------------------------
// "lobby failure paths"
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: a host `leave` in a seated 1v1 lobby sets \
host.left, and the guest's view(...).phase reads \"terminal\" with a non-nil \
view(...).terminal_text -- the text is lobby_model's, not coordinator's raw \
Terminal."]
fn ends_a_guest_session_with_a_stable_reason_when_the_host_aborts() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Would assert: in a locked 2v2 lobby, view(host).connected drops \
from 4 to 3 when one guest leaves, and view(host).phase falls back to \
\"assigned\" -- lobby_model's own connected-count and phase projections."]
fn drops_a_departed_guest_and_voids_the_ownership_that_named_it() {}

#[test]
#[ignore = "requires @gc/screens' lobby_model (TypeScript, \
v2/ts/packages/screens/src/lobby_model.ts) and @gc/online's lobby_link \
(TypeScript, v2/ts/packages/online/src/lobby_link.ts) -- no Rust port exists \
for either. Constructs a guest whose lobby_model template answers a manifest \
with a mismatched content_id (via game.online.protocol_fixture, ported to \
gc_netcode::protocol_fixture, but mutated through a lobby_model-shaped \
`template` callback this crate has no equivalent hook for) and asserts \
view(guest).phase == \"terminal\" with terminal.reason == \
\"manifest_mismatch\" and terminal.detail containing \"content_id\" -- reading \
lobby_model's view of the coordinator terminal, not the coordinator terminal \
directly."]
fn terminates_a_guest_whose_local_identity_differs_from_the_manifest() {}
