// Ported from spec/screens/online_match_flow_spec.lua.
//
// The Lua original mounts real `OnlineLobby`/`OnlineMatch` screens over an
// in-process fake star transport, completes the real manual offer/answer
// handshake, and drives a real `game.online.match_driver` + `sim.match` to
// full time and an acknowledged result -- all Rust-owned
// (`crates/gc-sim`/`crates/gc-netcode`; v2/README.md §2.1), plus
// `game.render.pitch`/`match_hud_render`/`render.frame` (`@gc/render`) and
// `game.input.bindings` (`@gc/input`).
//
// Several things changed since this file was first ported as a wall of
// `it.skip`:
//
// - `@gc/input` IS now a declared dependency of this package
//   (`package.json`) -- that half of the original blocker note was stale.
// - `@gc/render` IS now a declared dependency too (`package.json`) --
//   also previously stale, and specifically relevant to the two cases
//   whose blocker named it explicitly.
// - `@gc/wasm` is a devDependency, reachable from this spec file (not from
//   `online_match.ts`/`online_match_model.ts`, which keep receiving a
//   coordinator only through their existing injected
//   `CoordinatorPort`/`MatchDriverPort` -- see those files' own headers).
//   It exports a real `Coordinator` (`crates/gc-wasm/src/coordinator_bridge.rs`)
//   over `gc_netcode::coordinator`'s actual reducer, not a fake of it.
//
// `online_match.ts` is entirely port-injected (`MatchDriverPort`,
// `MatchPresentationPort`, `MatchSessionPort`, `MatchLobbyLinkPort`,
// `LobbyFramingPort`, `MatchContractPort`, an observer port, and a
// `newMatch` factory returning `real_match.ts`'s `RealMatchScreenPort`) --
// following the pattern `match_screen.spec.ts` already established for
// `match.ts`/`SimHostPort`. Two cases below already ran against
// hand-written fakes for all of that with no `@gc/wasm` dependency at all.
// Two more now run against a real `Coordinator`, driven through
// `realCoordinatorPort()`/`realOnlineModelPorts()` below (a solo host
// reaching an agreed "completed" result; a real host+guest pair, relayed
// through `pump()`, proving the *other* peer's `"peer_abort"` reason is the
// coordinator's own protocol decision, not this file's invention). Five
// stay `it.skip`, each re-examined against the now-reachable `Coordinator`
// and `@gc/render` rather than assumed still blocked for their original
// reason:
//
// - Two ("keeps control inside the frozen owned set...",
//   "makes switching inert in 4v4...") were filed as needing both the real
//   assignment algorithm *and* `match.ts`'s real switch consumption.
//   `gc_netcode::coordinator::plan_assignments` is still not part of
//   `coordinator_bridge.rs`'s bound surface (same gap `lobby.spec.ts`'s
//   header documents), and separately `match_driver_bridge.rs`'s `advance`
//   batch *does* now carry a real `live` map from the real
//   `gc_netcode::match_driver` -- but `match.ts` (owned elsewhere this
//   batch, and by its own header still only porting its
//   rollback-consumption seam) has no code yet that turns that map into
//   `state.controlled`. A spec-local fake that did so would be exactly the
//   "asserting against your own fake" this port must not do.
// - One ("draws a live online frame with its combat model and HUD") named
//   `@gc/render` as its blocker; that dependency edge now exists, but
//   nothing in this batch turns `OnlineMatch`'s real state into a
//   `RenderFrame` for it to draw -- that translation is `match.ts`'s
//   `draw()`, same gap as above from a different angle.
// - One ("drives and shows every accepted family...") named `@gc/render`
//   too, but its second, still-real blocker is `sim.combat`'s
//   readiness/telegraph timing, which has no wasm bridge at all.
// - One ("routes the lobby's synchronized start...") is a permanent
//   architectural fact, not a milestone gap: `@gc/app` depends on
//   `@gc/screens`, never the reverse.
//
// See each remaining `it.skip`'s own comment for the detail.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import type { Result } from "@gc/core";
import type { CombatPresentationData } from "@gc/presentation";
import {
  OnlineMatch,
  type LobbyFramingPort,
  type LobbyFrameBuffer,
  type MatchDriverPort,
  type MatchLobbyLinkPort,
  type MatchPresentationPort,
  type MatchSessionPort,
  type OnlineMatchDispatchEvent,
  type OnlineMatchModelPort,
  type OnlineMatchOptions,
  type OnlineMatchRequest,
  type OnlineMatchState,
} from "./online_match.ts";
import {
  ABORT_PROMPT,
  command,
  ended,
  exitRoute,
  newOnlineMatchModel,
  type CoordinatorAction,
  type CoordinatorEvent,
  type CoordinatorOutcome,
  type CoordinatorPort,
  type CoordinatorStateCore,
  type CoordinatorTerminal,
  type OnlineMatchModel,
  type OnlineMatchModelPorts,
  type ProtocolMessage,
  type ProtocolPort,
} from "./online_match_model.ts";
import type { MatchContractPort, RealMatchScreenPort } from "./real_match.ts";
import type { ProductMatchResult, TeamData } from "./content.ts";
// `@gc/wasm` is a devDependency of this package (package.json), reachable
// from a spec file only -- see this file's header. `online_match.ts`/
// `online_match_model.ts` must keep receiving a coordinator through their
// existing injected `CoordinatorPort`s instead.
import { loadSimHost } from "@gc/wasm";
import type { Coordinator } from "@gc/wasm";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const TICK_SECONDS = 1 / 60;

const TEAM_HOME: TeamData = {
  id: "team.home.fixture",
  name: "Home Fixture",
  color: [0.1, 0.5, 0.9],
  formation: "formation.fixture",
  roster: ["zyro_vex", "mika_olu", "rok_tann", "sela_dwin", "ozzo"],
};

const TEAM_AWAY: TeamData = {
  id: "team.away.fixture",
  name: "Away Fixture",
  color: [0.9, 0.3, 0.1],
  formation: "formation.fixture",
  roster: ["drell", "morv", "krag", "tox_vren", "gax_oru"],
};

// `combatState` is `null` in every case below (no fake `_combat_state` --
// see `fakeMatchScreen`), so `combat.model` takes its early `enabled: false`
// return and never reads this. It still has to type-check against
// `CombatPresentationData`'s closed `ActionFamilyId` key set, hence the
// four minimal entries.
const COMBAT_DATA: CombatPresentationData = {
  action_families: {
    unarmed: { id: "unarmed", name: "Unarmed", windup_ticks: 1, recovery_ticks: 1, cooldown_ticks: 1, front_arc_degrees: 1 },
    guard: { id: "guard", name: "Guard", windup_ticks: 1, recovery_ticks: 1, cooldown_ticks: 1, front_arc_degrees: 1 },
    light_melee: { id: "light_melee", name: "Light Melee", windup_ticks: 1, recovery_ticks: 1, cooldown_ticks: 1, front_arc_degrees: 1 },
    ranged: { id: "ranged", name: "Ranged", windup_ticks: 1, recovery_ticks: 1, cooldown_ticks: 1, front_arc_degrees: 1 },
  },
  equipment_presentations: {},
  loadouts: {},
};

// ---------------------------------------------------------------------------
// Fake `CoordinatorPort`/`ProtocolPort` for `online_match_model.ts` -- a
// message-relay/terminal-flip bookkeeping fake only, mirroring
// `online_match_model.spec.ts`'s own (see this file's header and that
// file's). Neither of the two cases below ever drives it into a `terminate`
// action -- see the remaining `it.skip`s for why the cases that would are
// still skipped -- but `OnlineMatch`'s construction requires the port to
// exist.
// ---------------------------------------------------------------------------

interface FakeCoordinatorState extends CoordinatorStateCore {
  readonly session_id: string;
}

function fakeCoordinatorState(role: "host" | "guest", hostLinkId?: string): FakeCoordinatorState {
  return {
    phase: "running",
    role,
    session_id: "session/fixture",
    ...(hostLinkId !== undefined ? { host_link_id: hostLinkId } : {}),
  };
}

function terminalReasonFor(event: CoordinatorEvent): string {
  if (event["kind"] === "abort") return "local_abort";
  if (event["kind"] === "link_lost") return String(event["code"] ?? "transport_lost");
  return "unknown";
}

function fakeCoordinatorPort(): CoordinatorPort<FakeCoordinatorState> {
  return {
    step(state, event): readonly [FakeCoordinatorState, CoordinatorOutcome] {
      if (event["kind"] === "abort" || event["kind"] === "link_lost") {
        const terminal: CoordinatorTerminal = {
          reason: terminalReasonFor(event),
          ...(typeof event["detail"] === "string" ? { detail: event["detail"] } : {}),
        };
        const outcome: CoordinatorOutcome = {
          accepted: true,
          actions: [
            { kind: "send", message: event, targets: ["peer"] },
            { kind: "terminate", terminal },
          ],
        };
        return [{ ...state, phase: "terminal" }, outcome];
      }
      // match_phase/hash_report/finish/control: relay-only, never terminal.
      return [state, { accepted: true, actions: [] }];
    },
  };
}

function fakeProtocolPort(): ProtocolPort {
  return {
    encode(message): string {
      return JSON.stringify(message);
    },
    decode(wire): ProtocolMessage | undefined {
      try {
        return JSON.parse(wire) as ProtocolMessage;
      } catch {
        return undefined;
      }
    },
  };
}

function modelPorts(): OnlineMatchModelPorts<FakeCoordinatorState> {
  return {
    coordinator: fakeCoordinatorPort(),
    protocol: fakeProtocolPort(),
    localResult: () => undefined,
    isCompleted: (terminal) => terminal.reason === "completed",
  };
}

function fakeModelPort(): OnlineMatchModelPort<OnlineMatchModel<FakeCoordinatorState, OnlineMatchRequest>> {
  return {
    command: (model, event: OnlineMatchDispatchEvent) => command(model, modelPorts(), event),
    ended,
    exitRoute,
    ABORT_PROMPT,
  };
}

// ---------------------------------------------------------------------------
// Fake driver/presentation/session/link/framing/contract/observer ports.
// Each is deliberately mechanical -- see this file's header.
// ---------------------------------------------------------------------------

interface FakeDriverState {
  status: "active" | "completed" | "failed";
  tick: number;
}

interface FakeCheckpoint {
  readonly tick: number;
  readonly hash: string;
}

interface FakeBatch {
  readonly control: readonly unknown[];
  readonly checkpoints: readonly FakeCheckpoint[];
  readonly live: Readonly<Record<string, string>>;
}

// `create()` ignores its options and always hands back the same
// pre-built `state`, so a test keeps a live handle to the driver
// `OnlineMatch` otherwise holds privately -- the same role `host.driver`
// plays directly in the Lua original.
function fakeMatchDriver(
  state: FakeDriverState,
  live: Readonly<Record<string, string>>
): MatchDriverPort<FakeDriverState, FakeBatch, unknown, FakeCheckpoint> {
  return {
    create: () => state,
    status: (d) => d.status,
    advance: (d, _sample) => {
      d.tick += 1;
      return { control: [], checkpoints: [], live };
    },
    currentSnapshot: () => undefined,
    snapshot: () => undefined,
    terminal: (d) => (d.status === "active" ? undefined : { status: d.status }),
    settled: (d) => d.status !== "active",
    diagnostics: (d) => ({
      transport_tick: d.tick,
      present_input_tick: d.tick,
      confirmed_output_tick: d.tick,
      rollback_count: 0,
      correction_count: 0,
      predicted_slot_samples: 0,
      status: d.status,
    }),
    observeCheckpoint: () => {},
    batchControl: (b) => b.control,
    batchCheckpoints: (b) => b.checkpoints,
    batchLive: (b) => b.live,
  };
}

function fakeMatchPresentation(): MatchPresentationPort<Record<string, never>, FakeDriverState, FakeBatch, FakeBatch> {
  return {
    create: () => ({}),
    consume: (_presentation, _driver, batch) => batch,
    presentedOutputs: () => [],
    presentedEventDiffs: () => [],
    presentedConfirmedSteps: () => [],
    presentedCorrections: () => [],
  };
}

function fakeMatchSession(): MatchSessionPort<OnlineMatchState> {
  return { playerIndex: () => 0 };
}

function fakeLink(): MatchLobbyLinkPort {
  return {
    star: {
      // Never polled by either case below: both keep the driver "active"
      // throughout, and `OnlineMatch.drainControl` only polls the star
      // once the driver has gone non-active.
      pollBatch: () => [],
      pollEvent: () => undefined,
    },
    send: () => {},
    apply: () => {},
  };
}

function fakeLobbyFraming(): LobbyFramingPort {
  return {
    newBuffer: (): LobbyFrameBuffer => ({}),
    absorb: () => [undefined, undefined] as const,
  };
}

function fakeContract(): MatchContractPort {
  return {
    newResult: (): Result<ProductMatchResult, string> => ({
      ok: false,
      error: "not exercised by this fixture -- neither case below reaches a result",
    }),
  };
}

function fakeObserver() {
  return {
    create: () => ({}),
    observeConfirmed: () => false,
    finish: () => ({ home_stats: {}, away_stats: {} }),
  };
}

// The shape `online_match.ts`'s private `driverSource()` returns -- opaque
// (`unknown`) on the public `newMatch` port, so this is this file's own
// declaration of what it needs to drive, matching that method's body.
interface RollbackSourceLike {
  needsLocalSample(): boolean;
  advance(tick: number, sample: unknown): unknown;
}

// Stands in for `match.ts`'s real fixed-clock `MatchScreen` (owned by
// another agent this batch, and itself only ports its rollback-consumption
// seam -- see `real_match.ts`'s header). One simulated tick per
// `TICK_SECONDS` of accumulated `dt`, calling the injected rollback source
// exactly the way the real fixed clock would -- the same role
// `match_screen.spec.ts`'s `FakeSimHost` plays for `SimHostPort`.
function fakeMatchScreen(
  state: OnlineMatchState,
  rollbackSource: () => unknown
): RealMatchScreenPort<OnlineMatchState, unknown> {
  const source = rollbackSource() as unknown as RollbackSourceLike;
  let accumulator = 0;
  let tick = 0;
  // `overlayLines` reaches through a private-field cast for `_combat_state`
  // (`online_match.ts`'s own comment there); `null` takes `combat.model`'s
  // early "disabled" return rather than reading a shape this fake doesn't
  // have. Built as an untyped local first so this extra field survives --
  // returning it straight from an object literal typed as
  // `RealMatchScreenPort` would trip TypeScript's excess-property check.
  const impl = {
    state,
    rollbackLab: false,
    rollbackConfirmedSteps: [],
    frameEvents: [],
    _combat_state: null,
    fullTimeConfirmed: () => false,
    resultCompletionBlocked: () => false,
    update(dt: number) {
      accumulator += dt;
      while (accumulator >= TICK_SECONDS - 1e-9) {
        accumulator -= TICK_SECONDS;
        if (source.needsLocalSample()) {
          tick += 1;
          source.advance(tick, {});
        }
      }
    },
    event: () => {},
    draw: () => {},
    teardown: () => {},
    applySettings: () => {},
  };
  return impl;
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

const HOST_REQUEST: OnlineMatchRequest = {
  role: "host",
  peer_id: "host",
  mode: "1v1",
  freeze: {},
  manifest: {},
  initial_snapshot: undefined,
  first_input_tick: 0,
  home: TEAM_HOME,
  away: TEAM_AWAY,
  arena: { id: "arena.fixture" },
  combat_enabled: true,
  live: "home_1",
  owned: ["home_1", "home_2", "home_3", "home_4"],
};

type HostOnlineMatch = OnlineMatch<
  FakeDriverState,
  FakeBatch,
  unknown,
  FakeCheckpoint,
  Record<string, never>,
  FakeBatch,
  FakeCoordinatorState,
  OnlineMatchModel<FakeCoordinatorState, OnlineMatchRequest>
>;

function buildHost(): { readonly match: HostOnlineMatch; readonly driver: FakeDriverState; readonly actions: { readonly go: string }[] } {
  const driver: FakeDriverState = { status: "active", tick: 0 };
  const actions: { readonly go: string }[] = [];
  const initialState: OnlineMatchState = {
    time_left: 600,
    score: { home: 0, away: 0 },
    controlled: 0,
    players: [{ id: "zyro_vex", team: "home", pos: new Vec2(0, 0), facing: new Vec2(0, 1) }],
  };
  const options: OnlineMatchOptions<
    FakeDriverState,
    FakeBatch,
    unknown,
    FakeCheckpoint,
    Record<string, never>,
    FakeBatch,
    FakeCoordinatorState
  > = {
    request: HOST_REQUEST,
    coordinator: fakeCoordinatorState("host"),
    link: fakeLink(),
    matchDriver: fakeMatchDriver(driver, { [HOST_REQUEST.peer_id]: HOST_REQUEST.live ?? "" }),
    matchPresentation: fakeMatchPresentation(),
    matchSession: fakeMatchSession(),
    lobbyFraming: fakeLobbyFraming(),
    contract: fakeContract(),
    observer: fakeObserver(),
    newMatch: (opts) => fakeMatchScreen(initialState, opts.rollbackSource),
    onAction: (action) => {
      actions.push(action);
    },
  };
  const match = new OnlineMatch(options, fakeModelPort(), (request, coordinator) => newOnlineMatchModel(request, coordinator));
  return { match, driver, actions };
}

function run(match: HostOnlineMatch, frames: number): void {
  for (let i = 0; i < frames; i += 1) {
    match.update(TICK_SECONDS);
  }
}

// ---------------------------------------------------------------------------
// Real-coordinator fixtures -- see this file's header for what these do and
// do not prove. Adapts `@gc/wasm`'s stateful `Coordinator` class to
// `online_match_model.ts`'s pure `CoordinatorPort<TState>.step` shape, the
// same `__handle`-carrying boundary cast `lobby.spec.ts` uses for its own
// (structurally different) `CoordinatorPort` adapter -- not shared between
// the two files because each package's own `CoordinatorPort` shape and
// event vocabulary differs, and duplicating a small adapter twice is
// cheaper than a cross-file dependency neither file's ownership allows.
// ---------------------------------------------------------------------------

const REAL_BUILD_ID = "build.97b60ea";

// Same fixture data as `lobby.spec.ts`'s `REAL_RUNTIME_WIRE`/`realManifest`
// -- transcribed verbatim from `crates/gc-netcode/src/protocol_fixture.rs`
// (values) plus the version constants it composes from `gc-sim`/
// `gc-netcode` (see that file's header for the full accounting). Kept as a
// plain object (not `lobby_model.ts`'s `SessionManifest`) because this
// file's coordinator only ever receives it as JSON text, never through
// `lobby_model.ts`'s typed surface.
const REAL_RUNTIME_WIRE = {
  version: 1,
  runtime_id: "lovejs",
  runtime_revision: "lovejs.11.5.omp0",
  presentation_id: "presentation.2026-07-25",
  capabilities: ["combat_feedback.v1", "control_channel.v1", "input_channel.v1"],
};

function realManifest1v1(sessionId: string) {
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
  return {
    version: 1,
    session_id: sessionId,
    protocol_version: 1,
    input_version: 2,
    snapshot_version: 13,
    tape_version: 2,
    combat_schema_version: 3,
    build_id: REAL_BUILD_ID,
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
    match_mode: "1v1",
    teams: [
      { team: "home", team_id: "team_nova", roster: home },
      { team: "away", team_id: "team_void", roster: away },
    ],
    slots: [
      { slot: "home_1", team: "home", player_id: "zyro_vex" },
      { slot: "home_2", team: "home", player_id: "mika_olu" },
      { slot: "home_3", team: "home", player_id: "rok_tann" },
      { slot: "home_4", team: "home", player_id: "sela_dwin" },
      { slot: "away_1", team: "away", player_id: "drell" },
      { slot: "away_2", team: "away", player_id: "morv" },
      { slot: "away_3", team: "away", player_id: "krag" },
      { slot: "away_4", team: "away", player_id: "tox_vren" },
    ],
  };
}

function realExpectation1v1(sessionId: string) {
  const manifest = realManifest1v1(sessionId);
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

// 1v1's degenerate seat plan for exactly one or two connected humans -- see
// this file's header and `lobby.spec.ts`'s own `realPlanAssignments` for
// why this is not a port of `gc_netcode::coordinator::plan_assignments`
// (not wasm-bound): with `slots_per_human` = 4 for 1v1, one connected human
// owns the whole home block and, if a second is connected, it owns the
// whole away block -- there is no contiguous-block boundary decision left
// to make for this specific mode. `assignSlots` independently validates the
// result.
function realAssignments1v1(manifest: ReturnType<typeof realManifest1v1>, hostPeerId: string, guestPeerId?: string) {
  return manifest.slots.map((slot, index) => {
    const isHome = index < 4;
    if (isHome) {
      return { producer_kind: "peer", producer_id: hostPeerId, team: slot.team, slot: slot.slot, player_id: slot.player_id };
    }
    if (guestPeerId !== undefined) {
      return { producer_kind: "peer", producer_id: guestPeerId, team: slot.team, slot: slot.slot, player_id: slot.player_id };
    }
    return {
      producer_kind: "bot",
      producer_id: `bot_${slot.slot}`,
      team: slot.team,
      slot: slot.slot,
      player_id: slot.player_id,
      bot_seed: index + 1,
    };
  });
}

// See `gc_wasm::coordinator_bridge::state_to_json`: every unset optional
// field is an explicit JSON `null`, not an omitted key. `CoordinatorStateCore`'s
// `host_link_id` (and this file's own reads of `.phase`/`.terminal`/
// `.result`) are TypeScript `undefined` -- left as `null`, `!== undefined`
// checks would see "already set" on a session that never set it.
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

interface RealOnlineCoordState extends CoordinatorStateCore {
  readonly __handle: Coordinator;
}

function stateFromHandle(handle: Coordinator): RealOnlineCoordState {
  const parsed = denull(JSON.parse(handle.stateJson())) as Record<string, unknown>;
  return { ...parsed, __handle: handle } as unknown as RealOnlineCoordState;
}

function handleOf(state: CoordinatorStateCore): Coordinator {
  return (state as unknown as RealOnlineCoordState).__handle;
}

function mapAction(raw: Record<string, unknown>): CoordinatorAction {
  const kind = raw["kind"];
  if (kind === "send") {
    return { kind: "send", message: raw["wire"], targets: raw["targets"] } as unknown as CoordinatorAction;
  }
  if (kind === "close") {
    return { kind: "close", link_id: raw["link_id"] } as unknown as CoordinatorAction;
  }
  if (kind === "terminate") {
    const terminal = raw["terminal"] as Record<string, unknown>;
    return { kind: "terminate", terminal: { reason: terminal["reason"], detail: terminal["detail"] } } as unknown as CoordinatorAction;
  }
  // "start_match" never occurs past the lobby handoff this file's real
  // coordinators are already constructed beyond, but pass it through
  // unrecognized rather than throwing -- `absorb` in `online_match_model.ts`
  // silently ignores an action kind it does not recognize.
  return raw as unknown as CoordinatorAction;
}

// Every applied event in the batch, not just the last: `tick()`/`control`/
// `link_lost` drain the whole queue in one call (one outcome per applied
// event, "control" and "link_lost" event before the trailing `Tick`'s own
// -- `Coordinator.tick`'s own doc), and a "terminate" action from an
// *earlier* outcome (a queued abort/hash-mismatch/etc, applied before the
// trailing tick) is exactly the shape a real inbound abort takes here. This
// file's own `CoordinatorOutcome` is only ever consumed for its `actions`
// list (`absorb` in `online_match_model.ts`), so keeping only the last
// outcome silently dropped that action -- a real bug this adapter had until
// the "aborts deliberately..." case below caught it.
function batchOutcome(json: string): CoordinatorOutcome {
  const parsed = JSON.parse(json) as { readonly outcomes: readonly Record<string, unknown>[] };
  if (parsed.outcomes.length === 0) {
    throw new Error("a coordinator call returned no outcome");
  }
  const actions: CoordinatorAction[] = [];
  let accepted = true;
  let reason: string | undefined;
  for (const outcome of parsed.outcomes) {
    for (const raw of outcome["actions"] as readonly Record<string, unknown>[]) {
      actions.push(mapAction(raw));
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

function wiresFrom(json: string): readonly string[] {
  const parsed = JSON.parse(json) as { readonly outcomes: readonly { readonly actions: readonly Record<string, unknown>[] }[] };
  const wires: string[] = [];
  for (const outcome of parsed.outcomes) {
    for (const action of outcome.actions) {
      if (action["kind"] === "send" && typeof action["wire"] === "string") {
        wires.push(action["wire"]);
      }
    }
  }
  return wires;
}

// Delivers every wire in `wires` to `to` (queued via `enqueueControlWire`,
// applied by the one `tick()` call that drains the queue -- the queue/drain
// seam `crates/gc-wasm/src/net_inbox.rs` documents), and returns whatever
// new wires that produced.
function deliver(wires: readonly string[], to: Coordinator, linkId: string): readonly string[] {
  if (wires.length === 0) {
    return [];
  }
  for (const wire of wires) {
    to.enqueueControlWire(linkId, wire);
  }
  return wiresFrom(to.tick());
}

// Pumps a host/guest pair to a fixed point, mirroring
// `gc_netcode::coordinator_driver::Driver::pump` -- round-based, bounded
// (a protocol bug fails loudly here rather than looping forever), not a
// port of the reducer itself: every state change still happens inside the
// real `Coordinator` calls this drives.
function pump(
  host: Coordinator,
  guest: Coordinator,
  // The link id *the guest* records an arrival under -- what the guest
  // constructor called `host_link_id` (`HOST_LINK_ON_GUEST` at every call
  // site below). Named for which coordinator reads it, not which
  // coordinator it "belongs to" on the wire -- the opposite naming bit
  // everyone here once.
  linkOnGuestForHost: string,
  // The link id *the host* records an arrival on for this guest
  // (`GUEST_LINK_ON_HOST` at every call site below).
  linkOnHostForGuest: string,
  fromHost: readonly string[],
  fromGuest: readonly string[]
): void {
  let toGuest = fromHost;
  let toHost = fromGuest;
  for (let round = 0; round < 16; round += 1) {
    if (toGuest.length === 0 && toHost.length === 0) {
      return;
    }
    const nextToHost = deliver(toGuest, guest, linkOnGuestForHost);
    const nextToGuest = deliver(toHost, host, linkOnHostForGuest);
    toGuest = nextToGuest;
    toHost = nextToHost;
  }
  throw new Error("real-coordinator relay did not settle within the round bound");
}

const GUEST_LINK_ON_HOST = "guest_1";
const HOST_LINK_ON_GUEST = "host";
const GUEST_PEER_ID = "guest_1";

// Reaches "running" for a solo, bot-filled 1v1 host -- no relay needed
// (`handle_finish` completes a session with no other admitted peer
// immediately; see this file's report for the trace).
function soloHostRunning(sessionId: string): RealOnlineCoordState {
  const host = loadSimHost();
  const handle = new host.Coordinator("host", sessionId, "host", undefined, undefined, JSON.stringify(REAL_RUNTIME_WIRE), REAL_BUILD_ID, undefined);
  const manifest = realManifest1v1(sessionId);
  handle.proposeManifest(JSON.stringify(manifest));
  handle.assignSlots(JSON.stringify(realAssignments1v1(manifest, "host")), false);
  handle.setReady(true);
  handle.beginCountdown("countdown.1", 0, 0);
  return stateFromHandle(handle);
}

// Reaches "running" for a real host+guest 1v1 pair, relayed through
// `pump()` -- the manual-connect handshake, manifest proposal, ownership,
// readiness, and the countdown/start barrier, exactly the sequence
// `gc_netcode::coordinator_driver::Driver::reach_start` drives natively.
function pairedRunning(sessionId: string): { readonly host: RealOnlineCoordState; readonly guest: RealOnlineCoordState } {
  const wasm = loadSimHost();
  const manifest = realManifest1v1(sessionId);
  const hostHandle = new wasm.Coordinator("host", sessionId, "host", undefined, undefined, JSON.stringify(REAL_RUNTIME_WIRE), REAL_BUILD_ID, undefined);
  const guestHandle = new wasm.Coordinator(
    "guest",
    sessionId,
    GUEST_PEER_ID,
    "host",
    HOST_LINK_ON_GUEST,
    JSON.stringify(REAL_RUNTIME_WIRE),
    REAL_BUILD_ID,
    JSON.stringify(realExpectation1v1(sessionId))
  );
  pump(hostHandle, guestHandle, HOST_LINK_ON_GUEST, GUEST_LINK_ON_HOST, [], wiresFrom(guestHandle.connect()));
  pump(hostHandle, guestHandle, HOST_LINK_ON_GUEST, GUEST_LINK_ON_HOST, wiresFrom(hostHandle.proposeManifest(JSON.stringify(manifest))), []);
  pump(
    hostHandle,
    guestHandle,
    HOST_LINK_ON_GUEST,
    GUEST_LINK_ON_HOST,
    wiresFrom(hostHandle.assignSlots(JSON.stringify(realAssignments1v1(manifest, "host", GUEST_PEER_ID)), false)),
    []
  );
  pump(hostHandle, guestHandle, HOST_LINK_ON_GUEST, GUEST_LINK_ON_HOST, wiresFrom(hostHandle.setReady(true)), []);
  pump(hostHandle, guestHandle, HOST_LINK_ON_GUEST, GUEST_LINK_ON_HOST, [], wiresFrom(guestHandle.setReady(true)));
  pump(hostHandle, guestHandle, HOST_LINK_ON_GUEST, GUEST_LINK_ON_HOST, wiresFrom(hostHandle.beginCountdown("countdown.1", 0, 0)), []);
  return { host: stateFromHandle(hostHandle), guest: stateFromHandle(guestHandle) };
}

function realCoordinatorPort(): CoordinatorPort<RealOnlineCoordState> {
  return {
    step(state, event) {
      const handle = handleOf(state);
      let json: string;
      switch (event["kind"]) {
        case "match_phase":
          json = handle.matchPhase(
            event["phase"] as string,
            event["tick"] as number,
            event["home_score"] as number,
            event["away_score"] as number
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
            event["final_hash"] as string
          );
          break;
        case "netcode_failure":
          json = handle.netcodeFailure(event["failure"] as string, undefined, event["detail"] as string | undefined);
          break;
        case "link_lost":
          // A transport-reported loss is network-originated -- queued, not
          // applied immediately (the same queue/drain seam as "control";
          // see `net_inbox.rs`'s doc).
          handle.enqueueLinkLost(event["link_id"] as string, event["code"] as string | undefined);
          json = handle.tick();
          break;
        case "abort":
          json = handle.abort(event["code"] as string | undefined, event["detail"] as string | undefined);
          break;
        case "leave":
          json = handle.leave();
          break;
        case "control":
          handle.enqueueControlWire(event["link_id"] as string, event["wire"] as string);
          json = handle.tick();
          break;
        case "tick":
          json = handle.tick();
          break;
        default:
          throw new Error(
            `realCoordinatorPort: unhandled event kind '${String(event["kind"])}' -- this fixture only wires what the real-coordinator flow cases exercise`
          );
      }
      return [stateFromHandle(handle), batchOutcome(json)];
    },
  };
}

function realProtocolPort(): ProtocolPort {
  return {
    // The real `Coordinator` already encodes a "send" action's `message`
    // (see `mapAction`) into its final wire text -- unlike the fake
    // `ProtocolPort` above, this one's `message` argument already *is* the
    // wire, so `encode` is the identity.
    encode: (message) => message as string,
    // `crates/gc-wasm/src/protocol_bridge.rs` binds decoding a wire's
    // routing header only, never its kind-specific body -- see this file's
    // sibling `lobby.spec.ts`'s header for the same gap. `absorbControl`
    // only uses a decoded body to emit an optional `observe_hash`
    // diagnostic effect; returning `undefined` here drops that one
    // side-observation, never coordinator correctness (the real
    // coordinator sees every wire regardless, via `enqueueControlWire`).
    decode: () => undefined,
  };
}

function realOnlineModelPorts(): OnlineMatchModelPorts<RealOnlineCoordState> {
  return {
    coordinator: realCoordinatorPort(),
    protocol: realProtocolPort(),
    localResult: (state) => (state as unknown as { readonly result?: unknown }).result,
    isCompleted: (terminal) => terminal.reason === "completed",
  };
}

function realOnlineModelPort(): OnlineMatchModelPort<OnlineMatchModel<RealOnlineCoordState, OnlineMatchRequest>> {
  return {
    command: (model, event: OnlineMatchDispatchEvent) => command(model, realOnlineModelPorts(), event),
    ended,
    exitRoute,
    ABORT_PROMPT,
  };
}

type RealHostOnlineMatch = OnlineMatch<
  FakeDriverState,
  FakeBatch,
  unknown,
  FakeCheckpoint,
  Record<string, never>,
  FakeBatch,
  RealOnlineCoordState,
  OnlineMatchModel<RealOnlineCoordState, OnlineMatchRequest>
>;

function buildRealHost(
  coordinator: RealOnlineCoordState,
  request: OnlineMatchRequest = HOST_REQUEST,
  link: MatchLobbyLinkPort = fakeLink(),
  matchDriverFor?: (driver: FakeDriverState, live: Readonly<Record<string, string>>) => MatchDriverPort<FakeDriverState, FakeBatch, unknown, FakeCheckpoint>
): { readonly match: RealHostOnlineMatch; readonly driver: FakeDriverState; readonly actions: { readonly go: string }[] } {
  const driver: FakeDriverState = { status: "active", tick: 0 };
  const actions: { readonly go: string }[] = [];
  const initialState: OnlineMatchState = {
    time_left: 600,
    score: { home: 0, away: 0 },
    controlled: 0,
    players: [{ id: "zyro_vex", team: "home", pos: new Vec2(0, 0), facing: new Vec2(0, 1) }],
  };
  const options: OnlineMatchOptions<
    FakeDriverState,
    FakeBatch,
    unknown,
    FakeCheckpoint,
    Record<string, never>,
    FakeBatch,
    RealOnlineCoordState
  > = {
    request,
    coordinator,
    link,
    matchDriver: (matchDriverFor ?? fakeMatchDriver)(driver, { [request.peer_id]: request.live ?? "" }),
    matchPresentation: fakeMatchPresentation(),
    matchSession: fakeMatchSession(),
    lobbyFraming: fakeLobbyFraming(),
    contract: fakeContract(),
    observer: fakeObserver(),
    newMatch: (opts) => fakeMatchScreen(initialState, opts.rollbackSource),
    onAction: (action) => {
      actions.push(action);
    },
  };
  const match = new OnlineMatch(options, realOnlineModelPort(), (req, coord) => newOnlineMatchModel(req, coord));
  return { match, driver, actions };
}

// `fakeMatchDriver` above never reports a checkpoint (the two already-passing
// cases don't need one) -- the real coordinator's `matchFinish` refuses an
// empty final hash (`online_match.ts`'s `lastCheckpointHash` doc), so a case
// that reaches a real agreed result needs its driver to report one, exactly
// once, the way a real driver reports its boundary hash as it runs.
function fakeMatchDriverWithHash(
  driver: FakeDriverState,
  live: Readonly<Record<string, string>>
): MatchDriverPort<FakeDriverState, FakeBatch, unknown, FakeCheckpoint> {
  const base = fakeMatchDriver(driver, live);
  let reported = false;
  return {
    ...base,
    advance: (d, sample) => {
      const batch = base.advance(d, sample);
      if (reported) {
        return batch;
      }
      reported = true;
      return { ...batch, checkpoints: [{ tick: d.tick, hash: "0123456789abcdef" }] };
    },
  };
}

function runReal(match: RealHostOnlineMatch, frames: number): void {
  for (let i = 0; i < frames; i += 1) {
    match.update(TICK_SECONDS);
  }
}

// Bypasses the star transport entirely (this file's `fakeLink`, whose
// `send` is a no-op, has no relay of its own) and delivers straight into
// the *other* peer's real coordinator via `enqueueControlWire` -- queued,
// not applied, until that peer's own next coordinator tick (drained inside
// `realCoordinatorPort`'s "tick" case). This is what "arrival is a tick
// event, never a callback reaching sim state between ticks" means in
// practice: `send` below only ever enqueues.
function relayLink(targetHandle: Coordinator, targetLinkId: string): MatchLobbyLinkPort {
  return {
    star: { pollBatch: () => [], pollEvent: () => undefined },
    send: (_linkId, wire) => {
      targetHandle.enqueueControlWire(targetLinkId, wire);
    },
    apply: () => {},
  };
}

describe("online match screen flow", () => {
  // Unblocked: driven against a real, solo, bot-filled 1v1 `Coordinator`
  // (`soloHostRunning` -- see this file's header). `handle_finish` in
  // `crates/gc-netcode/src/coordinator.rs` completes a session with an
  // empty target list (no other admitted peer to disagree with)
  // immediately on `Finish`, no acknowledgement round-trip needed -- so
  // this reaches a genuinely agreed (`TerminalReason::Completed`) result
  // for real, through the real reducer, without needing a second peer.
  it("carries a 1v1 session from the lobby to an agreed result", () => {
    const coordinator = soloHostRunning("session.case1");
    const { match, driver } = buildRealHost(coordinator, HOST_REQUEST, fakeLink(), fakeMatchDriverWithHash);
    runReal(match, 10);
    expect(ended(match.model)).toBe(false);

    // The fake match driver stands in for the real OMP-3 driver reaching
    // full time -- `reportDriver` is what turns that into the coordinator's
    // own match-phase/finish protocol, which is the real thing under test
    // here (see this file's header for what `fakeMatchDriver` does and does
    // not fake).
    driver.status = "completed";
    match.update(TICK_SECONDS);
    runReal(match, 5);

    expect(ended(match.model)).toBe(true);
    const model = match.model as {
      readonly terminal?: { readonly reason: string };
      readonly result?: { readonly home_score: number; readonly away_score: number };
    };
    expect(model.terminal?.reason).toBe("completed");
    expect(exitRoute(match.model)).toBe("result");
    expect(model.result?.home_score).toBe(0);
    expect(model.result?.away_score).toBe(0);
  });

  // Ported as `it.skip`, re-examined: which slot is LIVE is
  // `gc_netcode::match_driver`'s real switch-rule output, and
  // `crates/gc-wasm/src/match_driver_bridge.rs`'s `advance` batch now
  // genuinely carries it (a real `live` map, not a fake). What is still
  // missing is `match.ts` consuming it: `match.state.controlled` is
  // supposed to come from `OnlineMatch`'s own `driverSource().controlledPlayer`
  // callback (already correct in `online_match.ts`, this file's own
  // package), but `match.ts` (owned by another agent this batch, and by its
  // own header still only porting its rollback-consumption seam) has no
  // code yet that calls it. A fake match screen that read `batch.live`
  // itself and set `state.controlled` accordingly would be reimplementing
  // that missing piece as a spec-local fake -- exactly the "asserting
  // against your own fake" this port must not do, just one layer further
  // in than originally filed.
  it.skip("keeps control inside the frozen owned set and off both keepers", () => {});

  // Ported as `it.skip`, re-examined: same real gap as above -- a singleton
  // owned set making every switch branch return the live slot is a real
  // property of `gc_netcode::match_driver`'s switch rule, reachable through
  // `MatchDriverBridge` now, but still unobservable through `state.controlled`
  // until `match.ts` consumes `driverSource().controlledPlayer`.
  it.skip("makes switching inert in 4v4 without branching on the mode", () => {});

  // Unblocked: nothing in this path (focus loss, a lost controller, one
  // pause request short of an abort) ever reaches the coordinator -- see
  // `online_match_model.ts`'s `command`: `focus_lost`/`controller_lost`
  // are pure local notices, and a *first* `pause_request` only arms
  // `abort_prompt`. What this proves is `OnlineMatch`'s own plumbing: that
  // `update` keeps calling into the fixed clock (and therefore the driver)
  // regardless of local interruptions, using a hand-written fake match
  // screen/driver in place of the real wasm-backed ones (this file's
  // header; the same "small fakes" pattern `match_screen.spec.ts` uses for
  // `SimHostPort`).
  it("keeps simulating through focus loss, a lost controller, and a pause request", () => {
    const { match, driver } = buildHost();
    run(match, 20);
    const before = driver.tick;

    match.focusLost();
    match.controllerLost();
    match.event({ kind: "action", action: "pause" });
    expect((match.model as { readonly abort_prompt: boolean }).abort_prompt).toBe(true);
    run(match, 20);

    const after = driver.tick;
    expect(after).toBeGreaterThan(before);
    expect(driver.status).toBe("active");
    // Any other input dismisses the prompt rather than aborting.
    match.event({ kind: "action", action: "confirm" });
    expect((match.model as { readonly abort_prompt: boolean }).abort_prompt).toBe(false);
  });

  // Unblocked: a deliberate abort is provably local (`local_abort`,
  // exercised by `online_match_model.spec.ts`'s "aborts on a second pause
  // request" case for real). What only this flow spec can prove is the
  // *other* peer's side -- that receiving the relayed abort produces
  // `terminal.reason === "peer_abort"` -- and that mapping is the real
  // coordinator's own protocol decision on an inbound wire message
  // (`apply_abort` in `crates/gc-netcode/src/coordinator.rs`), not
  // something `online_match_model.ts` computes locally (`"peer_abort"`
  // does not appear anywhere in that module). Driven here against a real
  // host+guest pair (`pairedRunning`), relayed through real
  // `Coordinator.enqueueControlWire`/`.tick()` calls via `relayLink`, not a
  // fake that invents the mapping.
  it("aborts deliberately and ends the session for every peer", () => {
    const { host: hostCoordinator, guest: guestCoordinator } = pairedRunning("session.case4");
    const hostHandle = handleOf(hostCoordinator);
    const guestHandle = handleOf(guestCoordinator);
    const guestRequest: OnlineMatchRequest = {
      ...HOST_REQUEST,
      role: "guest",
      peer_id: GUEST_PEER_ID,
      live: "away_1",
      owned: ["away_1", "away_2", "away_3", "away_4"],
    };
    const { match: hostMatch } = buildRealHost(hostCoordinator, HOST_REQUEST, relayLink(guestHandle, HOST_LINK_ON_GUEST));
    const { match: guestMatch } = buildRealHost(guestCoordinator, guestRequest, relayLink(hostHandle, GUEST_LINK_ON_HOST));
    runReal(hostMatch, 5);
    runReal(guestMatch, 5);

    // First press only arms the prompt (see the "keeps simulating..." case
    // above); the second confirms a deliberate abort.
    hostMatch.event({ kind: "action", action: "pause" });
    expect((hostMatch.model as { readonly abort_prompt: boolean }).abort_prompt).toBe(true);
    hostMatch.event({ kind: "action", action: "pause" });

    expect(ended(hostMatch.model)).toBe(true);
    const hostModel = hostMatch.model as { readonly terminal?: { readonly reason: string } };
    expect(hostModel.terminal?.reason).toBe("local_abort");

    // The abort wire is already queued on the guest's real coordinator
    // (`relayLink`'s `send` only enqueues) but not yet applied -- arrival is
    // a tick event, never a callback reaching sim state between ticks (this
    // file's header; `crates/gc-wasm/src/net_inbox.rs`'s doc). The guest's
    // model has not observed anything yet.
    expect(ended(guestMatch.model)).toBe(false);

    runReal(guestMatch, 3);

    expect(ended(guestMatch.model)).toBe(true);
    const guestModel = guestMatch.model as { readonly terminal?: { readonly reason: string } };
    expect(guestModel.terminal?.reason).toBe("peer_abort");
    expect(exitRoute(guestMatch.model)).toBe("terminal");
  });

  // Unblocked: `overlayLines` reads `this.match.state` and diagnostics off
  // the injected ports directly and formats them into plain lines -- no
  // `love.graphics`/`@gc/render` involved (this screen's own overlay is
  // raw text this milestone; see `online_match.ts`'s header). Passing
  // `combatState: null` from the fake match screen takes `combat.model`'s
  // early `{ enabled: false }` return, so this only proves the overlay's
  // own labels -- exactly what the Lua original's text-search assertions
  // check, not any particular combat readout.
  it("shows the controlled player, its loadout, and the network state", () => {
    const { match } = buildHost();
    run(match, 30);
    const text = match.overlayLines(COMBAT_DATA).join("\n");
    expect(text.includes("control ")).toBe(true);
    expect(text.includes("owned ")).toBe(true);
    expect(text.includes("family ")).toBe(true);
    expect(text.includes("net tick ")).toBe(true);
    expect(ended(match.model)).toBe(false);
  });
});

// Re-examined: `@gc/render` (pitch/HUD drawing) *is* now a declared
// dependency of `@gc/screens` (`package.json`) -- that half of the
// original blocker is gone. The other half is not: telegraph/readiness/
// kickoff-hold timing is `sim.combat`'s real physics (Rust-owned,
// `crates/gc-sim`), and no wasm crate binds it -- `@gc/wasm`'s bound
// surface this batch is `Coordinator`/`MatchDriverBridge`/
// `RollbackEventsTimeline`/`FixedClock`/`TuningRegistry`, none of which
// expose a per-tick combat readiness/telegraph query. Reproducing "every
// quiet frame past kickoff read ready, and committing zero times there
// proves nothing" with a fake would mean re-implementing `sim.combat`'s
// request-rejection gates, not just relaying messages. `@gc/input` being a
// declared dependency now (the earlier blocker note was stale on that
// point too) doesn't change this, so it stays skipped.
describe.skip("online combat families [needs sim.combat's real readiness/telegraph timing (Rust-owned, crates/gc-sim, no wasm bridge)]", () => {
  it.skip("drives and shows every accepted family from local keyboard and gamepad", () => {});
});

// Not re-examined because there is nothing to re-examine: `@gc/app`
// depends on `@gc/screens`, never the other way around (its own
// `package.json`; v2/README.md's directory table: `@gc/app` is the layer
// meant to wire screens together). This package's `package.json` does not,
// and architecturally should not, depend on `@gc/app` -- the same
// reasoning `lobby_flow.spec.ts`'s "is reachable from the title and
// returns to it" is skipped under. This is a permanent fact about the
// layering, not a milestone-scoped gap that a newly-reachable dependency
// could close.
describe.skip("online match app routing [@gc/app depends on @gc/screens, not the reverse -- see this file's header]", () => {
  it.skip("routes the lobby's synchronized start into the online match", () => {});
});

// Re-examined: `@gc/render`'s `pitch`/`matchHud` drawing *is* now reachable
// (a declared dependency, and its pure `pitchDrawCommands`/`matchHudCommands`
// path needs no WebGL context -- see `packages/render/src/pitch.spec.ts`'s
// own pattern). What is still missing is the translation this case's whole
// point rests on: a `RenderFrame` built from a *live online frame*, not a
// hand-built fixture (a fixture would only reprove `@gc/render`'s own pure
// path, which `pitch.spec.ts`/`match_hud.spec.ts` already cover in that
// package). That translation is `gc_render::frame_buffer::encode` read back
// via `@gc/wasm`'s `buildRenderFrame` off a real `SimSession`, wired into a
// drawable frame by `match.ts`'s `draw()` -- unbuilt this milestone (same
// gap the two "genuinely hard" control-ownership cases above hit from a
// different angle: `match.ts` only ports its rollback-consumption seam so
// far). Nothing in this file can stand in for that without either touching
// `match.ts` (owned by another agent this batch) or asserting against a
// frame this test invented itself.
describe.skip("online match renderer smoke [needs match.ts's real online RenderFrame wiring, not yet built this milestone]", () => {
  it.skip("draws a live online frame with its combat model and HUD", () => {});
});
