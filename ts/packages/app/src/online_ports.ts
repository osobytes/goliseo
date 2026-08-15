// Production `OnlinePorts` (`app.ts`) -- wires the real browser-star WebRTC
// transport (`@gc/transport`), the real online lobby/match screens
// (`@gc/screens`'s `OnlineLobby`/`OnlineMatch`), and the real session
// coordinator + online match driver (`@gc/wasm`'s `Coordinator`/
// `MatchDriverBridge`) into `App`'s injected `online` seam. Before this
// module existed, `browser_main.ts` never supplied `OnlinePorts` at all, so
// `App.showLobby` threw ("no online ports were injected into this App") --
// see `app.ts`'s header and this issue's own title for that gap.
//
// `browser_main.ts` supplies the browser-target wasm host
// (`browser_online_wasm_host.ts`); `online_ports.spec.ts` supplies the
// Node-target one (`@gc/wasm`'s `loadSimHost`, a devDependency, spec-only)
// against the exact same `OnlineWasmHost` contract (`online_wasm_host.ts`),
// so the spec drives THIS wiring, not a parallel reimplementation of it.
//
// ## The match-content gap, disclosed
//
// `LobbyModelPorts.protocolFixture` wraps `gc_netcode::protocol_fixture`
// (Rust-owned; ARCHITECTURE.md §1.1), which has no production
// content-pipeline counterpart yet -- `browser_content.ts`'s own header
// documents the identical gap for `matchContract` ("the real content
// pipeline... has no TypeScript-reachable export yet"). `matchDriverFixtureManifestJson`
// and the runtime-identity wire below are therefore the SAME fixture
// content the real-coordinator specs already drive for real
// (`lobby.spec.ts`'s `realCoordinatorPort`, `online_match_flow.spec.ts`'s
// `realMatchDriverPort`) -- not a shortcut invented for this file. Its
// rosters (`zyro_vex`, `mika_olu`, ... / `drell`, `morv`, ...) are exactly
// Nebula's and Orion's real starting fives (`session.ts`'s own
// `home_team_id`/`away_team_id` defaults), so an online match plays with
// real playable content; only the coordinator's own identity/versioning
// fields (`build_id`, `content_id`, ...) are fixture-fixed rather than
// derived from a real content pipeline. Fixing that is a content-pipeline
// milestone, not this issue's scope (#550 explicitly excludes "any change
// to the session protocol... or `gc-netcode`").
//
// ## What "real" means for the match driver's own transport
//
// `MatchDriverBridge` owns a SEPARATE internal transport
// (`initializeTransport`/`openPeer`/`enqueueInbound`/`drainOutboundJson`,
// `crates/gc-wasm/src/match_driver_bridge.rs`) from `LobbyLink`'s -- the
// lobby's session-coordinator control wire and the match driver's own
// input/control sync are two protocols sharing one star. `realMatchDriverPort`
// below bridges that internal transport to the SAME real star the lobby
// already opened (via `OnlineLobbyScreen.link.star` -- `app.ts`'s own
// comment: "the lobby keeps its link: the match borrows the same star"),
// polling it every `advance()` and draining `drainOutboundJson()` back out
// over it -- the same "poll inbound, mutate, drain outbound" shape
// `LobbyLink.poll`/`.send` already establish for the coordinator's own
// control channel. `OnlineMatch.drainControl` only takes over polling the
// star directly once the driver has gone non-active (its own comment says
// so), so there is no double-poll: `MatchDriverPort.batchControl` below
// hands the driver's own "control" channel inbound straight back to
// `OnlineMatch` while the driver IS active.

import { ok, err, type Result } from "@gc/core";
import type { KeyboardState } from "@gc/input";
import { frameBuffer } from "@gc/render";
import {
  LobbyLink,
  newFrameBuffer as newLobbyFrameBuffer,
  absorb as absorbLobbyFrame,
} from "@gc/online";
import {
  HOST_PEER_ID,
  MAX_GUESTS,
  browserStarEval,
  type StarTransportAdapter,
  type TransportChannel,
  type TransportMessage,
  type TransportRole,
} from "@gc/transport";
import {
  OnlineLobby,
  OnlineMatch,
  onlineMatchModelCommand,
  onlineMatchModelEnded,
  onlineMatchModelExitRoute,
  newOnlineMatchModel,
  MatchScreen,
  MatchScreenAsOnlineMatchScreen,
  ONLINE_MATCH_ABORT_PROMPT,
  type CoordinatorNewGuestOptions,
  type CoordinatorNewHostOptions,
  type CoordinatorOutcome,
  type CoordinatorPort,
  type CoordinatorState,
  type CoordinatorStateCore,
  type Fnv1a64Port,
  type InputFramePort,
  type InputSlotId,
  type InputTeam,
  type LobbyClipboard,
  type LobbyFrameBuffer,
  type LobbyLinkInstance,
  type LobbyModelPorts,
  type LobbyRole,
  type MatchContractPort,
  type MatchDriverPort,
  type MatchLobbyLinkPort,
  type MatchPresentationPort,
  type MatchSessionPort,
  type OnlineMatchCoordinatorEvent,
  type OnlineMatchCoordinatorPort,
  type OnlineMatchDispatchEvent,
  type OnlineMatchModel,
  type OnlineMatchModelPort,
  type OnlineMatchModelPorts,
  type OnlineMatchOptions,
  type OnlineMatchProtocolPort,
  type OnlineMatchRequest,
  type OnlineMatchState,
  type ProtocolFixturePort,
  type LobbyProtocolPort,
  type RenderPort,
  type RoomSignalingFactory,
  type SessionManifest,
  type SessionMatchMode,
  type SessionMatchModeShape,
  type SessionSlotProducer,
  type TeamData,
  type TransportContractPort,
} from "@gc/screens";
import { matchContract } from "./match_contract.ts";
import { matchObserver, type MatchObserver } from "./match_observer.ts";
import type {
  AppAction,
  OnlineLobbyCoordinatorState,
  OnlineLobbyScreen,
  OnlineMatchScreen,
  OnlinePorts,
} from "./app.ts";
import type { MatchContractContent } from "./content.ts";
import type { OnlineMatchDriverHandle, OnlineWasmHost } from "./online_wasm_host.ts";

export { browserStarEval };

// ---------------------------------------------------------------------------
// Protocol-level constants. Fixed and closed, mirroring `gc_sim::input_frame`'s
// canonical slot layout and `gc_netcode::protocol`'s match-mode shapes --
// wire/protocol constants, not `gc-data` content (AGENTS.md §8), and not
// wasm-bound (the same gap `lobby.spec.ts`'s own header records for
// `previewLive`/`plan_assignments`'s degenerate case). Transcribed from the
// same source those specs already transcribe them from.
// ---------------------------------------------------------------------------

const CANONICAL_SLOTS: readonly { readonly id: InputSlotId; readonly team: InputTeam }[] = [
  { id: "home_1", team: "home" },
  { id: "home_2", team: "home" },
  { id: "home_3", team: "home" },
  { id: "home_4", team: "home" },
  { id: "away_1", team: "away" },
  { id: "away_2", team: "away" },
  { id: "away_3", team: "away" },
  { id: "away_4", team: "away" },
];

const MATCH_MODE_SHAPES: Readonly<Record<SessionMatchMode, SessionMatchModeShape>> = {
  "1v1": { humans: 2, slots_per_human: 4, team_humans: 1 },
  "2v2": { humans: 4, slots_per_human: 2, team_humans: 2 },
  "4v4": { humans: 8, slots_per_human: 1, team_humans: 4 },
};

// `gc_netcode::protocol_fixture::runtime()`'s own literal fields
// (`crates/gc-netcode/src/protocol_fixture.rs`) -- not wasm-bound (only
// `manifest`/`freeze`/`peer_ids`/... are, per `match_driver_fixture_bridge.rs`'s
// own bound-surface list), so this is a transcription, the same one
// `lobby.spec.ts`'s `REAL_RUNTIME_WIRE` and `online_match_flow.spec.ts`'s
// identical constant already are.
const RUNTIME_IDENTITY_WIRE = {
  version: 1,
  runtime_id: "lovejs",
  runtime_revision: "lovejs.11.5.omp0",
  presentation_id: "presentation.2026-07-25",
  capabilities: ["combat_feedback.v1", "control_channel.v1", "input_channel.v1"],
} as const;

// The real, shipped content an online match actually plays with -- exactly
// `session.ts`'s own offline defaults (`home_team_id: "nebula"`,
// `away_team_id: "orion"`, `arena_id: "helios_crown"`), restated here rather
// than imported (this module has no reason to depend on `session.ts`'s
// session-flow concerns) with the same placeholder colors `browser_main.ts`
// uses until `gc-data`'s per-team presentation palette has a
// TypeScript-reachable route (ARCHITECTURE.md §4 rule 6).
const ONLINE_HOME_TEAM_ID = "nebula";
const ONLINE_AWAY_TEAM_ID = "orion";
const ONLINE_ARENA_ID = "helios_crown";
const ONLINE_HOME_TEAM: TeamData = {
  id: ONLINE_HOME_TEAM_ID,
  name: "Nebula FC",
  color: [0.35, 0.75, 1.0],
  formation: "2-1-1",
  roster: ["ozzo", "brakka", "veil_nyx", "rok_tann", "zyro_vex"],
  squad: [
    "ozzo",
    "brakka",
    "veil_nyx",
    "rok_tann",
    "zyro_vex",
    "mika_olu",
    "sela_dwin",
    "tib_quell",
  ],
};
const ONLINE_AWAY_TEAM: TeamData = {
  id: ONLINE_AWAY_TEAM_ID,
  name: "Orion Miners",
  color: [1.0, 0.55, 0.25],
  formation: "1-1-2",
  roster: ["gax_oru", "drell", "morv", "krag", "tox_vren"],
};

// Mirrors `browser_main.ts`'s own match constants -- `gc-sim`'s `match::new`
// defaults, restated for the same reason those are (no product screen ever
// overrides them).
const MATCH_DURATION_SECONDS = 120;
const MATCH_MAX_GOALS = 99;
const TICK_SECONDS = 1 / 60;

function slotIndexOf(slot: InputSlotId): number | undefined {
  const index = CANONICAL_SLOTS.findIndex((entry) => entry.id === slot);
  return index === -1 ? undefined : index + 1;
}

const inputFramePort: InputFramePort = {
  slotCount: CANONICAL_SLOTS.length,
  slot: (index: number) => CANONICAL_SLOTS[index - 1],
};

const protocolPort: LobbyProtocolPort = {
  matchModes: MATCH_MODE_SHAPES,
  // The real coordinator already encodes a "send" action's wire text
  // (`mapCoordinatorAction` below sets `message` to the coordinator's own
  // `wire` field) -- this port's `message` argument already IS the wire, so
  // `encode` is the identity, mirroring `online_match_flow.spec.ts`'s
  // `realProtocolPort` exactly.
  encode: (message: unknown) => message as string,
  slotIndex: slotIndexOf,
};

const transportContractPort: TransportContractPort = {
  hostPeerId: HOST_PEER_ID,
  maxGuests: MAX_GUESTS,
};

// A cosmetic digest only -- `LobbySignalRecord.fingerprint`, shown so two
// humans can eyeball-confirm they pasted the blob they think they did. NOT
// the pinned, cross-language `gc-core::fnv1a64` this port is named after:
// that one needs a shared-vector pin before a second implementation is safe
// to write (ARCHITECTURE.md §1.2; `lobby.spec.ts`'s header records the same
// reasoning for its own fake). This is a real FNV-1a-64, just never compared
// against a Rust-computed value -- a UI affordance, not wire protocol.
const FNV_OFFSET_BASIS = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const MASK_64 = 0xffffffffffffffffn;
function fnv1a64Hex(text: string): string {
  let hash = FNV_OFFSET_BASIS;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= BigInt(text.charCodeAt(index) & 0xff);
    hash = (hash * FNV_PRIME) & MASK_64;
  }
  return hash.toString(16).padStart(16, "0");
}
const fnv1a64Port: Fnv1a64Port = { hash: fnv1a64Hex };

function protocolFixturePort(wasm: OnlineWasmHost): ProtocolFixturePort {
  return {
    manifest: (mode: SessionMatchMode): SessionManifest =>
      JSON.parse(wasm.matchDriverFixtureManifestJson(mode)) as SessionManifest,
    runtime: () => RUNTIME_IDENTITY_WIRE,
  };
}

// ---------------------------------------------------------------------------
// The real session coordinator -- one `step` implementation shared by both
// the lobby's full `CoordinatorPort` (`lobby_model.ts`) and the match's
// `step`-only `CoordinatorPort<TState>` (`online_match_model.ts`), because
// it is the SAME live coordinator instance continuing from one into the
// other (`app.ts`'s `startOnlineMatch` reads `lobby.state.model.coordinator`
// straight through into `newOnlineMatchScreen`'s `coordinator` option --
// see this file's header). Mirrors `lobby.spec.ts`'s `realCoordinatorPort`
// and `online_match_flow.spec.ts`'s identically-named one, unified into one
// exhaustive `step` covering every event kind either caller dispatches.
// ---------------------------------------------------------------------------

/** A real coordinator state, carrying the live wasm handle through every
 * snapshot -- the one crossing between the stateful `Coordinator` class and
 * both callers' pure `step(state, event) -> [state, outcome]` shape.
 * Mirrors `lobby_flow.spec.ts`'s `toPublic`/`lobby.spec.ts`'s
 * `RealCoordState` boundary cast, not a structural one. */
interface RealCoordinatorState extends CoordinatorStateCore {
  readonly __handle: OnlineMatchDriverHandleCoordinator;
  readonly [key: string]: unknown;
}

// Renamed locally to avoid a naming collision with `online_wasm_host.ts`'s
// own `OnlineCoordinatorHandle` while keeping this file's imports flat.
type OnlineMatchDriverHandleCoordinator = import("./online_wasm_host.ts").OnlineCoordinatorHandle;

// `gc_wasm::coordinator_bridge`'s JSON encoder writes an explicit JSON
// `null` for every unset optional field -- `CoordinatorState`'s own optional
// fields are TypeScript `undefined`, and `lobby_model.ts`/`online_match_model.ts`
// check several of them with `!== undefined`. Left as JSON `null`, those
// checks would see "already set" on a session that never set it. Mirrors
// `lobby.spec.ts`'s/`online_match_flow.spec.ts`'s identical `denull`.
function denull(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(denull);
  }
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      out[key] = entry === null ? undefined : denull(entry);
    }
    return out;
  }
  return value;
}

function stateFromHandle(handle: OnlineMatchDriverHandleCoordinator): RealCoordinatorState {
  const parsed = denull(JSON.parse(handle.stateJson())) as Record<string, unknown>;
  return { ...parsed, __handle: handle } as unknown as RealCoordinatorState;
}

function handleOf(state: object): OnlineMatchDriverHandleCoordinator {
  return (state as unknown as RealCoordinatorState).__handle;
}

interface RawCoordinatorAction {
  readonly kind: string;
  readonly message?: unknown;
  readonly targets?: readonly string[];
  readonly link_id?: string;
  readonly freeze?: unknown;
  readonly terminal?: { readonly reason: string; readonly detail?: string };
}

function mapCoordinatorAction(raw: Record<string, unknown>): RawCoordinatorAction {
  const kind = raw["kind"];
  if (kind === "send") {
    return { kind: "send", message: raw["wire"], targets: raw["targets"] as readonly string[] };
  }
  if (kind === "close") {
    return { kind: "close", link_id: raw["link_id"] as string };
  }
  if (kind === "start_match") {
    return { kind: "start_match", freeze: raw["freeze"] };
  }
  if (kind === "terminate") {
    const terminal = raw["terminal"] as Record<string, unknown>;
    return {
      kind: "terminate",
      terminal: { reason: terminal["reason"] as string, detail: terminal["detail"] as string },
    };
  }
  // Anything else (there is nothing else today) passes through unrecognized
  // rather than throwing -- both `lobby_model.ts`'s `absorb` and
  // `online_match_model.ts`'s equivalent silently ignore an action kind
  // they do not recognize.
  return raw as unknown as RawCoordinatorAction;
}

interface RawCoordinatorOutcome {
  readonly accepted: boolean;
  readonly reason?: string;
  readonly actions: readonly RawCoordinatorAction[];
}

// Collects EVERY outcome's actions in one call (not just the last), because
// `tick()` drains its whole queue in one call and a "terminate"/"start_match"
// action from an earlier queued event must not be dropped -- the bug
// `online_match_flow.spec.ts`'s header records catching under the
// last-outcome-only version. `accepted`/`reason` still come from the LAST
// outcome, which is what a single dispatched (non-`tick`) event's own
// acceptance means; every other event kind here produces exactly one
// outcome anyway, so the two readings agree there.
function batchOutcomeFromJson(json: string): RawCoordinatorOutcome {
  const parsed = JSON.parse(json) as { readonly outcomes: readonly Record<string, unknown>[] };
  if (parsed.outcomes.length === 0) {
    throw new Error("online_ports: a coordinator call returned no outcome");
  }
  const actions: RawCoordinatorAction[] = [];
  let accepted = true;
  let reason: string | undefined;
  for (const outcome of parsed.outcomes) {
    for (const raw of outcome["actions"] as readonly Record<string, unknown>[]) {
      actions.push(mapCoordinatorAction(raw));
    }
    if (outcome["accepted"] !== true) {
      accepted = false;
      if (typeof outcome["reason"] === "string") {
        reason = outcome["reason"];
      }
    }
  }
  return { accepted, ...(reason !== undefined ? { reason } : {}), actions };
}

// Every coordinator event kind either `lobby_model.ts` (`connect`,
// `propose_manifest`, `assign_slots`, `prefer_pair`, `set_ready`,
// `begin_countdown`, `leave`, `abort`, `tick`, `control`, `link_lost`) or
// `online_match_model.ts` (`match_phase`, `hash_report`, `finish`,
// `netcode_failure`, `link_lost`, `abort`, `leave`, `control`, `tick`)
// dispatches -- confirmed exhaustively by reading both modules' own `step`
// call sites, not guessed.
function stepCoordinator(
  state: object,
  event: { readonly kind: string; readonly [key: string]: unknown },
): readonly [RealCoordinatorState, RawCoordinatorOutcome] {
  const handle = handleOf(state);
  let json: string;
  switch (event["kind"]) {
    case "connect":
      json = handle.connect();
      break;
    case "propose_manifest":
      json = handle.proposeManifest(JSON.stringify(event["manifest"]));
      break;
    case "assign_slots":
      json = handle.assignSlots(
        JSON.stringify(event["assignments"]),
        Boolean(event["preserve_claims"]),
      );
      break;
    case "prefer_pair":
      json = handle.preferPair(event["slots"] as string[]);
      break;
    case "set_ready":
      json = handle.setReady(Boolean(event["ready"]));
      break;
    case "begin_countdown":
      json = handle.beginCountdown(
        event["countdown_id"] as string,
        event["remaining_ticks"] as number,
        event["first_input_tick"] as number,
      );
      break;
    case "match_phase":
      json = handle.matchPhase(
        event["phase"] as string,
        event["tick"] as number,
        event["home_score"] as number,
        event["away_score"] as number,
      );
      break;
    case "hash_report":
      json = handle.hashReport(event["tick"] as number, event["boundary_hash"] as string);
      break;
    case "finish":
      json = handle.matchFinish(
        event["final_tick"] as number,
        event["home_score"] as number,
        event["away_score"] as number,
        event["final_hash"] as string,
      );
      break;
    case "netcode_failure":
      json = handle.netcodeFailure(
        event["failure"] as string,
        event["peer_id"] as string | undefined,
        event["detail"] as string | undefined,
      );
      break;
    case "leave":
      json = handle.leave();
      break;
    case "abort":
      json = handle.abort(
        event["code"] as string | undefined,
        event["detail"] as string | undefined,
      );
      break;
    case "control":
      handle.enqueueControlWire(event["link_id"] as string, event["wire"] as string);
      json = handle.tick();
      break;
    case "link_lost":
      handle.enqueueLinkLost(event["link_id"] as string, event["code"] as string | undefined);
      json = handle.tick();
      break;
    case "tick":
      json = handle.tick();
      break;
    default:
      throw new Error(`online_ports: unhandled coordinator event kind '${String(event["kind"])}'`);
  }
  return [stateFromHandle(handle), batchOutcomeFromJson(json)];
}

function ownedSlotsOfAssignments(
  assignments: readonly SessionSlotProducer[] | undefined,
  peerId: string,
): readonly InputSlotId[] {
  return (assignments ?? [])
    .filter((producer) => producer.producer_kind === "peer" && producer.producer_id === peerId)
    .map((producer) => producer.slot);
}

function previewLiveOfAssignments(
  assignments: readonly SessionSlotProducer[] | undefined,
): Readonly<Record<string, InputSlotId>> {
  const live: Record<string, InputSlotId> = {};
  for (const producer of assignments ?? []) {
    if (producer.producer_kind === "peer" && live[producer.producer_id] === undefined) {
      live[producer.producer_id] = producer.slot;
    }
  }
  return live;
}

// `gc_netcode::coordinator::plan_assignments`'s general team_humans/
// slots_per_human seat-distribution algorithm is a `pub fn` but is NOT part
// of `coordinator_bridge.rs`'s bound wasm surface -- confirmed by
// `lobby.spec.ts`'s own header, which documents the identical gap. This is
// therefore not a port of that algorithm; it is the same "falls straight
// out of `matchModes` shape data" contiguous-block layout `lobby.spec.ts`'s
// `realPlanAssignments` already uses for its one-peer case, generalized
// here to every seated peer (not just one) because production lobbies
// genuinely seat more than one human. Safe to generalize: the real
// `assignSlots` call independently validates whatever this proposes and
// rejects a wrong block boundary loudly rather than accepting it silently.
function planAssignmentsFor(
  manifest: SessionManifest,
  seating: readonly string[],
): readonly SessionSlotProducer[] | undefined {
  const shape = MATCH_MODE_SHAPES[manifest.match_mode];
  if (seating.length > shape.humans) {
    return undefined;
  }
  const peerBySlotIndex = new Map<number, string>();
  seating.forEach((peerId, order) => {
    for (let offset = 0; offset < shape.slots_per_human; offset += 1) {
      peerBySlotIndex.set(order * shape.slots_per_human + offset, peerId);
    }
  });
  // `gc_netcode::protocol::validate_slot_producer` requires `player_id` on
  // EVERY producer (peer or bot) and `bot_seed` on a bot one -- neither is
  // part of `SessionSlotProducer`'s own (narrower) declared shape, which
  // only carries what `lobby_model.ts`'s own view logic reads. Built
  // untyped and cast at the end, exactly mirroring `lobby.spec.ts`'s
  // `realPlanAssignments` (whose header explains why: "so the extra field
  // survives TypeScript's excess-property check").
  return CANONICAL_SLOTS.map((entry, index) => {
    const manifestSlot = manifest.slots[index];
    if (manifestSlot === undefined) {
      throw new Error("online_ports: manifest is missing a canonical slot");
    }
    const peerId = peerBySlotIndex.get(index);
    const raw =
      peerId !== undefined
        ? {
            slot: entry.id,
            team: entry.team,
            producer_kind: "peer",
            producer_id: peerId,
            player_id: manifestSlot.player_id,
          }
        : {
            slot: entry.id,
            team: entry.team,
            producer_kind: "bot",
            producer_id: `bot.${entry.id}`,
            player_id: manifestSlot.player_id,
            bot_seed: index + 1,
          };
    return raw as unknown as SessionSlotProducer;
  });
}

function createCoordinator(
  wasm: OnlineWasmHost,
  options: CoordinatorNewHostOptions | CoordinatorNewGuestOptions,
): RealCoordinatorState {
  const handle =
    options.role === "guest"
      ? new wasm.Coordinator(
          "guest",
          options.session_id,
          options.peer_id,
          options.host_peer_id,
          options.host_link_id,
          JSON.stringify(options.runtime),
          options.build_id,
          JSON.stringify(options.expectation),
        )
      : new wasm.Coordinator(
          "host",
          options.session_id,
          options.peer_id,
          undefined,
          undefined,
          JSON.stringify(options.runtime),
          options.build_id,
          undefined,
        );
  return stateFromHandle(handle);
}

/** The lobby's full `CoordinatorPort` (`lobby_model.ts`). */
function lobbyCoordinatorPort(wasm: OnlineWasmHost): CoordinatorPort {
  return {
    create: (options) => createCoordinator(wasm, options) as unknown as CoordinatorState,
    step: (state, event) =>
      stepCoordinator(state, event) as unknown as readonly [CoordinatorState, CoordinatorOutcome],
    planAssignments: planAssignmentsFor,
    ownedSlots: (state, peerId) =>
      ownedSlotsOfAssignments(
        (state as unknown as { readonly assignments?: readonly SessionSlotProducer[] }).assignments,
        peerId,
      ),
    previewLive: previewLiveOfAssignments,
    // Always false: safe (a republish the coordinator considers unchanged
    // is refused as idempotent, per `lobby_model.ts`'s own
    // `publishAssignments` comment), and matches `lobby.spec.ts`'s own
    // choice for the same reason.
    ownershipSeatsRoster: () => false,
  };
}

/** The match's `step`-only `CoordinatorPort<TState>` (`online_match_model.ts`),
 * over the SAME live coordinator handle the lobby's port continues from. */
function matchCoordinatorPort<
  TState extends CoordinatorStateCore,
>(): OnlineMatchCoordinatorPort<TState> {
  return {
    step: (state: TState, event: OnlineMatchCoordinatorEvent) =>
      stepCoordinator(state, event) as unknown as ReturnType<
        OnlineMatchCoordinatorPort<TState>["step"]
      >,
  };
}

// ---------------------------------------------------------------------------
// The real online match driver -- bridges `MatchDriverBridge`'s own
// internal transport to the real star. See this file's header.
// ---------------------------------------------------------------------------

/** Mirrors `@gc/wasm`'s internal (unexported) `binary_string.ts` convention
 * -- see that file's own header. Re-implemented locally rather than reached
 * into by relative path: `@gc/wasm`'s `package.json` `exports` field only
 * publishes `"."`/`"./web"`, and this is five lines either way. */
function byteStringFromBytes(bytes: Uint8Array): string {
  let out = "";
  const chunkSize = 4096;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    const chunk = bytes.subarray(offset, Math.min(offset + chunkSize, bytes.length));
    out += String.fromCharCode(...chunk);
  }
  return out;
}

function bytesFromByteString(value: string): Uint8Array {
  const bytes = new Uint8Array(value.length);
  for (let index = 0; index < value.length; index += 1) {
    bytes[index] = value.charCodeAt(index) & 0xff;
  }
  return bytes;
}

interface RealOnlineDriver {
  readonly bridge: OnlineMatchDriverHandle;
  readonly session: { readonly free: () => void };
  readonly roster: {
    readonly ids: readonly string[];
    readonly teams: readonly ("home" | "away")[];
  };
  tickCount: number;
}

interface RealOnlineCheckpoint {
  readonly tick: number;
  readonly hash: string;
}

interface RawTransportEnvelopeJson {
  readonly peer_id: string;
  readonly channel: TransportChannel;
  readonly message: {
    readonly version: number;
    readonly kind: "input" | "event" | "state";
    readonly seq: number;
    readonly tick?: number;
    readonly payload_bytes: readonly number[];
  };
}

interface RealOnlineBatch {
  readonly control: readonly RawTransportEnvelopeJson[];
  readonly checkpoints: readonly RealOnlineCheckpoint[];
  readonly live: Readonly<Record<string, string>>;
}

function rosterFromManifestJson(manifestJson: string): {
  readonly ids: readonly string[];
  readonly teams: readonly ("home" | "away")[];
} {
  const manifest = JSON.parse(manifestJson) as {
    readonly teams: readonly {
      readonly team: "home" | "away";
      readonly roster: readonly { readonly player_id: string }[];
    }[];
  };
  const ids: string[] = [];
  const teams: ("home" | "away")[] = [];
  for (const team of manifest.teams) {
    for (const player of team.roster) {
      ids.push(player.player_id);
      teams.push(team.team);
    }
  }
  return { ids, teams };
}

// Mirrors `gc_netcode::match_driver`'s own slot -> roster-player mapping:
// `MatchSessionPort.playerIndex` reads the manifest's own `slots` list
// (`{slot, player_id}`), not any array-order convention.
function realMatchSessionPort(manifestJson: string): MatchSessionPort<OnlineMatchState> {
  const manifest = JSON.parse(manifestJson) as {
    readonly slots: readonly { readonly slot: string; readonly player_id: string }[];
  };
  return {
    playerIndex(state, live) {
      const slot = manifest.slots.find((entry) => entry.slot === live);
      if (slot === undefined) {
        return undefined;
      }
      const index = state.players.findIndex((player) => player.id === slot.player_id);
      return index === -1 ? undefined : index;
    },
  };
}

// Never silently discarded -- see `reportTransportEvent`'s own doc for why
// a `star_error`/`peer_error`/`star_state` still has to reach *somewhere*
// observable even though this port has no coordinator handle to route a
// `netcode_failure` through.
function reportTransportEvent(event: {
  readonly kind: string;
  readonly peer_id?: string;
  readonly code?: string;
  readonly message?: string;
  readonly state?: string;
}): void {
  const detail =
    event.kind === "star_state"
      ? (event.state ?? "")
      : [event.code, event.message].filter((part) => part !== undefined).join(": ");
  console.error(
    `online_ports: transport ${event.kind}${event.peer_id !== undefined ? ` (peer ${event.peer_id})` : ""}${detail ? `: ${detail}` : ""}`,
  );
}

// Feeds every message currently queued on the real star (both channels)
// into the bridge's own inbound queue, and mirrors peer connect/disconnect
// events onto it -- the bridge's internal transport tracks its own peer
// registry separately from the star's, so a state change on one has to be
// told to the other explicitly (see this file's header).
//
// `star_error`/`peer_error`/`star_state` events used to be silently
// dropped here (only `peer_state` was read) -- the dev harness
// (`tools/browser_online_match/web/online_peer.ts`'s own inbound-drain
// loop) treats exactly these three as worth reporting, and this mirrors
// that intent. There is no coordinator handle at this layer to route a
// real `netcodeFailure` call through (that lives on `OnlineCoordinatorHandle`,
// owned by the session coordinator, not the match driver -- see this
// file's header on the two being separate protocols over one star), so
// `reportTransportEvent` is the seam: today it logs, and is the one place
// a future coordinator-level failure report would hook in from.
function drainStarIntoBridge(star: StarTransportAdapter, bridge: OnlineMatchDriverHandle): void {
  for (const received of star.pollBatch()) {
    bridge.enqueueInbound(
      received.peer_id,
      received.channel,
      received.message.type,
      received.message.seq,
      received.message.tick,
      bytesFromByteString(received.message.payload),
    );
  }
  let event = star.pollEvent();
  while (event !== null) {
    if (event.kind === "peer_state" && event.peer_id !== undefined) {
      if (event.state === "connected") {
        bridge.setPeerConnected(event.peer_id);
      } else if (
        event.state === "disconnected" ||
        event.state === "closed" ||
        event.state === "error"
      ) {
        bridge.setPeerDisconnected(event.peer_id, event.message ?? event.state);
      }
    } else if (
      event.kind === "star_error" ||
      event.kind === "peer_error" ||
      event.kind === "star_state"
    ) {
      reportTransportEvent(event);
    }
    event = star.pollEvent();
  }
}

function drainBridgeIntoStar(star: StarTransportAdapter, bridge: OnlineMatchDriverHandle): void {
  const outbound = JSON.parse(bridge.drainOutboundJson()) as readonly RawTransportEnvelopeJson[];
  for (const envelope of outbound) {
    const message: TransportMessage = {
      version: envelope.message.version,
      type: envelope.message.kind,
      seq: envelope.message.seq,
      ...(envelope.message.tick !== undefined ? { tick: envelope.message.tick } : {}),
      payload: byteStringFromBytes(new Uint8Array(envelope.message.payload_bytes)),
    };
    star.send(envelope.peer_id, envelope.channel, message);
  }
}

function controlEntriesFromBatch(entries: readonly RawTransportEnvelopeJson[]): readonly {
  readonly channel: string;
  readonly peer_id: string;
  readonly message: { readonly payload: string };
}[] {
  return entries
    .filter((entry) => entry.channel === "control")
    .map((entry) => ({
      channel: entry.channel,
      peer_id: entry.peer_id,
      message: { payload: byteStringFromBytes(new Uint8Array(entry.message.payload_bytes)) },
    }));
}

interface InputSampleLike {
  readonly move_x: number;
  readonly move_y: number;
  readonly held: number;
  readonly edges: number;
}

// Exported for `online_ports.spec.ts`'s direct coverage -- the netcode
// review that asked for it found this port (and `drainStarIntoBridge`/
// `drainBridgeIntoStar` below) had zero automated coverage of its own;
// every existing match-phase spec elsewhere in this tree drives a
// hand-rolled fake `MatchDriverPort`, never this production one. Not part
// of `OnlinePortsDeps`/`createOnlinePorts`'s own public contract -- a
// production caller only ever reaches this indirectly, through
// `OnlinePorts.newOnlineMatchScreen`.
export function realMatchDriverPort(
  wasm: OnlineWasmHost,
  star: StarTransportAdapter,
): MatchDriverPort<RealOnlineDriver, RealOnlineBatch, unknown, RealOnlineCheckpoint> {
  return {
    create(options) {
      const freezeJson = JSON.stringify(options.freeze);
      const manifestJson = JSON.stringify(options.manifest);
      const session = new wasm.Session(
        ONLINE_HOME_TEAM_ID,
        ONLINE_AWAY_TEAM_ID,
        // Determinism requires an agreed seed; the manifest is the one
        // value every peer independently arrives at identically (proposed
        // once by the host, validated byte-identical by every guest --
        // `gc_netcode::protocol::validate_manifest`), so a seed embedded in
        // it (when the fixture template sets one) is what every peer uses.
        // `0` matches `protocol_fixture::manifest`'s own unset-seed
        // convention when it carries none.
        (JSON.parse(manifestJson) as { readonly seed?: number }).seed ?? 0,
        MATCH_DURATION_SECONDS,
        MATCH_MAX_GOALS,
      );
      const bridge = new wasm.MatchDriverBridge(
        session,
        options.role,
        options.peer_id,
        freezeJson,
        manifestJson,
        MAX_GUESTS,
      );
      bridge.initializeTransport();
      // Mirror every peer link the star already opened (during the lobby
      // phase, before this driver existed) into the bridge's own registry
      // -- see this file's header on why the two transports track peer
      // state independently.
      for (const peerId of star.peerIds()) {
        if (peerId === options.peer_id) {
          continue;
        }
        bridge.openPeer(peerId);
        if (star.peerState(peerId) === "connected") {
          bridge.setPeerConnected(peerId);
        }
      }
      return { bridge, session, roster: rosterFromManifestJson(manifestJson), tickCount: 0 };
    },
    status: (d) => JSON.parse(d.bridge.statusJson()) as string,
    advance: (d, sample) => {
      drainStarIntoBridge(star, d.bridge);
      const s = sample as InputSampleLike;
      const wire = wasm.inputFrameNewSample(s.move_x, s.move_y, s.held, s.edges);
      const batchJson = d.bridge.advance(wire);
      drainBridgeIntoStar(star, d.bridge);
      d.tickCount += 1;
      return JSON.parse(batchJson) as RealOnlineBatch;
    },
    currentSnapshot: () => undefined,
    snapshot: (d, boundary) => d.bridge.snapshotLookup(boundary),
    terminal: (d) => {
      const raw = JSON.parse(d.bridge.terminalJson()) as {
        readonly status: string;
        readonly failure?: string;
        readonly detail?: string;
      } | null;
      return raw ?? undefined;
    },
    settled: (d) => JSON.parse(d.bridge.statusJson()) !== "active",
    diagnostics: (d) => {
      const raw = JSON.parse(d.bridge.diagnosticsJson()) as Record<string, unknown>;
      const status = JSON.parse(d.bridge.statusJson()) as string;
      return {
        transport_tick: Number(raw["step"] ?? 0),
        present_input_tick: Number(raw["input_tick"] ?? 0),
        confirmed_output_tick: Number(raw["confirmed_output_tick"] ?? 0),
        rollback_count: Number(raw["rollbacks"] ?? 0),
        correction_count: Number(raw["corrections"] ?? 0),
        predicted_slot_samples: 0,
        status,
      };
    },
    observeCheckpoint: (d, tick, hash) => {
      d.bridge.observeCheckpoint(tick, hash);
    },
    batchControl: (b) => controlEntriesFromBatch(b.control),
    batchCheckpoints: (b) => b.checkpoints,
    batchLive: (b) => b.live,
    // `wasm.buildMatchDriverRenderFrame` returns the raw flat wire (a
    // `Float64Array`, `gc_render::frame_buffer::encode`'s layout) -- this
    // used to be handed straight to `MatchScreen` as if it were already a
    // decoded `RenderFrame`, which is why `frame.players`/`.control` came
    // back `undefined` and `MatchScreen.onlineFrame` threw the moment
    // `OnlineMatch`'s own constructor read `this.match.state`
    // (`online_ports.spec.ts`'s match-phase continuation case is what
    // caught this). `frameBuffer.decode` is the same decode step
    // `browser_sim_host.ts`'s own `frame()` runs for the offline path.
    // `toRenderFrame` is not needed here: `MatchScreen.onlineFrame` reads
    // `frame.players`/`.control` directly and gets `roster` from a
    // SEPARATE `.roster()` call, never from inside the frame object, and
    // `DecodedRenderFrame` already carries `players`/`control` structurally
    // matching `OnlineRenderFramePlayers`/`OnlineRenderFrameControl`.
    frame: (d) => frameBuffer.decode(wasm.buildMatchDriverRenderFrame(d.bridge, 0)),
    roster: (d) => d.roster,
    tick: (d) => d.tickCount,
    dispose: (d) => {
      d.session.free();
    },
  };
}

// A straight passthrough -- `@gc/online`'s real `newOnlineMatchPresentation`
// needs a standalone `RollbackEventsPort` this milestone does not
// additionally wire (the match driver's own rollback-events feed is
// internal to `MatchDriverBridge`, per that class's own doc). Matches
// `online_match_flow.spec.ts`'s `identityMatchPresentation` exactly, and
// for the same reason: `OnlineMatch.driverSource().advance` only reads
// `presentedConfirmedSteps`, which this milestone's wiring never populates
// separately from `checkpoints`.
function identityMatchPresentation<TDriver, TBatch>(): MatchPresentationPort<
  Record<string, never>,
  TDriver,
  TBatch,
  TBatch
> {
  return {
    create: () => ({}),
    consume: (_presentation, _driver, batch) => batch,
    presentedOutputs: () => [],
    presentedEventDiffs: () => [],
    presentedConfirmedSteps: () => [],
    presentedCorrections: () => [],
  };
}

function contractPortFor(content: Pick<MatchContractContent, "teams">): MatchContractPort {
  return {
    newResult: (opts) => matchContract.newResult({ teams: content.teams }, opts),
  };
}

interface ObserverPort {
  create(state: OnlineMatchState): MatchObserver;
  observeConfirmed(observer: MatchObserver, step: unknown): boolean;
  finish(observer: MatchObserver): ReturnType<typeof matchObserver.finish>;
}

// `OnlineMatchState.players` (`online_match.ts`) carries `id`/`team`/`pos`/
// `facing` only -- no `is_keeper` (confirmed by reading `MatchScreen.onlineState`'s
// own builder in `match.ts`: it derives `players[]` from `roster().ids`/
// `.teams` and `frame().players`, neither of which threads `is_keeper`
// through for the ONLINE construction mode, unlike the offline path's
// `RealMatchScreenPort.state.roster`). A disclosed simplification, not a
// silent one: every online player observes as a non-keeper, so keeper-only
// stats (saves, claims) under-count for whichever slot is actually the
// keeper. Reported as a residual gap in this issue's PR rather than solved
// here -- fixing it needs `OnlineHostPort.roster()`/`RenderFrameRoster` to
// carry keeper identity for the online path too, a `@gc/screens`/`@gc/render`
// change outside this issue's scope.
function observerPortFor(): ObserverPort {
  return {
    create: (state) =>
      matchObserver.new({
        players: state.players.map((player) => ({
          id: player.id,
          team: player.team,
          is_keeper: false,
        })),
        events: [],
        score: state.score,
      }),
    observeConfirmed: (observer, step) =>
      matchObserver.observeConfirmed(
        observer,
        step as Parameters<typeof matchObserver.observeConfirmed>[1],
        TICK_SECONDS,
      ),
    finish: (observer) => matchObserver.finish(observer),
  };
}

// ---------------------------------------------------------------------------
// The lobby's real `LobbyLinkInstance` -- carries lobby control traffic over
// the real star via `@gc/online`'s `LobbyLink`, adapted to
// `online_lobby.ts`'s `LobbyLinkInstance` shape (`apply` returns a
// `[ok, error]` tuple; `@gc/online`'s own `apply` returns a `Result`).
// ---------------------------------------------------------------------------

function realLobbyLink(star: StarTransportAdapter): LobbyLinkInstance<StarTransportAdapter, never> {
  const link = new LobbyLink(star);
  return {
    star,
    send(linkId, wire) {
      link.send(linkId, wire);
    },
    apply(effect) {
      const result = link.apply(effect);
      return result.ok ? [true, undefined] : [false, result.error];
    },
    poll(): never[] {
      return link.poll() as unknown as never[];
    },
  };
}

// ---------------------------------------------------------------------------
// `OnlinePorts` itself.
// ---------------------------------------------------------------------------

export interface OnlinePortsDeps {
  /** The `@gc/wasm` surface (browser- or Node-target; see `online_wasm_host.ts`). */
  readonly wasm: OnlineWasmHost;
  /** Builds and initializes a fresh `StarTransportAdapter` for a lobby
   * role, or `undefined` if it failed to initialize -- the real
   * browser-star transport in production (`@gc/transport`'s `browserStar`),
   * a fake star in a spec. */
  readonly starFactory: (role: TransportRole, peerId: string) => StarTransportAdapter | undefined;
  readonly renderer: RenderPort;
  readonly keyboard: KeyboardState;
  /** Match content this online session plays with. Defaults to the real
   * shipped Nebula/Orion pair (`session.ts`'s own offline defaults) -- see
   * this file's header for what is and is not "real" about that content. */
  readonly content?: Pick<MatchContractContent, "teams">;
  /** Called once, the first time the online lobby screen is actually
   * entered (`OnlinePorts.newLobbyScreen`) -- #550's "on lobby entry fetch"
   * TURN-credentials hook. Never awaited here: a slow/failed fetch must
   * never block reaching the lobby (`turn_credentials.ts`'s own
   * degrade-silently contract). Optional so a spec that does not exercise
   * ICE at all can omit it. */
  readonly onLobbyEntry?: () => void;
  /** The manual-signaling copy/paste seam (`lobby.ts`'s "invite"/"paste"
   * commands). Defaults to `OnlineLobby`'s own no-op fallback (the offer/
   * answer blob is always ALSO shown as on-screen text either way, per
   * `lobby.ts`'s own layout -- a missing clipboard degrades the flow to
   * manual select/copy, it does not break it). `browser_main.ts` supplies
   * a real one; a spec typically omits this entirely. */
  readonly clipboard?: LobbyClipboard;
  /** The room-code signaling channel (#552) -- `room_signaling_port.ts`'s
   * real, WebSocket-backed factory in production (`browser_main.ts`), a
   * fake in a spec. Defaults to `OnlineLobby`'s own inert fallback (its
   * `room_open_host`/`room_open_guest` effects simply do nothing), so a
   * spec that never drives the room-code path can omit this entirely --
   * and so can a static-host deploy with no such Worker, which is exactly
   * the "manual signaling still works" fallback this issue's acceptance
   * criteria ask for. */
  readonly roomSignaling?: RoomSignalingFactory;
}

type RealOnlineMatchModel = OnlineMatchModel<RealCoordinatorState, OnlineMatchRequest>;

/** Builds the real `OnlinePorts` `App` (`app.ts`) is injected with. */
export function createOnlinePorts(deps: OnlinePortsDeps): OnlinePorts {
  const content = deps.content ?? {
    teams: { [ONLINE_HOME_TEAM_ID]: ONLINE_HOME_TEAM, [ONLINE_AWAY_TEAM_ID]: ONLINE_AWAY_TEAM },
  };
  const fixture = protocolFixturePort(deps.wasm);
  const modelPorts: LobbyModelPorts = {
    coordinator: lobbyCoordinatorPort(deps.wasm),
    protocol: protocolPort,
    protocolFixture: fixture,
    transportContract: transportContractPort,
    fnv1a64: fnv1a64Port,
    inputFrame: inputFramePort,
  };
  const matchModelPorts: OnlineMatchModelPorts<RealCoordinatorState> = {
    coordinator: matchCoordinatorPort<RealCoordinatorState>(),
    protocol: {
      // The real coordinator's `send` action already encodes its own final
      // wire text -- identity, mirroring `online_match_flow.spec.ts`'s
      // `realProtocolPort`.
      encode: (message: unknown) => message as string,
      // `crates/gc-wasm`'s protocol bridge decodes a wire's routing header
      // only, never its kind-specific body (`online_match_flow.spec.ts`'s
      // own `realProtocolPort` doc) -- `undefined` drops only an optional
      // `observe_hash` diagnostic effect, never coordinator correctness
      // (the coordinator itself sees every wire regardless, via
      // `enqueueControlWire`).
      decode: () => undefined,
    } satisfies OnlineMatchProtocolPort,
    localResult: (state) => (state as unknown as { readonly result?: unknown }).result,
    isCompleted: (terminal) => terminal.reason === "completed",
  };
  const matchModelPort: OnlineMatchModelPort<RealOnlineMatchModel> = {
    command: (model, event: OnlineMatchDispatchEvent) =>
      onlineMatchModelCommand(model, matchModelPorts, event),
    ended: onlineMatchModelEnded,
    exitRoute: onlineMatchModelExitRoute,
    ABORT_PROMPT: ONLINE_MATCH_ABORT_PROMPT,
  };

  return {
    // Wrapped rather than passed as a bare method reference: `ProtocolFixturePort.manifest`
    // is declared with method shorthand (`lobby_model.ts`), so extracting it
    // unbound would risk losing whatever `this` it might need -- it does
    // not need one here (a plain closure), but the wrapper is what makes
    // that true by construction rather than by accident.
    matchManifestTemplate: (mode: SessionMatchMode) => fixture.manifest(mode),

    newLobbyScreen(onAction: (action: AppAction) => void, options: unknown): OnlineLobbyScreen {
      deps.onLobbyEntry?.();
      const modelOptions = options as {
        readonly template?: (mode: SessionMatchMode) => SessionManifest;
      };
      const screen = new OnlineLobby({ w: 960, h: 540 }, onAction, {
        starFactory: (role: LobbyRole, peerId: string) => deps.starFactory(role, peerId),
        newLink: (star: StarTransportAdapter) => realLobbyLink(star),
        ...(deps.clipboard !== undefined ? { clipboard: deps.clipboard } : {}),
        ...(deps.roomSignaling !== undefined ? { roomSignaling: deps.roomSignaling } : {}),
        modelPorts,
        modelOptions: { seed: Math.floor(Date.now() % 1_000_000), ...modelOptions },
      });
      // Structural cast: `OnlineLobby`'s own `state`/`link` fields are
      // exactly what `app.ts`'s narrow `OnlineLobbyScreen` reads (see that
      // file's header for why it declares its own structural type rather
      // than importing `@gc/screens`'s).
      return screen as unknown as OnlineLobbyScreen;
    },

    requestMatchSession(options): Result<unknown, string> {
      const o = options as {
        readonly role: "host" | "guest";
        readonly peerId: string;
        readonly manifest: unknown;
        readonly freeze: unknown;
      };
      if (o.manifest === undefined || o.freeze === undefined) {
        return err("an online match needs a proposed manifest and a frozen session");
      }
      const manifest = o.manifest as { readonly match_mode: SessionMatchMode };
      const freeze = o.freeze as {
        readonly owned: Readonly<Record<string, readonly string[]>>;
        readonly live: Readonly<Record<string, string>>;
        readonly first_input_tick: number;
      };
      const live = freeze.live[o.peerId];
      const request: OnlineMatchRequest = {
        role: o.role,
        peer_id: o.peerId,
        mode: manifest.match_mode,
        freeze: o.freeze,
        manifest: o.manifest,
        initial_snapshot: undefined,
        first_input_tick: freeze.first_input_tick,
        home: ONLINE_HOME_TEAM,
        away: ONLINE_AWAY_TEAM,
        arena: { id: ONLINE_ARENA_ID },
        combat_enabled: false,
        ...(live !== undefined ? { live } : {}),
        owned: freeze.owned[o.peerId] ?? [],
      };
      return ok(request);
    },

    newOnlineMatchScreen(options): OnlineMatchScreen {
      const o = options as {
        readonly request: OnlineMatchRequest;
        readonly coordinator: OnlineLobbyCoordinatorState;
        readonly link: LobbyLinkInstance<StarTransportAdapter, never>;
        readonly onAction: (action: AppAction) => void;
      };
      const star = o.link.star;
      const manifestJson = JSON.stringify(o.request.manifest);
      const matchScreenOptions: OnlineMatchOptions<
        RealOnlineDriver,
        RealOnlineBatch,
        unknown,
        RealOnlineCheckpoint,
        Record<string, never>,
        RealOnlineBatch,
        RealCoordinatorState
      > = {
        request: o.request,
        coordinator: o.coordinator as unknown as RealCoordinatorState,
        link: o.link as unknown as MatchLobbyLinkPort,
        onAction: o.onAction,
        matchDriver: realMatchDriverPort(deps.wasm, star),
        matchPresentation: identityMatchPresentation(),
        matchSession: realMatchSessionPort(manifestJson),
        lobbyFraming: {
          // `@gc/online`'s `LobbyFrameBuffer` (a real `{count,next,bytes,chunks}`
          // reassembly buffer) and `@gc/screens`'s own `LobbyFrameBuffer`
          // (a deliberately opaque `{_opaque?: never}` marker -- that
          // package never looks inside one) share no properties, so this
          // crossing needs the two-step `unknown` cast, not a direct one.
          newBuffer: (): LobbyFrameBuffer => newLobbyFrameBuffer() as unknown as LobbyFrameBuffer,
          absorb: (buffer, payload) => {
            const result = absorbLobbyFrame(
              buffer as unknown as ReturnType<typeof newLobbyFrameBuffer>,
              payload,
            );
            if (!result.ok) {
              return [undefined, result.error];
            }
            return [result.value ?? undefined, undefined];
          },
        },
        contract: contractPortFor(content),
        observer: observerPortFor(),
        newMatch: (opts) => {
          const matchScreen = new MatchScreen(
            {
              createHost: () => {
                throw new Error(
                  "online_ports: an ONLINE-mode MatchScreen must never construct a base SimHost",
                );
              },
              renderer: deps.renderer,
              keyboard: deps.keyboard,
            },
            {
              profile: "online",
              online: opts.rollbackSource(),
              ...(opts.combatEnabled !== undefined ? { combat_enabled: opts.combatEnabled } : {}),
            },
          );
          return new MatchScreenAsOnlineMatchScreen(matchScreen);
        },
      };
      const match = new OnlineMatch(matchScreenOptions, matchModelPort, (request, coordinator) =>
        newOnlineMatchModel(request, coordinator),
      );
      // Structural cast: `OnlineMatch`'s `event`/`update`/`draw`/`teardown`/
      // `focusLost`/`controllerLost`/`applySettings` are exactly `app.ts`'s
      // narrow `OnlineMatchScreen` -- same reasoning as `newLobbyScreen`'s
      // cast above.
      return match as unknown as OnlineMatchScreen;
    },
  };
}
