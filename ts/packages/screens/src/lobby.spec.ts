// This spec mostly exercises `lobby.ts`/`lobby_model.ts` against a small
// hand-scripted `CoordinatorPort` fake (mirroring
// `match_presentation.spec.ts`'s `fakeRollbackEvents`) that tracks only
// phase/manifest/assignment bookkeeping, rather than the real coordinator
// (Rust-owned, `crates/gc-netcode`; ARCHITECTURE.md §1.1). Most of what it
// asserts turns out to be about `lobby.ts`/`lobby_model.ts`'s own layout
// and control-flow -- role choice, signaling digests, invitation effects,
// focus movement, terminal-reason text -- and none of that needs the
// coordinator to arbitrate admission, slot assignment, or pair preferences
// for real, so the fake is enough to give most of these assertions real
// coverage.
//
// Three cases genuinely need the coordinator's real slot-assignment and
// pair-preference protocol logic to produce meaningful output (exactly
// which slots read "LIVE" vs "AI (OWNED)" vs "AI FILL", and exactly which
// slots offer a pair control). `@gc/wasm` now declares `@gc/screens` as a
// dependent (this package carries it as a devDependency, resolvable from a
// spec file -- production code in this package still only ever receives a
// coordinator through the injected `CoordinatorPort`, never imports
// `@gc/wasm` directly), so the stated "no dependency edge" blocker is gone.
//
// That unblocks all three, driven for real below via `realCoordinatorPort()`,
// a thin adapter over `@gc/wasm`'s `Coordinator` -- the actual
// `gc_netcode::coordinator` reducer (`crates/gc-wasm/src/coordinator_bridge.rs`),
// not a reproduction of it. One genuine gap remains, and it is why "all
// three unblocked" needed rechecking rather than being assumed:
// `gc_netcode::coordinator::plan_assignments` (the team_humans/
// slots_per_human seat-distribution algorithm a host runs before publishing
// ownership) is a `pub fn` on the Rust side but is *not* part of
// `coordinator_bridge.rs`'s bound surface -- that file binds `assignSlots`,
// which validates an already-computed assignment, never the function that
// computes one from a manifest and a seating order. So `realPlanAssignments`
// below does not attempt a faithful, general port of it (that would be
// exactly the "re-implementing the coordinator's rules a second time"
// ARCHITECTURE.md §1.1 exists to prevent) -- it only covers the single
// degenerate case every case below actually needs: a lone connected host,
// bot-filling the rest. For that case the answer is not really an
// "algorithm" to reimplement, it falls straight out of `matchModes` shape
// data already injected via `ProtocolPort`: the one connected peer owns the
// first `slots_per_human` canonical slots, in canonical order, full stop.
// `realPlanAssignments` refuses (throws) for more than one seated peer
// rather than silently guessing at a multi-peer contiguous-block layout,
// and its output is independently validated by the real `assignSlots` call
// every case below makes -- a wrong block boundary fails loudly as a
// rejected assignment, not as a silently-wrong assertion. A case that
// needed two or more *simultaneously connected* humans (contiguous-block
// seating across peers, not just within one) would still have to stay
// `it.skip` on this same gap; none of the three below do.
//
// A fourth (`renders every state without touching a real display`) is
// unblocked below too, for an unrelated reason: `@gc/ui` (with its `draw`
// module and `GraphicsBackend`) *is* a declared dependency of this package
// (`online_lobby.ts` already draws through it), so the stated blocker --
// "this package does not own that seam" -- was stale.

import { describe, expect, it } from "vitest";
import { draw, hit } from "@gc/ui";
import type { GraphicsBackend, Layout } from "@gc/ui";
// `@gc/wasm` is a devDependency of this package (package.json), reachable
// from a spec file only -- see this file's header. Production code in this
// package (`lobby.ts`/`lobby_model.ts`) must keep receiving a coordinator
// through the injected `CoordinatorPort` instead.
import { loadSimHost } from "@gc/wasm";
import type { Coordinator } from "@gc/wasm";
import {
  newState,
  layout as lobbyLayout,
  update as lobbyUpdate,
  type LobbyScreenEvent,
  type LobbyScreenState,
} from "./lobby.ts";
import { ROOM_FAILURE_TEXT, view as lobbyModelView } from "./lobby_model.ts";
import type {
  CoordinatorAction,
  CoordinatorEvent,
  CoordinatorNewGuestOptions,
  CoordinatorNewHostOptions,
  CoordinatorOutcome,
  CoordinatorPort,
  CoordinatorState,
  Fnv1a64Port,
  InputFramePort,
  InputSlotId,
  LobbyModelPorts,
  ProtocolFixturePort,
  ProtocolPort,
  SessionManifest,
  SessionMatchMode,
  SessionSlotProducer,
  TransportContractPort,
} from "./lobby_model.ts";

const VP = { w: 960, h: 540 };

const SLOT_ORDER: readonly { readonly id: InputSlotId; readonly team: "home" | "away" }[] = [
  { id: "home_1", team: "home" },
  { id: "home_2", team: "home" },
  { id: "home_3", team: "home" },
  { id: "home_4", team: "home" },
  { id: "away_1", team: "away" },
  { id: "away_2", team: "away" },
  { id: "away_3", team: "away" },
  { id: "away_4", team: "away" },
];

function fakeInputFrame(): InputFramePort {
  return {
    slotCount: SLOT_ORDER.length,
    slot(index) {
      return SLOT_ORDER[index - 1];
    },
  };
}

function fakeProtocol(): ProtocolPort {
  return {
    matchModes: {
      "1v1": { humans: 2, slots_per_human: 4, team_humans: 1 },
      "2v2": { humans: 4, slots_per_human: 2, team_humans: 2 },
      "4v4": { humans: 8, slots_per_human: 1, team_humans: 4 },
    },
    encode(message) {
      return JSON.stringify(message);
    },
    slotIndex(slot) {
      const index = SLOT_ORDER.findIndex((entry) => entry.id === slot);
      return index === -1 ? undefined : index + 1;
    },
  };
}

function fakeManifest(mode: SessionMatchMode): SessionManifest {
  const rosterEntry = (playerId: string, position: string) => ({ position, player_id: playerId });
  return {
    session_id: "session_alpha",
    match_mode: mode,
    build_id: "build.fixture",
    source_id: "source.fixture",
    content_id: "content.fixture",
    tuning_id: "tuning.fixture",
    match_config_id: "match_config.fixture",
    fixture_id: "fixture.default_mixed.v1",
    arena_id: "arena.goliseo",
    combat_rules_id: "combat_interaction.fixture",
    gameplay_ai_policy_id: "gameplay_ai.fixture",
    combat_status: "provisional_fixture",
    slots: [
      { player_id: "zyro_vex" },
      { player_id: "mika_olu" },
      { player_id: "rok_tann" },
      { player_id: "sela_dwin" },
      { player_id: "drell" },
      { player_id: "morv" },
      { player_id: "krag" },
      { player_id: "tox_vren" },
    ],
    teams: [
      {
        team: "home",
        roster: [
          rosterEntry("ozzo", "keeper"),
          rosterEntry("zyro_vex", "forward"),
          rosterEntry("mika_olu", "defender"),
          rosterEntry("rok_tann", "midfielder"),
          rosterEntry("sela_dwin", "forward"),
        ],
      },
      {
        team: "away",
        roster: [
          rosterEntry("gax_oru", "keeper"),
          rosterEntry("drell", "defender"),
          rosterEntry("morv", "defender"),
          rosterEntry("krag", "midfielder"),
          rosterEntry("tox_vren", "forward"),
        ],
      },
    ],
  };
}

function fakeProtocolFixture(): ProtocolFixturePort {
  return {
    manifest: fakeManifest,
    runtime: () => ({ runtime_id: "fixture" }),
  };
}

function fakeTransportContract(): TransportContractPort {
  return { hostPeerId: "host", maxGuests: 7 };
}

function fakeFnv1a64(): Fnv1a64Port {
  return {
    hash(text) {
      let h = 0;
      for (let i = 0; i < text.length; i += 1) {
        h = (h * 31 + text.charCodeAt(i)) | 0;
      }
      return Math.abs(h).toString(16).padStart(8, "0");
    },
  };
}

// Phase/manifest/assignment bookkeeping only -- no admission, slot, or
// preference arbitration. See this file's header.
function fakeCoordinatorPort(): CoordinatorPort {
  return {
    create(options: CoordinatorNewHostOptions | CoordinatorNewGuestOptions): CoordinatorState {
      return {
        role: options.role,
        peer_id: options.peer_id,
        phase: "handshake",
        // The host counts itself as a seated human; a guest starts having
        // only heard of itself too.
        peers: [{ peer_id: options.peer_id, ready: false }],
      };
    },
    step(
      state: CoordinatorState,
      event: CoordinatorEvent,
    ): readonly [CoordinatorState, CoordinatorOutcome] {
      switch (event["kind"]) {
        case "propose_manifest": {
          const manifest = event["manifest"] as SessionManifest;
          return [
            {
              ...state,
              phase: "manifest",
              manifest,
              manifest_id: "manifest-fixture",
              peers: state.peers.map((peer) => ({
                ...peer,
                accepted_manifest_id: "manifest-fixture",
              })),
            },
            { accepted: true, actions: [] },
          ];
        }
        case "assign_slots": {
          const assignments = event["assignments"] as readonly SessionSlotProducer[];
          return [
            { ...state, phase: "assigned", assignments },
            { accepted: true, actions: [] },
          ];
        }
        case "set_ready": {
          const ready = event["ready"] as boolean;
          return [
            { ...state, phase: "ready", peers: state.peers.map((peer) => ({ ...peer, ready })) },
            { accepted: true, actions: [] },
          ];
        }
        case "begin_countdown": {
          const action: CoordinatorAction = { kind: "start_match", freeze: {} };
          return [
            {
              ...state,
              phase: "countdown",
              countdown_remaining: event["remaining_ticks"] as number,
            },
            { accepted: true, actions: [action] },
          ];
        }
        case "abort": {
          return [
            { ...state, phase: "terminal", terminal: { reason: "local_abort" } },
            { accepted: true, actions: [] },
          ];
        }
        case "leave": {
          return [
            { ...state, phase: "terminal", terminal: { reason: "guest_left" } },
            { accepted: true, actions: [] },
          ];
        }
        default:
          return [state, { accepted: true, actions: [] }];
      }
    },
    planAssignments(
      manifest: SessionManifest,
      seating: readonly string[],
    ): readonly SessionSlotProducer[] | undefined {
      // Not a faithful port of the real assignment algorithm -- see this
      // file's header. Seats every slot, humans first from `seating`, the
      // rest to a shared bot producer; enough to keep `lock` from stalling.
      return manifest.slots.map((_, index) => {
        const entry = SLOT_ORDER[index];
        if (entry === undefined) {
          throw new Error("fixture manifest exceeds the canonical slot count");
        }
        const human = seating[0];
        return {
          producer_kind: human !== undefined ? "peer" : "bot",
          producer_id: human ?? "bot",
          team: entry.team,
          slot: entry.id,
        } satisfies SessionSlotProducer;
      });
    },
    ownedSlots(state: CoordinatorState, peerId: string): readonly InputSlotId[] {
      return (state.assignments ?? [])
        .filter((producer) => producer.producer_kind === "peer" && producer.producer_id === peerId)
        .map((producer) => producer.slot);
    },
    previewLive(assignments) {
      const live: Record<string, InputSlotId> = {};
      for (const producer of assignments ?? []) {
        if (producer.producer_kind === "peer" && live[producer.producer_id] === undefined) {
          live[producer.producer_id] = producer.slot;
        }
      }
      return live;
    },
    ownershipSeatsRoster() {
      return false;
    },
  };
}

function ports(): LobbyModelPorts {
  return {
    coordinator: fakeCoordinatorPort(),
    protocol: fakeProtocol(),
    protocolFixture: fakeProtocolFixture(),
    transportContract: fakeTransportContract(),
    fnv1a64: fakeFnv1a64(),
    inputFrame: fakeInputFrame(),
  };
}

function clickOn(
  currentLayout: Layout,
  id: string,
): { readonly kind: "click"; readonly x: number; readonly y: number } {
  const widget = hit.find(currentLayout, id);
  if (!widget?.rect) {
    throw new Error(`missing widget ${id}`);
  }
  return {
    kind: "click",
    x: widget.rect.x + widget.rect.w / 2,
    y: widget.rect.y + widget.rect.h / 2,
  };
}

function click(state: LobbyScreenState, id: string): LobbyScreenState {
  const [next] = lobbyUpdate(state, clickOn(lobbyLayout(state), id));
  return next;
}

function dispatch(state: LobbyScreenState, event: LobbyScreenEvent): LobbyScreenState {
  const [next] = lobbyUpdate(state, event);
  return next;
}

function view(state: LobbyScreenState) {
  return lobbyModelView(state.ports, state.model);
}

function assertWithinCanvas(currentLayout: Layout): void {
  for (const widget of currentLayout) {
    if (!widget.rect) {
      continue;
    }
    expect(widget.rect.x).toBeGreaterThanOrEqual(0);
    expect(widget.rect.y).toBeGreaterThanOrEqual(0);
    expect(widget.rect.x + widget.rect.w).toBeLessThanOrEqual(VP.w);
    expect(widget.rect.y + widget.rect.h).toBeLessThanOrEqual(VP.h);
  }
}

function hosting(): LobbyScreenState {
  return click(newState(VP, ports()), "role_host");
}

// Every method a no-op: real draw code executes against this, so a missing
// field or a bad projection fails here rather than on a device.
// `getDimensions` answers a fixed 1280x720.
function fakeGraphicsBackend(): GraphicsBackend {
  const noop = () => {};
  return {
    getDimensions: () => ({ width: 1280, height: 720 }),
    setColor: noop,
    setLineWidth: noop,
    rectangle: noop,
    circle: noop,
    ellipse: noop,
    polygon: noop,
    line: noop,
    print: noop,
    printf: noop,
    // Enough for `draw.ts`'s centring arithmetic to run: a fixed 13px line,
    // one line per ~7px of width. It is not a font, and no case here asserts
    // pixel positions -- these specs prove drawing does not throw.
    measureText: (text: string, wrapWidth: number) => {
      const lines = wrapWidth > 0 ? Math.max(1, Math.ceil((text.length * 7) / wrapWidth)) : 1;
      return { lines, lineHeight: 16.25, height: (lines - 1) * 16.25 + 13 };
    },
    push: noop,
    pop: noop,
    translate: noop,
    scale: noop,
    setFont: noop,
  };
}

// ---------------------------------------------------------------------------
// Real-coordinator fixtures -- see this file's header for what these do and
// do not prove, and for the two remaining gaps (`plan_assignments` not
// wasm-bound; the wire manifest/runtime shape).
// ---------------------------------------------------------------------------

// `gc_netcode::protocol::validate_runtime` requires exactly this shape
// (`RUNTIME_FIELDS`, version pinned to `RUNTIME_IDENTITY_VERSION`).
// Transcribed verbatim from `crates/gc-netcode/src/protocol_fixture.rs`'s
// own `runtime()` -- the same literal the Rust suite itself pins against.
// Content, not logic (AGENTS.md §8).
const REAL_RUNTIME_WIRE = {
  version: 1,
  runtime_id: "lovejs",
  runtime_revision: "lovejs.11.5.omp0",
  presentation_id: "presentation.2026-07-25",
  capabilities: ["combat_feedback.v1", "control_channel.v1", "input_channel.v1"],
};

// `gc_netcode::protocol::validate_manifest` requires every one of these
// fields, several pinned to exact version constants
// (`protocol::{VERSION,MANIFEST_VERSION}`, `gc_sim::input_frame::VERSION`,
// `gc_sim::match_snapshot::COMBAT_VERSION`, `gc_sim::input_tape::COMBAT_VERSION`,
// `gc_sim::combat_snapshot::VERSION`), `combat_status` restricted to
// `COMBAT_STATUSES`, `tick_rate` pinned to `gc_sim::fixed_clock::TICK_RATE`.
// `lobby_model.ts`'s own `SessionManifest` (used by the fake `CoordinatorPort`
// above) has none of this -- it only carries what this package's own view
// logic reads. Transcribed verbatim from
// `crates/gc-netcode/src/protocol_fixture.rs`'s own `manifest()` (values)
// plus the version constants it composes from `gc-sim`/`gc-netcode`
// (gathered by reading those crates' `pub const`s), not derived or guessed.
// Built untyped and cast at the end (mirroring `lobby_flow.spec.ts`'s
// `toPublic`) so the extra fields survive TypeScript's excess-property
// check on `SessionManifest`'s narrower nested shapes.
function realManifest(mode: SessionMatchMode): SessionManifest {
  const player = (playerId: string, position: string, loadoutId?: string, familyId?: string) => ({
    player_id: playerId,
    position,
    ...(loadoutId !== undefined ? { loadout_id: loadoutId } : {}),
    ...(familyId !== undefined ? { family_id: familyId } : {}),
  });
  const home = [
    player("ozzo", "keeper"),
    player("zyro_vex", "forward", "loadout_spring_gloves", "unarmed"),
    player("mika_olu", "defender", "loadout_emberguard_shield", "guard"),
    player("rok_tann", "midfielder", "loadout_vector_blade", "light_melee"),
    player("sela_dwin", "forward", "loadout_prism_launcher", "ranged"),
  ];
  const away = [
    player("gax_oru", "keeper"),
    player("drell", "defender", "loadout_spring_gloves", "unarmed"),
    player("morv", "defender", "loadout_emberguard_shield", "guard"),
    player("krag", "midfielder", "loadout_vector_blade", "light_melee"),
    player("tox_vren", "forward", "loadout_prism_launcher", "ranged"),
  ];
  const outfieldIds = [
    "zyro_vex",
    "mika_olu",
    "rok_tann",
    "sela_dwin",
    "drell",
    "morv",
    "krag",
    "tox_vren",
  ];
  const slots = outfieldIds.map((playerId, index) => {
    const entry = SLOT_ORDER[index];
    if (entry === undefined) {
      throw new Error("fixture slot order is missing a canonical slot");
    }
    return { slot: entry.id, team: entry.team, player_id: playerId };
  });
  const raw = {
    version: 1,
    session_id: "session_alpha",
    protocol_version: 1,
    input_version: 2,
    snapshot_version: 14,
    tape_version: 2,
    combat_schema_version: 3,
    build_id: "build.97b60ea",
    source_id: "source.97b60ea",
    content_id: "content.omp3.v1",
    tuning_id: "tuning.omp3.v1",
    match_config_id: "match_config.direct_host.v1",
    fixture_id: "fixture.default_mixed.v1",
    arena_id: "arena.goliseo",
    combat_rules_id: "combat_interaction.accepted_2026_07_23",
    gameplay_ai_policy_id: "gameplay_ai.combat.v1",
    combat_status: "provisional_114",
    seed: 20001,
    tick_rate: 60,
    duration_ticks: 7200,
    max_goals: 99,
    match_mode: mode,
    teams: [
      { team: "home", team_id: "team_nova", roster: home },
      { team: "away", team_id: "team_void", roster: away },
    ],
    slots,
  };
  return raw as unknown as SessionManifest;
}

function realProtocolFixture(): ProtocolFixturePort {
  return { manifest: realManifest, runtime: () => REAL_RUNTIME_WIRE };
}

// The Rust-side seat-distribution algorithm's degenerate, single-peer case
// -- see this file's header for why this is not a port of the general
// algorithm. `SLOT_PRODUCER_FIELDS`'s wire shape (`gc_netcode::protocol`)
// needs `player_id` and, for a bot, an integer `bot_seed`; neither is part
// of `SessionSlotProducer`'s own (narrower) shape, so the raw records are
// built untyped and cast, same reasoning as `realManifest`.
function realPlanAssignments(
  protocol: ProtocolPort,
  manifest: SessionManifest,
  seating: readonly string[],
): readonly SessionSlotProducer[] {
  if (seating.length > 1) {
    throw new Error(
      "realPlanAssignments only covers a single connected peer -- see this file's header",
    );
  }
  const shape = protocol.matchModes[manifest.match_mode];
  const humanPeer = seating[0];
  return SLOT_ORDER.map((entry, index) => {
    const manifestSlot = manifest.slots[index];
    if (manifestSlot === undefined) {
      throw new Error("fixture manifest is missing a canonical slot");
    }
    const owned = humanPeer !== undefined && index < shape.slots_per_human;
    const raw = owned
      ? {
          producer_kind: "peer",
          producer_id: humanPeer,
          team: entry.team,
          slot: entry.id,
          player_id: manifestSlot.player_id,
        }
      : {
          producer_kind: "bot",
          producer_id: `bot_${entry.id}`,
          team: entry.team,
          slot: entry.id,
          player_id: manifestSlot.player_id,
          bot_seed: index + 1,
        };
    return raw as unknown as SessionSlotProducer;
  });
}

// `gc_wasm::coordinator_bridge`'s JSON encoder writes an explicit JSON
// `null` for every unset optional field (`state_to_json`'s
// `Json::opt_str`/`.map_or(Json::Null, ...)` calls) -- `CoordinatorState`'s
// own optional fields are TypeScript `undefined`, and `lobby_model.ts`
// checks several of them with `!== undefined` (`state.manifest_id`,
// `peer.accepted_manifest_id`, ...). Left as JSON `null`, those checks
// would see "already set" on a session that never proposed anything.
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

// The one crossing between the real wasm `Coordinator` (a stateful class:
// every call mutates one instance and returns an outcome) and
// `CoordinatorPort`'s pure `step(state, event) -> [state, outcome]` shape
// `lobby_model.ts` threads immutably. `__handle` carries the live instance
// through every `CoordinatorState` snapshot this adapter hands back; a
// `step` call ignores the *content* of the state it's given and always
// mutates the one instance reachable through it, then re-derives a fresh
// snapshot from `stateJson()`. Mirrors `lobby_flow.spec.ts`'s `toPublic`
// boundary cast, not a structural one.
interface RealCoordState extends CoordinatorState {
  readonly __handle: Coordinator;
}

function stateFromHandle(handle: Coordinator): CoordinatorState {
  const parsed = denull(JSON.parse(handle.stateJson())) as Record<string, unknown>;
  return { ...parsed, __handle: handle } as unknown as CoordinatorState;
}

function handleOf(state: CoordinatorState): Coordinator {
  return (state as unknown as RealCoordState).__handle;
}

function mapAction(raw: Record<string, unknown>): CoordinatorAction {
  const kind = raw["kind"];
  if (kind === "send") {
    return {
      kind: "send",
      message: raw["wire"],
      targets: raw["targets"],
    } as unknown as CoordinatorAction;
  }
  if (kind === "close") {
    return { kind: "close", link_id: raw["link_id"] } as unknown as CoordinatorAction;
  }
  if (kind === "start_match") {
    return { kind: "start_match", freeze: raw["freeze"] } as unknown as CoordinatorAction;
  }
  // "terminate" -- `lobby_model.ts`'s `absorb` only recognizes
  // send/close/start_match and silently ignores anything else; the
  // terminal reason itself is read off `state.terminal`
  // (`stateFromHandle`, sourced from the coordinator's own state), so
  // nothing observable is lost by passing it through unrecognized.
  return raw as unknown as CoordinatorAction;
}

// Every `Coordinator` method used below returns `{"outcomes": [...],
// "summary": {...}}` -- one outcome per applied event, always ending with
// the outcome of whichever event this call itself applied (`Coordinator`'s
// own doc). A single-shot call (everything except `tick`) always has
// exactly one.
function batchOutcome(json: string): CoordinatorOutcome {
  const parsed = JSON.parse(json) as { readonly outcomes: readonly Record<string, unknown>[] };
  const outcome = parsed.outcomes[parsed.outcomes.length - 1];
  if (outcome === undefined) {
    throw new Error("a coordinator call returned no outcome");
  }
  return {
    accepted: outcome["accepted"] as boolean,
    ...(typeof outcome["reason"] === "string" ? { reason: outcome["reason"] } : {}),
    actions: (outcome["actions"] as readonly Record<string, unknown>[]).map(mapAction),
  };
}

// Drives the real `@gc/wasm` `Coordinator` through `CoordinatorPort`'s
// interface. Only wires the event kinds the three real-coordinator cases
// below actually dispatch (host-only sessions: create, propose_manifest,
// assign_slots, prefer_pair) -- an event kind outside that set throws
// rather than silently no-op-ing, since this fixture makes no claim to be a
// general `CoordinatorPort` implementation.
function realCoordinatorPort(): CoordinatorPort {
  return {
    create(options) {
      const host = loadSimHost();
      const handle =
        options.role === "guest"
          ? new host.Coordinator(
              "guest",
              options.session_id,
              options.peer_id,
              options.host_peer_id,
              options.host_link_id,
              JSON.stringify(options.runtime),
              options.build_id,
              JSON.stringify(options.expectation),
            )
          : new host.Coordinator(
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
    },
    step(state, event) {
      const handle = handleOf(state);
      let json: string;
      switch (event["kind"]) {
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
        default:
          throw new Error(
            `realCoordinatorPort: unhandled event kind '${String(event["kind"])}' -- this fixture only wires what the real-coordinator lobby cases exercise`,
          );
      }
      return [stateFromHandle(handle), batchOutcome(json)];
    },
    planAssignments(manifest, seating) {
      return realPlanAssignments(fakeProtocol(), manifest, seating);
    },
    ownedSlots(state, peerId) {
      return (state.assignments ?? [])
        .filter((producer) => producer.producer_kind === "peer" && producer.producer_id === peerId)
        .map((producer) => producer.slot);
    },
    // Matches `gc_netcode::coordinator::preview_live` exactly ("the first
    // owned slot in canonical order, derived from the assignments alone" --
    // that function's own doc comment): a one-line fold with no design
    // choice left in it, unlike seat planning. Not wasm-bound either, but
    // safe to state directly for that reason.
    previewLive(assignments) {
      const live: Record<string, InputSlotId> = {};
      for (const producer of assignments ?? []) {
        if (producer.producer_kind === "peer" && live[producer.producer_id] === undefined) {
          live[producer.producer_id] = producer.slot;
        }
      }
      return live;
    },
    ownershipSeatsRoster() {
      return false;
    },
  };
}

function realPorts(): LobbyModelPorts {
  return {
    coordinator: realCoordinatorPort(),
    protocol: fakeProtocol(),
    protocolFixture: realProtocolFixture(),
    transportContract: fakeTransportContract(),
    fnv1a64: fakeFnv1a64(),
    inputFrame: fakeInputFrame(),
  };
}

// Locks a real-coordinator, single-human, bot-filled session in `mode` --
// the setup every case below shares.
function lockedRealHost(mode: SessionMatchMode): LobbyScreenState {
  let state = click(newState(VP, realPorts()), "role_host");
  state = click(state, `mode_${mode}`);
  state = dispatch(state, { kind: "lobby", command: { kind: "bot_fill" } });
  state = dispatch(state, { kind: "lobby", command: { kind: "lock" } });
  return state;
}

describe("online lobby screen", () => {
  it("opens on an explicit host or guest choice", () => {
    const state = newState(VP, ports());
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "role_host")).not.toBeNull();
    expect(hit.find(currentLayout, "role_guest")).not.toBeNull();
    expect(hit.find(currentLayout, "invite")).toBeNull();
    expect(view(state).phase).toBe("role");
  });

  it("offers a room-code path alongside the manual one, on the pre-role screen", () => {
    const state = newState(VP, ports());
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "role_host")).not.toBeNull();
    expect(hit.find(currentLayout, "role_guest")).not.toBeNull();
    expect(hit.find(currentLayout, "room_code_host")).not.toBeNull();
    expect(hit.find(currentLayout, "room_code_join")).not.toBeNull();
  });

  it("cycles the guest identity that answers the host's Nth invitation", () => {
    let state = newState(VP, ports());
    expect(view(state).peer_id).toBe("guest_1");
    state = click(state, "identity");
    expect(view(state).peer_id).toBe("guest_2");
    state = click(state, "role_guest");
    expect(view(state).role).toBe("guest");
    // Identity is baked into the coordinator, so it stops moving.
    state = dispatch(state, { kind: "lobby", command: { kind: "identity" } });
    expect(view(state).peer_id).toBe("guest_2");
  });

  it("offers exactly the supported match modes to a host", () => {
    const state = hosting();
    const currentLayout = lobbyLayout(state);
    for (const mode of ["1v1", "2v2", "4v4"]) {
      expect(hit.find(currentLayout, `mode_${mode}`)).not.toBeNull();
    }
    expect(hit.find(currentLayout, "mode_3v3")).toBeNull();
  });

  it("derives the required peer count from the selected mode", () => {
    let state = hosting();
    expect(view(state).required).toBe(8);
    state = click(state, "mode_2v2");
    expect(view(state).required).toBe(4);
    expect(view(state).mode).toBe("2v2");
    state = click(state, "mode_1v1");
    expect(view(state).required).toBe(2);
    const peers = hit.find(lobbyLayout(state), "peer_count");
    expect(peers?.text?.includes("1 / 2")).toBe(true);
  });

  it("shows all eight canonical slots and both protected keepers", () => {
    const state = hosting();
    const currentLayout = lobbyLayout(state);
    for (const slot of [
      "home_1",
      "home_2",
      "home_3",
      "home_4",
      "away_1",
      "away_2",
      "away_3",
      "away_4",
    ]) {
      expect(hit.find(currentLayout, `slot_${slot}`)).not.toBeNull();
    }
    const homeKeeper = hit.find(currentLayout, "keeper_home");
    expect(homeKeeper?.text?.includes("PROTECTED AI")).toBe(true);
    expect(hit.find(currentLayout, "keeper_away")).not.toBeNull();
  });

  it("never renders a pasted or exported signaling blob", () => {
    let state = hosting();
    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "paste", text: "SDPSECRET".repeat(40) },
    });
    for (const widget of lobbyLayout(state)) {
      if (widget.text) {
        expect(widget.text.includes("SDPSECRET")).toBe(false);
      }
    }
  });

  it("keeps only a digest of a signaling blob it has handed over", () => {
    let state = hosting();
    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "signal", peer_id: "guest_1", signal: "offer:blob" },
    });
    expect(view(state).has_outgoing).toBe(true);
    const nextState = dispatch(state, { kind: "lobby", command: { kind: "copy" } });
    const clipboard = nextState.effects.find((effect) => effect.kind === "clipboard");
    expect(clipboard?.kind === "clipboard" ? clipboard.text : undefined).toBe("offer:blob");
    expect(view(nextState).has_outgoing).toBe(false);
    expect(nextState.model.outgoing).toBeUndefined();
    const record = view(nextState).exported;
    expect(record?.direction).toBe("offer");
    expect(record?.bytes).toBe("offer:blob".length);
  });

  it("routes the paste control through a clipboard request", () => {
    const state = hosting();
    const nextState = dispatch(state, { kind: "lobby", command: { kind: "paste_request" } });
    expect(nextState.effects.length).toBe(1);
    expect(nextState.effects[0]?.kind).toBe("paste_request");
  });

  it("emits transport effects for an invitation", () => {
    let state = hosting();
    state = click(state, "mode_1v1");
    const nextState = click(state, "invite");
    expect(nextState.effects.map((effect) => effect.kind).join(",")).toBe(
      "open_peer,request_offer",
    );
  });

  it("disables controls that the current phase forbids", () => {
    const state = hosting();
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "copy_signal")?.data?.disabled).toBe(true);
    expect(hit.find(currentLayout, "ready")?.data?.disabled).toBe(true);
    expect(hit.find(currentLayout, "start")?.data?.disabled).toBe(true);
    // A disabled control is not activated by a click on it.
    const nextState = click(state, "ready");
    expect(view(nextState).error).toBeUndefined();
  });

  it("leaves on back and on the leave control", () => {
    const state = hosting();
    const [, action] = lobbyUpdate(state, { kind: "action", action: "back" });
    expect(action?.go).toBe("main_menu");
    const [, clicked] = lobbyUpdate(state, clickOn(lobbyLayout(state), "leave"));
    expect(clicked?.go).toBe("main_menu");
  });

  it("moves focus with directional actions", () => {
    const state = newState(VP, ports());
    expect(state.focus).toBe("role_host");
    const [moved] = lobbyUpdate(state, { kind: "action", action: "down" });
    expect(moved.focus).not.toBe(state.focus);
    const focused = hit.find(lobbyLayout(moved), moved.focus);
    expect(focused?.focused).toBe(true);
  });

  it("keeps every state inside the virtual canvas", () => {
    const roleState = newState(VP, ports());
    const hostState = hosting();
    const guestState = click(newState(VP, ports()), "role_guest");
    let readyState = dispatch(hosting(), { kind: "lobby", command: { kind: "bot_fill" } });
    readyState = dispatch(readyState, { kind: "lobby", command: { kind: "lock" } });
    for (const state of [roleState, hostState, guestState, readyState]) {
      assertWithinCanvas(lobbyLayout(state));
    }
  });

  // Unblocked: `online_lobby.ts` already drives `draw.layout` against a
  // `GraphicsBackend` (this package's own seam -- `@gc/ui` is a declared
  // dependency), so the stated blocker ("this package does not own that
  // seam") was stale. No implementation of `GraphicsBackend` exists yet
  // (`graphics_backend.ts`'s header), so a hand-written no-op
  // `GraphicsBackend` -- `fakeGraphicsBackend` above -- plays that role
  // here.
  it("renders every state without touching a real display", () => {
    const roleState = newState(VP, ports());
    const hostState = hosting();
    const locked = dispatch(dispatch(hosting(), { kind: "lobby", command: { kind: "bot_fill" } }), {
      kind: "lobby",
      command: { kind: "lock" },
    });
    const backend = fakeGraphicsBackend();
    for (const state of [roleState, hostState, locked]) {
      expect(() => draw.layout(backend, lobbyLayout(state), VP)).not.toThrow();
    }
  });

  it("surfaces a terminal reason in the layout", () => {
    let state = hosting();
    state = dispatch(state, { kind: "lobby", command: { kind: "leave" } });
    const trouble = hit.find(lobbyLayout(state), "trouble");
    expect(trouble?.text?.includes("YOU ENDED THE SESSION")).toBe(true);
  });

  it("surfaces a dropped guest's build on a lobby that is still standing", () => {
    const state = hosting();
    if (!state.model.coordinator) {
      throw new Error("hosting() must produce a coordinator");
    }
    const withDeparture: LobbyScreenState = {
      ...state,
      model: {
        ...state.model,
        coordinator: {
          ...state.model.coordinator,
          departure: {
            peer_id: "guest_1",
            reason: "build_mismatch",
            code: "protocol_error",
            detail: "a guest aborted with manifest_mismatch",
          },
        },
      },
    };
    const text = hit.find(lobbyLayout(withDeparture), "trouble")?.text ?? "";
    expect(text.includes("DIFFERENT BUILD")).toBe(true);
    expect(text.includes("INSTALL THE SAME BUILD ON BOTH")).toBe(true);
    expect(text.includes("MANIFEST_MISMATCH")).toBe(true);
  });

  it("lets a finished session outrank a dropped guest", () => {
    const state = hosting();
    if (!state.model.coordinator) {
      throw new Error("hosting() must produce a coordinator");
    }
    let withDeparture: LobbyScreenState = {
      ...state,
      model: {
        ...state.model,
        coordinator: {
          ...state.model.coordinator,
          departure: { peer_id: "guest_1", reason: "build_mismatch", code: "protocol_error" },
        },
      },
    };
    withDeparture = dispatch(withDeparture, { kind: "lobby", command: { kind: "leave" } });
    const text = hit.find(lobbyLayout(withDeparture), "trouble")?.text ?? "";
    expect(text.includes("YOU ENDED THE SESSION")).toBe(true);
    expect(text.includes("DIFFERENT BUILD")).toBe(false);
  });

  // Unblocked: driven against the real `@gc/wasm` `Coordinator` -- see this
  // file's header. 1v1 seats a lone connected human on all four home
  // slots; only the first (canonical order) is that human's opening LIVE
  // slot, the rest are AI while still owned by them, and every away slot
  // is a declared bot fill.
  it("names the AI-driven slots inside a human's owned set", () => {
    const state = lockedRealHost("1v1");
    expect(view(state).error).toBeUndefined();
    const currentLayout = lobbyLayout(state);
    const home1 = hit.find(currentLayout, "slot_home_1");
    expect(home1?.text?.includes("LIVE")).toBe(true);
    for (const slot of ["home_2", "home_3", "home_4"]) {
      const widget = hit.find(currentLayout, `slot_${slot}`);
      expect(widget?.text?.includes("AI (OWNED)")).toBe(true);
    }
    for (const slot of ["away_1", "away_2", "away_3", "away_4"]) {
      const widget = hit.find(currentLayout, `slot_${slot}`);
      expect(widget?.text?.includes("AI FILL")).toBe(true);
    }
  });

  // Unblocked: same real coordinator. 2v2 seats the lone connected human on
  // home_1/home_2 (`slots_per_human` = 2); a pair control is offered on
  // every *other* same-team slot -- trading the human's non-live slot
  // (home_2) for it -- and nowhere else: not on the two slots already
  // owned (nothing to trade for), not on the away team (wrong team).
  it("offers a pair control only where there is a pair to choose", () => {
    const state = lockedRealHost("2v2");
    expect(view(state).error).toBeUndefined();
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "prefer_home_3")).not.toBeNull();
    expect(hit.find(currentLayout, "prefer_home_4")).not.toBeNull();
    for (const slot of ["home_1", "home_2", "away_1", "away_2", "away_3", "away_4"]) {
      expect(hit.find(currentLayout, `prefer_${slot}`)).toBeNull();
    }
  });

  // Unblocked: same real coordinator. In 1v1 the lone human already owns
  // every home slot (nothing on its own team left to trade for); in 4v4 it
  // owns exactly one slot (nothing to trade away). Both are properties of
  // the real ownership the coordinator published, not of this screen's own
  // logic.
  it("offers no pair control at all in 1v1 or 4v4", () => {
    for (const mode of ["1v1", "4v4"] as const) {
      const state = lockedRealHost(mode);
      expect(view(state).error).toBeUndefined();
      const currentLayout = lobbyLayout(state);
      for (const slot of [
        "home_1",
        "home_2",
        "home_3",
        "home_4",
        "away_1",
        "away_2",
        "away_3",
        "away_4",
      ]) {
        expect(hit.find(currentLayout, `prefer_${slot}`)).toBeNull();
      }
    }
  });
});

// ---------------------------------------------------------------------------
// Room-code signaling (#552) -- UI-logic coverage per AGENTS.md §9: layout
// and update, headless, plus the "update does not mutate its input state"
// assertion `formation.spec.ts`'s own header names as the pattern. The
// end-to-end handshake (two real peers, offer/answer traveling over a fake
// room-code channel with no copy/paste) is `@gc/app`'s
// `room_code_lobby.spec.ts`, mirroring `online_ports.spec.ts`'s two-peer
// pattern -- this file stays about `lobby.ts`/`lobby_model.ts`'s own
// control flow, the same split every other describe block above draws.
// ---------------------------------------------------------------------------

describe("room-code entry (#552)", () => {
  function joining(): LobbyScreenState {
    return click(newState(VP, ports()), "room_code_join");
  }

  it("enters a composer sub-view instead of the manual role buttons", () => {
    const state = joining();
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "room_code_slots")).not.toBeNull();
    expect(hit.find(currentLayout, "role_host")).toBeNull();
    expect(hit.find(currentLayout, "role_guest")).toBeNull();
    expect(hit.find(currentLayout, "room_code_join")).toBeNull();
    expect(view(state).room_entry?.chars).toEqual(["", "", "", "", "", ""]);
    expect(view(state).room_entry?.cursor).toBe(0);
  });

  it("types a character, advances the cursor, and renders the cursor slot bracketed", () => {
    let state = joining();
    state = dispatch(state, { kind: "key", key: "a", pressed: true });
    expect(view(state).room_entry?.chars).toEqual(["A", "", "", "", "", ""]);
    expect(view(state).room_entry?.cursor).toBe(1);
    const slots = hit.find(lobbyLayout(state), "room_code_slots");
    expect(slots?.text).toBe("A [_] _ _ _ _");
  });

  it("ignores a character outside the room-code alphabet", () => {
    let state = joining();
    state = dispatch(state, { kind: "key", key: "!", pressed: true });
    expect(view(state).room_entry?.chars).toEqual(["", "", "", "", "", ""]);
    expect(view(state).room_entry?.cursor).toBe(0);
  });

  it("ignores a key release (pressed: false)", () => {
    let state = joining();
    state = dispatch(state, { kind: "key", key: "a", pressed: false });
    expect(view(state).room_entry?.chars).toEqual(["", "", "", "", "", ""]);
  });

  it("backspace clears the current slot, or steps back and clears the previous one", () => {
    let state = joining();
    state = dispatch(state, { kind: "key", key: "a", pressed: true });
    state = dispatch(state, { kind: "key", key: "b", pressed: true });
    expect(view(state).room_entry?.chars).toEqual(["A", "B", "", "", "", ""]);
    expect(view(state).room_entry?.cursor).toBe(2);
    // Cursor sits on an empty slot -- backspace steps back and clears "B".
    state = dispatch(state, { kind: "key", key: "Backspace", pressed: true });
    expect(view(state).room_entry?.chars).toEqual(["A", "", "", "", "", ""]);
    expect(view(state).room_entry?.cursor).toBe(1);
    // Cursor now sits on the just-cleared slot -- backspace clears "A" in place.
    state = dispatch(state, { kind: "key", key: "Backspace", pressed: true });
    expect(view(state).room_entry?.chars).toEqual(["", "", "", "", "", ""]);
    expect(view(state).room_entry?.cursor).toBe(0);
  });

  it("cycles the character at the cursor with up/down, for a gamepad with no keyboard", () => {
    let state = joining();
    state = dispatch(state, { kind: "action", action: "up" });
    expect(view(state).room_entry?.chars[0]).toBe("0"); // first alphabet symbol
    state = dispatch(state, { kind: "action", action: "up" });
    expect(view(state).room_entry?.chars[0]).toBe("1");
    state = dispatch(state, { kind: "action", action: "down" });
    expect(view(state).room_entry?.chars[0]).toBe("0");
    // Wraps at the bottom of the alphabet.
    state = dispatch(state, { kind: "action", action: "down" });
    expect(view(state).room_entry?.chars[0]).toBe("Z");
  });

  it("moves the cursor with left/right instead of the usual focus navigation", () => {
    let state = joining();
    state = dispatch(state, { kind: "key", key: "a", pressed: true });
    expect(view(state).room_entry?.cursor).toBe(1);
    state = dispatch(state, { kind: "action", action: "left" });
    expect(view(state).room_entry?.cursor).toBe(0);
    // Clamped, not wrapped.
    state = dispatch(state, { kind: "action", action: "left" });
    expect(view(state).room_entry?.cursor).toBe(0);
    state = dispatch(state, { kind: "action", action: "right" });
    state = dispatch(state, { kind: "action", action: "right" });
    expect(view(state).room_entry?.cursor).toBe(2);
  });

  it("refuses to submit an incomplete code, and does not open a connection", () => {
    let state = joining();
    state = dispatch(state, { kind: "key", key: "a", pressed: true });
    const nextState = click(state, "room_code_slots");
    expect(view(nextState).error).toBeDefined();
    expect(nextState.effects.some((effect) => effect.kind === "room_open_guest")).toBe(false);
    expect(view(nextState).room_entry).toBeDefined();
  });

  it("submits a complete code, opening a guest room-code connection with it", () => {
    let state = joining();
    for (const ch of "A3F9K2") {
      state = dispatch(state, { kind: "key", key: ch, pressed: true });
    }
    const nextState = click(state, "room_code_slots");
    expect(nextState.effects).toEqual([{ kind: "room_open_guest", code: "A3F9K2" }]);
    expect(view(nextState).room_active).toBe(true);
    expect(view(nextState).room_entry).toBeUndefined();
    expect(view(nextState).error).toBeUndefined();
  });

  it("update does not mutate its input state", () => {
    const state = joining();
    const before = { ...state.model };
    dispatch(state, { kind: "key", key: "a", pressed: true });
    expect(state.model).toEqual(before);
  });

  it("update does not mutate its input state on submit, even when it fails", () => {
    const state = joining();
    const before = { ...state.model };
    dispatch(state, { kind: "lobby", command: { kind: "room_submit" } });
    expect(state.model).toEqual(before);
  });

  it("hosting with a room code requests a connection and disables the button meanwhile", () => {
    const state = click(newState(VP, ports()), "room_code_host");
    expect(state.effects).toEqual([{ kind: "room_open_host" }]);
    expect(view(state).room_status).toBe("connecting");
    // Still pre-role: `room_created` (fed in by the impure `roomSignaling`
    // port once the Worker answers) is what actually picks the host role.
    expect(view(state).role).toBeUndefined();
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "room_code_host")?.data?.disabled).toBe(true);
  });

  // Round-2 council review, blocking finding 2: during the async window
  // after `room_pick`/`room_submit` (`room_active` already `true`, no
  // `coordinator` yet), every OTHER way into a role -- the manual buttons
  // and a second room-code attempt -- must be unreachable too, not just
  // the one button already being retried. This is the layout half of the
  // fix; `roomPick`'s/`command()`'s own guards are the command-level half
  // (`lobby_model.ts`'s own tests cover those directly).
  it("disables every other role-choosing control while a room-code attempt is in flight", () => {
    const state = click(newState(VP, ports()), "room_code_host");
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "role_host")?.data?.disabled).toBe(true);
    expect(hit.find(currentLayout, "role_guest")?.data?.disabled).toBe(true);
    expect(hit.find(currentLayout, "room_code_join")?.data?.disabled).toBe(true);
    expect(hit.find(currentLayout, "room_code_host")?.data?.disabled).toBe(true);
    // A click on a disabled control is not activated at all.
    const raced = click(state, "role_host");
    expect(view(raced).role).toBeUndefined();
    expect(view(raced).error).toBeUndefined();
  });

  it("room_created picks the host role, shows the code, and hides the manual signal controls", () => {
    let state = click(newState(VP, ports()), "room_code_host");
    state = dispatch(state, { kind: "lobby", command: { kind: "room_created", code: "A3F9K2" } });
    expect(view(state).role).toBe("host");
    expect(view(state).room_code).toBe("A3F9K2");
    expect(view(state).room_active).toBe(true);
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "room_code_display")?.text).toBe("ROOM CODE  A3F9K2");
    expect(hit.find(currentLayout, "copy_signal")).toBeNull();
    expect(hit.find(currentLayout, "paste_signal")).toBeNull();
  });

  it("room_joined picks the guest role and hides the manual signal controls too", () => {
    let state = joining();
    for (const ch of "A3F9K2") {
      state = dispatch(state, { kind: "key", key: ch, pressed: true });
    }
    state = click(state, "room_code_slots");
    state = dispatch(state, { kind: "lobby", command: { kind: "room_joined" } });
    expect(view(state).role).toBe("guest");
    expect(view(state).room_active).toBe(true);
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "copy_signal")).toBeNull();
    expect(hit.find(currentLayout, "room_code_active")).not.toBeNull();
  });

  it("surfaces a handshake failure as a readable error, not a hang", () => {
    let state = joining();
    for (const ch of "A3F9K2") {
      state = dispatch(state, { kind: "key", key: ch, pressed: true });
    }
    state = click(state, "room_code_slots");
    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "room_failed", reason: "handshake_failed" },
    });
    expect(view(state).error).toBe(ROOM_FAILURE_TEXT["handshake_failed"]);
    expect(view(state).role).toBeUndefined();
  });

  // Round-2 council review, cheap finding: `roomPick`'s host branch already
  // cleared a stale `room_error` when starting a fresh attempt; the guest
  // branch (opening the code composer) did not, so a guest whose previous
  // attempt failed and who then re-opened the composer still saw the old
  // failure line until a NEW one replaced it.
  it("clears a stale room-code failure when re-opening the code composer", () => {
    let state = joining();
    for (const ch of "A3F9K2") {
      state = dispatch(state, { kind: "key", key: ch, pressed: true });
    }
    state = click(state, "room_code_slots");
    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "room_failed", reason: "handshake_failed" },
    });
    expect(view(state).room_error).toBe(ROOM_FAILURE_TEXT["handshake_failed"]);
    // A failed submit already leaves `room_entry` cleared and `role`
    // unchosen, so the layout is back on the pre-role screen -- confirm
    // that before re-opening the composer, so this test does not silently
    // stop exercising what it claims to.
    expect(view(state).role).toBeUndefined();
    expect(view(state).room_entry).toBeUndefined();

    state = click(state, "room_code_join"); // re-open the composer
    expect(view(state).room_entry).toBeDefined();
    expect(view(state).room_error).toBeUndefined();
  });

  // `online_lobby.ts`'s `update(dt)` dispatches a `signal`/`room_*` command
  // from a polled event and then a `tick` in the SAME call, every frame --
  // and `command()`'s own top strips `model.error` on every dispatch,
  // `tick` included. A failure surfaced only through `error` would be
  // visible for under one frame before its own trailing tick erased it;
  // `room_error` is the dedicated field that does not (`@gc/app`'s
  // `room_code_lobby.spec.ts` caught this for real, in its own
  // failure-state case, driving the whole stack through `OnlineLobby`).
  it("keeps a room-code failure readable across the tick that follows it in the same frame", () => {
    let state = joining();
    for (const ch of "A3F9K2") {
      state = dispatch(state, { kind: "key", key: ch, pressed: true });
    }
    state = click(state, "room_code_slots");
    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "room_failed", reason: "handshake_failed" },
    });
    state = dispatch(state, { kind: "lobby", command: { kind: "tick" } });
    expect(view(state).error).toBeUndefined(); // cleared, as every command clears it
    expect(view(state).room_error).toBe(ROOM_FAILURE_TEXT["handshake_failed"]); // still readable
    const trouble = hit.find(lobbyLayout(state), "trouble");
    expect(trouble?.text).toBe(ROOM_FAILURE_TEXT["handshake_failed"]?.toUpperCase());
  });

  it("leaving a room-code connection in progress closes it", () => {
    const state = click(newState(VP, ports()), "room_code_host");
    const nextState = click(state, "leave");
    expect(nextState.effects.some((effect) => effect.kind === "room_close")).toBe(true);
    expect(view(nextState).room_active).toBe(false);
  });

  it("keeps the manual signaling controls fully reachable outside room-code mode", () => {
    const state = hosting();
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "copy_signal")).not.toBeNull();
    expect(hit.find(currentLayout, "paste_signal")).not.toBeNull();
    expect(view(state).room_active).toBe(false);
  });

  // Round-2 council review, blocking finding 1: `roomFailed`/`roomDropped`
  // used to leave `room_active` stuck `true`, and `lobby.ts`'s layout
  // gates the manual copy/paste controls on `!room_active` -- so a player
  // whose room-code attempt failed and who then picked a manual role got a
  // lobby with NO way to exchange a manual offer/answer at all, silently
  // contradicting this issue's own "manual signaling still works"
  // acceptance criterion.
  it("keeps the manual fallback reachable after a failed room-code attempt", () => {
    let state = joining();
    for (const ch of "A3F9K2") {
      state = dispatch(state, { kind: "key", key: ch, pressed: true });
    }
    state = click(state, "room_code_slots");
    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "room_failed", reason: "handshake_failed" },
    });
    expect(view(state).room_active).toBe(false);

    state = click(state, "role_guest");
    expect(view(state).role).toBe("guest");
    expect(view(state).error).toBeUndefined();
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "copy_signal")).not.toBeNull();
    expect(hit.find(currentLayout, "paste_signal")).not.toBeNull();
  });

  it("keeps the manual fallback reachable after a room-code connection drops", () => {
    let state = click(newState(VP, ports()), "room_code_host");
    state = dispatch(state, { kind: "lobby", command: { kind: "room_created", code: "A3F9K2" } });
    expect(view(state).room_active).toBe(true);
    state = dispatch(state, { kind: "lobby", command: { kind: "room_dropped" } });
    expect(view(state).room_active).toBe(false);
    const currentLayout = lobbyLayout(state);
    expect(hit.find(currentLayout, "copy_signal")).not.toBeNull();
    expect(hit.find(currentLayout, "paste_signal")).not.toBeNull();
  });

  // Command-level half of blocking finding 2 -- `lobby_model.ts`'s own
  // guards, driven directly (bypassing focus/click, and therefore the
  // layout's `disabled` flag too) so a synthetic or future-bug dispatch is
  // covered just as much as a real click would be.
  it("refuses a manual role command while a room-code connection is in flight", () => {
    const state = dispatch(newState(VP, ports()), {
      kind: "lobby",
      command: { kind: "room_pick", role: "host" },
    });
    expect(view(state).room_active).toBe(true);
    expect(state.model.coordinator).toBeUndefined();

    const raced = dispatch(state, { kind: "lobby", command: { kind: "role", role: "host" } });
    expect(raced.model.coordinator).toBeUndefined();
    expect(view(raced).error).toBeDefined();
    // The room-code attempt itself is untouched by the refused race.
    expect(view(raced).room_active).toBe(true);
  });

  it("refuses a second room-code attempt while one is already in flight", () => {
    const state = dispatch(newState(VP, ports()), {
      kind: "lobby",
      command: { kind: "room_pick", role: "host" },
    });
    const raced = dispatch(state, {
      kind: "lobby",
      command: { kind: "room_pick", role: "guest" },
    });
    expect(view(raced).room_entry).toBeUndefined();
    expect(view(raced).error).toBeDefined();
    expect(view(raced).room_active).toBe(true);
  });
});

// The presentation pass over an unchanged model: same ids, same commands, new
// arrangement. These cases pin what the redesign is *for*, so a later layout
// edit that quietly undoes it goes red.
describe("online lobby presentation", () => {
  it("groups seats into team columns instead of one flat list of eight", () => {
    const currentLayout = lobbyLayout(hosting());
    const columnOf = (id: string): number | undefined => hit.find(currentLayout, id)?.rect?.x;

    const homeColumn = columnOf("slot_home_1");
    const awayColumn = columnOf("slot_away_1");
    expect(homeColumn).toBeDefined();
    expect(awayColumn).toBeDefined();
    expect(homeColumn, "the two teams must not share a column").not.toBe(awayColumn);

    for (const slot of ["home_2", "home_3", "home_4"]) {
      expect(columnOf(`slot_${slot}`), `${slot} left its team's column`).toBe(homeColumn);
    }
    for (const slot of ["away_2", "away_3", "away_4"]) {
      expect(columnOf(`slot_${slot}`), `${slot} left its team's column`).toBe(awayColumn);
    }
    // Each keeper sits under its own team, not in a separate block.
    expect(columnOf("keeper_home")).toBe(homeColumn);
    expect(columnOf("keeper_away")).toBe(awayColumn);
  });

  it("stacks each team's four slots vertically, so a column reads as a team", () => {
    const currentLayout = lobbyLayout(hosting());
    const rows = ["home_1", "home_2", "home_3", "home_4"].map(
      (slot) => hit.find(currentLayout, `slot_${slot}`)?.rect?.y ?? -1,
    );
    for (let i = 1; i < rows.length; i += 1) {
      expect(rows[i] ?? -1, "slots should descend within their column").toBeGreaterThan(
        rows[i - 1] ?? -1,
      );
    }
  });

  it("shows an invite code rather than a byte count, and never the blob", () => {
    let state = hosting();
    expect(hit.find(lobbyLayout(state), "signal_out")?.text).toContain("NO INVITE YET");

    state = dispatch(state, {
      kind: "lobby",
      command: { kind: "signal", peer_id: "guest_1", signal: "offer:blob" },
    });
    const invite = hit.find(lobbyLayout(state), "signal_out")?.text ?? "";
    expect(invite).toContain("GC://JOIN/");
    expect(invite).not.toContain("offer:blob");

    const fingerprint = view(state).exported?.fingerprint;
    expect(fingerprint, "the code must be the model's own digest").toBeDefined();
    expect(invite).toContain((fingerprint ?? "").toUpperCase());
  });

  it("reads peers and what is still missing as one line", () => {
    const state = click(hosting(), "mode_1v1");
    const line = hit.find(lobbyLayout(state), "peer_count")?.text ?? "";
    expect(line).toContain("1 / 2");
    expect(line).toContain("WAITING ON 1 PLAYER");
  });

  it("offers no seating or signaling controls before a role is chosen", () => {
    const currentLayout = lobbyLayout(newState(VP, ports()));
    for (const id of ["slot_home_1", "keeper_home", "seat_1", "signal_out", "peer_count"]) {
      expect(hit.find(currentLayout, id), `${id} should not exist without a role`).toBeNull();
    }
    expect(hit.find(currentLayout, "role_host")).not.toBeNull();
  });

  // Two overlapping rects are a click landing on the wrong control, and
  // `hit.at`'s back-to-front scan makes the winner an accident of push order.
  // This screen now has two signalling paths sharing one control block --
  // and `room_code` outlives `room_active`, so the room-code readout and the
  // manual copy/paste controls are on screen together after a drop.
  it("never overlaps two widgets, in any state the lobby can reach", () => {
    const roomHosted = dispatch(click(newState(VP, ports()), "room_code_host"), {
      kind: "lobby",
      command: { kind: "room_created", code: "A3F9K2" },
    });
    const cases: readonly (readonly [string, LobbyScreenState])[] = [
      ["role", newState(VP, ports())],
      ["host", hosting()],
      ["guest", click(newState(VP, ports()), "role_guest")],
      ["room composer", click(newState(VP, ports()), "room_code_join")],
      ["room host", roomHosted],
      ["room dropped", dispatch(roomHosted, { kind: "lobby", command: { kind: "room_dropped" } })],
    ];

    for (const [name, state] of cases) {
      const currentLayout = lobbyLayout(state);
      assertWithinCanvas(currentLayout);
      for (let i = 0; i < currentLayout.length; i += 1) {
        for (let j = i + 1; j < currentLayout.length; j += 1) {
          const a = currentLayout[i];
          const b = currentLayout[j];
          if (!a?.rect || !b?.rect) {
            continue;
          }
          const overlaps =
            a.rect.x < b.rect.x + b.rect.w &&
            b.rect.x < a.rect.x + a.rect.w &&
            a.rect.y < b.rect.y + b.rect.h &&
            b.rect.y < a.rect.y + a.rect.h;
          expect(overlaps, `${name}: "${a.id}" overlaps "${b.id}"`).toBe(false);
        }
      }
    }
  });

  it("keeps the room code on screen after a drop, beside the manual controls it restores", () => {
    const hosted = dispatch(click(newState(VP, ports()), "room_code_host"), {
      kind: "lobby",
      command: { kind: "room_created", code: "A3F9K2" },
    });
    expect(hit.find(lobbyLayout(hosted), "copy_signal")).toBeNull();

    const dropped = dispatch(hosted, { kind: "lobby", command: { kind: "room_dropped" } });
    const currentLayout = lobbyLayout(dropped);
    expect(hit.find(currentLayout, "room_code_display")?.text).toBe("ROOM CODE  A3F9K2");
    expect(hit.find(currentLayout, "copy_signal")).not.toBeNull();
    expect(hit.find(currentLayout, "signal_out")).not.toBeNull();
  });
});
