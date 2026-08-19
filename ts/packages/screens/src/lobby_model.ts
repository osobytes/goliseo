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
  | { readonly kind: "room_send"; readonly to?: string; readonly signal: string }
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
}

/** A guest's in-progress room-code composer: `ROOM_CODE_LENGTH` character
 * slots (empty string until typed) plus the cursor position being edited.
 * Keyboard input types a character and advances the cursor; a controller
 * cycles the character at the cursor (up/down) and moves it (left/right) --
 * `lobby.ts`'s `update()` is where both are wired. */
export interface RoomCodeEntry {
  readonly chars: readonly string[];
  readonly cursor: number;
}

export type RoomSignalingStatus = "connecting" | "connected" | "failed";

// Mirrors `infra/src/room_code.ts`'s own alphabet and length exactly --
// see `@gc/online`'s `room_signaling.ts` for the identical, independently
// disclosed duplication on the wire-protocol side. `infra/` must stay
// outside the game's dependency graph (AGENTS.md §8/§11) and `@gc/screens`
// must stay outside `@gc/online`'s (this module's header), so the same
// constant is duplicated a second time here, for the code composer's
// character-cycling UI alone -- if the Worker's alphabet or length ever
// changes, this needs a matching update too.
export const ROOM_CODE_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export const ROOM_CODE_LENGTH = 6;

export const DEFAULT_MODE: SessionMatchMode = "4v4";
export const COUNTDOWN_ID = "countdown.1";
export const COUNTDOWN_TICKS = 180;
export const FIRST_INPUT_TICK = 0;
export const MAX_SIGNAL_BYTES = 16384;
export const GUEST_LINK_PREFIX = "guest_";
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
  let next: LobbyModel = { ...model, imported };
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
  effects.push({ kind: "room_send", ...(to !== undefined ? { to } : {}), signal: command.signal });
  const { outgoing: _outgoing, ...rest } = next;
  return {
    ...rest,
    status: direction === "offer" ? "Offer sent automatically." : "Answer sent automatically.",
  };
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
  const invited = invite({ ...model, room_queue: rest }, ports, effects);
  if (invited.pending_link === undefined) {
    // `invite()` refused (capacity, admission closed, ...) -- keep the
    // guest queued rather than dropping it; the next event that clears
    // `pending_link` will try again.
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

function roomEntry(chars: readonly string[], cursor: number): RoomCodeEntry {
  return { chars, cursor };
}

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
      room_entry: roomEntry(new Array(ROOM_CODE_LENGTH).fill(""), 0),
      status: "Enter the room code your host is showing.",
    };
  }
  effects.push({ kind: "room_open_host" });
  const { room_error: _roomError, ...rest } = model;
  return {
    ...rest,
    room_status: "connecting",
    room_active: true,
    status: "Requesting a room code.",
  };
}

function roomKey(model: LobbyModel, key: string): LobbyModel {
  const entry = model.room_entry;
  if (!entry) {
    return model;
  }
  if (key === "Backspace") {
    if (entry.chars[entry.cursor] === "" && entry.cursor === 0) {
      return model;
    }
    const chars = [...entry.chars];
    const cursor = chars[entry.cursor] !== "" ? entry.cursor : entry.cursor - 1;
    chars[cursor] = "";
    return { ...model, room_entry: roomEntry(chars, cursor) };
  }
  if (key.length !== 1) {
    return model;
  }
  const upper = key.toUpperCase();
  if (!ROOM_CODE_ALPHABET.includes(upper) || entry.cursor >= ROOM_CODE_LENGTH) {
    return model;
  }
  const chars = [...entry.chars];
  chars[entry.cursor] = upper;
  return {
    ...model,
    room_entry: roomEntry(chars, Math.min(ROOM_CODE_LENGTH - 1, entry.cursor + 1)),
  };
}

function roomCursor(model: LobbyModel, delta: unknown): LobbyModel {
  const entry = model.room_entry;
  if (!entry || typeof delta !== "number") {
    return model;
  }
  const cursor = Math.max(0, Math.min(ROOM_CODE_LENGTH - 1, entry.cursor + delta));
  return { ...model, room_entry: roomEntry(entry.chars, cursor) };
}

function roomCycle(model: LobbyModel, delta: unknown): LobbyModel {
  const entry = model.room_entry;
  if (!entry || typeof delta !== "number") {
    return model;
  }
  const current = entry.chars[entry.cursor] ?? "";
  const n = ROOM_CODE_ALPHABET.length;
  const index = current === "" ? -1 : ROOM_CODE_ALPHABET.indexOf(current);
  const nextIndex = (((index + delta) % n) + n) % n;
  const chars = [...entry.chars];
  chars[entry.cursor] = ROOM_CODE_ALPHABET[nextIndex] as string;
  return { ...model, room_entry: roomEntry(chars, entry.cursor) };
}

function roomSubmit(model: LobbyModel, effects: LobbyEffect[]): LobbyModel {
  const entry = model.room_entry;
  if (!entry || entry.chars.some((ch) => ch === "")) {
    return { ...model, error: "enter all six characters of the room code" };
  }
  const code = entry.chars.join("");
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
  const { room_entry: _entry, room_status: _status, room_error: _roomError, ...rest } = model;
  return { ...rest, room_active: false, status: "Host a session or join one with a pasted offer." };
}

function roomCreated(
  model: LobbyModel,
  ports: LobbyModelPorts,
  code: string,
  effects: LobbyEffect[],
): LobbyModel {
  const next = chooseRole(model, ports, "host", effects);
  const { room_error: _roomError, ...rest } = next;
  return { ...rest, room_code: code, room_status: "connected", status: `Room code ${code}.` };
}

function roomJoined(model: LobbyModel, ports: LobbyModelPorts, effects: LobbyEffect[]): LobbyModel {
  const next = chooseRole(model, ports, "guest", effects);
  const { room_error: _roomError, ...rest } = next;
  return { ...rest, room_status: "connected" };
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
  if (model.pending_link !== undefined) {
    return { ...model, room_queue: [...model.room_queue, guestId] };
  }
  const invited = invite(model, ports, effects);
  if (invited.pending_link === undefined) {
    // `invite()` refused (capacity, admission closed, ...) -- keep the
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
  effects: LobbyEffect[],
): LobbyModel {
  if (model.role === "host") {
    // A guest's signal can only ever be an answer to whichever invite is
    // currently pending -- this module never has more than one in flight
    // (`invite()`'s own "finish the pending invitation first" guard). A
    // signal for any other guest id is stray (e.g. arrived after that
    // guest already left) and is ignored rather than misrouted onto an
    // unrelated link.
    const linkId = model.pending_link;
    if (linkId === undefined || guestId === undefined || model.room_guest_map[linkId] !== guestId) {
      return model;
    }
  }
  return importSignal(model, ports, signal, effects);
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
    case "ready":
      next = setReady(next, ports, cmd.ready, effects);
      break;
    case "start":
      next = beginCountdown(next, ports, effects);
      break;
    case "leave":
      next = leave(next, ports, effects);
      break;
    case "tick":
      [next] = step(next, ports, { kind: "tick" }, effects);
      break;
    case "signal":
      next = onSignal(next, ports, cmd, effects);
      break;
    case "peer_connected":
      next = onPeerConnected(next, ports, cmd, effects);
      break;
    case "control":
      next = (() => {
        const [stepped] = step(
          next,
          ports,
          { kind: "control", link_id: cmd.link_id, wire: cmd.wire },
          effects,
        );
        return publishAssignments(stepped, ports, effects);
      })();
      break;
    case "link_lost":
      [next] = step(next, ports, { kind: "link_lost", link_id: cmd.link_id }, effects);
      break;
    case "link_error":
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
      next = roomJoined(next, ports, effects);
      break;
    case "room_guest_joined":
      next = roomGuestJoined(next, ports, cmd.guest_id, effects);
      break;
    case "room_guest_left":
      next = roomGuestLeft(next, ports, cmd.guest_id, effects);
      break;
    case "room_peer_signal":
      next = roomPeerSignal(next, ports, cmd.guest_id, cmd.signal, effects);
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
  readonly can_lock: boolean;
  /** Host-side ownership can still be republished. */
  readonly can_configure: boolean;
  readonly can_ready: boolean;
  readonly ready: boolean;
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
    can_lock:
      model.role === "host" && phase === "handshake" && (connected >= required || model.bot_fill),
    can_configure: model.role === "host" && configurable(model),
    can_ready: phase === "assigned" || phase === "ready",
    ready,
    can_start: model.role === "host" && phase === "ready",
    started: model.started,
    can_share: model.role === "host" && model.room_code !== undefined && ports.joinLink.canShare,
    ...(model.room_entry !== undefined ? { room_entry: model.room_entry } : {}),
    ...(model.room_code !== undefined ? { room_code: model.room_code } : {}),
    ...(model.room_status !== undefined ? { room_status: model.room_status } : {}),
    ...(model.room_error !== undefined ? { room_error: model.room_error } : {}),
    room_active: model.room_active,
  };
}
