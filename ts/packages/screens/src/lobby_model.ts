// The pure lobby model for the manual-connect online session.
//
// It owns no truth of its own. The session coordinator decides admission,
// the manifest, ownership, readiness, the countdown, and the terminal
// reason; this module presents that truth, records the few genuinely local
// choices (role, match mode, seating order, bot fill), and turns everything
// into data the screen can lay out and the link layer can execute.
//
// Every function here is pure: `command` returns a fresh model plus an
// ordered effect list, and `view` derives presentation from the model
// alone. Transport calls, clipboard access, and rendering live outside.
//
// # Cross-boundary ports
//
// `CoordinatorPort`, `ProtocolPort`, and `ProtocolFixturePort` wrap
// Rust-owned implementations (`crates/gc-netcode`; ARCHITECTURE.md §1.1) with
// no wasm bridge this milestone. `Fnv1a64Port` wraps a Rust-owned hash
// (`crates/gc-core`) that needs a shared-vector pin across languages before
// a second implementation is safe to write (ARCHITECTURE.md §1.2), and adding
// one is not this module's job. `TransportContractPort` wraps a TS-owned
// implementation (`packages/transport`) that is not a declared dependency
// of this package. All four are threaded through as `LobbyModelPorts`,
// following `@gc/online`'s `match_presentation.ts` precedent
// (`RollbackEventsPort`/`MatchDriverPort`). `InputFramePort` likewise wraps
// Rust-owned state; only the two constants this module reads
// (`SLOT_COUNT`/`slot`) are threaded through the same port bundle.
// `JoinLinkPort` (#598) is the odd one out in this list: it wraps no
// Rust/wasm state at all, only a browser fact (`window.location`'s origin,
// `navigator.share`'s presence) `browser_main.ts` resolves once at boot --
// injected through the same bundle regardless, because `view()` needs it
// exactly as purely as it needs everything else here.
//
// `CoordinatorState` and the session/protocol shapes it carries
// (`SessionManifest`, `SessionSlotProducer`, ...) are given concrete
// structural types rather than kept fully opaque, because this module
// genuinely reads their fields to build its view -- the same choice
// `match_presentation.ts` makes for `RollbackConfirmedStateView` versus the
// truly-opaque `MatchSnapshot`.
//
// # Room-code signaling (#552)
//
// The manual flow above (`invite`/`copy`/`paste_request`/`paste`) still
// carries every WebRTC offer/answer blob -- room codes are a SECOND way for
// that same blob to travel between two peers, not a replacement mechanism.
// `@gc/online`'s `room_signaling.ts` owns the pure wire protocol against
// the room-code Worker (`infra/src/room_durable_object.ts`'s module doc is
// the source of truth for it); `@gc/screens` cannot depend on `@gc/online`
// (ARCHITECTURE.md §2), so the room-code effects below (`room_open_host`,
// `room_open_guest`, `room_send`, `room_close`) and the events fed back in
// (`room_created`, `room_joined`, `room_guest_joined`, `room_guest_left`,
// `room_peer_signal`, `room_failed`, `room_dropped`) are this module's own
// structurally-typed seam, executed by `online_lobby.ts`'s impure
// `roomSignaling` port -- the same injected-port pattern `open_star`/
// `LobbyLinkInstance` already establish for the star transport.
//
// Once a room-code connection is active (`model.room_active`), `onSignal`
// sends a local offer/answer blob over it automatically instead of waiting
// for a `copy` command, and `room_peer_signal` feeds an incoming blob
// straight into `importSignal` instead of waiting for `paste_request` --
// the whole point of a room code is that a player never clicks "copy" or
// "paste". The manual flow is untouched: `model.room_active` starts and
// stays `false` unless a room-code path was chosen, so nothing here changes
// behavior for it.
//
// A host's room-code guest connection ids (issued by the Durable Object,
// `crypto.randomUUID()`) are a different identifier space from this
// module's own link ids (`guest_N`, chosen by `invite()`). `model.room_guest_map`
// records the 1:1 mapping this module assigns at invite time (host-only);
// `model.room_queue` holds guest ids still waiting their turn, because
// `invite()` only ever has one invitation in flight
// (`model.pending_link`) -- exactly the constraint the manual flow already
// enforces, room codes do not relax it.
//
// ## A guest learns its own slot from the host (#601)
//
// The host hands out `guest_N` sequentially (`invite()`), but a room-code
// guest has no way to learn WHICH `N` it was given -- unlike the manual
// flow, there is no "identity" control for a player to click before
// connecting. Guessing (defaulting to `guest_1`, or whatever the local
// `identity` command last cycled to) is wrong the moment a second guest
// joins the same room: two guests both presenting as `guest_1` collide at
// the coordinator's own admission check (`peer_id` must be unique), and the
// second is refused.
//
// The fix travels over the SAME relay that already carries the offer
// itself, not a new channel: `onSignal`'s host branch stamps the
// invitation's own link id onto `room_send` as `slot` (`LobbyEffect`'s own
// doc), and a guest's `roomPeerSignal` adopts that value as `model.peer_id`
// -- BEFORE creating a coordinator -- the first time a signal arrives with
// no coordinator yet. That is why `roomJoined` (fired by `room_joined`,
// well before any offer exists) no longer calls `chooseRole` itself: doing
// so would lock in the guest's own guess before the host's slot has had any
// chance to correct it. `model.role` is set immediately regardless (a
// room-code guest is unambiguously a guest the moment the Worker admits it)
// -- only coordinator creation, and the `open_star` effect that goes with
// it, waits. This never reaches the manual flow: a manually-paired guest's
// `chooseRole` still runs from the "role" command, synchronously, exactly
// as before.
//
// `LobbyModel.last_dropped_signal` is a separate, narrower diagnostic: when
// `roomPeerSignal`'s host branch drops a signal whose sender does not match
// the currently pending invitation (correct routing, but previously silent
// -- see that function's own doc), it now records why, muted (no `error`,
// no `room_error`) for a details/terminal card to surface later (#566).
//
// ## The two-click START collapse (#610)
//
// Starting a friendly match used to need three separate host actions (LOCK
// MATCH, READY, START COUNTDOWN) plus a READY from every guest -- five
// clicks of protocol ceremony a player cannot tell apart. The coordinator's
// own wire protocol is UNCHANGED by this: peers still exchange exactly the
// same `propose_manifest` / `assign_slots` / `set_ready` / `begin_countdown`
// events, in the same order (`crates/gc-netcode`'s `coordinator.rs`). What
// changes is who issues them, and when.
//
// A single host "start" command (`requestStart`) now drives all four in
// one going. Some of those steps cannot complete synchronously: a peer's
// manifest acceptance, then its own readiness, each arrive over the wire as
// a separate `control` event, so `advanceStart` is a small state machine
// rather than a straight-line function -- it is called again after EVERY
// `control` event (the point every inbound wire message lands,
// `online_lobby.ts`'s `update()`), each time re-attempting whichever step
// the coordinator's CURRENT phase now allows. `model.start_requested`
// records that a start is in flight so those later calls know to keep
// advancing rather than starting a fresh one.
//
// A guest's own readiness collapses the same way, but unconditionally: a
// room-code guest's admission link IS its consent to play (owner decision,
// every match mode), so `autoReadyGuest` dispatches `set_ready` itself the
// moment the guest's own phase reaches "assigned" -- the same `control`
// hook, since assignment is the only way a guest's phase gets there
// (`apply_assignments` in `coordinator.rs`). This applies to every guest,
// not only ones that arrived through a room code: the manual copy/paste
// flow shares the same coordinator events, and its guest's own READY
// widget is gone too (`lobby.ts`), so it needs the same automatic
// advancement to ever become ready at all.
//
// Neither hook second-guesses `lockSession`/`setReady`/`beginCountdown`
// themselves -- they call the exact same functions the old three-click flow
// used, unedited. The collapse is additive orchestration on top, which is
// also why the old primitives (`"lock"`, `"ready"` as individual
// `LobbyCommand`s) still work exactly as they did.

// The room-code composer's editing primitives (#610: shared with the
// multiplayer front door's own inline entry) -- re-exported further down,
// by name, so nothing importing them from this module has to change.
import {
  newRoomCodeEntry,
  roomCodeCursor,
  roomCodeCycle,
  roomCodeKey,
  roomCodeText,
  type RoomCodeEntry,
} from "./room_code_entry.ts";

export type LobbyRole = "host" | "guest";
export type LobbySignalDirection = "offer" | "answer";
export type SessionMatchMode = "1v1" | "2v2" | "4v4";
export type InputTeam = "home" | "away";
export type InputSlotId = string;
export type CoordinatorSlotDriver = "human" | "ai";
export type CoordinatorTerminalReason =
  | "completed"
  | "local_abort"
  | "peer_abort"
  | "guest_left"
  | "host_left"
  | "removed"
  | "transport_lost"
  | "protocol_violation"
  | "manifest_mismatch"
  | "build_mismatch"
  | "invalid_assignment"
  | "start_ack_timeout"
  | "start_never_arrived"
  | "input_channel_failure"
  | "late_input"
  | "hash_mismatch";

export interface LobbySignalRecord {
  readonly direction: LobbySignalDirection;
  readonly peer_id: string;
  readonly bytes: number;
  /** Short digest; the blob itself is never retained. */
  readonly fingerprint: string;
}

export type LobbyEffect =
  | { readonly kind: "open_star"; readonly role: LobbyRole; readonly peer_id: string }
  | { readonly kind: "open_peer"; readonly peer_id: string }
  | { readonly kind: "request_offer"; readonly peer_id: string }
  | { readonly kind: "accept_offer"; readonly signal: string }
  | { readonly kind: "accept_answer"; readonly peer_id: string; readonly signal: string }
  | { readonly kind: "send"; readonly link_id: string; readonly wire: string }
  | { readonly kind: "close"; readonly link_id: string; readonly detail?: string }
  | { readonly kind: "clipboard"; readonly text: string }
  /** The one-click join link's native share sheet (#598) -- distinct from
   * `"clipboard"` because a friend's clipboard is written to silently while
   * a share sheet is a whole OS surface `online_lobby.ts` hands off to a
   * separate injected port, never the clipboard one. */
  | { readonly kind: "share"; readonly text: string }
  | { readonly kind: "paste_request" }
  | { readonly kind: "start_match"; readonly freeze: unknown }
  | { readonly kind: "shutdown" }
  | { readonly kind: "leave" }
  // Room-code signaling (#552) -- see this module's header.
  | { readonly kind: "room_open_host" }
  | { readonly kind: "room_open_guest"; readonly code: string }
  // `slot` is present only on a HOST's outgoing offer (`onSignal`'s own
  // comment) -- the invitation link id (`guest_N`) this offer answers,
  // carried so the guest on the other end can adopt it as its own identity
  // before creating a coordinator (#601, this module's header).
  | {
      readonly kind: "room_send";
      readonly to?: string;
      readonly signal: string;
      readonly slot?: string;
    }
  | { readonly kind: "room_close" };

// --- game.online.protocol / protocol_fixture / coordinator, injected -------

export interface SessionMatchModeShape {
  readonly humans: number;
  readonly slots_per_human: number;
  readonly team_humans: number;
}

export interface SessionRosterEntry {
  readonly position: string;
  readonly player_id: string;
}

export interface SessionManifestTeam {
  readonly team: InputTeam;
  readonly roster: readonly SessionRosterEntry[];
}

export interface SessionManifest {
  session_id: string;
  match_mode: SessionMatchMode;
  seed?: number;
  readonly build_id: string;
  readonly source_id: string;
  readonly content_id: string;
  readonly tuning_id: string;
  readonly match_config_id: string;
  readonly fixture_id: string;
  readonly arena_id: string;
  readonly combat_rules_id: string;
  readonly gameplay_ai_policy_id: string;
  readonly combat_status: string;
  readonly slots: readonly { readonly player_id: string }[];
  readonly teams: readonly SessionManifestTeam[];
}

export interface CoordinatorManifestExpectation {
  readonly build_id: string;
  readonly source_id: string;
  readonly content_id: string;
  readonly tuning_id: string;
  readonly match_config_id: string;
  readonly fixture_id: string;
  readonly arena_id: string;
  readonly combat_rules_id: string;
  readonly gameplay_ai_policy_id: string;
  readonly combat_status: string;
}

export interface SessionSlotProducer {
  readonly producer_kind: "peer" | "bot";
  readonly producer_id: string;
  readonly team: InputTeam;
  readonly slot: InputSlotId;
}

// The real coordinator's `preference` field genuinely holds "pending"
// between a request and its verdict (the guest branch of pair-preference
// handling in the Rust-owned coordinator, `crates/gc-netcode`) -- this
// union used to omit it and callers worked around the gap with a local
// cast/widening (see
// `lobby_flow.spec.ts`'s former `FakePreference.status` comment). "pending"
// never appears in a `pair_preference_result` wire message (only the host's
// verdict does), so this addition is purely about representing the
// coordinator's own local, not-yet-answered state correctly.
export type SessionPreferenceStatus = "pending" | "granted" | "unchanged" | "rejected";
export type SessionPreferenceRejection =
  | "already_taken"
  | "wrong_team"
  | "invalid_slot"
  | "detached"
  | "not_seated"
  | "superseded"
  | "after_freeze"
  | "no_response"
  | "reseated";

export interface SessionPreference {
  readonly slots: readonly InputSlotId[];
  readonly status: SessionPreferenceStatus;
  readonly reason?: SessionPreferenceRejection;
}

export interface CoordinatorPeer {
  readonly peer_id: string;
  readonly ready: boolean;
  readonly accepted_manifest_id?: string;
}

export interface CoordinatorDeparture {
  readonly peer_id: string;
  readonly reason: CoordinatorTerminalReason;
  readonly code: string;
  readonly detail?: string;
}

export interface CoordinatorTerminal {
  readonly reason: CoordinatorTerminalReason;
  readonly detail?: string;
}

export type SessionLifecyclePhase =
  "handshake" | "manifest" | "assigned" | "ready" | "countdown" | "running" | "result" | "terminal";

export interface CoordinatorState {
  readonly role: LobbyRole;
  readonly peer_id: string;
  readonly phase: SessionLifecyclePhase;
  readonly peers: readonly CoordinatorPeer[];
  readonly manifest?: SessionManifest;
  readonly manifest_id?: string;
  readonly assignments?: readonly SessionSlotProducer[];
  readonly preference?: SessionPreference;
  readonly countdown_remaining?: number;
  readonly terminal?: CoordinatorTerminal;
  readonly departure?: CoordinatorDeparture;
}

export interface CoordinatorNewHostOptions {
  readonly role: "host";
  readonly session_id: string;
  readonly peer_id: string;
  readonly runtime: unknown;
  readonly build_id: string;
}

export interface CoordinatorNewGuestOptions {
  readonly role: "guest";
  readonly session_id: string;
  readonly peer_id: string;
  readonly host_peer_id: string;
  readonly host_link_id: string;
  readonly runtime: unknown;
  readonly build_id: string;
  readonly expectation: CoordinatorManifestExpectation;
}

export interface CoordinatorAction {
  readonly kind: "send" | "close" | "start_match";
  readonly message?: unknown;
  readonly targets?: readonly string[];
  readonly link_id?: string;
  readonly freeze?: unknown;
}

export interface CoordinatorOutcome {
  readonly accepted: boolean;
  readonly reason?: string;
  readonly actions: readonly CoordinatorAction[];
}

export type CoordinatorEvent = { readonly kind: string; readonly [key: string]: unknown };

/** `game.online.coordinator`, injected -- see this module's header. */
export interface CoordinatorPort {
  /** Named `create` rather than `new` because `new` is a reserved word. */
  create(options: CoordinatorNewHostOptions | CoordinatorNewGuestOptions): CoordinatorState;
  step(
    state: CoordinatorState,
    event: CoordinatorEvent,
  ): readonly [CoordinatorState, CoordinatorOutcome];
  planAssignments(
    manifest: SessionManifest,
    seating: readonly string[],
  ): readonly SessionSlotProducer[] | undefined;
  ownedSlots(state: CoordinatorState, peerId: string): readonly InputSlotId[];
  /** The opening live slot per human owner, keyed by producer id. */
  previewLive(
    assignments: readonly SessionSlotProducer[] | undefined,
  ): Readonly<Record<string, InputSlotId>>;
  ownershipSeatsRoster(state: CoordinatorState): boolean;
}

export interface ProtocolMessage {
  readonly kind: string;
  readonly [key: string]: unknown;
}

/** `game.online.protocol`, injected -- see this module's header. */
export interface ProtocolPort {
  readonly matchModes: Readonly<Record<SessionMatchMode, SessionMatchModeShape>>;
  encode(message: unknown): string | undefined;
  slotIndex(slot: InputSlotId): number | undefined;
}

/** `game.online.protocol_fixture`, injected -- see this module's header. */
export interface ProtocolFixturePort {
  manifest(mode: SessionMatchMode): SessionManifest;
  runtime(): unknown;
}

/** `game.transport.contract`, injected -- see this module's header. */
export interface TransportContractPort {
  readonly hostPeerId: string;
  readonly maxGuests: number;
}

/** `core.fnv1a64`, injected -- see this module's header. */
export interface Fnv1a64Port {
  hash(text: string): string;
}

/** `sim.input_frame`, injected -- see this module's header. */
export interface InputFramePort {
  readonly slotCount: number;
  slot(index: number): { readonly id: InputSlotId; readonly team: InputTeam } | undefined;
}

/**
 * The one-click join link (#598): builds the shareable URL for a room code,
 * and reports whether the platform offers a native share sheet. Both facts
 * live behind `window.location`/`navigator.share` -- browser globals this
 * module must never read itself (AGENTS.md §2's "pure" rule) -- so they are
 * injected exactly like every other environment fact this port bundle
 * already carries (`TransportContractPort.hostPeerId`, for one). The app
 * shell (`browser_main.ts`) supplies the real implementation; a spec
 * supplies a fake. `canShare` in particular is a plain capability flag
 * rather than a method the model could call to "check" sharing -- `view()`
 * reads it to decide whether the SHARE control renders at all, and a
 * boolean is the whole of what it needs.
 */
export interface JoinLinkPort {
  /** The full, shareable URL a friend can click to land directly in this
   * room -- typically `${origin}/?room=${code}`, composed by the app shell
   * from a fact (the page origin) this module never touches. */
  urlFor(code: string): string;
  /** Whether `navigator.share` exists on this device, resolved once by the
   * app shell. Pure code never sniffs `navigator` to find out. */
  readonly canShare: boolean;
}

export interface LobbyModelPorts {
  readonly coordinator: CoordinatorPort;
  readonly protocol: ProtocolPort;
  readonly protocolFixture: ProtocolFixturePort;
  readonly transportContract: TransportContractPort;
  readonly fnv1a64: Fnv1a64Port;
  readonly inputFrame: InputFramePort;
  readonly joinLink: JoinLinkPort;
}

export interface LobbyModelOptions {
  /** Coordinator identity claimed by this peer. */
  readonly peer_id?: string;
  readonly session_id?: string;
  readonly seed?: number;
  readonly template?: (mode: SessionMatchMode) => SessionManifest;
}

export interface LobbyModel {
  readonly role?: LobbyRole;
  readonly peer_id: string;
  readonly session_id: string;
  readonly seed?: number;
  readonly mode: SessionMatchMode;
  readonly bot_fill: boolean;
  /** Host-side human seating order; index N owns block N. */
  readonly seating: readonly string[];
  readonly coordinator?: CoordinatorState;
  /** Host-side count of opened guest links. */
  readonly guests: number;
  /** Host-side link awaiting its answer blob. */
  readonly pending_link?: string;
  /** Local signaling blob awaiting export; cleared on copy. */
  readonly outgoing?: string;
  readonly exported?: LobbySignalRecord;
  readonly imported?: LobbySignalRecord;
  readonly status: string;
  readonly error?: string;
  /** A synchronized start action has been observed. */
  readonly started: boolean;
  /** Host-only: `true` once the collapsed START MATCH command (#610) has
   * been issued. Lock, publish, host-ready, and begin-countdown each need
   * the coordinator to be in a specific phase, and every phase past the
   * first is only reached after a remote round trip (a peer's manifest
   * acceptance, then its own readiness) -- this flag is what tells
   * `advanceStart` there is a start in flight to keep advancing, on every
   * later event that could have moved the coordinator closer to `ready`.
   * See this module's header addendum on the two-click collapse. */
  readonly start_requested: boolean;
  readonly template: (mode: SessionMatchMode) => SessionManifest;

  // --- Room-code signaling (#552) -- see this module's header. ---------

  /** The composer for a guest's not-yet-submitted code, present only while
   * choosing "join with a room code" and before the room-code connection
   * has been requested. */
  readonly room_entry?: RoomCodeEntry;
  /** Set once the room-code Worker confirms this room (host: `created`;
   * guest: `joined`). */
  readonly room_code?: string;
  readonly room_status?: RoomSignalingStatus;
  /** Why the room-code connection failed, when `room_status === "failed"`.
   * A SEPARATE field from `model.error` on purpose: `command()`'s own top
   * strips `error` on every dispatch, including the automatic `tick` this
   * screen's owner fires every frame (`online_lobby.ts`'s `update()`), so a
   * failure surfaced only through `error` would be visible for under one
   * frame before its own trailing tick erased it. `room_error` persists
   * until the next room-code attempt starts or the failure is left behind
   * (`roomPick`/`roomCreated`/`roomJoined`/`roomCancel`/`leave` all clear
   * it); `layout()` reads it as a fallback once `error` itself is gone. */
  readonly room_error?: string;
  /** A room-code connection is in progress or established -- gates the
   * auto-send/auto-import behavior described in this module's header.
   * `false` for the entire manual-signaling flow. */
  readonly room_active: boolean;
  /** Host-only: room-code guest id -> this module's own link id
   * (`guest_N`), assigned at invite time. */
  readonly room_guest_map: Readonly<Record<string, string>>;
  /** Host-only: room-code guest ids still waiting their turn to be invited,
   * in arrival order. */
  readonly room_queue: readonly string[];
  /** Host-only: set the last time `roomPeerSignal` dropped a signal whose
   * sender did not match the pending invitation (#601) -- see this
   * module's header, "A guest learns its own slot from the host". Muted
   * (no `error`/`room_error`): a future details/terminal card renders it
   * (#566), it does not interrupt the player. */
  readonly last_dropped_signal?: LobbyDroppedSignal;
  /** Host-only: set once a room-code guest tried to join after admission
   * closed (the manifest was already proposed) -- #610 round-2 review,
   * blocking finding 3. A quiet, persistent record shown alongside the
   * roster (`troubleText`'s own fallback chain); it does not clear on its
   * own, since there is nothing to resolve (the guest was correctly
   * turned away, not queued). */
  readonly late_joiner_note?: string;
  /** Guest-only: ticks remaining before "no offer arrived" is treated as
   * "the match already started" (#610 round-2 review, blocking finding 3),
   * set by `roomJoined` and counted down on every `"tick"`. Cleared the
   * moment a coordinator exists (an offer arrived) or a real relay failure
   * already explained itself -- see `checkRoomJoinDeadline`'s own doc. */
  readonly room_join_deadline?: number;
}

/** Why `roomPeerSignal`'s host branch dropped a signal instead of routing
 * it to `importSignal` -- see `LobbyModel.last_dropped_signal`'s own doc. */
export interface LobbyDroppedSignal {
  /** The room-code guest id the signal actually came from, when the relay
   * reported one. */
  readonly from?: string;
  /** The guest id the currently pending invitation expects instead, when
   * there is one. */
  readonly expected?: string;
  readonly reason: "no_pending_invite" | "sender_unknown" | "sender_mismatch";
}

// The composer type and its editing primitives now live in
// `room_code_entry.ts` (#610), shared with the multiplayer front door's own
// inline entry -- re-exported here so nothing importing them from this
// module (`lobby.ts`, `@gc/screens`'s `index.ts`) has to change.
export { ROOM_CODE_ALPHABET, ROOM_CODE_LENGTH, type RoomCodeEntry } from "./room_code_entry.ts";

export type RoomSignalingStatus = "connecting" | "connected" | "failed";

export const DEFAULT_MODE: SessionMatchMode = "4v4";
export const COUNTDOWN_ID = "countdown.1";
export const COUNTDOWN_TICKS = 180;
export const FIRST_INPUT_TICK = 0;
export const MAX_SIGNAL_BYTES = 16384;
export const GUEST_LINK_PREFIX = "guest_";
// A room-code guest admitted after the host already proposed a manifest
// (mid #610 collapse, or later) is never invited -- `invite()`'s own
// "admission closed" guard -- and there is no clean host -> guest channel
// to tell it so before any offer exists (#610 round-2 review, blocking
// finding 3: `room_send` only ever carries a real SDP signal, and abusing
// it for anything else would have the guest try to parse the rejection AS
// one). 10 seconds is generous room-code-relay-plus-WebRTC admission
// latency (the coordinator's own `PREFERENCE_TIMEOUT_TICKS`,
// `crates/gc-netcode`, gives an ordinary in-session request 5s; this
// crosses one extra hop, the room-code Worker itself, before any offer
// can even be attempted) while still telling a genuinely late guest
// something within a session, not a hang.
export const ROOM_JOIN_TIMEOUT_TICKS = 600;
export const MODES: readonly SessionMatchMode[] = ["1v1", "2v2", "4v4"];

// Human-readable equivalents of the coordinator's terminal reasons. The wire
// codes stay closed; these strings exist only so a tester can act on a
// failure without reading the protocol document.
export const TERMINAL_TEXT: Readonly<Record<CoordinatorTerminalReason, string>> = {
  completed: "The session finished.",
  local_abort: "You ended the session.",
  peer_abort: "A peer ended the session.",
  guest_left: "A guest left the session.",
  host_left: "The host left the session.",
  removed: "The host disconnected you.",
  transport_lost: "The connection to a peer was lost.",
  protocol_violation: "A peer sent traffic this session cannot accept.",
  manifest_mismatch: "Match identity differs between peers.",
  build_mismatch: "The peers are running different builds. Install the same build on both.",
  invalid_assignment: "Published slot ownership was unusable.",
  start_ack_timeout: "A peer never reached the start boundary.",
  start_never_arrived: "The host never confirmed the start.",
  input_channel_failure: "The input channel failed.",
  late_input: "Input arrived too late to resimulate.",
  hash_mismatch: "Peers disagreed about the simulation.",
};

// What the host says when a seat empties without the session ending. Every
// reason `drop_guest` can record appears here.
export const DEPARTURE_TEXT: Partial<Record<CoordinatorTerminalReason, string>> = {
  guest_left: "A guest left the lobby.",
  transport_lost: "The connection to a guest was lost.",
  // A guest announcing `host_left` to the host is saying the link is going from
  // its side, whatever it believes about who left.
  host_left: "A guest closed its link to the lobby.",
  protocol_violation: "A guest was dropped: it sent traffic this session cannot accept.",
  // Two observations, and no claim about which caused which. The host can see
  // that the guest disagreed about this session's identity and that it declared
  // a different build; it cannot see that the second is why the first happened.
  // A diagnostic that is confidently wrong is worse than a generic one, so this
  // says what is known and names the check.
  build_mismatch:
    "A guest was dropped: it disagreed about this session's identity, " +
    "and it declared a different build. Install the same build on both to rule that out.",
};

// Plain-language equivalents of a pair request's state and of every reason it
// can be refused for. Keyed by status for the outcomes that are not refusals
// and by the typed reason for the ones that are.
export const PREFERENCE_TEXT: Readonly<Record<string, string>> = {
  pending: "Waiting for the host to answer your pair request.",
  granted: "The host gave you the pair you asked for.",
  unchanged: "You already control that pair.",
  already_taken: "Another player already chose that pair.",
  wrong_team: "Those players are not on your team.",
  invalid_slot: "That is not a valid pair in this match mode.",
  detached: "A pair request has to keep one of the players you already control.",
  not_seated: "You control no players in this ownership.",
  superseded: "Ownership changed while your request was in flight. Ask again.",
  after_freeze: "The countdown froze the pairs; they cannot change now.",
  no_response: "The host never answered your pair request. Ask again.",
  reseated: "Ownership changed and your pair had to be seated again. Ask again.",
};

// Plain-language equivalents of a room-code signaling failure -- the raw
// tokens `room_signaling_port.ts` (`@gc/app`) reports, either its own
// classification (`handshake_failed`, `malformed_frame`, `connection_lost`)
// or the room-code Worker's own error code, forwarded verbatim
// (`infra/src/room_durable_object.ts`'s `{type:"error", error}` frame /
// `infra/src/room_state.ts`'s own error strings). A browser `WebSocket`
// cannot read the HTTP status of a failed upgrade at all (`room_signaling.ts`'s
// `RoomSignalingFailureReason` doc, the exhaustive list of what stays
// pre-upgrade -- a per-IP rate limit, a malformed code shape, the room's
// own per-code join-attempt limit, a genuine network error) still
// collapses to the generic `handshake_failed` -- but as of #599, an
// admission-time reason (a bad, full, expired, or closed code; a host-claim
// collision) is no longer one of those: the room-code Worker completes the
// upgrade and sends it in-band instead, so `room_not_found`/`room_full`/
// `room_expired`/`room_closed`/`host_already_claimed` below are genuinely
// reachable player-facing outcomes, not defensive placeholders.
// `host_left` is a DIFFERENT kind of event (the room-code connection ending
// because the host's own socket dropped, not an admission rejection at
// all -- `online_lobby.ts`'s `roomCommandFor` maps it onto this same
// `room_failed` pipeline because the presentation -- readable message, end
// the room-code connection -- is identical). Unmapped tokens fall back to
// the raw token itself (mirrors `PREFERENCE_TEXT`'s own `?? key`
// fallback), never thrown.
export const ROOM_FAILURE_TEXT: Readonly<Record<string, string>> = {
  handshake_failed: "Could not reach the room service. Check the code, or try again.",
  malformed_frame: "The room service sent something this game could not read. Try again.",
  connection_lost: "The connection to the room service was lost.",
  protocol_error: "The room service reported an unexpected problem.",
  room_not_found: "No room with that code — check it and try again.",
  room_not_open: "That code is not an open room.",
  room_closed: "That room has closed.",
  room_expired: "That code has expired — ask for a fresh one.",
  already_joined: "You are already connected to that room.",
  room_full: "That room is full.",
  host_already_claimed: "That room already has a host.",
  host_left: "The host left. Ask for a new code.",
  message_too_large: "That message was too large to send.",
  missing_target: "The room service could not tell who that message was for.",
  unknown_target: "That peer is no longer in the room.",
  no_host: "The host is no longer connected.",
  unknown_sender: "The room service did not recognize the sender.",
  invalid_envelope: "The room service could not read that message.",
  binary_not_supported: "The room service does not support that message type.",
  // Not a relay-reported token -- a LOCAL deadline (`ROOM_JOIN_TIMEOUT_TICKS`,
  // `checkRoomJoinDeadline`) for the one admission-rejection reason the
  // relay itself can never carry: the host already started, so no offer is
  // ever coming (#610 round-2 review, blocking finding 3).
  match_started: "That match already started — ask for a new code.",
};

function defaultTemplate(ports: LobbyModelPorts): (mode: SessionMatchMode) => SessionManifest {
  return (mode) => ports.protocolFixture.manifest(mode);
}

function fingerprint(ports: LobbyModelPorts, text: string): string {
  return ports.fnv1a64.hash(text).slice(0, 8);
}

function copy(model: LobbyModel): LobbyModel {
  return { ...model, seating: [...model.seating] };
}

function manifestFor(model: LobbyModel, mode: SessionMatchMode): SessionManifest {
  const manifest = { ...model.template(mode) };
  manifest.session_id = model.session_id;
  manifest.match_mode = mode;
  if (model.seed !== undefined) {
    manifest.seed = model.seed;
  }
  return manifest;
}

function expectationFor(model: LobbyModel): CoordinatorManifestExpectation {
  const manifest = manifestFor(model, model.mode);
  return {
    build_id: manifest.build_id,
    source_id: manifest.source_id,
    content_id: manifest.content_id,
    tuning_id: manifest.tuning_id,
    match_config_id: manifest.match_config_id,
    fixture_id: manifest.fixture_id,
    arena_id: manifest.arena_id,
    combat_rules_id: manifest.combat_rules_id,
    gameplay_ai_policy_id: manifest.gameplay_ai_policy_id,
    combat_status: manifest.combat_status,
  };
}

// The build this peer declares in its handshake. It is the same value the
// lobby's own BUILD row prints and the same one a guest holds as an
// expectation, taken from the injected template so a test's build is
// whatever its template says it is.
function buildIdFor(model: LobbyModel): string {
  return manifestFor(model, model.mode).build_id;
}

export function newLobbyModel(ports: LobbyModelPorts, options?: LobbyModelOptions): LobbyModel {
  const template = options?.template ?? defaultTemplate(ports);
  const sessionId = options?.session_id ?? template(DEFAULT_MODE).session_id;
  return {
    // A guest's coordinator identity and its transport link identity are the
    // same string: the host's Nth invitation opens link `guest_N`, and the
    // guest must answer as that peer for the star to bind them.
    peer_id: options?.peer_id ?? `${GUEST_LINK_PREFIX}1`,
    session_id: sessionId,
    ...(options?.seed !== undefined ? { seed: options.seed } : {}),
    mode: DEFAULT_MODE,
    bot_fill: false,
    seating: [],
    guests: 0,
    status: "Host a session or join one with a pasted offer.",
    started: false,
    start_requested: false,
    template,
    room_active: false,
    room_guest_map: {},
    room_queue: [],
  };
}

function effectiveMode(model: LobbyModel): SessionMatchMode {
  return model.coordinator?.manifest?.match_mode ?? model.mode;
}

function requiredHumans(ports: LobbyModelPorts, model: LobbyModel): number {
  const shape = ports.protocol.matchModes[effectiveMode(model)];
  if (shape === undefined) {
    throw new Error("unknown match mode");
  }
  return shape.humans;
}

// Ownership can only be published once the roster is stable, so seating is
// rebuilt from the coordinator roster while preserving any order the host has
// already chosen. A departed peer drops out; a new one is appended.
function refreshSeating(ports: LobbyModelPorts, model: LobbyModel): LobbyModel {
  const state = model.coordinator;
  if (!state || state.role !== "host") {
    return model;
  }
  const present = new Set(state.peers.map((peer) => peer.peer_id));
  const seating: string[] = [];
  for (const id of model.seating) {
    if (present.has(id)) {
      seating.push(id);
      present.delete(id);
    }
  }
  for (const peer of state.peers) {
    if (present.has(peer.peer_id)) {
      seating.push(peer.peer_id);
      present.delete(peer.peer_id);
    }
  }
  return { ...model, seating };
}

function absorb(
  model: LobbyModel,
  ports: LobbyModelPorts,
  outcome: CoordinatorOutcome,
  effects: LobbyEffect[],
): LobbyModel {
  let next = model;
  if (!outcome.accepted && outcome.reason) {
    next = { ...next, error: outcome.reason };
  }
  for (const action of outcome.actions) {
    if (action.kind === "send") {
      const wire = action.message !== undefined ? ports.protocol.encode(action.message) : undefined;
      if (wire !== undefined) {
        for (const target of action.targets ?? []) {
          effects.push({ kind: "send", link_id: target, wire });
        }
      } else {
        next = { ...next, error: "a control message could not be encoded" };
      }
    } else if (action.kind === "close") {
      if (action.link_id === undefined) {
        throw new Error("a close action requires a link_id");
      }
      effects.push({ kind: "close", link_id: action.link_id });
    } else if (action.kind === "start_match") {
      next = { ...next, started: true };
      effects.push({ kind: "start_match", freeze: action.freeze });
    }
  }
  return next;
}

function step(
  model: LobbyModel,
  ports: LobbyModelPorts,
  event: CoordinatorEvent,
  effects: LobbyEffect[],
): readonly [LobbyModel, CoordinatorOutcome | undefined] {
  const state = model.coordinator;
  if (!state) {
    return [model, undefined];
  }
  // A session that already ended keeps its reason. Late transport traffic is
  // expected after a termination and must not overwrite it with "the session
  // already ended".
  if (state.phase === "terminal" && event.kind !== "tick") {
    return [model, undefined];
  }
  const [nextState, outcome] = ports.coordinator.step(state, event);
  let next = absorb({ ...model, coordinator: nextState }, ports, outcome, effects);
  next = refreshSeating(ports, next);
  return [next, outcome];
}

function plannedAssignments(
  ports: LobbyModelPorts,
  model: LobbyModel,
): readonly SessionSlotProducer[] | undefined {
  const state = model.coordinator;
  if (!state || state.role !== "host" || !state.manifest) {
    return undefined;
  }
  return ports.coordinator.planAssignments(state.manifest, model.seating);
}

// Publishing is deferred until every admitted peer has accepted the
// manifest, because #163 refuses ownership before that. Byte-identical
// ownership is refused as idempotent by the coordinator, so this is safe to
// call after any roster or seating change without churning readiness.
function publishAssignments(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
  force?: boolean,
): LobbyModel {
  const state = model.coordinator;
  if (!state || state.role !== "host") {
    return model;
  }
  if (state.phase !== "manifest" && state.phase !== "assigned" && state.phase !== "ready") {
    return model;
  }
  // Ownership that already seats the whole roster needs no plan. Republishing
  // one would overwrite pairs guests were granted and clear readiness for
  // nothing; only a roster change or an explicit host swap reaches past this.
  if (!force && ports.coordinator.ownershipSeatsRoster(state)) {
    return model;
  }
  for (const peer of state.peers) {
    if (peer.accepted_manifest_id !== state.manifest_id) {
      return model;
    }
  }
  const assignments = plannedAssignments(ports, model);
  if (!assignments) {
    return model;
  }
  const [next] = step(
    model,
    ports,
    {
      kind: "assign_slots",
      assignments,
      // The plan is derived from `model.seating`, which knows the roster and
      // nothing about the pairs guests were granted. A roster change asks the
      // coordinator to seat the new roster around the claims that still fit
      // and to drop the rest with a reason; only the host reasserting its own
      // order overrules them all.
      preserve_claims: !force,
    },
    effects,
  );
  return next;
}

function configurable(model: LobbyModel): boolean {
  const state = model.coordinator;
  if (!state) {
    return false;
  }
  return (
    state.phase !== "countdown" &&
    state.phase !== "running" &&
    state.phase !== "result" &&
    state.phase !== "terminal"
  );
}

function chooseRole(
  model: LobbyModel,
  ports: LobbyModelPorts,
  role: LobbyRole,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.coordinator) {
    return { ...model, error: "the session role is already chosen" };
  }
  let next: LobbyModel = { ...model, role };
  if (role === "host") {
    next = { ...next, peer_id: ports.transportContract.hostPeerId };
  }
  effects.push({ kind: "open_star", role, peer_id: next.peer_id });
  if (role === "host") {
    next = {
      ...next,
      coordinator: ports.coordinator.create({
        role: "host",
        session_id: next.session_id,
        peer_id: next.peer_id,
        runtime: ports.protocolFixture.runtime(),
        build_id: buildIdFor(next),
      }),
      status: "Pick a match mode, then invite peers.",
    };
  } else {
    next = {
      ...next,
      coordinator: ports.coordinator.create({
        role: "guest",
        session_id: next.session_id,
        peer_id: next.peer_id,
        host_peer_id: ports.transportContract.hostPeerId,
        host_link_id: ports.transportContract.hostPeerId,
        runtime: ports.protocolFixture.runtime(),
        build_id: buildIdFor(next),
        expectation: expectationFor(next),
      }),
      status: "Paste the host's offer to connect.",
    };
  }
  return refreshSeating(ports, next);
}

function setMode(model: LobbyModel, ports: LobbyModelPorts, mode: SessionMatchMode): LobbyModel {
  if (model.role !== "host") {
    return { ...model, error: "only the host chooses the match mode" };
  }
  if (ports.protocol.matchModes[mode] === undefined) {
    return { ...model, error: "unsupported match mode" };
  }
  const state = model.coordinator;
  if (!state) {
    throw new Error("the host must have a coordinator before choosing a mode");
  }
  // The mode lives in the immutable manifest, so it locks at proposal rather
  // than at countdown. Every configuration choice downstream of it -- seating,
  // and therefore readiness -- is discarded when it changes.
  if (state.manifest_id !== undefined) {
    return { ...model, error: "the match mode is fixed once the manifest is proposed" };
  }
  let next = model;
  if (model.mode !== mode) {
    next = refreshSeating(ports, { ...next, seating: [] });
  }
  next = { ...next, mode };
  return { ...next, status: `${mode} seats ${requiredHumans(ports, next)} humans.` };
}

function invite(model: LobbyModel, ports: LobbyModelPorts, effects: LobbyEffect[]): LobbyModel {
  if (model.role !== "host") {
    return { ...model, error: "only the host invites peers" };
  }
  const state = model.coordinator;
  if (!state) {
    throw new Error("the host must have a coordinator before inviting");
  }
  if (state.manifest_id !== undefined) {
    return { ...model, error: "admission closed when the manifest was proposed" };
  }
  if (model.pending_link !== undefined) {
    return { ...model, error: "finish the pending invitation first" };
  }
  if (model.guests >= requiredHumans(ports, model) - 1) {
    return {
      ...model,
      error: `${effectiveMode(model)} seats ${requiredHumans(ports, model)} humans`,
    };
  }
  if (model.guests >= ports.transportContract.maxGuests) {
    return { ...model, error: "the star transport is at guest capacity" };
  }
  const guests = model.guests + 1;
  const linkId = `${GUEST_LINK_PREFIX}${guests}`;
  effects.push({ kind: "open_peer", peer_id: linkId });
  effects.push({ kind: "request_offer", peer_id: linkId });
  return { ...model, guests, pending_link: linkId, status: `Creating an offer for ${linkId}.` };
}

function exportSignal(model: LobbyModel, effects: LobbyEffect[]): LobbyModel {
  if (model.outgoing === undefined) {
    return { ...model, error: "no signaling blob is waiting" };
  }
  effects.push({ kind: "clipboard", text: model.outgoing });
  // The blob leaves the model the moment it is handed over: nothing renders
  // it, nothing logs it, and a later screenshot cannot leak it.
  const { outgoing: _outgoing, ...rest } = model;
  return { ...rest, status: "Signal copied. Send it to your peer." };
}

// --- one-click join link (#598) --------------------------------------------
//
// `model.room_code` is only ever set on the host side (`roomCreated` below;
// a guest's `roomJoined` never sets it), so gating on it is already the
// whole of "host only" -- the explicit `role` check below is defense in
// depth, matching every other host-only command's own guard style, not a
// case that can actually diverge from it today.

function copyLink(model: LobbyModel, ports: LobbyModelPorts, effects: LobbyEffect[]): LobbyModel {
  if (model.role !== "host" || model.room_code === undefined) {
    return { ...model, error: "no room code to share yet" };
  }
  effects.push({ kind: "clipboard", text: ports.joinLink.urlFor(model.room_code) });
  return { ...model, status: "Join link copied." };
}

function shareLink(model: LobbyModel, ports: LobbyModelPorts, effects: LobbyEffect[]): LobbyModel {
  if (model.role !== "host" || model.room_code === undefined) {
    return { ...model, error: "no room code to share yet" };
  }
  if (!ports.joinLink.canShare) {
    return { ...model, error: "sharing is not available on this device" };
  }
  effects.push({ kind: "share", text: ports.joinLink.urlFor(model.room_code) });
  return { ...model, status: "Opening the share sheet." };
}

function importSignal(
  model: LobbyModel,
  ports: LobbyModelPorts,
  text: unknown,
  effects: LobbyEffect[],
): LobbyModel {
  if (typeof text !== "string" || text.length === 0) {
    return { ...model, error: "the pasted signal is empty" };
  }
  if (text.length > MAX_SIGNAL_BYTES) {
    return { ...model, error: "the pasted signal is too large to be a signaling blob" };
  }
  if (model.role !== "host" && model.role !== "guest") {
    return { ...model, error: "choose host or guest before pasting a signal" };
  }
  if (model.role === "host" && model.pending_link === undefined) {
    return { ...model, error: "invite a peer before pasting an answer" };
  }
  const hostSide = model.role === "host";
  // Only the shape of the blob is retained. The bytes go straight to the
  // transport and are never held, rendered, or logged.
  const imported: LobbySignalRecord = {
    direction: hostSide ? "answer" : "offer",
    peer_id: hostSide ? (model.pending_link as string) : ports.transportContract.hostPeerId,
    bytes: text.length,
    fingerprint: fingerprint(ports, text),
  };
  // A signal reaching this point is routing correctly RIGHT NOW -- any
  // earlier drop `roomPeerSignal`'s host branch traced (#601,
  // `last_dropped_signal`'s own doc) is stale history once the connection
  // has recovered, not a live diagnostic to keep showing.
  const { last_dropped_signal: _lastDroppedSignal, ...withoutDrop } = model;
  let next: LobbyModel = { ...withoutDrop, imported };
  if (hostSide) {
    const pending = model.pending_link as string;
    effects.push({ kind: "accept_answer", peer_id: pending, signal: text });
    next = { ...next, status: `Answer accepted for ${pending}.` };
  } else {
    effects.push({ kind: "accept_offer", signal: text });
    next = { ...next, status: "Offer accepted. Copy your answer back to the host." };
  }
  return next;
}

function lockSession(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role !== "host") {
    return { ...model, error: "only the host proposes the manifest" };
  }
  const state = model.coordinator;
  if (!state) {
    throw new Error("the host must have a coordinator before locking");
  }
  if (state.manifest_id !== undefined) {
    return { ...model, error: "the manifest is already proposed" };
  }
  const required = requiredHumans(ports, model);
  if (state.peers.length < required && !model.bot_fill) {
    return {
      ...model,
      error: `${effectiveMode(model)} needs ${required} humans; ${state.peers.length} are connected`,
    };
  }
  const [stepped, outcome] = step(
    model,
    ports,
    { kind: "propose_manifest", manifest: manifestFor(model, model.mode) },
    effects,
  );
  let next = stepped;
  if (outcome?.accepted) {
    next = { ...next, status: "Manifest proposed. Waiting for peers to accept." };
    // Flush anything already queued for admission -- once the manifest is
    // proposed, `admissionClosed` makes every one of them unreachable
    // forever (#610 round-2 review, blocking finding 3), and nothing else
    // would otherwise touch the queue until some LATER peer-connection
    // event happened to drain it.
    next = drainRoomQueue(next, ports, effects);
  }
  return publishAssignments(next, ports, effects);
}

function swapSeats(
  model: LobbyModel,
  ports: LobbyModelPorts,
  index: unknown,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role !== "host") {
    return { ...model, error: "only the host reassigns ownership" };
  }
  if (!configurable(model)) {
    return { ...model, error: "ownership is frozen" };
  }
  if (typeof index !== "number" || index < 1 || index + 1 > model.seating.length) {
    return { ...model, error: "there is no seat to swap with" };
  }
  const seating = [...model.seating];
  const a = seating[index - 1];
  const b = seating[index];
  if (a === undefined || b === undefined) {
    return { ...model, error: "there is no seat to swap with" };
  }
  seating[index - 1] = b;
  seating[index] = a;
  const next: LobbyModel = {
    ...model,
    seating,
    status: "Ownership republished; readiness cleared.",
  };
  // The host reasserting its own seating order overrides every pair a guest
  // was granted, and the coordinator drops their claims with it.
  return publishAssignments(next, ports, effects, true);
}

// The set this peer would ask for if it wanted `slot`: its current owned set
// with the last slot it does not open the match on traded away. The live
// slot is kept because a preference refines the pair you already control
// rather than moving you somewhere else, which is also the rule the host
// enforces.
//
// An owned set with nothing to trade -- a single slot in `4v4`, a whole
// outfield line in `1v1` -- yields nothing here, so those modes offer no
// control at all. No mode is named to make that true.
function pairRequest(
  model: LobbyModel,
  ports: LobbyModelPorts,
  slot: InputSlotId,
): readonly InputSlotId[] | undefined {
  const state = model.coordinator;
  if (!state) {
    return undefined;
  }
  const owned = ports.coordinator.ownedSlots(state, model.peer_id);
  if (owned.length < 2) {
    return undefined;
  }
  const request: InputSlotId[] = [];
  for (let index = 0; index < owned.length - 1; index += 1) {
    const candidate = owned[index];
    if (candidate === slot) {
      return undefined;
    }
    if (candidate !== undefined) {
      request.push(candidate);
    }
  }
  if (owned[owned.length - 1] === slot) {
    return undefined;
  }
  request.push(slot);
  request.sort((left, right) => {
    const leftIndex = ports.protocol.slotIndex(left);
    const rightIndex = ports.protocol.slotIndex(right);
    if (leftIndex === undefined || rightIndex === undefined) {
      throw new Error("unknown slot id");
    }
    return leftIndex - rightIndex;
  });
  return request;
}

function requestPair(
  model: LobbyModel,
  ports: LobbyModelPorts,
  slot: unknown,
  effects: LobbyEffect[],
): LobbyModel {
  if (!model.coordinator) {
    return model;
  }
  const request = typeof slot === "string" ? pairRequest(model, ports, slot) : undefined;
  if (!request) {
    return { ...model, error: "there is no pair to ask for on that slot" };
  }
  const [next, outcome] = step(model, ports, { kind: "prefer_pair", slots: request }, effects);
  if (outcome?.accepted) {
    return { ...next, status: "Pair request sent to the host." };
  }
  return next;
}

function setReady(
  model: LobbyModel,
  ports: LobbyModelPorts,
  ready: unknown,
  effects: LobbyEffect[],
): LobbyModel {
  if (typeof ready !== "boolean") {
    return { ...model, error: "readiness must be a boolean" };
  }
  const [next] = step(model, ports, { kind: "set_ready", ready }, effects);
  return next;
}

function beginCountdown(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role !== "host") {
    return { ...model, error: "only the host starts the countdown" };
  }
  const [next, outcome] = step(
    model,
    ports,
    {
      kind: "begin_countdown",
      countdown_id: COUNTDOWN_ID,
      remaining_ticks: COUNTDOWN_TICKS,
      first_input_tick: FIRST_INPUT_TICK,
    },
    effects,
  );
  if (outcome?.accepted) {
    return { ...next, status: "Countdown started. Configuration is frozen." };
  }
  return next;
}

// --- the two-click START collapse (#610) -- see this module's header ------

/** The local peer's own entry in `state.peers` -- present for host and
 * guest alike (`coordinator.rs`'s `create` always seats the local peer at
 * index 0); both halves of the collapse below need it. */
function localPeer(state: CoordinatorState): CoordinatorPeer | undefined {
  return state.peers.find((peer) => peer.peer_id === state.peer_id);
}

/** Guest side: readiness is automatic once assigned, in every match mode
 * (owner decision, #610) -- not a separate click. Runs after every
 * `control` event because assignment is the ONLY way a guest's own phase
 * reaches "assigned" (`apply_assignments` in `coordinator.rs`), whether
 * that is a fresh admission or a later reseat (a 2v2 TAKE swap republishes
 * ownership and clears readiness the same way a first publish does) -- so
 * this fires again after a swap too, exactly matching "TAKE remains
 * available before the host starts" with no extra state to track. */
function autoReadyGuest(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  const state = model.coordinator;
  if (!state || state.role !== "guest" || state.phase !== "assigned") {
    return model;
  }
  const peer = localPeer(state);
  if (peer === undefined || peer.ready) {
    return model;
  }
  return setReady(model, ports, true, effects);
}

/** Host side: keeps the collapsed START flow moving as far as the CURRENT
 * coordinator phase allows, one already-existing step at a time -- propose
 * (+ the publish `lockSession` already attempts inline), publish again
 * (idempotent, safe to retry -- covers the case where the first attempt
 * above landed before any peer had accepted), host readiness, and finally
 * the countdown itself. Each `if` is exactly one of the old three clicks;
 * none of them but the first can fire before its own remote round trip
 * (this module's header) actually lands, which is why this function gets
 * called again on every later `control` event rather than looping here. */
function advanceStart(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  if (!model.start_requested || model.role !== "host") {
    return model;
  }
  let next = model;
  let state = next.coordinator;
  if (!state) {
    return next;
  }
  if (state.manifest_id === undefined) {
    const [stepped, outcome] = step(
      next,
      ports,
      { kind: "propose_manifest", manifest: manifestFor(next, next.mode) },
      effects,
    );
    next = stepped;
    if (outcome?.accepted) {
      next = { ...next, status: "Manifest proposed. Waiting for peers to accept." };
      // Flush anything already queued for admission -- see `lockSession`'s
      // identical call for why (#610 round-2 review, blocking finding 3).
      next = drainRoomQueue(next, ports, effects);
    }
    state = next.coordinator;
  }
  if (state && (state.phase === "manifest" || state.phase === "assigned")) {
    next = publishAssignments(next, ports, effects);
    state = next.coordinator;
  }
  if (state && state.phase === "assigned") {
    const peer = localPeer(state);
    if (peer !== undefined && !peer.ready) {
      next = setReady(next, ports, true, effects);
      state = next.coordinator;
    }
  }
  if (state && state.phase === "ready") {
    next = beginCountdown(next, ports, effects);
  }
  return next;
}

/** Runs after every event that steps the coordinator and could therefore
 * have moved its phase -- today that is `"control"` (an inbound wire
 * message) and `"link_lost"` (a local transport-level drop, #610 round-2
 * review, blocking finding 2). Both need the exact same three follow-ups:
 * retry the publish (pre-dates #610 -- a host using the individual "lock"
 * command still needs it, since the first attempt inside `lockSession`
 * usually runs before any peer has accepted), then either advance a host's
 * own in-flight "start" collapse or auto-ready a guest the instant its own
 * assignment lands. A caller that steps the coordinator through some OTHER
 * event and skips this is exactly how a link drop mid-collapse used to
 * strand a host on a permanently disabled "STARTING…" button. */
function advanceAfterCoordinatorEvent(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  const published = publishAssignments(model, ports, effects);
  return published.role === "guest"
    ? autoReadyGuest(published, ports, effects)
    : advanceStart(published, ports, effects);
}

/** The single START MATCH command (#610): the host's side of the collapse.
 * Validated once, the same way `lockSession` always was -- once
 * `start_requested` is set, only `advanceStart` drives it further, so a
 * stray repeat (a synthetic dispatch after the button should already be
 * disabled; a raw model-level re-send) just re-attempts the same in-flight
 * intent rather than re-running this check against a stale snapshot. */
function requestStart(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role !== "host") {
    return { ...model, error: "only the host starts the match" };
  }
  const state = model.coordinator;
  if (!state) {
    throw new Error("the host must have a coordinator before starting");
  }
  if (state.manifest_id === undefined) {
    const required = requiredHumans(ports, model);
    if (state.peers.length < required && !model.bot_fill) {
      return {
        ...model,
        error: `${effectiveMode(model)} needs ${required} humans; ${state.peers.length} are connected`,
      };
    }
  }
  return advanceStart({ ...model, start_requested: true }, ports, effects);
}

function leave(model: LobbyModel, ports: LobbyModelPorts, effects: LobbyEffect[]): LobbyModel {
  const state = model.coordinator;
  let next = model;
  if (state && state.phase !== "terminal") {
    if (state.role === "guest") {
      [next] = step(next, ports, { kind: "leave" }, effects);
    } else {
      [next] = step(
        next,
        ports,
        { kind: "abort", code: "host_abort", detail: "host left the lobby" },
        effects,
      );
    }
  }
  // The departure notice is still queued on the transport, so the link is
  // deliberately left open here: the owning screen tears it down once the
  // notice has had its chance to leave. Shutting down now would drop it.
  effects.push({ kind: "leave" });
  if (next.room_active) {
    effects.push({ kind: "room_close" });
  }
  return { ...next, room_active: false };
}

function onSignal(
  model: LobbyModel,
  ports: LobbyModelPorts,
  command: { readonly peer_id: string; readonly signal: string },
  effects: LobbyEffect[],
): LobbyModel {
  const direction: LobbySignalDirection = model.role === "host" ? "offer" : "answer";
  const exported: LobbySignalRecord = {
    direction,
    peer_id: command.peer_id,
    bytes: command.signal.length,
    fingerprint: fingerprint(ports, command.signal),
  };
  const next: LobbyModel = {
    ...model,
    outgoing: command.signal,
    exported,
    status:
      direction === "offer"
        ? "Offer ready. Copy it to your peer."
        : "Answer ready. Copy it back to the host.",
  };
  // Room-code mode (#552): the blob travels over the room-code relay the
  // instant it exists, with no "copy" click -- see this module's header.
  // The manual flow is untouched (`room_active` stays false for it).
  if (!next.room_active) {
    return next;
  }
  const to = next.role === "host" ? next.room_guest_map[command.peer_id] : undefined;
  if (next.role === "host" && to === undefined) {
    // Defensive: no room-code guest is mapped to this link (should not
    // happen -- every host-side link this module opens for a room-code
    // guest is mapped at invite time). Fall back to the manual "outgoing"
    // state above rather than silently dropping the blob.
    return next;
  }
  effects.push({
    kind: "room_send",
    ...(to !== undefined ? { to } : {}),
    signal: command.signal,
    // The host's own invitation link id (`guest_N`) this offer answers --
    // the only thing on this side of the relay that knows it (#601, this
    // module's header). A guest's own outgoing "answer" needs no slot: the
    // host already knows which invitation is pending from `pending_link`.
    ...(next.role === "host" ? { slot: command.peer_id } : {}),
  });
  const { outgoing: _outgoing, ...rest } = next;
  return {
    ...rest,
    status: direction === "offer" ? "Offer sent automatically." : "Answer sent automatically.",
  };
}

// #610 round-2 review, blocking finding 3: once the manifest is proposed,
// `invite()`'s own guard refuses admission FOREVER, not just for the
// instant it is called -- so any caller that reacts to a refusal by
// re-queuing (as `roomGuestJoined`/`drainRoomQueue` both used to,
// unconditionally) sends a late joiner into a queue that will retry, and
// fail, for the rest of the session. This is the one true condition that
// distinguishes "wait and retry" from "will never succeed, stop asking."
function admissionClosed(model: LobbyModel): boolean {
  return model.coordinator?.manifest_id !== undefined;
}

// A quiet, persistent host-side record that a late joiner was turned away
// -- see `LobbyModel.late_joiner_note`'s own doc.
function lateJoinerTurnedAway(model: LobbyModel): LobbyModel {
  return { ...model, late_joiner_note: "A player tried to join after the match started." };
}

function drainRoomQueue(
  model: LobbyModel,
  ports: LobbyModelPorts,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role !== "host" || model.pending_link !== undefined) {
    return model;
  }
  const [nextGuestId, ...rest] = model.room_queue;
  if (nextGuestId === undefined) {
    return model;
  }
  if (admissionClosed(model)) {
    // Drop this ONE queued guest with the quiet note and keep draining --
    // an earlier queue entry closing admission mid-collapse must not
    // strand every guest still behind it in the queue forever either.
    return drainRoomQueue(lateJoinerTurnedAway({ ...model, room_queue: rest }), ports, effects);
  }
  const invited = invite({ ...model, room_queue: rest }, ports, effects);
  if (invited.pending_link === undefined) {
    // `invite()` refused for a genuinely transient reason (capacity, a
    // pending link already in flight) -- keep the guest queued rather than
    // dropping it; the next event that clears `pending_link` will try
    // again.
    return { ...invited, room_queue: [nextGuestId, ...invited.room_queue] };
  }
  return {
    ...invited,
    room_guest_map: { ...invited.room_guest_map, [invited.pending_link]: nextGuestId },
  };
}

function onPeerConnected(
  model: LobbyModel,
  ports: LobbyModelPorts,
  command: { readonly peer_id: string },
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role === "host") {
    let next = model.pending_link === command.peer_id ? withoutPendingLink(model) : model;
    next = { ...next, status: `${command.peer_id} connected.` };
    return drainRoomQueue(next, ports, effects);
  }
  const [next] = step(
    { ...model, status: "Connected to the host." },
    ports,
    { kind: "connect" },
    effects,
  );
  return next;
}

function withoutPendingLink(model: LobbyModel): LobbyModel {
  const { pending_link: _pendingLink, ...rest } = model;
  return rest;
}

// ---------------------------------------------------------------------------
// Room-code signaling (#552) -- see this module's header.
// ---------------------------------------------------------------------------

function roomPick(
  model: LobbyModel,
  ports: LobbyModelPorts,
  role: LobbyRole,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.coordinator) {
    return { ...model, error: "the session role is already chosen" };
  }
  // A room-code attempt already in flight (or established) must finish or
  // be cancelled (`room_cancel`/`leave`) before another one starts -- this
  // is the room-code buttons' own side of the same race `chooseRole`'s
  // call site guards for the manual "role" command (round-2 council
  // review, blocking finding 2): `lobby.ts`'s layout already disables
  // `room_code_host`/`room_code_join` for this window, and this is the
  // belt to that layout's braces.
  if (model.room_active) {
    return { ...model, error: "a room-code connection is already in progress" };
  }
  if (role === "guest") {
    const { room_error: _roomError, ...rest } = model;
    return {
      ...rest,
      room_entry: newRoomCodeEntry(),
      status: "Enter the room code your host is showing.",
    };
  }
  effects.push({ kind: "room_open_host" });
  const { room_error: _roomError, last_dropped_signal: _lastDroppedSignal, ...rest } = model;
  return {
    ...rest,
    room_status: "connecting",
    room_active: true,
    status: "Requesting a room code.",
  };
}

// The character-editing rules themselves live in `room_code_entry.ts`
// (#610) -- these four stay as the model-level wiring: read `model.room_entry`,
// guard on `unknown` command payloads (the `LobbyCommand` boundary), and
// write the result back onto the model.

function roomKey(model: LobbyModel, key: string): LobbyModel {
  const entry = model.room_entry;
  if (!entry) {
    return model;
  }
  return { ...model, room_entry: roomCodeKey(entry, key) };
}

function roomCursor(model: LobbyModel, delta: unknown): LobbyModel {
  const entry = model.room_entry;
  if (!entry || typeof delta !== "number") {
    return model;
  }
  return { ...model, room_entry: roomCodeCursor(entry, delta) };
}

function roomCycle(model: LobbyModel, delta: unknown): LobbyModel {
  const entry = model.room_entry;
  if (!entry || typeof delta !== "number") {
    return model;
  }
  return { ...model, room_entry: roomCodeCycle(entry, delta) };
}

function roomSubmit(model: LobbyModel, effects: LobbyEffect[]): LobbyModel {
  const code = model.room_entry !== undefined ? roomCodeText(model.room_entry) : undefined;
  if (code === undefined) {
    return { ...model, error: "enter all six characters of the room code" };
  }
  effects.push({ kind: "room_open_guest", code });
  const { room_entry: _entry, room_error: _roomError, ...rest } = model;
  return {
    ...rest,
    room_status: "connecting",
    room_active: true,
    status: `Connecting to room ${code}.`,
  };
}

function roomCancel(model: LobbyModel, effects: LobbyEffect[]): LobbyModel {
  if (model.room_entry === undefined && model.room_status === undefined) {
    return model;
  }
  if (model.room_active) {
    effects.push({ kind: "room_close" });
  }
  const {
    room_entry: _entry,
    room_status: _status,
    room_error: _roomError,
    last_dropped_signal: _lastDroppedSignal,
    ...rest
  } = model;
  return { ...rest, room_active: false, status: "Host a session or join one with a pasted offer." };
}

function roomCreated(
  model: LobbyModel,
  ports: LobbyModelPorts,
  code: string,
  effects: LobbyEffect[],
): LobbyModel {
  const next = chooseRole(model, ports, "host", effects);
  const { room_error: _roomError, last_dropped_signal: _lastDroppedSignal, ...rest } = next;
  return { ...rest, room_code: code, room_status: "connected", status: `Room code ${code}.` };
}

// Deliberately does NOT call `chooseRole` -- unlike `roomCreated` (the host
// branch, which already knows its own identity), a room-code guest does not
// yet know which invitation slot it is answering; that arrives later, on
// the host's relayed offer (`roomPeerSignal`'s guest branch, this module's
// header). Choosing a coordinator identity here would lock in whatever
// `model.peer_id` already defaulted to, which is exactly #601's bug.
// `model.role` is still set immediately: a room-code guest is unambiguously
// a guest the instant the Worker admits it, and nothing downstream (view(),
// `importSignal`'s role check) needs a coordinator to already exist for
// that to be true.
function roomJoined(model: LobbyModel): LobbyModel {
  if (model.coordinator) {
    return { ...model, error: "the session role is already chosen" };
  }
  const { room_error: _roomError, ...rest } = model;
  return {
    ...rest,
    role: "guest",
    room_status: "connected",
    status: "Waiting for the host to assign your seat.",
    // #610 round-2 review, blocking finding 3: starts the deadline that
    // catches "the host already started, no offer is ever coming" --
    // `checkRoomJoinDeadline`'s own doc.
    room_join_deadline: ROOM_JOIN_TIMEOUT_TICKS,
  };
}

/** Counts down `LobbyModel.room_join_deadline` on every `"tick"` --
 * see its own doc and `ROOM_JOIN_TIMEOUT_TICKS`'s. Deliberately separate
 * from `step()`'s own `"tick"` handling: a guest waiting for an offer has
 * NO coordinator yet, which is exactly the state `step()` treats as
 * nothing-to-do (`if (!state) return [model, undefined];`), so this is the
 * only clock that ever runs for it. */
function checkRoomJoinDeadline(model: LobbyModel, effects: LobbyEffect[]): LobbyModel {
  if (model.room_join_deadline === undefined) {
    return model;
  }
  if (model.coordinator !== undefined || model.room_status === "failed" || !model.room_active) {
    // An offer arrived and resolved into a coordinator, a real relay
    // failure already explained itself (must not be overwritten by the
    // generic timeout copy below), or this guest is no longer even in a
    // room-code attempt -- the deadline no longer applies either way.
    const { room_join_deadline: _deadline, ...rest } = model;
    return rest;
  }
  const remaining = model.room_join_deadline - 1;
  if (remaining > 0) {
    return { ...model, room_join_deadline: remaining };
  }
  if (model.room_active) {
    effects.push({ kind: "room_close" });
  }
  const text = ROOM_FAILURE_TEXT["match_started"] as string;
  const { room_join_deadline: _deadline, room_entry: _entry, ...rest } = model;
  return { ...rest, room_status: "failed", room_active: false, error: text, room_error: text };
}

function roomGuestJoined(
  model: LobbyModel,
  ports: LobbyModelPorts,
  guestId: string,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role !== "host") {
    return model;
  }
  // #610 round-2 review, blocking finding 3: a guest admitted by the room
  // relay after the manifest was already proposed can NEVER be invited --
  // see `admissionClosed`'s own doc. Turn it away right here, quietly,
  // instead of queueing it to fail forever.
  if (admissionClosed(model)) {
    return lateJoinerTurnedAway(model);
  }
  if (model.pending_link !== undefined) {
    return { ...model, room_queue: [...model.room_queue, guestId] };
  }
  const invited = invite(model, ports, effects);
  if (invited.pending_link === undefined) {
    // `invite()` refused for a genuinely transient reason -- keep the
    // guest queued; a later `drainRoomQueue` call retries it.
    return { ...invited, room_queue: [...invited.room_queue, guestId] };
  }
  return {
    ...invited,
    room_guest_map: { ...invited.room_guest_map, [invited.pending_link]: guestId },
  };
}

function roomGuestLeft(
  model: LobbyModel,
  ports: LobbyModelPorts,
  guestId: string,
  effects: LobbyEffect[],
): LobbyModel {
  const room_queue = model.room_queue.filter((id) => id !== guestId);
  const linkId = model.pending_link;
  if (linkId === undefined || model.room_guest_map[linkId] !== guestId) {
    return { ...model, room_queue };
  }
  // The currently-pending invite's guest vanished before ever connecting --
  // free the link so the next queued guest (if any) can be invited.
  const { [linkId]: _dropped, ...room_guest_map } = model.room_guest_map;
  const reason = "a peer disconnected before joining";
  const next = withoutPendingLink({
    ...model,
    room_queue,
    room_guest_map,
    error: reason,
    room_error: reason,
  });
  return drainRoomQueue(next, ports, effects);
}

function roomPeerSignal(
  model: LobbyModel,
  ports: LobbyModelPorts,
  guestId: string | undefined,
  signal: string,
  slot: string | undefined,
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role === "host") {
    // A guest's signal can only ever be an answer to whichever invite is
    // currently pending -- this module never has more than one in flight
    // (`invite()`'s own "finish the pending invitation first" guard). A
    // signal for any other guest id is stray (e.g. arrived after that
    // guest already left) and is ignored rather than misrouted onto an
    // unrelated link -- but not silently: `last_dropped_signal` records why
    // (#601, this module's header), so a future regression here has a trace
    // instead of "nothing happened" as its only symptom.
    const linkId = model.pending_link;
    const expected = linkId !== undefined ? model.room_guest_map[linkId] : undefined;
    if (linkId === undefined || guestId === undefined || expected !== guestId) {
      const reason: LobbyDroppedSignal["reason"] =
        linkId === undefined
          ? "no_pending_invite"
          : guestId === undefined
            ? "sender_unknown"
            : "sender_mismatch";
      return {
        ...model,
        last_dropped_signal: {
          ...(guestId !== undefined ? { from: guestId } : {}),
          ...(expected !== undefined ? { expected } : {}),
          reason,
        },
      };
    }
    return importSignal(model, ports, signal, effects);
  }
  // Guest role: the host's relayed offer is where this guest first learns
  // which invitation slot it is answering (#601, this module's header).
  // `chooseRole`'s coordinator creation is deferred from `roomJoined` to
  // exactly here, the first time a signal arrives with no coordinator yet,
  // so the adopted `peer_id` -- not whatever `model.peer_id` defaulted to --
  // is what the coordinator is built with.
  if (model.coordinator) {
    return importSignal(model, ports, signal, effects);
  }
  const withSlot = slot !== undefined ? { ...model, peer_id: slot } : model;
  return importSignal(chooseRole(withSlot, ports, "guest", effects), ports, signal, effects);
}

function roomFailed(model: LobbyModel, reason: string): LobbyModel {
  const text = ROOM_FAILURE_TEXT[reason] ?? reason;
  // Both fields, deliberately: `error` for the first frame (consistent
  // with every other command's error surface), `room_error` so the message
  // survives the trailing `tick` command that same frame's `update(dt)`
  // dispatches right after -- see `room_error`'s own doc on `LobbyModel`.
  //
  // `room_active: false` matters just as much as either: it is what
  // `lobby.ts`'s layout reads to decide whether the manual copy/paste
  // controls are shown at all (`view.role && !view.room_active`). Leaving
  // it `true` after a failure -- the connection this flag was tracking no
  // longer exists -- made the manual fallback this issue's own acceptance
  // criteria promise ("manual signaling still works") unreachable for a
  // player who tried a room code first and picked a manual role after it
  // failed. Mirrors `roomCancel`/`leave`, which already clear it on every
  // other way out of the room-code path.
  return { ...model, room_status: "failed", room_active: false, error: text, room_error: text };
}

function roomDropped(model: LobbyModel): LobbyModel {
  if (model.room_status !== "connected") {
    return model;
  }
  const text = ROOM_FAILURE_TEXT["connection_lost"] as string;
  // `room_active: false` -- see `roomFailed`'s own comment; the same
  // reasoning applies to a connection that was live and then dropped.
  return { ...model, room_status: "failed", room_active: false, error: text, room_error: text };
}

export type LobbyCommand =
  | { readonly kind: "role"; readonly role: LobbyRole }
  | { readonly kind: "mode"; readonly mode: SessionMatchMode }
  // Which invitation this peer is answering. It must agree with the host's
  // Nth opened link, so it is chosen before the role is locked in.
  | { readonly kind: "identity" }
  | { readonly kind: "bot_fill" }
  | { readonly kind: "invite" }
  | { readonly kind: "copy" }
  // The one-click join link (#598): host-only, valid once `room_code` is
  // set. "copy" (above) is the pre-existing manual offer/answer blob;
  // these two are the room-code hero's own share actions.
  | { readonly kind: "copy_link" }
  | { readonly kind: "share_link" }
  | { readonly kind: "paste_request" }
  | { readonly kind: "paste"; readonly text: unknown }
  | { readonly kind: "lock" }
  | { readonly kind: "swap"; readonly index: unknown }
  | { readonly kind: "pair"; readonly slot: unknown }
  | { readonly kind: "ready"; readonly ready: unknown }
  | { readonly kind: "start" }
  | { readonly kind: "leave" }
  | { readonly kind: "tick" }
  | { readonly kind: "signal"; readonly peer_id: string; readonly signal: string }
  | { readonly kind: "peer_connected"; readonly peer_id: string }
  | { readonly kind: "control"; readonly link_id: string; readonly wire: string }
  | { readonly kind: "link_lost"; readonly link_id: string }
  | { readonly kind: "link_error"; readonly detail?: string }
  // Room-code signaling (#552) -- see this module's header. The first six
  // are UI-driven (the code composer); the rest are fed in by
  // `online_lobby.ts`'s `roomSignaling` port, translating its events.
  | { readonly kind: "room_pick"; readonly role: LobbyRole }
  | { readonly kind: "room_key"; readonly key: string }
  | { readonly kind: "room_cursor"; readonly delta: unknown }
  | { readonly kind: "room_cycle"; readonly delta: unknown }
  | { readonly kind: "room_submit" }
  | { readonly kind: "room_cancel" }
  | { readonly kind: "room_created"; readonly code: string }
  | { readonly kind: "room_joined" }
  | { readonly kind: "room_guest_joined"; readonly guest_id: string }
  | { readonly kind: "room_guest_left"; readonly guest_id: string }
  | {
      readonly kind: "room_peer_signal";
      readonly guest_id?: string;
      readonly signal: string;
      // The invitation slot this offer answers -- present only when a host
      // sent it (#601, this module's header). Absent for a guest's own
      // outgoing answer, and for anything relayed before this module's own
      // room-code seam carried it.
      readonly slot?: string;
    }
  | { readonly kind: "room_failed"; readonly reason: string }
  | { readonly kind: "room_dropped" };

// The single pure entry point. Unknown commands are ignored rather than
// fatal so a future screen control cannot crash a live session.
export function command(
  model: LobbyModel,
  ports: LobbyModelPorts,
  cmd: LobbyCommand,
): readonly [LobbyModel, readonly LobbyEffect[]] {
  const effects: LobbyEffect[] = [];
  const { error: _clearedError, ...withoutError } = copy(model);
  let next: LobbyModel = withoutError;
  switch (cmd.kind) {
    case "role":
      // Guarded HERE rather than inside `chooseRole` itself: `roomCreated`/
      // `roomJoined` call `chooseRole` directly while `room_active` is
      // ALREADY `true` (set the moment a room-code attempt starts, well
      // before either arrives) -- a guard inside `chooseRole` would reject
      // its own success path. This is specifically the manual "role"
      // command's own guard: a role pick may not race an in-flight or
      // established room-code connection. `lobby.ts`'s layout already
      // disables `role_host`/`role_guest` for the same window; this is the
      // belt to that layout's braces, so a stale layout or a synthetic
      // dispatch cannot wedge the lobby the way round-2 council review's
      // blocking finding 2 did. See `roomPick`'s matching guard for the
      // room-code buttons' own side of the same race.
      next = next.room_active
        ? { ...next, error: "a room-code connection is in progress" }
        : chooseRole(next, ports, cmd.role, effects);
      break;
    case "mode":
      next = setMode(next, ports, cmd.mode);
      break;
    case "identity": {
      if (next.coordinator) {
        next = { ...next, error: "identity is fixed once the session starts" };
        break;
      }
      // The same race the "role" command's own guard above refuses
      // (round-2 council review, blocking finding 2) -- and #601's own
      // deferred-slot window makes it reachable here in a NEW way: a
      // room-code guest can now have `role === "guest"` with no
      // `coordinator` yet (`roomJoined`'s own doc), so the guard just
      // above no longer covers this command for that guest at all. A
      // room-code guest's identity is the host's to assign
      // (`roomPeerSignal`'s own doc), never this manual-flow-only
      // control's, for the entire window a room-code connection is in
      // flight or established.
      if (next.room_active) {
        next = { ...next, error: "identity is assigned by the room code, not chosen manually" };
        break;
      }
      const match = /^guest_(\d+)$/.exec(next.peer_id);
      let index = match?.[1] !== undefined ? Number(match[1]) : 1;
      index = (index % ports.transportContract.maxGuests) + 1;
      next = {
        ...next,
        peer_id: `${GUEST_LINK_PREFIX}${index}`,
        status: `Joining as ${GUEST_LINK_PREFIX}${index}.`,
      };
      break;
    }
    case "bot_fill": {
      if (next.role !== "host") {
        next = { ...next, error: "only the host approves bot fills" };
        break;
      }
      const botFill = !next.bot_fill;
      next = {
        ...next,
        bot_fill: botFill,
        status: botFill
          ? "Empty seats will be filled with AI."
          : "Every seat must be a connected human.",
      };
      break;
    }
    case "invite":
      next = invite(next, ports, effects);
      break;
    case "copy":
      next = exportSignal(next, effects);
      break;
    case "copy_link":
      next = copyLink(next, ports, effects);
      break;
    case "share_link":
      next = shareLink(next, ports, effects);
      break;
    case "paste_request":
      effects.push({ kind: "paste_request" });
      break;
    case "paste":
      next = importSignal(next, ports, cmd.text, effects);
      break;
    case "lock":
      next = lockSession(next, ports, effects);
      break;
    case "swap":
      next = swapSeats(next, ports, cmd.index, effects);
      break;
    case "pair":
      next = requestPair(next, ports, cmd.slot, effects);
      break;
    // Still a fully working, independent command (#610 round-2 review):
    // `autoReadyGuest` calls the SAME `setReady` helper internally rather
    // than reaching back through `command()`, so this case is never
    // required for the collapse to work -- no widget in `lobby.ts` emits
    // it any more. It remains reachable directly (raw model dispatch, a
    // test, a future non-UI caller) and, if used, simply bypasses the
    // auto-ready guarantee for that one manual call: a guest that manually
    // un-readies itself (`ready: false`) stays not-ready until IT is
    // assigned again (a swap, a reseat), which re-fires `autoReadyGuest`
    // exactly as it would for any other not-yet-ready guest.
    case "ready":
      next = setReady(next, ports, cmd.ready, effects);
      break;
    case "start":
      next = requestStart(next, ports, effects);
      break;
    case "leave":
      next = leave(next, ports, effects);
      break;
    case "tick": {
      const [stepped] = step(next, ports, { kind: "tick" }, effects);
      next = checkRoomJoinDeadline(stepped, effects);
      break;
    }
    case "signal":
      next = onSignal(next, ports, cmd, effects);
      break;
    case "peer_connected":
      next = onPeerConnected(next, ports, cmd, effects);
      break;
    case "control": {
      const [stepped] = step(
        next,
        ports,
        { kind: "control", link_id: cmd.link_id, wire: cmd.wire },
        effects,
      );
      next = advanceAfterCoordinatorEvent(stepped, ports, effects);
      break;
    }
    case "link_lost": {
      // #610 round-2 review, blocking finding 2: this event can ALSO move
      // the coordinator's phase (dropping a not-yet-accepted or
      // not-yet-ready guest can be exactly what makes `all_ready` true for
      // everyone left) -- it needs the identical follow-up chain "control"
      // gets, or a link drop mid-collapse leaves `start_requested` stuck
      // `true` with nothing left to ever call `advanceStart` again, and the
      // host stares at a permanently disabled "STARTING…" with LEAVE as the
      // only way out. See `advanceAfterCoordinatorEvent`'s own doc.
      const [stepped] = step(next, ports, { kind: "link_lost", link_id: cmd.link_id }, effects);
      next = advanceAfterCoordinatorEvent(stepped, ports, effects);
      break;
    }
    case "link_error":
      // Deliberately NOT run through `advanceAfterCoordinatorEvent`: this
      // is a local "a send failed" diagnostic (`online_lobby.ts`'s `run()`,
      // `this.link.apply(effect)` returning an error) that never calls
      // `step()` at all, so it cannot have moved the coordinator's phase --
      // there is nothing new for `publishAssignments`/`advanceStart`/
      // `autoReadyGuest` to react to. Investigated for the same asymmetry
      // as "link_lost" (#610 round-2 review, blocking finding 2) and found
      // not to need it.
      next = { ...next, error: cmd.detail ?? "the transport reported a failure" };
      break;
    case "room_pick":
      next = roomPick(next, ports, cmd.role, effects);
      break;
    case "room_key":
      next = roomKey(next, cmd.key);
      break;
    case "room_cursor":
      next = roomCursor(next, cmd.delta);
      break;
    case "room_cycle":
      next = roomCycle(next, cmd.delta);
      break;
    case "room_submit":
      next = roomSubmit(next, effects);
      break;
    case "room_cancel":
      next = roomCancel(next, effects);
      break;
    case "room_created":
      next = roomCreated(next, ports, cmd.code, effects);
      break;
    case "room_joined":
      next = roomJoined(next);
      break;
    case "room_guest_joined":
      next = roomGuestJoined(next, ports, cmd.guest_id, effects);
      break;
    case "room_guest_left":
      next = roomGuestLeft(next, ports, cmd.guest_id, effects);
      break;
    case "room_peer_signal":
      next = roomPeerSignal(next, ports, cmd.guest_id, cmd.signal, cmd.slot, effects);
      break;
    case "room_failed":
      next = roomFailed(next, cmd.reason);
      break;
    case "room_dropped":
      next = roomDropped(next);
      break;
    default:
      return [model, []];
  }
  return [next, effects];
}

// ---------------------------------------------------------------------------
// Derived presentation
// ---------------------------------------------------------------------------

export interface LobbySlotView {
  readonly slot: InputSlotId;
  readonly team: InputTeam;
  readonly player_id: string;
  /** Producer id, or undefined while ownership is unpublished. */
  readonly owner?: string;
  readonly owner_kind?: "peer" | "bot";
  readonly driver: CoordinatorSlotDriver | "pending";
  /** The opening live slot of its human owner. */
  readonly live: boolean;
  readonly local_owner: boolean;
  /** The local peer could ask the host for this slot. */
  readonly can_prefer: boolean;
}

export interface LobbySeatView {
  readonly index: number;
  readonly peer_id: string;
  readonly is_local: boolean;
  readonly ready: boolean;
  readonly slots: readonly InputSlotId[];
}

export interface LobbyPreferenceView {
  /** The pair the local peer asked for. */
  readonly slots: readonly InputSlotId[];
  readonly status: SessionPreferenceStatus;
  readonly reason?: SessionPreferenceRejection;
  /** Plain language for the status, or for the typed reason. */
  readonly text: string;
}

export interface LobbyKeeperView {
  readonly team: InputTeam;
  readonly player_id: string;
}

export interface LobbyIdentityRow {
  readonly label: string;
  readonly value: string;
}

export interface LobbyView {
  readonly role?: LobbyRole;
  readonly peer_id: string;
  readonly phase: SessionLifecyclePhase | "role";
  readonly mode: SessionMatchMode;
  readonly mode_locked: boolean;
  /** False while a guest has not yet seen the manifest. */
  readonly mode_known: boolean;
  readonly required: number;
  readonly connected: number;
  readonly ready_count: number;
  readonly bot_fill: boolean;
  readonly slots: readonly LobbySlotView[];
  readonly keepers: readonly LobbyKeeperView[];
  readonly seats: readonly LobbySeatView[];
  /** The local peer's last pair request. */
  readonly preference?: LobbyPreferenceView;
  readonly identity: readonly LobbyIdentityRow[];
  readonly countdown?: number;
  /** Host-side: why the last guest was dropped. */
  readonly departure?: CoordinatorDeparture;
  readonly departure_text?: string;
  readonly terminal?: CoordinatorTerminal;
  readonly terminal_text?: string;
  readonly exported?: LobbySignalRecord;
  readonly imported?: LobbySignalRecord;
  readonly has_outgoing: boolean;
  readonly status: string;
  readonly error?: string;
  readonly can_invite: boolean;
  /** Host-side ownership can still be republished (also gates the TAKE
   * seat-swap control and the START MATCH button's own visibility -- see
   * `can_start`'s own doc, #610). */
  readonly can_configure: boolean;
  /** The local peer has been marked ready by the coordinator -- for a
   * guest, that happens automatically the moment it is assigned a seat
   * (#610); a host readies itself as the third step of its own START
   * MATCH command. No control toggles this any more; it is read-only
   * presentation. */
  readonly ready: boolean;
  /** Host-only: the single START MATCH command is available. Folds in what
   * used to be `can_lock` -- the same "enough humans, or bot-fill" gate --
   * because START now performs that lock itself (#610); it is `false`
   * again the instant the collapse is under way (`phase` has left
   * "handshake"), which is exactly what a disabled-but-still-visible
   * button (`can_configure`) communicates as "starting...". */
  readonly can_start: boolean;
  readonly started: boolean;
  /** Whether the SHARE (native share sheet) control should render next to
   * COPY LINK -- true only once a host has a room code AND the injected
   * `JoinLinkPort` reports the platform actually offers one (#598: a
   * capability flag, never sniffed here). */
  readonly can_share: boolean;

  // --- Room-code signaling (#552) -- see this module's header. ---------

  readonly room_entry?: RoomCodeEntry;
  readonly room_code?: string;
  readonly room_status?: RoomSignalingStatus;
  /** Persists across a `tick` where `error` itself does not -- see
   * `room_error`'s own doc on `LobbyModel`. */
  readonly room_error?: string;
  readonly room_active: boolean;
  /** See `LobbyModel.last_dropped_signal`'s own doc (#601). */
  readonly last_dropped_signal?: LobbyDroppedSignal;
  /** See `LobbyModel.late_joiner_note`'s own doc (#610). */
  readonly late_joiner_note?: string;
}

function visibleAssignments(
  ports: LobbyModelPorts,
  model: LobbyModel,
): readonly SessionSlotProducer[] | undefined {
  const state = model.coordinator;
  if (state?.assignments) {
    return state.assignments;
  }
  return plannedAssignments(ports, model);
}

function visibleManifest(model: LobbyModel): SessionManifest {
  const state = model.coordinator;
  if (state?.manifest) {
    return state.manifest;
  }
  return manifestFor(model, model.mode);
}

// The team a peer is seated on, or undefined while it owns nothing.
function teamOf(
  assignments: readonly SessionSlotProducer[] | undefined,
  peerId: string,
): InputTeam | undefined {
  for (const producer of assignments ?? []) {
    if (producer.producer_kind === "peer" && producer.producer_id === peerId) {
      return producer.team;
    }
  }
  return undefined;
}

function preferenceView(model: LobbyModel): LobbyPreferenceView | undefined {
  const preference = model.coordinator?.preference;
  if (!preference) {
    return undefined;
  }
  const key: string =
    preference.status === "rejected" && preference.reason ? preference.reason : preference.status;
  return {
    slots: preference.slots,
    status: preference.status,
    ...(preference.reason !== undefined ? { reason: preference.reason } : {}),
    text: PREFERENCE_TEXT[key] ?? key,
  };
}

function rosterView(
  ports: LobbyModelPorts,
  model: LobbyModel,
): readonly [readonly LobbySlotView[], readonly LobbyKeeperView[]] {
  const manifest = visibleManifest(model);
  const assignments = visibleAssignments(ports, model);
  // The opening live slot is the coordinator's rule, not the lobby's; the
  // freeze records the same table for the same assignments.
  const live = ports.coordinator.previewLive(assignments);
  const state = model.coordinator;
  const configurableOwnership =
    state !== undefined && (state.phase === "assigned" || state.phase === "ready");
  const slots: LobbySlotView[] = [];
  for (let index = 1; index <= ports.inputFrame.slotCount; index += 1) {
    const slot = ports.inputFrame.slot(index);
    if (!slot) {
      throw new Error("unknown canonical input slot");
    }
    const entry = manifest.slots[index - 1];
    const producer = assignments?.[index - 1];
    const isLive =
      producer !== undefined &&
      producer.producer_kind === "peer" &&
      live[producer.producer_id] === producer.slot;
    const sameTeam =
      producer !== undefined &&
      assignments !== undefined &&
      producer.team === teamOf(assignments, model.peer_id);
    slots.push({
      slot: slot.id,
      team: slot.team,
      player_id: entry?.player_id ?? "?",
      ...(producer?.producer_id !== undefined ? { owner: producer.producer_id } : {}),
      ...(producer?.producer_kind !== undefined ? { owner_kind: producer.producer_kind } : {}),
      driver: producer ? (isLive ? "human" : "ai") : "pending",
      live: isLive,
      local_owner:
        producer !== undefined &&
        producer.producer_kind === "peer" &&
        producer.producer_id === model.peer_id,
      can_prefer:
        configurableOwnership && sameTeam && pairRequest(model, ports, slot.id) !== undefined,
    });
  }
  const keepers: LobbyKeeperView[] = [];
  for (const team of manifest.teams) {
    for (const player of team.roster) {
      if (player.position === "keeper") {
        keepers.push({ team: team.team, player_id: player.player_id });
      }
    }
  }
  return [slots, keepers];
}

function seatsView(ports: LobbyModelPorts, model: LobbyModel): readonly LobbySeatView[] {
  const state = model.coordinator;
  if (!state) {
    return [];
  }
  const assignments = visibleAssignments(ports, model);
  const order: string[] =
    state.role === "host" ? [...model.seating] : state.peers.map((peer) => peer.peer_id);
  const seats: LobbySeatView[] = [];
  order.forEach((peerId, index) => {
    const owned: InputSlotId[] = [];
    for (const producer of assignments ?? []) {
      if (producer.producer_kind === "peer" && producer.producer_id === peerId) {
        owned.push(producer.slot);
      }
    }
    let ready = false;
    for (const peer of state.peers) {
      if (peer.peer_id === peerId) {
        ready = peer.ready;
      }
    }
    seats.push({
      index: index + 1,
      peer_id: peerId,
      is_local: peerId === state.peer_id,
      ready,
      slots: owned,
    });
  });
  return seats;
}

function identityView(model: LobbyModel): readonly LobbyIdentityRow[] {
  const manifest = visibleManifest(model);
  const state = model.coordinator;
  return [
    { label: "MODE", value: manifest.match_mode },
    {
      label: "MANIFEST",
      value: state?.manifest_id ? state.manifest_id.slice(0, 12) : "unproposed",
    },
    { label: "BUILD", value: manifest.build_id },
    { label: "CONTENT", value: manifest.content_id },
    { label: "TUNING", value: manifest.tuning_id },
    { label: "RULES", value: manifest.combat_rules_id },
    { label: "COMBAT", value: manifest.combat_status },
  ];
}

export function view(ports: LobbyModelPorts, model: LobbyModel): LobbyView {
  const state = model.coordinator;
  const [slots, keepers] = rosterView(ports, model);
  const required = requiredHumans(ports, model);
  const connected = state?.peers.length ?? 0;
  let readyCount = 0;
  let ready = false;
  if (state) {
    for (const peer of state.peers) {
      if (peer.ready) {
        readyCount += 1;
        if (peer.peer_id === state.peer_id) {
          ready = true;
        }
      }
    }
  }
  const phase: SessionLifecyclePhase | "role" = state?.phase ?? "role";
  const terminal = state?.terminal;
  const departure = state?.departure;
  const preference = preferenceView(model);
  return {
    ...(model.role !== undefined ? { role: model.role } : {}),
    peer_id: model.peer_id,
    phase,
    mode: effectiveMode(model),
    mode_locked: state !== undefined && state.manifest_id !== undefined,
    mode_known: model.role === "host" || (state !== undefined && state.manifest !== undefined),
    required,
    connected,
    ready_count: readyCount,
    bot_fill: model.bot_fill,
    slots,
    keepers,
    seats: seatsView(ports, model),
    ...(preference !== undefined ? { preference } : {}),
    identity: identityView(model),
    ...(state?.countdown_remaining !== undefined ? { countdown: state.countdown_remaining } : {}),
    ...(departure !== undefined ? { departure } : {}),
    ...(departure !== undefined ? { departure_text: DEPARTURE_TEXT[departure.reason] } : {}),
    ...(terminal !== undefined ? { terminal } : {}),
    ...(terminal !== undefined ? { terminal_text: TERMINAL_TEXT[terminal.reason] } : {}),
    ...(model.exported !== undefined ? { exported: model.exported } : {}),
    ...(model.imported !== undefined ? { imported: model.imported } : {}),
    has_outgoing: model.outgoing !== undefined,
    status: model.status,
    ...(model.error !== undefined ? { error: model.error } : {}),
    can_invite:
      model.role === "host" &&
      phase === "handshake" &&
      model.pending_link === undefined &&
      connected < required,
    can_configure: model.role === "host" && configurable(model),
    ready,
    // The same "enough humans, or bot-fill" gate the old `can_lock` used --
    // START now performs that lock itself, so this is the whole of what
    // used to be two separately-gated buttons (#610). A host alone with
    // bot-fill on can still start: `connected` is always at least 1 (the
    // host itself is always one of its own peers), so `bot_fill` is what
    // carries that case exactly as it did for `can_lock`. (#610 round-2
    // review: an earlier draft also required `connected >= 1 || bot_fill`
    // here -- redundant, since `required` is never 0 for any real match
    // mode and `connected >= required` already implies `connected >= 1`;
    // removed rather than kept as decoration.)
    can_start:
      model.role === "host" && phase === "handshake" && (connected >= required || model.bot_fill),
    started: model.started,
    can_share: model.role === "host" && model.room_code !== undefined && ports.joinLink.canShare,
    ...(model.room_entry !== undefined ? { room_entry: model.room_entry } : {}),
    ...(model.room_code !== undefined ? { room_code: model.room_code } : {}),
    ...(model.room_status !== undefined ? { room_status: model.room_status } : {}),
    ...(model.room_error !== undefined ? { room_error: model.room_error } : {}),
    room_active: model.room_active,
    ...(model.last_dropped_signal !== undefined
      ? { last_dropped_signal: model.last_dropped_signal }
      : {}),
    ...(model.late_joiner_note !== undefined ? { late_joiner_note: model.late_joiner_note } : {}),
  };
}
